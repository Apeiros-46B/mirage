use std::{ops::{Deref, Index, IndexMut}, slice::{Iter, IterMut}, sync::Arc};

use anyhow::Result;
use ash::{khr, vk};
use winit::dpi::PhysicalSize;

use crate::{
    gpu::{alloc::GPUArena, ctx::{Destroy, Recreate, VulkanCtx}},
    util::array_try_from_fn,
};

pub const FRAMES_IN_FLIGHT: usize = 2;
const SWAPCHAIN_MIN_IMGS: u32 = 3;
const IMG_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

// {{{ image and sync data
pub struct RenderTarget<'a> {
    /// automatically cleared after every frame
    pub arena: &'a mut GPUArena,
    pub img: RenderImage,
    pub cmd: vk::CommandBuffer,
    pub idx: usize,
    pub did_resize: bool,
    pub base_barrier: vk::ImageMemoryBarrier2<'static>,
}

// we have FRAMES_IN_FLIGHT of these
pub struct FrameData {
    pub alloc: GPUArena,
    pub cmd: vk::CommandBuffer,
    pub img_available_sem: vk::Semaphore,
    pub in_flight_fence: vk::Fence,
}

impl Destroy for FrameData {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            ctx.dev.destroy_semaphore(self.img_available_sem, None);
            ctx.dev.destroy_fence(self.in_flight_fence, None);
        }
    }
}

// we have at least SWAPCHAIN_MIN_IMGS of these
pub struct RenderImage {
    pub img: vk::Image,
    pub view: vk::ImageView,
    pub extent: vk::Extent2D,
}

struct SwapchainImage {
    img: vk::Image,
    view: vk::ImageView,
    // signalled by renderer when it finishes rendering to this image
    // waited on by `present` before displaying
    render_sem: vk::Semaphore,
}

impl Destroy for SwapchainImage {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            // don't destroy img.img because it's owned by the swapchain
            ctx.dev.destroy_semaphore(self.render_sem, None);
            ctx.dev.destroy_image_view(self.view, None);
        }
    }
}
// }}}

// {{{ swapchain
pub struct Swapchain {
    loader: khr::swapchain::Device,
    handle: vk::SwapchainKHR,

    pub fmt: vk::Format,
    pub extent: vk::Extent2D,
    imgs: Vec<SwapchainImage>,

    // index of the image currently being rendered into
    // None between present() and the next acq_next_img()
    img_idx: Option<u32>,
}

impl Destroy for Swapchain {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            std::mem::take(&mut self.imgs).destroy(ctx);
            if self.handle != vk::SwapchainKHR::null() {
                self.loader.destroy_swapchain(self.handle, None);
                self.handle = vk::SwapchainKHR::null();
            }
        }
    }
}

impl Swapchain {
    pub fn new(ctx: &VulkanCtx, size: (u32, u32)) -> Result<Self> {
        let loader = khr::swapchain::Device::new(&ctx.inst, &ctx.dev);

        let mut s = Self {
            loader,
            handle: vk::SwapchainKHR::null(),
            fmt: vk::Format::default(),
            extent: vk::Extent2D::default(),
            imgs: Vec::with_capacity(SWAPCHAIN_MIN_IMGS as usize),
            img_idx: None,
        };

        s.recreate(ctx, size)?;
        Ok(s)
    }

    // returns false if size is zero (minimized)
    pub fn recreate(
        &mut self,
        ctx: &VulkanCtx,
        sz: (u32, u32),
    ) -> Result<bool> {
        if sz.0 == 0 || sz.1 == 0 { return Ok(false); }

        unsafe { ctx.dev.device_wait_idle()?; }

        let caps = ctx.surf.get_caps(&ctx.gpu)?;
        let fmt = self.choose_fmt(ctx)?;
        let mode = self.choose_mode(ctx)?;
        let extent = Self::choose_extent(&caps, sz);
        let count = Self::choose_img_count(&caps);

        let ci = vk::SwapchainCreateInfoKHR::default()
            .surface(ctx.surf.handle)
            .min_image_count(count)
            .image_format(fmt.format)
            .image_color_space(fmt.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(
                vk::ImageUsageFlags::COLOR_ATTACHMENT
              | vk::ImageUsageFlags::TRANSFER_DST
            )
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(mode)
            .clipped(true)
            .old_swapchain(self.handle);

        let new_handle = unsafe { self.loader.create_swapchain(&ci, None)? };
        // after this point, safe to tear down state since new_handle is valid

        let new_imgs = Self::create_imgs(ctx, &self.loader, new_handle, fmt.format)
            .map_err(|e| {
                unsafe { self.loader.destroy_swapchain(new_handle, None); }
                e
            })?;

        let old_handle = std::mem::replace(&mut self.handle, new_handle);
        let mut old_imgs = std::mem::replace(&mut self.imgs, new_imgs);

        unsafe {
            old_imgs.destroy(ctx);
            if old_handle != vk::SwapchainKHR::null() {
                self.loader.destroy_swapchain(old_handle, None);
            }
        }

        self.fmt = fmt.format;
        self.extent = extent;
        self.img_idx = None;

        Ok(true)
    }

    // returns None if the swapchain needs recreation (OUT_OF_DATE)
    pub fn acquire(
        &mut self,
        to_signal: vk::Semaphore,
    ) -> Result<Option<RenderImage>> {
        assert!(self.img_idx.is_none(), "acquire called twice without present");

        let result = unsafe {
            self.loader.acquire_next_image(
                self.handle,
                u64::MAX,
                to_signal,
                vk::Fence::null(),
            )
        };

        let idx = match result {
            Ok((idx, _suboptimal)) => idx,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        self.img_idx = Some(idx);
        let img = &self.imgs[idx as usize];
        Ok(Some(RenderImage {
            img: img.img,
            view: img.view,
            extent: self.extent,
        }))
    }

    // returns false if swapchain needs recreation
    pub fn present(&mut self, queue: vk::Queue) -> Result<bool> {
        let idx = self.img_idx.take().expect("present called without acquire");

        let present_info = vk::PresentInfoKHR::default()
            .swapchains(std::slice::from_ref(&self.handle))
            .image_indices(std::slice::from_ref(&idx))
            .wait_semaphores(std::slice::from_ref(&self.imgs[idx as usize].render_sem));

        let needs_recreate = match unsafe { self.loader.queue_present(queue, &present_info) } {
            Ok(suboptimal) => suboptimal,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => true,
            Err(e) => return Err(e.into()),
        };

        Ok(!needs_recreate)
    }

    // barrier template for the current swapchain image
    pub fn base_barrier(&self) -> vk::ImageMemoryBarrier2<'static> {
        let idx = self.img_idx.expect("base_barrier called outside acquire/present");
        vk::ImageMemoryBarrier2::default()
            .image(self.imgs[idx as usize].img)
            .subresource_range(IMG_SUBRES_RANGE)
    }

    // returns the semaphore the renderer must signal when it finishes rendering
    pub fn render_sem(&self) -> vk::Semaphore {
        let idx = self.img_idx.expect("render_sem called outside acquire/present");
        self.imgs[idx as usize].render_sem
    }

    fn choose_fmt(&self, ctx: &VulkanCtx) -> Result<vk::SurfaceFormatKHR> {
        let fmts = ctx.surf.get_fmts(&ctx.gpu)?;
        Ok(fmts.iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_SRGB &&
                f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .copied()
            .unwrap_or(fmts[0])
        )
    }

    fn choose_mode(&self, ctx: &VulkanCtx) -> Result<vk::PresentModeKHR> {
        let modes = ctx.surf.get_modes(&ctx.gpu)?;
        Ok(modes.into_iter()
            .find(|&m| m == vk::PresentModeKHR::FIFO)
            // .find(|&m| m == vk::PresentModeKHR::MAILBOX)
            .unwrap_or(vk::PresentModeKHR::FIFO)
        )
    }

    fn choose_extent(caps: &vk::SurfaceCapabilitiesKHR, size: (u32, u32)) -> vk::Extent2D {
        // current_extent is u32::MAX when the WM/compositor defers to us
        if caps.current_extent.width != u32::MAX {
            return caps.current_extent;
        }
        vk::Extent2D {
            width:  size.0.clamp(caps.min_image_extent.width,  caps.max_image_extent.width),
            height: size.1.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        }
    }

    fn create_imgs(
        ctx: &VulkanCtx,
        loader: &khr::swapchain::Device,
        handle: vk::SwapchainKHR,
        fmt: vk::Format,
    ) -> Result<Vec<SwapchainImage>> {
        let raw = unsafe { loader.get_swapchain_images(handle)? };
        let mut imgs = Vec::with_capacity(raw.len());

        // try block would be nice here
        let result = (|| -> Result<()> {
            for img in raw {
                ctx.debug.set_name(img, c"swap img")?;

                let view = unsafe {
                    ctx.dev.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(img)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(fmt)
                            .subresource_range(IMG_SUBRES_RANGE),
                        None,
                    )?
                };
                let render_sem = unsafe {
                    ctx.dev.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?
                };
                imgs.push(SwapchainImage { img, view, render_sem });
            }
            Ok(())
        })();

        match result {
            Ok(()) => Ok(imgs),
            Err(e) => {
                unsafe { imgs.destroy(ctx); }
                Err(e)
            }
        }
    }

    fn choose_img_count(caps: &vk::SurfaceCapabilitiesKHR) -> u32 {
        let preferred = caps.min_image_count.max(SWAPCHAIN_MIN_IMGS);
        match caps.max_image_count {
            0 => preferred, // no upper limit
            n => preferred.min(n),
        }
    }
}
// }}}

// {{{ frame manager
pub struct FrameManager {
    pub swapchain: Swapchain,

    frame_idx: usize,
    frames: [FrameData; FRAMES_IN_FLIGHT],
    cmd_pool: vk::CommandPool,

    resize_requested: bool,
    inner_surf_sz: (u32, u32),
}

impl Destroy for FrameManager {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            self.swapchain.destroy(ctx);
            self.frames.destroy(ctx);
            ctx.dev.destroy_command_pool(self.cmd_pool, None);
        }
    }
}

impl FrameManager {
    pub fn new(ctx: Arc<VulkanCtx>, sz: PhysicalSize<u32>) -> Result<Self> {
        let swapchain = Swapchain::new(&ctx, sz.into())?;

        let cmd_pool_info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(ctx.gpu.graphics_idx);
        let cmd_pool = unsafe { ctx.dev.create_command_pool(&cmd_pool_info, None)? };

        let cmd_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(cmd_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(FRAMES_IN_FLIGHT as u32);
        let cmd_bufs = unsafe { ctx.dev.allocate_command_buffers(&cmd_info)? };

        for cmd in &cmd_bufs {
            ctx.debug.set_name(*cmd, c"swap cmd")?;
        }

        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let sem_info = vk::SemaphoreCreateInfo::default();

        let frames = array_try_from_fn(|i| -> Result<FrameData> {
            Ok(FrameData {
                cmd: cmd_bufs[i],
                img_available_sem: unsafe {
                    ctx.dev.create_semaphore(&sem_info, None)?
                },
                in_flight_fence: unsafe {
                    ctx.dev.create_fence(&fence_info, None)?
                },
                alloc: GPUArena::new(ctx.clone(), 16 * 1024 * 1024)?,
            })
        })?;

        Ok(Self {
            resize_requested: false,
            inner_surf_sz: sz.into(),
            swapchain,
            frame_idx: 0,
            frames,
            cmd_pool,
        })
    }

    pub fn begin_frame(
        &mut self,
        ctx: &VulkanCtx,
    ) -> Result<Option<RenderTarget<'_>>> {
        let mut did_resize = false;
        let frame = &mut self.frames[self.frame_idx];

        unsafe {
            ctx.dev.wait_for_fences(
                std::slice::from_ref(&frame.in_flight_fence),
                true,
                u64::MAX,
            )?;
        }

        // the GPU is done with the frame, we can clear the arena now
        frame.alloc.clear();

        if self.resize_requested {
            self.swapchain.recreate(ctx, self.inner_surf_sz)?;
            self.resize_requested = false;
            did_resize = true;
        }

        let img = match self.swapchain.acquire(frame.img_available_sem)? {
            Some(t) => t,
            None => {
                self.resize_requested = true;
                return Ok(None);
            },
        };

        unsafe {
            ctx.dev.reset_fences(std::slice::from_ref(&frame.in_flight_fence))?;
            ctx.dev.reset_command_buffer(
                frame.cmd,
                vk::CommandBufferResetFlags::empty(),
            )?;

            let cmd_i = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            ctx.dev.begin_command_buffer(frame.cmd, &cmd_i)?;
        }

        Ok(Some(RenderTarget {
            arena: &mut frame.alloc,
            did_resize,
            idx: self.frame_idx,
            cmd: frame.cmd,
            img,
            base_barrier: self.swapchain.base_barrier(),
        }))
    }

    pub fn submit_frame(&mut self, ctx: &VulkanCtx) -> Result<()> {
        let frame = &self.frames[self.frame_idx];

        unsafe { ctx.dev.end_command_buffer(frame.cmd)? };

        let cmd_info = vk::CommandBufferSubmitInfo::default().command_buffer(frame.cmd);
        let wait_info = vk::SemaphoreSubmitInfo::default()
            .semaphore(frame.img_available_sem)
            .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);
        let sig_info = vk::SemaphoreSubmitInfo::default()
            .semaphore(self.swapchain.render_sem())
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS);

        let submit_info = vk::SubmitInfo2::default()
            .wait_semaphore_infos(std::slice::from_ref(&wait_info))
            .command_buffer_infos(std::slice::from_ref(&cmd_info))
            .signal_semaphore_infos(std::slice::from_ref(&sig_info));

        unsafe {
            ctx.dev.queue_submit2(
                ctx.graphics_qu,
                std::slice::from_ref(&submit_info),
                frame.in_flight_fence
            )?;
        }

        let needs_recreate = !self.swapchain.present(ctx.graphics_qu)?;
        if needs_recreate {
            self.resize_requested = true;
        }

        self.frame_idx = (self.frame_idx + 1) % FRAMES_IN_FLIGHT;
        Ok(())
    }

    pub fn request_resize(&mut self, sz: PhysicalSize<u32>) {
        self.inner_surf_sz = sz.into();
        self.resize_requested = true;
    }

    pub fn is_minimized(&self) -> bool {
        self.inner_surf_sz.0 == 0 || self.inner_surf_sz.1 == 0
    }
}
// }}}

// {{{ utility for storing resources per frame in flight
#[derive(Debug)]
pub struct PerFrame<T> {
    items: [T; FRAMES_IN_FLIGHT],
}

impl<T> PerFrame<T> {
    pub fn try_init<F, E>(mut cb: F) -> Result<Self, E>
    where
        F: FnMut(usize) -> Result<T, E>,
    {
        let items = array_try_from_fn(&mut cb)?;
        Ok(Self { items })
    }

    pub fn init<F>(mut cb: F) -> Self
    where
        F: FnMut(usize) -> T,
    {
        let items = std::array::from_fn(&mut cb);
        Self { items }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> Iter<'_, T> {
        self.items.iter()
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        self.items.iter_mut()
    }
}

impl<T> From<[T; FRAMES_IN_FLIGHT]> for PerFrame<T> {
    fn from(items: [T; FRAMES_IN_FLIGHT]) -> Self {
        Self { items }
    }
}

impl<T> Deref for PerFrame<T> {
    type Target = [T; FRAMES_IN_FLIGHT];

    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl<T> Index<usize> for PerFrame<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.items[index]
    }
}

impl<T> IndexMut<usize> for PerFrame<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.items[index]
    }
}

impl<T: Destroy> Destroy for PerFrame<T> {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        for item in self.iter_mut() {
            unsafe {
                item.destroy(ctx);
            }
        }
    }
}

// TODO: make this more resilient against errors
impl<T: Destroy> Recreate for PerFrame<T> {
    type Item = T;

    unsafe fn recreate<F>(&mut self, ctx: &VulkanCtx, mut f: F) -> Result<()>
    where
        F: FnMut() -> Result<Self::Item>,
    {
        for item in self.iter_mut() {
            unsafe { item.destroy(ctx); }
        }

        let mut new_items = Vec::with_capacity(self.len());

        for _ in 0..self.len() {
            match f() {
                Ok(new_item) => new_items.push(new_item),
                Err(e) => {
                    for mut discarded in new_items {
                        unsafe { discarded.destroy(ctx); }
                    }
                    return Err(e);
                }
            }
        }

        for (item, new_item) in self.iter_mut().zip(new_items.into_iter()) {
            *item = new_item;
        }

        Ok(())
    }
}
// }}}

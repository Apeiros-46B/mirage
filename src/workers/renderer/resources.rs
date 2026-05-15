use std::sync::Arc;

use ash::vk;
use triple_buffer::Output;
use winit::dpi::PhysicalSize;

use crate::{
    gpu::{
        ctx::{Destroy, VulkanCtx},
        desc::DescriptorAllocator,
        img::AllocatedImage,
    },
    workers::{msgs::CameraState, renderer::frames::{PerFrame, RenderTarget}},
};

pub struct Resources {
    pub desc: DescriptorAllocator,

    depth: PerFrame<AllocatedImage>,
    depth_idxs: PerFrame<u32>,
    color: PerFrame<AllocatedImage>,
    color_idxs: PerFrame<u32>,
    visbuf: PerFrame<AllocatedImage>,
    visbuf_idxs: PerFrame<u32>,

    camera_rx: Output<CameraState>,
}

impl Destroy for Resources {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            self.depth.destroy(ctx);
            self.color.destroy(ctx);
            self.visbuf.destroy(ctx);

            self.desc.destroy(ctx);
        }
    }
}

impl Resources {
    pub fn new(
        ctx: Arc<VulkanCtx>,
        scr_sz: PhysicalSize<u32>,
        camera_rx: Output<CameraState>,
    ) -> anyhow::Result<Self> {
        let extent = vk::Extent2D { width: scr_sz.width, height: scr_sz.height };

        let depth = make_depth_targets(&ctx, extent)?;
        let color = make_color_targets(&ctx, extent)?;
        let visbuf = make_visbuf_targets(&ctx, extent)?;

        let mut desc = DescriptorAllocator::new(ctx)?;
        let depth_idxs = desc.alloc_sampled_imgs(&depth).into();
        let color_idxs = desc.alloc_storage_imgs(&color).into();
        let visbuf_idxs = desc.alloc_storage_imgs(&visbuf).into();

        Ok(Self {
            desc,

            depth,
            depth_idxs,
            color,
            color_idxs,
            visbuf,
            visbuf_idxs,

            camera_rx,
        })
    }

    pub unsafe fn resize(
        &mut self,
        ctx: &VulkanCtx,
        extent: vk::Extent2D,
    ) -> anyhow::Result<()> {
        unsafe {
            self.depth.destroy(ctx);
            self.depth = make_depth_targets(ctx, extent)?;
            self.desc.update_sampled_imgs(self.depth_idxs[0], &self.depth);

            self.color.destroy(ctx);
            self.color = make_color_targets(ctx, extent)?;
            self.desc.update_storage_imgs(self.color_idxs[0], &self.color);

            self.visbuf.destroy(ctx);
            self.visbuf = make_visbuf_targets(ctx, extent)?;
            self.desc.update_storage_imgs(self.visbuf_idxs[0], &self.visbuf);
        }

        Ok(())
    }

    pub fn view(&mut self, target: &mut RenderTarget<'_>) -> ResourceView<'_> {
        let mut camera_buf = target.tmp_arena.alloc_of::<CameraState>();
        unsafe { camera_buf.write(self.camera_rx.read()); }

        ResourceView {
            camera_bda: camera_buf.bda,

            depth: &self.depth[target.idx],
            depth_idx: self.depth_idxs[target.idx],
            color: &self.color[target.idx],
            color_idx: self.color_idxs[target.idx],
            visbuf: &self.visbuf[target.idx],
            visbuf_idx: self.visbuf_idxs[target.idx],
        }
    }
}

pub struct ResourceView<'a> {
    pub camera_bda: u64, // BDA pointer to CameraState

    pub depth: &'a AllocatedImage,
    pub depth_idx: u32,
    pub visbuf: &'a AllocatedImage,
    pub visbuf_idx: u32,
    pub color: &'a AllocatedImage,
    pub color_idx: u32,
}

impl<'a, 'b> ResourceView<'a> {
    pub fn depth_barrier(&self) -> vk::ImageMemoryBarrier2<'b> {
        vk::ImageMemoryBarrier2::default()
            .image(self.depth.img)
            .subresource_range(DEPTH_SUBRES_RANGE)
    }

    pub fn color_barrier(&self) -> vk::ImageMemoryBarrier2<'b> {
        vk::ImageMemoryBarrier2::default()
            .image(self.color.img)
            .subresource_range(COLOR_SUBRES_RANGE)
    }

    pub fn visbuf_barrier(&self) -> vk::ImageMemoryBarrier2<'b> {
        vk::ImageMemoryBarrier2::default()
            .image(self.visbuf.img)
            .subresource_range(COLOR_SUBRES_RANGE)
    }
}

const DEPTH_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::DEPTH,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

fn make_depth_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> anyhow::Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::D32_SFLOAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            DEPTH_SUBRES_RANGE,
            c"depth target",
        )
    })
}

const COLOR_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

fn make_color_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> anyhow::Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::R8G8B8A8_UNORM,
            vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC,
            COLOR_SUBRES_RANGE,
            c"color target",
        )
    })
}

fn make_visbuf_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> anyhow::Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::R64_UINT,
            vk::ImageUsageFlags::STORAGE,
            COLOR_SUBRES_RANGE,
            c"visbuf target",
        )
    })
}

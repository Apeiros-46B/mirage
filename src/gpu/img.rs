use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

use crate::gpu::ctx::{Destroy, VulkanCtx};

pub struct AllocatedImage {
    pub img: vk::Image,
    pub view: vk::ImageView,
    pub mem: Option<Allocation>,
    pub fmt: vk::Format,
    pub extent: vk::Extent3D,
}

impl Destroy for AllocatedImage {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            ctx.dev.destroy_image_view(self.view, None);
            ctx.dev.destroy_image(self.img, None);
        }
        if let Some(mem) = self.mem.take() {
            ctx.alloc.lock().unwrap().free(mem).unwrap();
        }
    }
}

impl AllocatedImage {
    pub fn new(
        ctx: &VulkanCtx,
        extent: vk::Extent3D,
        fmt: vk::Format,
        usage: vk::ImageUsageFlags,
        subres_range: vk::ImageSubresourceRange,
        name: &str,
    ) -> anyhow::Result<Self> {
        let image_ci = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(extent)
            .format(fmt)
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let img = unsafe { ctx.dev.create_image(&image_ci, None)? };

        let requirements = unsafe { ctx.dev.get_image_memory_requirements(img) };
        let mem = ctx.alloc.lock().unwrap().allocate(&AllocationCreateDesc {
            name,
            requirements,
            location: gpu_allocator::MemoryLocation::GpuOnly,
            linear: false,
            allocation_scheme: AllocationScheme::DedicatedImage(img),
        })?;

        unsafe { ctx.dev.bind_image_memory(img, mem.memory(), mem.offset())? };

        let view_ci = vk::ImageViewCreateInfo::default()
            .image(img)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(fmt)
            .subresource_range(subres_range);

        let view = unsafe { ctx.dev.create_image_view(&view_ci, None)? };

        Ok(Self {
            img,
            view,
            mem: Some(mem),
            fmt,
            extent
        })
    }
}

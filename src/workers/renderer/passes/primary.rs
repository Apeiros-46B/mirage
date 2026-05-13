use anyhow::bail;
use ash::vk;

use crate::{
    gpu::{
        ctx::{Destroy, VulkanCtx},
        img::AllocatedImage,
        sync::ImageBarrierExt,
        task::Task,
    },
    workers::renderer::{FrameCtx, frames::PerFrame},
};

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/primary-pass.spv"));

const COLOR_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

pub struct PrimaryPass {
    depth_sampler: vk::Sampler,

    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
}

impl Destroy for PrimaryPass {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            ctx.dev.destroy_pipeline_layout(self.pipeline_layout, None);
            ctx.dev.destroy_pipeline(self.pipeline, None);
            ctx.dev.destroy_sampler(self.depth_sampler, None);
        }
    }
}

impl PrimaryPass {
    pub fn new(ctx: &VulkanCtx) -> anyhow::Result<Self> {
        let code = ash::util::read_spv(&mut std::io::Cursor::new(SPV))?;
        let shader_info = vk::ShaderModuleCreateInfo::default().code(&code);
        let module = unsafe { ctx.dev.create_shader_module(&shader_info, None)? };

        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
        let sampler = unsafe { ctx.dev.create_sampler(&sampler_info, None)? };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX)
            .offset(0)
            .size(16); // camera ptr (8 bytes) + world ptr (8 bytes)

        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(std::slice::from_ref(&push_range));

        let layout = unsafe { ctx.dev.create_pipeline_layout(&layout_info, None)? };

        let compute_stage_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(module)
            .name(c"main");

        let pipeline_info = vk::ComputePipelineCreateInfo::default()
            .layout(layout)
            .stage(compute_stage_info);

        let pipeline = unsafe {
            let res = ctx.dev.create_compute_pipelines(
                vk::PipelineCache::null(),
                &[pipeline_info],
                None,
            );
            match res {
                Ok(pipelines) => pipelines[0],
                Err(_) => bail!("couldn't create pipelines")
            }
        };

        Ok(Self {
            depth_sampler: sampler,
            pipeline,
            pipeline_layout: layout,
        })
    }
}

impl Task<FrameCtx<'_>> for PrimaryPass {
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &mut FrameCtx,
    ) -> anyhow::Result<()> {
        // TODO
        Ok(())
    }
}

pub fn make_color_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> anyhow::Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::R8G8B8A8_SRGB,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            COLOR_SUBRES_RANGE,
            "primary_pass_out",
        )
    })
}

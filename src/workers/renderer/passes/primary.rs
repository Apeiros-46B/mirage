use anyhow::bail;
use ash::vk;
use bytemuck::{Pod, Zeroable};

use crate::{
    gpu::{
        ctx::{Destroy, VulkanCtx},
        desc::DescriptorBindInfo,
        img::AllocatedImage,
        sync::ImageBarrierExt,
        task::Task,
    },
    workers::renderer::{
        FrameCtx,
        frames::PerFrame,
    },
};
use super::depth::DEPTH_SUBRES_RANGE;

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/primary-pass.spv"));

pub const COLOR_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

pub struct PrimaryPass {
    desc_set: vk::DescriptorSet,
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct PushConstants {
    camera_bda: u64,
    world_bda: u64,
    depth_idx: u32,
    color_storage_idx: u32,
}

impl Destroy for PrimaryPass {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            ctx.dev.destroy_pipeline_layout(self.pipeline_layout, None);
            ctx.dev.destroy_pipeline(self.pipeline, None);
        }
    }
}

impl PrimaryPass {
    pub fn new(
        ctx: &VulkanCtx,
        desc_bind_info: DescriptorBindInfo,
    ) -> anyhow::Result<Self> {
        let code = ash::util::read_spv(&mut std::io::Cursor::new(SPV))?;
        let shader_info = vk::ShaderModuleCreateInfo::default().code(&code);
        let module = unsafe { ctx.dev.create_shader_module(&shader_info, None)? };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(std::mem::size_of::<PushConstants>() as u32);

        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(std::slice::from_ref(&desc_bind_info.layout))
            .push_constant_ranges(std::slice::from_ref(&push_range));

        let layout = unsafe { ctx.dev.create_pipeline_layout(&layout_info, None)? };
        ctx.debug.set_name(layout, c"primary pipe layout")?;

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
        ctx.debug.set_name(pipeline, c"primary pipe")?;

        unsafe {
            ctx.dev.destroy_shader_module(module, None);
        }

        Ok(Self {
            desc_set: desc_bind_info.set,
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
        let cmd = frame.in_flight.cmd;

        unsafe {
            ctx.dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::COMPUTE, self.pipeline);
            ctx.dev.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                std::slice::from_ref(&self.desc_set),
                &[],
            );
            ctx.dev.cmd_push_constants(
                cmd,
                self.pipeline_layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytemuck::bytes_of(&PushConstants {
                    camera_bda: frame.camera_bda,
                    world_bda: 0,
                    depth_idx: frame.depth_idx,
                    color_storage_idx: frame.color_storage_idx,
                }),
            );

            let group_sz = 16;
            let extent = frame.color.extent;
            ctx.dev.cmd_dispatch(
                cmd,
                (extent.width + group_sz - 1) / group_sz,
                (extent.height + group_sz - 1) / group_sz,
                1,
            );
        }

        color_barrier = color_barrier.next_phase(
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::PipelineStageFlags2::BLIT,
            vk::AccessFlags2::TRANSFER_READ,
        );

        let mut swap_barrier = frame.in_flight.base_barrier
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags2::BLIT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE);

        unsafe {
            ctx.dev.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default()
                    .image_memory_barriers(&[swap_barrier, color_barrier]),
            );

            let blit_ofs = [
                vk::Offset3D::default()
                    .x(0)
                    .y(0)
                    .z(0),
                vk::Offset3D::default()
                    .x(frame.color.extent.width as i32)
                    .y(frame.color.extent.height as i32)
                    .z(1),
            ];
            let blit_subres = vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .mip_level(0)
                .base_array_layer(0)
                .layer_count(1);
            ctx.dev.cmd_blit_image(
                cmd,
                frame.color.img,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                frame.in_flight.img.img,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[
                    vk::ImageBlit::default()
                        .src_offsets(blit_ofs)
                        .dst_offsets(blit_ofs)
                        .src_subresource(blit_subres)
                        .dst_subresource(blit_subres)
                ],
                vk::Filter::NEAREST,
            );
        }

        swap_barrier = swap_barrier.next_phase(
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::COLOR_ATTACHMENT_READ
            | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        );
        unsafe { ctx.dev.cmd_pipeline_barrier2(cmd, &swap_barrier.dep_info()) };

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
            vk::Format::R8G8B8A8_UNORM,
            vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC,
            COLOR_SUBRES_RANGE,
            "primary_pass_out",
        )
    })
}

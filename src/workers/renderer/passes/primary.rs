use anyhow::{Result, bail};
use ash::vk;
use bytemuck::{Pod, Zeroable};

use crate::{
    gpu::{
        ctx::{Destroy, VulkanCtx},
        desc::DescriptorBindInfo,
        task::Task,
    },
    workers::renderer::FrameCtx,
};

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/primary-pass.spv"));

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct PushConstants {
    camera_bda: u64, // CameraState*
    chunk_metadata_bda: u64, // ChunkMetadata*
    chunk_roots_bda: u64, // Node**
    chunk_id_idx: u32,
    visbuf_idx: u32,
    extent_x: u32,
    extent_y: u32,
}

pub struct PrimaryPass {
    desc_set: vk::DescriptorSet,
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
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
        desc_bind_info: &DescriptorBindInfo,
    ) -> Result<Self> {
        // TODO: reloadable shader
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
    ) -> Result<()> {
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
                    camera_bda: frame.resources.camera_bda,
                    chunk_metadata_bda: frame.resources.chunk_metadata_bda,
                    chunk_roots_bda: frame.resources.chunk_roots_bda,
                    chunk_id_idx: frame.resources.chunk_id_idx,
                    visbuf_idx: frame.resources.visbuf_idx,
                    extent_x: frame.resources.color.extent.width,
                    extent_y: frame.resources.color.extent.height,
                }),
            );

            let group_sz = 16;
            let extent = frame.resources.color.extent;
            ctx.dev.cmd_dispatch(
                cmd,
                (extent.width + group_sz - 1) / group_sz,
                (extent.height + group_sz - 1) / group_sz,
                1,
            );
        }

        Ok(())
    }
}

// TODO: get rid of boilerplate here
use anyhow::Result;
use ash::vk;
use bytemuck::{Pod, Zeroable};

use crate::{
    gpu::{
        ctx::{Destroy, VulkanCtx},
        task::Task,
        desc::DescriptorBindInfo,
    },
    workers::renderer::FrameCtx,
};

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/post-pass.spv"));

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct PushConstants {
    color_idx: u32,
    encode_srgb: u32, // bool
}

pub struct PostPass {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    desc_set: vk::DescriptorSet,
    encode_srgb: bool,
}

impl Destroy for PostPass {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            ctx.dev.destroy_pipeline_layout(self.layout, None);
            ctx.dev.destroy_pipeline(self.pipeline, None);
        }
    }
}

impl PostPass {
    pub fn new(ctx: &VulkanCtx, bind_info: &DescriptorBindInfo, fmt: vk::Format) -> Result<Self> {
        let code = ash::util::read_spv(&mut std::io::Cursor::new(SPV))?;
        let shader_info = vk::ShaderModuleCreateInfo::default().code(&code);
        let module = unsafe { ctx.dev.create_shader_module(&shader_info, None)? };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(std::mem::size_of::<PushConstants>() as u32);

        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(std::slice::from_ref(&push_range))
            .set_layouts(std::slice::from_ref(&bind_info.layout));

        let layout = unsafe { ctx.dev.create_pipeline_layout(&layout_info, None)? };

        let vert_stage_ci = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(module)
            .name(c"vsh");
        let frag_stage_ci = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(module)
            .name(c"fsh");
        let stages = [vert_stage_ci, frag_stage_ci];

        // empty, use gl_VertexIndex
        let vert_input_ci = vk::PipelineVertexInputStateCreateInfo::default();

        let input_asm_ci = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

        let viewport_state_ci = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let dyn_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dyn_info = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&dyn_states);

        let rast_ci = vk::PipelineRasterizationStateCreateInfo::default()
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0);

        let multismp_ci = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let depth_sten_ci = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false);

        let col_blend_atmt_state = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false);
        let col_blend_ci = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(std::slice::from_ref(&col_blend_atmt_state));
        let fmts = [fmt];
        let mut rendering_ci = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&fmts);

        let pipeline_ci = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vert_input_ci)
            .input_assembly_state(&input_asm_ci)
            .viewport_state(&viewport_state_ci)
            .rasterization_state(&rast_ci)
            .multisample_state(&multismp_ci)
            .depth_stencil_state(&depth_sten_ci)
            .color_blend_state(&col_blend_ci)
            .dynamic_state(&dyn_info)
            .layout(layout)
            .push_next(&mut rendering_ci);

        let pipeline = unsafe {
            ctx.dev.create_graphics_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&pipeline_ci),
                None,
            ).map_err(|(_, err)| err)?[0]
        };

        unsafe { ctx.dev.destroy_shader_module(module, None); }

        Ok(Self {
            pipeline,
            layout,
            desc_set: bind_info.set,
            encode_srgb: !super::is_srgb_format(fmt),
        })
    }
}

impl Task<FrameCtx<'_>> for PostPass {
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &mut FrameCtx,
    ) -> Result<()> {
        let cmd = frame.in_flight.cmd;

        let col_atmt = vk::RenderingAttachmentInfo::default()
            .image_view(frame.in_flight.img.view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::STORE);
        let col_atmts = [col_atmt];

        let rendering_info = vk::RenderingInfo::default()
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: frame.in_flight.img.extent,
            })
            .layer_count(1)
            .color_attachments(&col_atmts);

        unsafe {
            ctx.dev.cmd_begin_rendering(cmd, &rendering_info);
            ctx.dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);

            ctx.dev.cmd_bind_descriptor_sets(
                cmd, vk::PipelineBindPoint::GRAPHICS, self.layout, 0,
                std::slice::from_ref(&self.desc_set), &[],
            );
            ctx.dev.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::bytes_of(&PushConstants {
                    color_idx: frame.resources.color_idx,
                    encode_srgb: u32::from(self.encode_srgb),
                }),
            );

            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: frame.in_flight.img.extent.width as f32,
                height: frame.in_flight.img.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            ctx.dev.cmd_set_viewport(cmd, 0, std::slice::from_ref(&viewport));

            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: frame.in_flight.img.extent,
            };
            ctx.dev.cmd_set_scissor(cmd, 0, std::slice::from_ref(&scissor));

            // single full-screen tri
            ctx.dev.cmd_draw(cmd, 3, 1, 0, 0);
            ctx.dev.cmd_end_rendering(cmd);
        }

        Ok(())
    }
}

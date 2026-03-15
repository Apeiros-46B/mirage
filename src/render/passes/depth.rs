use ash::vk;

use crate::{gpu::{ctx::VulkanCtx, img::AllocatedImage, pass::Task}, render::passes::FrameCtx};

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/depth_prepass.spv"));

const DEPTH_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::DEPTH,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

pub struct DepthPrepass {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    depth_target: AllocatedImage,
    world_bda: u64,
}

fn make_depth_target(ctx: &VulkanCtx, extent: vk::Extent2D) -> anyhow::Result<AllocatedImage> {
    AllocatedImage::new(
        ctx,
        extent.into(),
        vk::Format::D32_SFLOAT,
        vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
        DEPTH_SUBRES_RANGE,
        "depth_prepass_out",
    )
}

impl DepthPrepass {
    // TODO: review
    pub fn new(ctx: &VulkanCtx, extent: vk::Extent2D) -> anyhow::Result<Self> {
        let code = ash::util::read_spv(&mut std::io::Cursor::new(SPV))?;
        let shader_info = vk::ShaderModuleCreateInfo::default().code(&code);
        let module = unsafe { ctx.dev.create_shader_module(&shader_info, None)? };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX)
            .offset(0)
            .size(72); // mat4 (64 bytes) + u64 world_bda (8 bytes)

        let layout_ci = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(std::slice::from_ref(&push_range));

        let layout = unsafe { ctx.dev.create_pipeline_layout(&layout_ci, None)? };

        let vert_stage_ci = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(module)
            .name(c"vertMain");
        // need a dummy fragment shader on some platforms
        // if one is not provided, app crashes on windows
        let frag_stage_ci = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(module)
            .name(c"fragMain");
        let stages = [vert_stage_ci, frag_stage_ci];

        // empty, use gl_VertexIndex
        let vert_input_ci = vk::PipelineVertexInputStateCreateInfo::default();

        let input_asm_ci = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

        // dynamic viewport/scissor to avoid resizing
        let viewport_state_ci = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let rast_ci = vk::PipelineRasterizationStateCreateInfo::default()
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0);

        let multismp_ci = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let depth_sten_ci = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS);

        let col_blend_ci = vk::PipelineColorBlendStateCreateInfo::default();

        let dyn_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dyn_state_ci = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&dyn_states);

        let mut rendering_ci = vk::PipelineRenderingCreateInfo::default()
            .depth_attachment_format(vk::Format::D32_SFLOAT);

        let pipeline_ci = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vert_input_ci)
            .input_assembly_state(&input_asm_ci)
            .viewport_state(&viewport_state_ci)
            .rasterization_state(&rast_ci)
            .multisample_state(&multismp_ci)
            .depth_stencil_state(&depth_sten_ci)
            .color_blend_state(&col_blend_ci)
            .dynamic_state(&dyn_state_ci)
            .layout(layout)
            .push_next(&mut rendering_ci);

        let pipeline = unsafe {
            ctx.dev.create_graphics_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&pipeline_ci),
                None,
            ).map_err(|(_, err)| err)?[0]
        };

        // don't need it anymore
        unsafe { ctx.dev.destroy_shader_module(module, None); }

        let depth_target = make_depth_target(ctx, extent)?;

        Ok(Self {
            pipeline,
            layout,
            depth_target,
            world_bda: 0,
        })
    }
}

impl Task<FrameCtx> for DepthPrepass {
    const NAME: &'static str = "DepthPrepass";

    fn resize(&mut self, ctx: &VulkanCtx, extent: vk::Extent2D) -> anyhow::Result<()> {
        unsafe {
            self.depth_target.destroy(ctx);
        }
        self.depth_target = make_depth_target(ctx, extent)?;
        Ok(())
    }

    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &FrameCtx,
        cmd: vk::CommandBuffer,
    ) -> anyhow::Result<()> {
        let depth_barrier = vk::ImageMemoryBarrier2::default()
            .image(self.depth_target.img)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0, level_count: 1,
                base_array_layer: 0, layer_count: 1,
            })
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .src_stage_mask(vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS)
            .dst_access_mask(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE);

        let dep = vk::DependencyInfo::default()
            .image_memory_barriers(std::slice::from_ref(&depth_barrier));

        unsafe { ctx.dev.cmd_pipeline_barrier2(cmd, &dep) };

        // render depth only, cleared to 1.0
        let depth_attachment = vk::RenderingAttachmentInfo::default()
            .image_view(self.depth_target.view)
            .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue { depth: 1.0, stencil: 0 },
            });

        let rendering_info = vk::RenderingInfo::default()
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: frame.target.extent,
            })
            .layer_count(1)
            .depth_attachment(&depth_attachment);

        unsafe {
            ctx.dev.cmd_begin_rendering(cmd, &rendering_info);
            ctx.dev.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);

            // hardcoded single chunk, vsh calculates cube corners using gl_VertexIndex
            //
            // later when chunks are dynamically loaded and culled, direct draw is replaced by
            // - binding an indirect buffer (filled by the compute culler)
            // - binding a buffer of `ChunkAABB` data (containing coords, chunk ID, and maybe LOD)
            // - `ctx.dev.cmd_draw_indirect(cmd, indirect_buf, ...)`

            ctx.dev.cmd_draw(cmd, 36, 1, 0, 0);
            ctx.dev.cmd_end_rendering(cmd);
        }

        let compute_read_barrier = vk::ImageMemoryBarrier2::default()
            .image(self.depth_target.img)
            .subresource_range(DEPTH_SUBRES_RANGE)
            .old_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_stage_mask(vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS)
            .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .src_access_mask(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ);

        let dep = vk::DependencyInfo::default()
            .image_memory_barriers(std::slice::from_ref(&compute_read_barrier));

        unsafe { ctx.dev.cmd_pipeline_barrier2(cmd, &dep); }

        Ok(())
    }
}

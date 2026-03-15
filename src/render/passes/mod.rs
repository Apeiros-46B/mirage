pub mod depth;

use ash::vk;

use crate::{gpu::{ctx::VulkanCtx, pass::Task}, render::present::RenderTarget};

pub struct FrameCtx {
    pub frame_idx: usize,
    pub time: f64, // time since engine start
    pub dt: f32, // time since last frame
    pub target: RenderTarget,
    pub swapchain_base_barrier: vk::ImageMemoryBarrier2<'static>,
}

pub struct ClearPass;

impl Task<FrameCtx> for ClearPass {
    const NAME: &'static str = "ClearPass";

    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &FrameCtx,
        cmd: vk::CommandBuffer,
    ) -> anyhow::Result<()> {
        unsafe {
            let to_color_barrier = frame.swapchain_base_barrier
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::empty())
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);

            let dep_info_color = vk::DependencyInfo::default()
                .image_memory_barriers(std::slice::from_ref(&to_color_barrier));
            ctx.dev.cmd_pipeline_barrier2(cmd, &dep_info_color);

            let color_atmt = vk::RenderingAttachmentInfo::default()
                .image_view(frame.target.view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0.0, 0.0, 0.0, 1.0],
                    },
                });

            let rendering_info = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: frame.target.extent,
                })
                .layer_count(1)
                .color_attachments(std::slice::from_ref(&color_atmt));

            ctx.dev.cmd_begin_rendering(cmd, &rendering_info);
            ctx.dev.cmd_end_rendering(cmd);
        }

        Ok(())
    }
}

pub struct PresentPass;

impl Task<FrameCtx> for PresentPass {
    const NAME: &'static str = "PresentPass";

    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &FrameCtx,
        cmd: vk::CommandBuffer,
    ) -> anyhow::Result<()> {
        unsafe {
            let to_present_barrier = frame.swapchain_base_barrier
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags2::empty());

            let dep_info_present = vk::DependencyInfo::default()
                .image_memory_barriers(std::slice::from_ref(&to_present_barrier));

            ctx.dev.cmd_pipeline_barrier2(cmd, &dep_info_present);
        }

        Ok(())
    }
}

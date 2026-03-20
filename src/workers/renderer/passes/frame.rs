use ash::vk;

use crate::{
    gpu::{ctx::{Destroy, VulkanCtx}, sync::ImageBarrierExt, task::Task},
    workers::renderer::FrameCtx,
};

pub struct ClearPass;

impl Destroy for ClearPass {
    unsafe fn destroy(&mut self, _ctx: &VulkanCtx) {}
}

impl Task<FrameCtx<'_>> for ClearPass {
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &mut FrameCtx,
    ) -> anyhow::Result<()> {
        let cmd = frame.in_flight.cmd;

        unsafe {
            let barrier = frame.in_flight.base_barrier
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::empty())
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);

            ctx.dev.cmd_pipeline_barrier2(cmd, &barrier.dep_info());

            let color_atmt = vk::RenderingAttachmentInfo::default()
                .image_view(frame.in_flight.img.view)
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
                    extent: frame.in_flight.img.extent,
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

impl Destroy for PresentPass {
    unsafe fn destroy(&mut self, _ctx: &VulkanCtx) {}
}

impl Task<FrameCtx<'_>> for PresentPass {
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &mut FrameCtx,
    ) -> anyhow::Result<()> {
        unsafe {
            let barrier = frame.in_flight.base_barrier
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags2::empty());

            ctx.dev.cmd_pipeline_barrier2(frame.in_flight.cmd, &barrier.dep_info());
        }

        Ok(())
    }
}

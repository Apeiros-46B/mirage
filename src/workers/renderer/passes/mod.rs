use crate::{gpu::{ctx::{Destroy, VulkanCtx}, sync::ImageBarrierExt, task::Task}, sync::worker::MessageHandler, workers::msgs::EguiUpdate};

use super::{FrameCtx, resources::Resources};
use ash::vk;
use blit::BlitPass;
use depth::DepthPrepass;
use egui::EguiPass;
use frame::ClearPass;
use primary::PrimaryPass;

pub mod blit;
pub mod depth;
pub mod egui;
pub mod frame;
pub mod primary;

pub struct Passes {
    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    primary_pass: PrimaryPass,
    blit_pass: BlitPass,
    egui_pass: EguiPass,
}

impl Destroy for Passes {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            self.egui_pass.destroy(ctx);
            self.blit_pass.destroy(ctx);
            self.primary_pass.destroy(ctx);
            self.depth_prepass.destroy(ctx);
            self.clear_pass.destroy(ctx);
        }
    }
}

impl Passes {
    pub fn new(
        ctx: &VulkanCtx,
        resources: &Resources,
        swap_fmt: vk::Format,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            clear_pass: ClearPass,
            depth_prepass: DepthPrepass::new(&ctx)?,
            primary_pass: PrimaryPass::new(&ctx, resources.desc.get_bind_info())?,
            blit_pass: BlitPass,
            egui_pass: EguiPass::new(&ctx, swap_fmt)?,
        })
    }
}

impl MessageHandler<EguiUpdate> for Passes {
    fn handle(&mut self, msg: EguiUpdate) {
        self.egui_pass.handle(msg);
    }
}

impl Task<FrameCtx<'_>> for Passes {
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &mut FrameCtx<'_>,
    ) -> anyhow::Result<()> {
        let cmd = frame.in_flight.cmd;

        let mut swap_barrier = frame.in_flight.base_barrier
            .begin(
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            )
            // don't use TOP_OF_PIPE default for swapchain images
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);
        unsafe {
            ctx.barrier(cmd, [swap_barrier]);
            self.clear_pass.execute(&ctx, frame)?;
        }

        let mut depth_barrier = frame.resources.depth_barrier()
            .begin(
                vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS,
                vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE,
            );
        unsafe {
            ctx.barrier(cmd, [depth_barrier]);
            self.depth_prepass.execute(&ctx, frame)?;
        }

        let mut color_barrier = frame.resources.color_barrier()
            .begin(
                vk::ImageLayout::GENERAL,
                vk::PipelineStageFlags2::COMPUTE_SHADER,
                vk::AccessFlags2::SHADER_WRITE,
            );
        depth_barrier.transition(
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags2::COMPUTE_SHADER,
            vk::AccessFlags2::SHADER_SAMPLED_READ,
        );
        unsafe {
            ctx.barrier(cmd, [depth_barrier, color_barrier]);
            self.primary_pass.execute(&ctx, frame)?;
        }

        color_barrier.transition(
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::PipelineStageFlags2::BLIT,
            vk::AccessFlags2::TRANSFER_READ,
        );
        swap_barrier.transition(
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::PipelineStageFlags2::BLIT,
            vk::AccessFlags2::TRANSFER_WRITE,
        );
        unsafe {
            ctx.barrier(cmd, [color_barrier, swap_barrier]);
            self.blit_pass.execute(&ctx, frame)?;
        }

        swap_barrier.transition(
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::COLOR_ATTACHMENT_READ
            | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        );
        unsafe {
            ctx.barrier(cmd, [swap_barrier]);
            self.egui_pass.execute(&ctx, frame)?;
        }

        swap_barrier.transition(
            vk::ImageLayout::PRESENT_SRC_KHR,
            vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
            vk::AccessFlags2::NONE,
        );
        unsafe {
            ctx.barrier(cmd, [swap_barrier]);
        }

        Ok(())
    }
}

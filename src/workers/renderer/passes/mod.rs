use anyhow::Result;
use ash::vk;

use crate::{
    gpu::{ctx::{Destroy, VulkanCtx}, sync::ImageBarrierExt, task::Task},
    sync::worker::MessageHandler,
    workers::msgs::EguiUpdate,
};
use super::{FrameCtx, resources::Resources};

use depth::DepthPrepass;
use egui::EguiPass;
use frame::ClearPass;
use post::PostPass;
use primary::PrimaryPass;
use vis::VisPass;

pub mod depth;
pub mod egui;
pub mod frame;
pub mod post;
pub mod primary;

// may replace with collect/shade passes later. probably need to be able to
// swap collect/shade both for just vis, for per-pixel debug visualizations
pub mod vis;

pub struct Passes {
    swap_fmt: vk::Format,
    clear_pass: ClearPass,
    depth_prepass: DepthPrepass,
    primary_pass: PrimaryPass,
    vis_pass: VisPass,
    post_pass: PostPass,
    egui_pass: EguiPass,
}

impl Destroy for Passes {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            self.egui_pass.destroy(ctx);
            self.post_pass.destroy(ctx);
            self.vis_pass.destroy(ctx);
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
    ) -> Result<Self> {
        let bind_info = resources.desc.get_bind_info();
        Ok(Self {
            swap_fmt,
            clear_pass: ClearPass,
            depth_prepass: DepthPrepass::new(&ctx)?,
            primary_pass: PrimaryPass::new(&ctx, &bind_info)?,
            vis_pass: VisPass::new(&ctx, &bind_info)?,
            post_pass: PostPass::new(ctx, &bind_info, swap_fmt)?,
            egui_pass: EguiPass::new(&ctx, swap_fmt)?,
        })
    }

    /// called after swapchain recreation with all previous GPU work complete
    pub unsafe fn set_fmt(
        &mut self,
        ctx: &VulkanCtx,
        resources: &Resources,
        fmt: vk::Format,
    ) -> Result<()> {
        if self.swap_fmt == fmt { return Ok(()); }

        let mut post = PostPass::new(ctx, &resources.desc.get_bind_info(), fmt)?;

        if let Err(err) = unsafe { self.egui_pass.set_format(ctx, fmt) } {
            unsafe { post.destroy(ctx); }
            return Err(err);
        }
        unsafe { self.post_pass.destroy(ctx); }

        self.post_pass = post;
        self.swap_fmt = fmt;
        Ok(())
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
    ) -> Result<()> {
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

        let depth_barrier = frame.resources.depth_barrier()
            .begin(
                vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS,
                vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE,
            );
        let mut chunk_id_barrier = frame.resources.chunk_id_barrier()
            .begin(
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            );
        unsafe {
            ctx.barrier(cmd, [depth_barrier, chunk_id_barrier]);
            self.depth_prepass.execute(&ctx, frame)?;
        }

        let mut visbuf_barrier = frame.resources.visbuf_barrier()
            .begin(
                vk::ImageLayout::GENERAL,
                vk::PipelineStageFlags2::COMPUTE_SHADER,
                vk::AccessFlags2::SHADER_WRITE,
            );
        chunk_id_barrier.transition(
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags2::COMPUTE_SHADER,
            vk::AccessFlags2::SHADER_SAMPLED_READ,
        );
        unsafe {
            ctx.barrier(cmd, [visbuf_barrier, chunk_id_barrier]);
            self.primary_pass.execute(&ctx, frame)?;
        }

        let mut color_barrier = frame.resources.color_barrier()
            .begin(
                vk::ImageLayout::GENERAL,
                vk::PipelineStageFlags2::COMPUTE_SHADER,
                vk::AccessFlags2::SHADER_WRITE,
            );
        visbuf_barrier.transition(
            vk::ImageLayout::GENERAL,
            vk::PipelineStageFlags2::COMPUTE_SHADER,
            vk::AccessFlags2::SHADER_READ,
        );
        unsafe {
            ctx.barrier(cmd, [
                visbuf_barrier,
                color_barrier,
                frame.resources.pt_accum_barrier(),
            ]);
            self.vis_pass.execute(&ctx, frame)?;
        }

        color_barrier.transition(
            vk::ImageLayout::GENERAL,
            vk::PipelineStageFlags2::FRAGMENT_SHADER,
            vk::AccessFlags2::SHADER_STORAGE_READ,
        );
        swap_barrier.transition(
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
        );
        unsafe {
            ctx.barrier(cmd, [color_barrier, swap_barrier]);
            self.post_pass.execute(&ctx, frame)?;
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

// only sRGB attachment formats apply the encoding in hardware
fn is_srgb_format(fmt: vk::Format) -> bool {
    matches!(fmt,
        vk::Format::R8_SRGB | vk::Format::R8G8_SRGB |
        vk::Format::R8G8B8_SRGB | vk::Format::B8G8R8_SRGB |
        vk::Format::R8G8B8A8_SRGB | vk::Format::B8G8R8A8_SRGB |
        vk::Format::A8B8G8R8_SRGB_PACK32
    )
}

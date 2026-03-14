mod present;

use std::time::Instant;

use ash::vk;
use rtrb::Consumer;

use crate::{
    gpu::ctx::VulkanCtx,
    render::present::Presenter,
    sync::{OneshotRx, RenderInit, RenderMsg},
};

pub struct Renderer {
    rx: Consumer<RenderMsg>,
    presenter: Presenter,
    ctx: VulkanCtx,
    // TODO: store this as an Arc since other threads also need it
    // it should be created on main thread and passed to us via RenderInit
    // then we also don't need to do the unsafe impl Send+Sync for raw handles
}

impl Renderer {
    fn new(init: RenderInit, rx: Consumer<RenderMsg>) -> anyhow::Result<Self> {
        let ctx = VulkanCtx::new(init.raw_disp, init.raw_win, init.win.clone())?;
        let presenter = Presenter::new(&ctx, init.win)?;
        Ok(Self { rx, ctx, presenter })
    }

    fn draw_frame(&mut self, start_time: &Instant) -> anyhow::Result<()> {
        let Some((target, cmd)) = self.presenter.begin_frame(&self.ctx)? else {
            return Ok(()); // minimized or out-of-date, skip drawing
        };

        unsafe {
            let elapsed = start_time.elapsed().as_secs_f32();
            let sine = (elapsed * std::f32::consts::PI).sin();
            let color_val = sine * 0.5 + 0.5;

            // the guy from frankenstein
            let clear_val = vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [color_val, color_val, color_val, 1.0],
                },
            };

            let to_color_barrier = self.presenter.swapchain.base_barrier()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::empty())
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);

            let dep_info_color = vk::DependencyInfo::default()
                .image_memory_barriers(std::slice::from_ref(&to_color_barrier));
            self.ctx.dev.cmd_pipeline_barrier2(cmd, &dep_info_color);

            let color_atmt = vk::RenderingAttachmentInfo::default()
                .image_view(target.view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(clear_val);

            let rendering_info = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: target.extent,
                })
                .layer_count(1)
                .color_attachments(std::slice::from_ref(&color_atmt));

            self.ctx.dev.cmd_begin_rendering(cmd, &rendering_info);

            // TODO

            self.ctx.dev.cmd_end_rendering(cmd);

            let to_present_barrier = self.presenter.swapchain.base_barrier()
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags2::empty());

            let dep_info_present = vk::DependencyInfo::default()
                .image_memory_barriers(std::slice::from_ref(&to_present_barrier));
            self.ctx.dev.cmd_pipeline_barrier2(cmd, &dep_info_present);
        }

        self.presenter.submit_frame(&self.ctx)
    }

    fn run(mut self) {
        let start_time = Instant::now();

        loop {
            while let Ok(msg) = self.rx.pop() {
                match msg {
                    RenderMsg::Exiting => return,
                    RenderMsg::Resized => {
                        self.presenter.resize_requested = true;
                    }
                }
            }

            if let Err(e) = self.draw_frame(&start_time) {
                eprintln!("render err: {e}");
            }
        }
    }

    pub fn start(init_rx: OneshotRx<RenderInit>, rx: Consumer<RenderMsg>) {
        // this call blocks until we receive necessary data from main
        // (window handles for surface creation, because on some platforms
        // you cannot extract the raw window handle on a non-main thread)
        let Some(init) = init_rx.recv() else { return };
        Self::new(init, rx).expect("failed to init renderer").run();
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // ctx is dropped by the compiler after this is done
        unsafe {
            let _ = self.ctx.dev.device_wait_idle();
            self.presenter.destroy(&self.ctx);
        }
    }
}

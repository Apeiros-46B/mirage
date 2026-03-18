use crate::{
    gpu::{ctx::VulkanCtx, pass::Task},
    workers::msgs::EguiUpdate,
};
use super::frame::FrameCtx;

#[derive(Default)]
pub struct EguiPass {
    staged_update: Option<EguiUpdate>,
}

impl EguiPass {
    pub fn apply_update(&mut self, msg: EguiUpdate) {
        if let Some(existing) = &mut self.staged_update {
            // overwrite stale geometry and screen state but append new texture deltas
            existing.primitives = msg.primitives;
            existing.px_per_pt = msg.px_per_pt;
            existing.screen_sz_px = msg.screen_sz_px;
            existing.textures_delta.append(msg.textures_delta);
        } else {
            self.staged_update = Some(msg);
        }
    }
}

impl Task<FrameCtx> for EguiPass {
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        args: &FrameCtx,
        cmd: ash::vk::CommandBuffer,
    ) -> anyhow::Result<()> {
        todo!()
    }
}

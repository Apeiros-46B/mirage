use anyhow::Result;
use ash::vk;

use crate::gpu::ctx::{Destroy, VulkanCtx};

pub trait Task<T>: Destroy {
    fn resize(&mut self, _ctx: &VulkanCtx, _extent: vk::Extent2D) -> Result<()> {
        Ok(())
    }

    // pre-barriers are hardcoded for now
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        args: &mut T,
    ) -> Result<()>;
}

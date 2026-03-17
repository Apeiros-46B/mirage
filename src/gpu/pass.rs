use ash::vk;

use crate::gpu::ctx::VulkanCtx;

pub trait Task<T> {
    fn resize(&mut self, _ctx: &VulkanCtx, _extent: vk::Extent2D) -> anyhow::Result<()> {
        Ok(())
    }

    // pre-barriers are hardcoded for now
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        args: &T,
        cmd: vk::CommandBuffer,
    ) -> anyhow::Result<()>;
}

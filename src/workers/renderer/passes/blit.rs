use ash::vk;

use crate::{gpu::{ctx::{Destroy, VulkanCtx}, task::Task}, workers::renderer::FrameCtx};

pub struct BlitPass;

impl Destroy for BlitPass {
    unsafe fn destroy(&mut self, _ctx: &VulkanCtx) {}
}

impl Task<FrameCtx<'_>> for BlitPass {
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &mut FrameCtx<'_>,
    ) -> anyhow::Result<()> {
        let blit_ofs = [
            vk::Offset3D::default(),
            vk::Offset3D::default()
                .x(frame.color.extent.width as i32)
                .y(frame.color.extent.height as i32)
                .z(1),
        ];
        let blit_subres = vk::ImageSubresourceLayers::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .mip_level(0)
            .base_array_layer(0)
            .layer_count(1);

        unsafe {
            ctx.dev.cmd_blit_image(
                frame.in_flight.cmd,
                frame.color.img,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                frame.in_flight.img.img,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[
                    vk::ImageBlit::default()
                        .src_offsets(blit_ofs)
                        .dst_offsets(blit_ofs)
                        .src_subresource(blit_subres)
                        .dst_subresource(blit_subres)
                ],
                vk::Filter::NEAREST,
            );
        }

        Ok(())
    }
}

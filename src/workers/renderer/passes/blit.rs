// use ash::vk;
//
// use crate::{gpu::{ctx::{Destroy, VulkanCtx}, task::Task}, workers::renderer::FrameCtx};
//
// #[derive(Default)]
// pub struct BlitPass;
//
// impl Destroy for BlitPass {
//     unsafe fn destroy(&mut self, _ctx: &VulkanCtx) {}
// }
//
// impl Task<FrameCtx<'_>> for BlitPass {
//     unsafe fn execute(
//         &mut self,
//         ctx: &VulkanCtx,
//         frame: &mut FrameCtx,
//     ) -> anyhow::Result<()> {
//         let cmd = frame.in_flight.cmd;
//         let dst_img = frame.in_flight.img.img;
//         let extent = frame.in_flight.img.extent;
//
//         let subres_range = vk::ImageSubresourceRange {
//             aspect_mask: vk::ImageAspectFlags::COLOR,
//             base_mip_level: 0,
//             level_count: 1,
//             base_array_layer: 0,
//             layer_count: 1,
//         };
//
//         let subres_layers = vk::ImageSubresourceLayers {
//             aspect_mask: vk::ImageAspectFlags::COLOR,
//             mip_level: 0,
//             base_array_layer: 0,
//             layer_count: 1,
//         };
//
//         let offsets = [
//             vk::Offset3D { x: 0, y: 0, z: 0 },
//             vk::Offset3D {
//                 x: extent.width as i32,
//                 y: extent.height as i32,
//                 z: 1,
//             },
//         ];
//
//         // 1. Pre-blit barriers
//         let color_barrier = vk::ImageMemoryBarrier2::default()
//             .image(frame.color_view.unwrap())
//             .subresource_range(subres_range)
//             .old_layout(vk::ImageLayout::GENERAL)
//             .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
//             .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
//             .dst_stage_mask(vk::PipelineStageFlags2::BLIT)
//             .src_access_mask(vk::AccessFlags2::SHADER_WRITE)
//             .dst_access_mask(vk::AccessFlags2::TRANSFER_READ);
//
//         let swapchain_barrier = vk::ImageMemoryBarrier2::default()
//             .image(dst_img)
//             .subresource_range(subres_range)
//             .old_layout(vk::ImageLayout::UNDEFINED)
//             .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
//             .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
//             .dst_stage_mask(vk::PipelineStageFlags2::BLIT)
//             .src_access_mask(vk::AccessFlags2::NONE)
//             .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE);
//
//         let dep_info_pre = vk::DependencyInfo::default()
//             .image_memory_barriers(&[color_barrier, swapchain_barrier]);
//
//         unsafe { ctx.dev.cmd_pipeline_barrier2(cmd, &dep_info_pre) };
//
//         // 2. Blit operation
//         let blit_region = vk::ImageBlit2::default()
//             .src_subresource(subres_layers)
//             .src_offsets(offsets)
//             .dst_subresource(subres_layers)
//             .dst_offsets(offsets);
//
//         let blit_info = vk::BlitImageInfo2::default()
//             .src_image(frame.color_img)
//             .src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
//             .dst_image(dst_img)
//             .dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
//             .regions(std::slice::from_ref(&blit_region))
//             .filter(vk::Filter::NEAREST); // NEAREST is standard for 1:1 scale copies
//
//         unsafe { ctx.dev.cmd_blit_image2(cmd, &blit_info) };
//
//         // 3. Post-blit barriers
//         let color_restore_barrier = vk::ImageMemoryBarrier2::default()
//             .image(frame.color_img)
//             .subresource_range(subres_range)
//             .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
//             .new_layout(vk::ImageLayout::GENERAL)
//             .src_stage_mask(vk::PipelineStageFlags2::BLIT)
//             .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
//             .src_access_mask(vk::AccessFlags2::TRANSFER_READ)
//             .dst_access_mask(vk::AccessFlags2::NONE);
//
//         let swapchain_egui_barrier = vk::ImageMemoryBarrier2::default()
//             .image(dst_img)
//             .subresource_range(subres_range)
//             .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
//             .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
//             .src_stage_mask(vk::PipelineStageFlags2::BLIT)
//             .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
//             .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
//             .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);
//
//         let dep_info_post = vk::DependencyInfo::default()
//             .image_memory_barriers(&[color_restore_barrier, swapchain_egui_barrier]);
//
//         unsafe { ctx.dev.cmd_pipeline_barrier2(cmd, &dep_info_post) };
//
//         Ok(())
//     }
// }

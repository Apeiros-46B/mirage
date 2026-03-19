use std::{collections::HashMap, mem};

use ash::vk;
use egui::{ClippedPrimitive, TextureId, TexturesDelta, epaint::Vertex};
use winit::dpi::PhysicalSize;

use crate::{
    gpu::{ctx::VulkanCtx, img::AllocatedImage, sync::ImageBarrierExt, task::Task},
    workers::{msgs::EguiUpdate, render::frames::PerFrame},
};
use super::frame::FrameCtx;

const SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/egui-pass.spv"));

pub struct EguiPass {
    primitives: Vec<ClippedPrimitive>,
    screen_sz_px: PhysicalSize<u32>,
    px_per_pt: f32,

    // transient
    textures_delta: TexturesDelta,

    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
    desc_set_layout: vk::DescriptorSetLayout,
    desc_pool: vk::DescriptorPool,
    sampler: vk::Sampler,

    textures: HashMap<TextureId, (AllocatedImage, vk::DescriptorSet)>,
    pending_frees: PerFrame<Vec<AllocatedImage>>,
}

impl EguiPass {
    // TODO: handle format changing (e.g. user moving app between monitors)
    // probably needs new Task method and refactor of presenter
    pub fn new(ctx: &VulkanCtx, fmt: vk::Format) -> anyhow::Result<Self> {
        let code = ash::util::read_spv(&mut std::io::Cursor::new(SPV))?;
        let shader_info = vk::ShaderModuleCreateInfo::default().code(&code);
        let module = unsafe { ctx.dev.create_shader_module(&shader_info, None)? };

        // sampler
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
        let sampler = unsafe { ctx.dev.create_sampler(&sampler_info, None)? };

        // {{{ descriptor sets and pipeline layout
        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);

        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(std::slice::from_ref(&binding));
        let desc_set_layout = unsafe {
            ctx.dev.create_descriptor_set_layout(&layout_info, None)?
        };

        let pool_size = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1024);
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(std::slice::from_ref(&pool_size))
            .max_sets(1024);
        let desc_pool = unsafe { ctx.dev.create_descriptor_pool(&pool_info, None)? };

        let push_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX)
            .offset(0)
            .size(8); // float2 (8 bytes)

        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(std::slice::from_ref(&push_range))
            .set_layouts(std::slice::from_ref(&desc_set_layout));
        let layout = unsafe { ctx.dev.create_pipeline_layout(&layout_info, None)? };
        // }}}

        // {{{ vertex input
        let bind_desc = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(mem::size_of::<Vertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);

        // see egui::epaint::Vertex
        let attr_descs = [
            vk::VertexInputAttributeDescription::default() // pos [f32; 2]
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default() // uv [f32; 2]
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(8),
            vk::VertexInputAttributeDescription::default() // color [u8; 4]
                .binding(0)
                .location(2)
                .format(vk::Format::R8G8B8A8_UNORM)
                .offset(16),
        ];

        let vert_input_info = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(std::slice::from_ref(&bind_desc))
            .vertex_attribute_descriptions(&attr_descs);
        // }}}

        // {{{ blending (egui uses premultiplied alpha)
        let blend_atmt = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_DST_ALPHA)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);

        let blend_info = vk::PipelineColorBlendStateCreateInfo::default()
            .attachments(std::slice::from_ref(&blend_atmt));
        // }}}

        // {{{ pipeline creation
        let vsh_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(module)
            .name(c"vsh");
        let fsh_info = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(module)
            .name(c"fsh");
        let stages = [vsh_info, fsh_info];

        let input_asm_info = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

        let viewport_info = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let rast_info = vk::PipelineRasterizationStateCreateInfo::default()
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);

        let multismp_info = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let depth_sten_info = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false);

        let fmts = [fmt];
        let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&fmts);

        let dyn_info = vk::PipelineDynamicStateCreateInfo::default()
            .dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vert_input_info)
            .input_assembly_state(&input_asm_info)
            .viewport_state(&viewport_info)
            .rasterization_state(&rast_info)
            .multisample_state(&multismp_info)
            .depth_stencil_state(&depth_sten_info)
            .color_blend_state(&blend_info)
            .dynamic_state(&dyn_info)
            .layout(layout)
            .push_next(&mut rendering_info);

        let pipeline = unsafe {
            ctx.dev.create_graphics_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&pipeline_info),
                None,
            ).map_err(|(_, err)| err)?[0]
        };

        unsafe { ctx.dev.destroy_shader_module(module, None) };
        // }}}

        Ok(Self {
            primitives: Vec::new(),
            screen_sz_px: winit::dpi::PhysicalSize::new(0, 0),
            px_per_pt: 1.0,

            textures_delta: Default::default(),

            pipeline,
            pipeline_layout: layout,
            desc_set_layout,
            desc_pool,
            sampler,

            textures: HashMap::new(),
            pending_frees: PerFrame::init(|_| Vec::new()),
        })
    }

    pub fn apply_update(&mut self, upd: EguiUpdate) {
        self.primitives = upd.primitives;
        self.screen_sz_px = upd.screen_sz_px;
        self.px_per_pt = upd.px_per_pt;
        self.textures_delta.append(upd.textures_delta);
    }
}

impl Task<FrameCtx<'_>> for EguiPass {
    unsafe fn execute(
        &mut self,
        ctx: &VulkanCtx,
        frame: &mut FrameCtx,
    ) -> anyhow::Result<()> {
        let cmd = frame.in_flight.cmd;
        let fif = frame.in_flight.idx;

        let mut deltas = egui::TexturesDelta::default();
        mem::swap(&mut self.textures_delta, &mut deltas);

        // clean up resources from finished draws
        for mut img in self.pending_frees[fif].drain(..) {
            unsafe { img.destroy(ctx); }
        }

        for id in deltas.free {
            if let Some((img, _set)) = self.textures.remove(&id) {
                self.pending_frees[fif].push(img);
            }
        }

        for (id, delta) in deltas.set {
            // {{{ allocate image
            let (pixels, width, height) = match &delta.image {
                egui::ImageData::Color(img) => (
                    bytemuck::cast_slice(&img.pixels),
                    img.width() as u32,
                    img.height() as u32,
                ),
            };

            let staging_sz = pixels.len() as u64;

            let (
                staging_buf,
                staging_ofs,
                staging_ptr
            ) = frame.in_flight.alloc.alloc(staging_sz, 16);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr(),
                    staging_ptr.0,
                    pixels.len(),
                );
            }

            let extent = vk::Extent3D { width, height, depth: 1 };
            let subres_range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1);

            let copy_region = vk::BufferImageCopy::default()
                .buffer_offset(staging_ofs)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(
                    if let Some(pos) = delta.pos {
                        vk::Offset3D { x: pos[0] as i32, y: pos[1] as i32, z: 0 }
                    } else {
                        vk::Offset3D { x: 0, y: 0, z: 0 }
                    },
                )
                .image_extent(extent);

            if let Some(_pos) = delta.pos {
                // {{{ update to existing image
                let (img, _) = self.textures.get(&id)
                    .expect("egui requested update for non-existent texture");

                let mut barrier = vk::ImageMemoryBarrier2::default()
                    .image(img.img)
                    .subresource_range(subres_range)
                    .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                    .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                    .src_access_mask(vk::AccessFlags2::SHADER_READ)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE);

                unsafe {
                    ctx.dev.cmd_pipeline_barrier2(cmd, &barrier.dep_info());

                    ctx.dev.cmd_copy_buffer_to_image(
                        cmd,
                        staging_buf,
                        img.img,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        std::slice::from_ref(&copy_region),
                    );
                }

                barrier = barrier.next_phase(
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::FRAGMENT_SHADER,
                    vk::AccessFlags2::SHADER_READ,
                );

                unsafe {
                    ctx.dev.cmd_pipeline_barrier2(cmd, &barrier.dep_info());
                }
                // }}}
            } else {
                // {{{ new image
                let img = AllocatedImage::new(
                    ctx,
                    extent,
                    vk::Format::R8G8B8A8_UNORM,
                    vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
                    subres_range,
                    "egui_tex",
                )?;

                let mut barrier = vk::ImageMemoryBarrier2::default()
                    .image(img.img)
                    .subresource_range(subres_range)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
                    .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                    .src_access_mask(vk::AccessFlags2::NONE)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE);

                unsafe {
                    ctx.dev.cmd_pipeline_barrier2(cmd, &barrier.dep_info());
                    ctx.dev.cmd_copy_buffer_to_image(
                        cmd,
                        staging_buf,
                        img.img,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        std::slice::from_ref(&copy_region),
                    );
                }

                barrier = barrier.next_phase(
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::FRAGMENT_SHADER,
                    vk::AccessFlags2::SHADER_READ,
                );

                unsafe {
                    ctx.dev.cmd_pipeline_barrier2(cmd, &barrier.dep_info());
                }

                let desc_set_info = vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.desc_pool)
                    .set_layouts(std::slice::from_ref(&self.desc_set_layout));
                let set = unsafe { ctx.dev.allocate_descriptor_sets(&desc_set_info)?[0] };

                let img_info = vk::DescriptorImageInfo::default()
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image_view(img.view)
                    .sampler(self.sampler);

                let write = vk::WriteDescriptorSet::default()
                    .dst_set(set).dst_binding(0).descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(std::slice::from_ref(&img_info));

                unsafe { ctx.dev.update_descriptor_sets(std::slice::from_ref(&write), &[]); }

                self.textures.insert(id, (img, set));
                // }}}
            }

            // }}}
        }

        if self.primitives.is_empty() {
            return Ok(());
        }

        // length
        let mut vert_ofs = 0;
        let mut idx_ofs = 0;

        // {{{ upload geometry
        let mut total_vert_sz = 0;
        let mut total_idx_sz = 0;

        for ClippedPrimitive { primitive, .. } in &self.primitives {
            if let egui::epaint::Primitive::Mesh(mesh) = primitive {
                total_vert_sz += mesh.vertices.len() * mem::size_of::<Vertex>();
                total_idx_sz += mesh.indices.len() * mem::size_of::<u32>();
            }
        }

        let (geom_buf, vbo_ofs, mut vert_ptr) = frame.in_flight.alloc.alloc(
            total_vert_sz as u64,
            16,
        );
        let (_, ibo_ofs, mut idx_ptr) = frame.in_flight.alloc.alloc(
            total_idx_sz as u64,
            16,
        );

        // copy geometry sequentially
        for ClippedPrimitive { primitive, .. } in &self.primitives {
            let egui::epaint::Primitive::Mesh(mesh) = primitive else { continue };

            let vert_sz = mesh.vertices.len() * mem::size_of::<Vertex>();
            let idx_sz = mesh.indices.len() * mem::size_of::<u32>();

            unsafe {
                std::ptr::copy_nonoverlapping(
                    mesh.vertices.as_ptr() as *const u8,
                    vert_ptr.0,
                    vert_sz,
                );
                std::ptr::copy_nonoverlapping(
                    mesh.indices.as_ptr() as *const u8,
                    idx_ptr.0,
                    idx_sz,
                );

                vert_ptr.0 = vert_ptr.0.add(vert_sz);
                idx_ptr.0 = idx_ptr.0.add(idx_sz);
            }
        }
        // }}}

        // render to swapchain image
        let color_atmt = vk::RenderingAttachmentInfo::default()
            .image_view(frame.in_flight.img.view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::LOAD) // blend with earlier content
            .store_op(vk::AttachmentStoreOp::STORE);

        let rendering_info = vk::RenderingInfo::default()
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: frame.in_flight.img.extent,
            })
            .layer_count(1)
            .color_attachments(std::slice::from_ref(&color_atmt));

        unsafe { ctx.dev.cmd_begin_rendering(cmd, &rendering_info); }

        // {{{ dispatch draws
        unsafe {
            ctx.dev.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline,
            );
            ctx.dev.cmd_bind_vertex_buffers(cmd, 0, &[geom_buf], &[vbo_ofs]);
            ctx.dev.cmd_bind_index_buffer(
                cmd,
                geom_buf,
                ibo_ofs,
                vk::IndexType::UINT32,
            );
        }

        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: frame.in_flight.img.extent.width as f32,
            height: frame.in_flight.img.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        unsafe {
            ctx.dev.cmd_set_viewport(cmd, 0, std::slice::from_ref(&viewport));
        }

        let push = [
            frame.in_flight.img.extent.width as f32 / self.px_per_pt,
            frame.in_flight.img.extent.height as f32 / self.px_per_pt,
        ];
        unsafe {
            ctx.dev.cmd_push_constants(
                cmd,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                bytemuck::bytes_of(&push),
            );
        }

        for ClippedPrimitive { clip_rect, primitive } in &self.primitives {
            let egui::epaint::Primitive::Mesh(mesh) = primitive else { continue };

            let min_x = (clip_rect.min.x * self.px_per_pt) as i32;
            let min_y = (clip_rect.min.y * self.px_per_pt) as i32;
            let max_x = (clip_rect.max.x * self.px_per_pt) as i32;
            let max_y = (clip_rect.max.y * self.px_per_pt) as i32;

            let extent_w = frame.in_flight.img.extent.width as i32;
            let extent_h = frame.in_flight.img.extent.height as i32;

            let clamped_w = (max_x - min_x).max(0).min(extent_w - min_x.max(0));
            let clamped_h = (max_y - min_y).max(0).min(extent_h - min_y.max(0));

            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: min_x.max(0), y: min_y.max(0) },
                extent: vk::Extent2D {
                    width: clamped_w as u32,
                    height: clamped_h as u32,
                },
            };
            unsafe {
                ctx.dev.cmd_set_scissor(cmd, 0, std::slice::from_ref(&scissor));
            }

            if let Some((_, set)) = self.textures.get(&mesh.texture_id) {
                unsafe {
                    ctx.dev.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipeline_layout,
                        0,
                        std::slice::from_ref(set),
                        &[],
                    );
                }
            }

            unsafe {
                ctx.dev.cmd_draw_indexed(
                    cmd,
                    mesh.indices.len() as u32,
                    1, // instance count
                    idx_ofs,
                    vert_ofs,
                    0, // first instance
                );
            }

            vert_ofs += mesh.vertices.len() as i32;
            idx_ofs += mesh.indices.len() as u32;
        }
        // }}}

        unsafe { ctx.dev.cmd_end_rendering(cmd); }

        Ok(())
    }
}

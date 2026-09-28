use std::sync::Arc;

use anyhow::Result;
use ash::vk;
use triple_buffer::Output;
use winit::dpi::PhysicalSize;

use crate::{
    gpu::{
        alloc::AllocatedMegabuffer,
        ctx::{Destroy, VulkanCtx},
        desc::DescriptorAllocator,
        img::AllocatedImage,
    },
    workers::{
        msgs::CameraState,
        renderer::frames::{PerFrame, RenderTarget},
    },
    world::{
        CHUNK_N,
        ChunkDirectory,
        ChunkDirectoryData,
        ChunkMetadata,
        Node,
        REALM_N,
    },
};

pub struct ChunkHeaders {
    // owner
    data: AllocatedMegabuffer,

    // views, each parallel array allocated contiguously
    pub roots: AllocatedMegabuffer, // accessed by primary pass
    pub metadata: AllocatedMegabuffer, // accessed by depth prepass
}

impl Destroy for ChunkHeaders {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe { self.data.destroy(ctx); }
    }
}

impl ChunkHeaders {
    fn new(ctx: &VulkanCtx) -> Result<Self> {
        let roots_sz = (CHUNK_N * size_of::<Node>()) as u64;
        let metadata_sz = (CHUNK_N * size_of::<ChunkMetadata>()) as u64;

        let data = AllocatedMegabuffer::new_alloc(
            &ctx,
            roots_sz + metadata_sz,
            c"chunk_headers",
        )?;

        unsafe {
            let roots = data.view(0, roots_sz);
            let metadata = data.view(roots_sz, metadata_sz);
            Ok(Self { data, roots, metadata })
        }
    }

    pub fn buf(&self) -> vk::Buffer {
        self.data.buf
    }
}

pub struct ChunkDirectoryBuffer {
    data: AllocatedMegabuffer,

    pub pages: AllocatedMegabuffer,
    pub roots: AllocatedMegabuffer,
}

impl Destroy for ChunkDirectoryBuffer {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe { self.data.destroy(ctx); }
    }
}

impl ChunkDirectoryBuffer {
    fn new(ctx: &VulkanCtx, directory: &ChunkDirectoryData) -> Result<Self> {
        let pages_sz = size_of_val(directory.pages.as_slice()) as u64;
        let roots_sz = (REALM_N * size_of::<u32>()) as u64;
        let data = AllocatedMegabuffer::new_alloc(
            ctx,
            pages_sz + roots_sz,
            c"chunk_directory",
        )?;

        unsafe {
            let pages = data.view(0, pages_sz);
            let roots = data.view(pages_sz, roots_sz);
            Ok(Self { data, pages, roots })
        }
    }

    pub fn buf(&self) -> vk::Buffer {
        self.data.buf
    }

    fn directory(&self) -> ChunkDirectory {
        ChunkDirectory {
            pages: self.pages.bda,
            roots: self.roots.bda,
        }
    }
}

pub struct Resources {
    pub desc: DescriptorAllocator,

    depth: PerFrame<AllocatedImage>,
    chunk_id: PerFrame<AllocatedImage>,
    chunk_id_idxs: PerFrame<u32>,
    color: PerFrame<AllocatedImage>,
    color_idxs: PerFrame<u32>,
    visbuf: PerFrame<AllocatedImage>,
    visbuf_idxs: PerFrame<u32>,

    camera_rx: Output<CameraState>,

    pub nodes: AllocatedMegabuffer,
    pub chunk_headers: ChunkHeaders,
    pub chunk_directory: ChunkDirectoryBuffer,
}

impl Destroy for Resources {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            self.chunk_directory.destroy(ctx);
            self.chunk_headers.destroy(ctx);
            self.nodes.destroy(ctx);

            self.depth.destroy(ctx);
            self.chunk_id.destroy(ctx);
            self.color.destroy(ctx);
            self.visbuf.destroy(ctx);

            self.desc.destroy(ctx);
        }
    }
}

impl Resources {
    pub fn new(
        ctx: Arc<VulkanCtx>,
        scr_sz: PhysicalSize<u32>,
        camera_rx: Output<CameraState>,
        directory: &ChunkDirectoryData,
    ) -> Result<Self> {
        let extent = vk::Extent2D { width: scr_sz.width, height: scr_sz.height };

        let depth = make_depth_targets(&ctx, extent)?;
        let chunk_id = make_chunk_id_targets(&ctx, extent)?;
        let color = make_color_targets(&ctx, extent)?;
        let visbuf = make_visbuf_targets(&ctx, extent)?;

        let nodes = AllocatedMegabuffer::new_alloc(
            &ctx,
            (1 << 22) * 16, // temporary size, TODO: allocate large heap and suballoc
            c"nodes",
        )?;
        let chunk_headers = ChunkHeaders::new(&ctx)?;
        let chunk_directory = ChunkDirectoryBuffer::new(&ctx, directory)?;

        let mut desc = DescriptorAllocator::new(ctx)?;
        let chunk_id_idxs = desc.alloc_sampled_imgs(&chunk_id).into();
        let color_idxs = desc.alloc_storage_imgs(&color).into();
        let visbuf_idxs = desc.alloc_storage_imgs(&visbuf).into();

        Ok(Self {
            desc,

            depth,
            chunk_id,
            chunk_id_idxs,
            color,
            color_idxs,
            visbuf,
            visbuf_idxs,

            nodes,
            chunk_headers,
            chunk_directory,

            camera_rx,
        })
    }

    pub unsafe fn resize(
        &mut self,
        ctx: &VulkanCtx,
        extent: vk::Extent2D,
    ) -> Result<()> {
        unsafe {
            self.depth.destroy(ctx);
            self.depth = make_depth_targets(ctx, extent)?;

            self.chunk_id.destroy(ctx);
            self.chunk_id = make_chunk_id_targets(ctx, extent)?;
            self.desc.update_sampled_imgs(self.chunk_id_idxs[0], &self.chunk_id);

            self.color.destroy(ctx);
            self.color = make_color_targets(ctx, extent)?;
            self.desc.update_storage_imgs(self.color_idxs[0], &self.color);

            self.visbuf.destroy(ctx);
            self.visbuf = make_visbuf_targets(ctx, extent)?;
            self.desc.update_storage_imgs(self.visbuf_idxs[0], &self.visbuf);
        }

        Ok(())
    }

    pub fn view(&mut self, target: &mut RenderTarget<'_>, num_chunks: u32) -> ResourceView<'_> {
        let mut camera_buf = target.arena.alloc_scratch::<CameraState>();
        camera_buf.write(self.camera_rx.read());

        ResourceView {
            camera_bda: camera_buf.bda,

            nodes_bda: self.nodes.bda,
            chunk_metadata_bda: self.chunk_headers.metadata.bda,
            chunk_roots_bda: self.chunk_headers.roots.bda,
            chunk_directory: self.chunk_directory.directory(),
            num_chunks,

            depth: &self.depth[target.idx],
            chunk_id: &self.chunk_id[target.idx],
            chunk_id_idx: self.chunk_id_idxs[target.idx],
            color: &self.color[target.idx],
            color_idx: self.color_idxs[target.idx],
            visbuf: &self.visbuf[target.idx],
            visbuf_idx: self.visbuf_idxs[target.idx],
        }
    }
}

pub struct ResourceView<'a> {
    // dynamic
    pub camera_bda: u64, // BDA pointer to CameraState

    // world buffer BDAs (handles stay on Resources for staging)
    pub nodes_bda: u64,
    pub chunk_metadata_bda: u64,
    pub chunk_roots_bda: u64,
    pub chunk_directory: ChunkDirectory,
    pub num_chunks: u32,

    // per frame
    pub depth: &'a AllocatedImage,
    pub chunk_id: &'a AllocatedImage,
    pub chunk_id_idx: u32,
    pub visbuf: &'a AllocatedImage,
    pub visbuf_idx: u32,
    pub color: &'a AllocatedImage,
    pub color_idx: u32,
}

impl<'a, 'b> ResourceView<'a> {
    pub fn depth_barrier(&self) -> vk::ImageMemoryBarrier2<'b> {
        vk::ImageMemoryBarrier2::default()
            .image(self.depth.img)
            .subresource_range(DEPTH_SUBRES_RANGE)
    }

    pub fn chunk_id_barrier(&self) -> vk::ImageMemoryBarrier2<'b> {
        vk::ImageMemoryBarrier2::default()
            .image(self.chunk_id.img)
            .subresource_range(COLOR_SUBRES_RANGE)
    }

    pub fn color_barrier(&self) -> vk::ImageMemoryBarrier2<'b> {
        vk::ImageMemoryBarrier2::default()
            .image(self.color.img)
            .subresource_range(COLOR_SUBRES_RANGE)
    }

    pub fn visbuf_barrier(&self) -> vk::ImageMemoryBarrier2<'b> {
        vk::ImageMemoryBarrier2::default()
            .image(self.visbuf.img)
            .subresource_range(COLOR_SUBRES_RANGE)
    }
}

const DEPTH_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::DEPTH,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

const COLOR_SUBRES_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

fn make_depth_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::D32_SFLOAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            DEPTH_SUBRES_RANGE,
            c"depth target",
        )
    })
}

fn make_chunk_id_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::R32_UINT,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            COLOR_SUBRES_RANGE,
            c"chunk ID target",
        )
    })
}

fn make_color_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::R8G8B8A8_UNORM,
            vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC,
            COLOR_SUBRES_RANGE,
            c"color target",
        )
    })
}

fn make_visbuf_targets(
    ctx: &VulkanCtx,
    extent: vk::Extent2D,
) -> Result<PerFrame<AllocatedImage>> {
    PerFrame::try_init(|_| {
        AllocatedImage::new(
            ctx,
            extent.into(),
            vk::Format::R64_UINT,
            vk::ImageUsageFlags::STORAGE,
            COLOR_SUBRES_RANGE,
            c"visbuf target",
        )
    })
}

use std::{marker::PhantomData, sync::Arc};

use anyhow::Result;
use ash::vk;
use gpu_allocator::{
    MemoryLocation,
    vulkan::{Allocation, AllocationCreateDesc, AllocationScheme},
};

use crate::gpu::{MappedPtr, ctx::{Destroy, VulkanCtx}};

/// gpu-only megabuffer
pub struct AllocatedMegabuffer {
    mem: Option<Allocation>,
    pub buf: vk::Buffer,
    pub bda: u64,
    pub sz: u64,
}

impl Destroy for AllocatedMegabuffer {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        if let Some(mem) = self.mem.take() {
            ctx.alloc.lock().unwrap().free(mem).unwrap();
        }
    }
}

impl AllocatedMegabuffer {
    pub fn new(
        ctx: &VulkanCtx,
        sz: u64,
        name: &std::ffi::CStr,
    ) -> Result<Self> {
        let buf_info = vk::BufferCreateInfo::default()
            .size(sz)
            .usage(
                vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buf = unsafe { ctx.dev.create_buffer(&buf_info, None)? };
        let requirements = unsafe { ctx.dev.get_buffer_memory_requirements(buf) };

        let mem = ctx.alloc.lock().unwrap().allocate(&AllocationCreateDesc {
            name: &name.to_string_lossy(),
            requirements,
            location: gpu_allocator::MemoryLocation::GpuOnly,
            linear: true,
            allocation_scheme: AllocationScheme::DedicatedBuffer(buf),
        })?;

        unsafe {
            ctx.dev.bind_buffer_memory(buf, mem.memory(), mem.offset())?;
        }

        let bda_info = vk::BufferDeviceAddressInfo::default().buffer(buf);
        let bda = unsafe { ctx.dev.get_buffer_device_address(&bda_info) };

        Ok(Self { mem: Some(mem), buf, bda, sz })
    }
}

/// temporary write-only (on the CPU side) container for a some items of type T.
/// only contains one item unless T = [u8], in which case it's manually managed bytes.
/// intended as a scratch space for reading in shaders, not as a staging buffer
pub struct AllocatedScratch<T: ?Sized = [u8]> {
    pub buf: vk::Buffer,
    /// offset pre-applied
    pub bda: u64,
    pub ofs: u64,
    pub ptr: MappedPtr,
    _phantom: PhantomData<T>,
}

impl<T: Sized> AllocatedScratch<T> {
    pub fn write(&mut self, val: &T) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                val as *const T,
                self.ptr.0 as *mut T,
                1,
            );
        }
    }
}

impl AllocatedScratch<[u8]> {
    pub unsafe fn write_bytes(&mut self, src: &[u8], ofs: usize) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                src.as_ptr(),
                self.ptr.0.add(ofs),
                src.len(),
            );
        }
    }
}

pub struct StagingSlice {
    pub buf: vk::Buffer,
    /// source offset within the arena buffer
    pub ofs: u64,
    pub len: u64,
    pub ptr: MappedPtr,
}

impl StagingSlice {
    pub unsafe fn write_bytes(&mut self, src: &[u8], ofs: usize) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                src.as_ptr(),
                self.ptr.0.add(ofs),
                src.len(),
            );
        }
    }

    pub fn get_buf_copy(&self, dst_ofs: u64) -> vk::BufferCopy {
        vk::BufferCopy::default()
            .src_offset(self.ofs)
            .dst_offset(dst_ofs)
            .size(self.len)
    }

    pub fn get_buf_img_copy(
        &self,
        subres: vk::ImageSubresourceLayers,
        offset: vk::Offset3D,
        extent: vk::Extent3D,
    ) -> vk::BufferImageCopy {
        vk::BufferImageCopy::default()
            .buffer_offset(self.ofs)
            .image_subresource(subres)
            .image_offset(offset)
            .image_extent(extent)
    }
}

/// CpuToGpu ephemeral allocations. can be used for scratch or staging
pub struct GPUArena {
    ctx: Arc<VulkanCtx>,

    pub buf: vk::Buffer,
    bda: u64,
    mem: Option<Allocation>,
    mapped_ptr: MappedPtr,

    sz: u64,
    cur_ofs: u64,
}

impl GPUArena {
    pub fn new(
        ctx: Arc<VulkanCtx>,
        sz: u64,
    ) -> Result<Self> {
        let buf_info = vk::BufferCreateInfo::default()
            .size(sz)
            .usage(
                vk::BufferUsageFlags::VERTEX_BUFFER
                | vk::BufferUsageFlags::INDEX_BUFFER
                | vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buf = unsafe { ctx.dev.create_buffer(&buf_info, None)? };
        let requirements = unsafe { ctx.dev.get_buffer_memory_requirements(buf) };

        let mem = ctx.alloc.lock().unwrap().allocate(&AllocationCreateDesc {
            name: "gpu_arena",
            requirements,
            location: MemoryLocation::CpuToGpu,
            linear: true,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
        })?;

        unsafe {
            ctx.dev.bind_buffer_memory(buf, mem.memory(), mem.offset())?;
        }

        let bda_info = vk::BufferDeviceAddressInfo::default().buffer(buf);
        let bda = unsafe { ctx.dev.get_buffer_device_address(&bda_info) };
        let mapped_ptr = mem.mapped_ptr().unwrap().as_ptr() as *mut u8;

        Ok(Self {
            ctx,
            buf,
            bda,
            mem: Some(mem),
            mapped_ptr: MappedPtr(mapped_ptr),
            sz,
            cur_ofs: 0,
        })
    }

    pub fn clear(&mut self) {
        self.cur_ofs = 0;
    }

    /// bump the offset, return slice's base offset
    fn bump(&mut self, sz: u64, align: u64) -> u64 {
        let align_diff = self.cur_ofs % align;
        if align_diff != 0 {
            self.cur_ofs += align - align_diff;
        }

        let ofs = self.cur_ofs;
        self.cur_ofs += sz;

        assert!(
            self.cur_ofs <= self.sz,
            "GPUArena overflow: capacity={}, requested={}",
            self.sz,
            self.cur_ofs
        );

        ofs
    }

    fn ptr_at(&self, ofs: u64) -> MappedPtr {
        MappedPtr(unsafe { self.mapped_ptr.0.add(ofs as usize) })
    }

    pub fn alloc_scratch<T>(&mut self) -> AllocatedScratch<T> {
        let ofs = self.bump(
            std::mem::size_of::<T>() as u64,
            std::mem::align_of::<T>() as u64,
        );
        AllocatedScratch {
            buf: self.buf,
            bda: self.bda + ofs,
            ofs,
            ptr: self.ptr_at(ofs),
            _phantom: PhantomData,
        }
    }

    pub fn alloc_raw_scratch(
        &mut self,
        sz: u64,
        align: u64,
    ) -> AllocatedScratch<[u8]> {
        let ofs = self.bump(sz, align);
        AllocatedScratch {
            buf: self.buf,
            bda: self.bda + ofs,
            ofs,
            ptr: self.ptr_at(ofs),
            _phantom: PhantomData,
        }
    }

    pub fn alloc_staging(&mut self, sz: u64, align: u64) -> StagingSlice {
        let ofs = self.bump(sz, align);
        StagingSlice {
            buf: self.buf,
            ofs,
            len: sz,
            ptr: self.ptr_at(ofs),
        }
    }

    pub fn stage_bytes(&mut self, bytes: &[u8]) -> StagingSlice {
        // TODO: optimalBufferCopyOffsetAlignment instead of hardcoded 16
        let mut s = self.alloc_staging(bytes.len() as u64, 16);
        unsafe { s.write_bytes(bytes, 0) };
        s
    }
}

impl Drop for GPUArena {
    fn drop(&mut self) {
        if let Some(mem) = self.mem.take() {
            unsafe {
                self.ctx.dev.destroy_buffer(self.buf, None);
            }

            // handle potential mutex poisoning during unwinding
            if let Ok(mut alloc) = self.ctx.alloc.lock() {
                let _ = alloc.free(mem);
            } else {
                eprintln!("failed to free GPUArena memory: mutex poisoned");
            }
        }
    }
}

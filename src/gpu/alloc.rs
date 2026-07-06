use std::{marker::PhantomData, sync::Arc};

use ash::vk;
use bytemuck::{Pod, bytes_of};
use gpu_allocator::{
    MemoryLocation,
    vulkan::{Allocation, AllocationCreateDesc, AllocationScheme},
};

use crate::gpu::{MappedPtr, ctx::VulkanCtx};

pub struct GPUAllocation<T = u8> {
    pub buf: vk::Buffer,
    /// offset pre-applied
    pub bda: u64,
    pub ofs: u64,
    pub ptr: MappedPtr,
    _phantom: PhantomData<T>,
}

impl<T> GPUAllocation<T> {
    pub unsafe fn write(&mut self, val: &T) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                val as *const T,
                self.ptr.0 as *mut T,
                1,
            );
        }
    }

    pub unsafe fn write_slice(&mut self, val: &[T]) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                val.as_ptr(),
                self.ptr.0 as *mut T,
                val.len(),
            );
        }
    }
}

impl GPUAllocation<u8> {
    pub unsafe fn write_data<T: Pod>(&mut self, val: &T) {
        let bytes = bytes_of(val);
        unsafe {
            self.write_slice(bytes);
        }
    }

    pub fn cast<T>(self) -> GPUAllocation<T> {
        GPUAllocation {
            buf: self.buf,
            bda: self.bda,
            ofs: self.ofs,
            ptr: self.ptr,
            _phantom: PhantomData,
        }
    }
}

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
    ) -> anyhow::Result<Self> {
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
        let reqs = unsafe { ctx.dev.get_buffer_memory_requirements(buf) };

        let mem = ctx.alloc.lock().unwrap().allocate(&AllocationCreateDesc {
            name: "gpu_arena",
            requirements: reqs,
            location: MemoryLocation::CpuToGpu,
            linear: true,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
        })?;

        unsafe {
            ctx.dev.bind_buffer_memory(buf, mem.memory(), mem.offset())?;
        }

        let bda_info = vk::BufferDeviceAddressInfo::default()
            .buffer(buf);

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

    pub fn alloc(
        &mut self,
        sz: u64,
        align: u64,
    ) -> GPUAllocation {
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

        GPUAllocation {
            buf: self.buf,
            bda: self.bda + ofs,
            ofs,
            ptr: MappedPtr(unsafe { self.mapped_ptr.0.add(ofs as usize) }),
            _phantom: PhantomData,
        }
    }

    pub fn alloc_of<T>(&mut self) -> GPUAllocation<T> {
        self.alloc(
            std::mem::size_of::<T>() as u64,
            std::mem::align_of::<T>() as u64,
        ).cast()
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

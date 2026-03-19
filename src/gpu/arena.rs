use std::sync::Arc;

use ash::vk;
use gpu_allocator::{
    MemoryLocation,
    vulkan::{Allocation, AllocationCreateDesc, AllocationScheme},
};

use crate::gpu::{MappedBuffer, ctx::VulkanCtx};

pub struct GPUArena {
    ctx: Arc<VulkanCtx>,

    pub buffer: vk::Buffer,
    mem: Option<Allocation>,
    mapped_ptr: MappedBuffer,

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
                | vk::BufferUsageFlags::TRANSFER_SRC,
            );

        let buffer = unsafe { ctx.dev.create_buffer(&buf_info, None)? };
        let reqs = unsafe { ctx.dev.get_buffer_memory_requirements(buffer) };

        let mem = ctx.alloc.lock().unwrap().allocate(&AllocationCreateDesc {
            name: "gpu_arena",
            requirements: reqs,
            location: MemoryLocation::CpuToGpu,
            linear: true,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
        })?;

        unsafe {
            ctx.dev.bind_buffer_memory(buffer, mem.memory(), mem.offset())?;
        }

        let mapped_ptr = mem.mapped_ptr().unwrap().as_ptr() as *mut u8;

        Ok(Self {
            ctx,
            buffer,
            mem: Some(mem),
            mapped_ptr: MappedBuffer(mapped_ptr),
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
    ) -> (vk::Buffer, u64, MappedBuffer) {
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

        let out_ptr = unsafe { self.mapped_ptr.0.add(ofs as usize) };

        (self.buffer, ofs, MappedBuffer(out_ptr))
    }
}

impl Drop for GPUArena {
    fn drop(&mut self) {
        if let Some(mem) = self.mem.take() {
            unsafe {
                self.ctx.dev.destroy_buffer(self.buffer, None);
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

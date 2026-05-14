// TODO: replace _ci, _ai naming convention everywhere with _info
use std::{ffi::c_char, mem::ManuallyDrop, sync::{Arc, Mutex}};

use ash::{ext::debug_utils, vk};
use gpu_allocator::{AllocationSizes, AllocatorDebugSettings, vulkan::{Allocator, AllocatorCreateDesc}};
use winit::{raw_window_handle::{RawDisplayHandle, RawWindowHandle}, window::Window};

use crate::gpu::{GPU, surf::Surface};

const DEV_EXTS: [*const c_char; 1] = [ash::khr::swapchain::NAME.as_ptr()];

// TODO: make Destroy implementations idempotent
pub trait Destroy {
    /// destroys the Vulkan resource and frees its associated memory
    /// - must be called only when the GPU is no longer using this resource
    /// - must be called exactly once
    unsafe fn destroy(&mut self, ctx: &VulkanCtx);
}

impl<T: Destroy> Destroy for [T] {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        for item in self.iter_mut() {
            unsafe {
                item.destroy(ctx);
            }
        }
    }
}

impl<T: Destroy> Destroy for Vec<T> {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        for item in self.iter_mut() {
            unsafe {
                item.destroy(ctx);
            }
        }
        self.clear();
    }
}

impl<T: Destroy> Destroy for Option<T> {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        if let Some(item) = self.as_mut() {
            unsafe {
                item.destroy(ctx);
            }
        }
    }
}


pub trait Recreate: Destroy {
    type Item;

    /// recreates the Vulkan resource
    unsafe fn recreate<F>(&mut self, ctx: &VulkanCtx, f: F) -> anyhow::Result<()>
    where
        F: FnMut() -> anyhow::Result<Self::Item>;
}

// TODO: impl Recreate for Vec<T: Recreate>


fn create_dev(
    inst: &ash::Instance,
    sel: &GPU,
) -> anyhow::Result<ash::Device> {
    let qfam_idxs = sel.uniq_idxs();
    let prios = [1.0f32];
    let mut qu_cis = Vec::with_capacity(qfam_idxs.len());
    for idx in qfam_idxs {
        qu_cis.push(vk::DeviceQueueCreateInfo::default()
            .queue_family_index(idx)
            .queue_priorities(&prios));
    }

    let features = vk::PhysicalDeviceFeatures::default()
        .shader_int64(true);

    let mut features_12 = vk::PhysicalDeviceVulkan12Features::default()
        // bindless
        .descriptor_binding_partially_bound(true)
        .runtime_descriptor_array(true)
        .shader_sampled_image_array_non_uniform_indexing(true)
        .descriptor_binding_sampled_image_update_after_bind(true)
        .descriptor_binding_storage_image_update_after_bind(true)

        // bda
        .buffer_device_address(true)
        .vulkan_memory_model(true);

    let mut features_13 = vk::PhysicalDeviceVulkan13Features::default()
        .synchronization2(true)
        .dynamic_rendering(true);

    let dev_ci = vk::DeviceCreateInfo::default()
        .queue_create_infos(&qu_cis)
        .enabled_extension_names(&DEV_EXTS)
        .enabled_features(&features)
        .push_next(&mut features_12)
        .push_next(&mut features_13);

    unsafe { Ok(inst.create_device(sel.pdev, &dev_ci, None)?) }
}

pub struct VulkanCtx {
    pub gpu: GPU,
    pub surf: Surface,
    pub alloc: ManuallyDrop<Mutex<Allocator>>,

    pub graphics_qu: vk::Queue,
    pub compute_qu: vk::Queue,
    pub transfer_qu: vk::Queue,

    pub dev: ash::Device,
    pub inst: ash::Instance,

    // TMP: debug utils
    pub debug: DebugUtils,

    // this is the only thing here that has a Drop impl
    pub entry: ash::Entry,
}

impl VulkanCtx {
    pub fn new(
        raw_disp: RawDisplayHandle,
        raw_win: RawWindowHandle,
        win: Arc<Window>,
    ) -> anyhow::Result<Self> {
        let entry = unsafe { ash::Entry::load()? };

        let req_exts = ash_window::enumerate_required_extensions(raw_disp)
            .expect("failed to get Vulkan extensions");

        let mut exts = vec![];
        for ext in req_exts {
            exts.push(*ext);
        }
        exts.push(debug_utils::NAME.as_ptr());

        let app_info = vk::ApplicationInfo::default()
            .api_version(vk::API_VERSION_1_3);

        let inst_ci = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&exts);

        let inst = unsafe { entry.create_instance(&inst_ci, None)? };

        let surf = Surface::new(&entry, &inst, raw_disp, raw_win, win)?;
        let gpu = GPU::select(&inst, &surf)?;
        let dev = create_dev(&inst, &gpu)?;

        let alloc = Allocator::new(&AllocatorCreateDesc {
            instance: inst.clone(),
            device: dev.clone(),
            physical_device: gpu.pdev,
            debug_settings: AllocatorDebugSettings::default(),
            buffer_device_address: true,
            allocation_sizes: AllocationSizes::default(),
        })?;

        let (graphics_qu, compute_qu, transfer_qu) = gpu.get_qus(&dev);

        let debug = DebugUtils {
            inst: debug_utils::Instance::new(&entry, &inst),
            dev: debug_utils::Device::new(&inst, &dev),
        };

        Ok(Self {
            gpu,
            surf,
            graphics_qu,
            compute_qu,
            transfer_qu,
            dev,
            inst,
            alloc: ManuallyDrop::new(Mutex::new(alloc)),
            debug,
            entry,
        })
    }

    pub unsafe fn barrier<const N: usize>(
        &self,
        cmd: vk::CommandBuffer,
        barriers: [vk::ImageMemoryBarrier2<'_>; N],
    ) {
        unsafe {
            self.dev.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default()
                    .image_memory_barriers(&barriers),
            );
        }
    }
}

impl Drop for VulkanCtx {
    fn drop(&mut self) {
        // entry is auto dropped AFTER this is called
        unsafe {
            let _ = self.dev.device_wait_idle();
            ManuallyDrop::drop(&mut self.alloc);
            self.dev.destroy_device(None);
            self.surf.destroy_surf();
            self.inst.destroy_instance(None);
        }
    }
}

pub struct DebugUtils {
    pub inst: debug_utils::Instance,
    pub dev: debug_utils::Device,
}

impl DebugUtils {
    pub fn begin_label(
        &self,
        cmd: vk::CommandBuffer,
        name: &std::ffi::CStr,
    ) {
        unsafe {
            self.dev.cmd_begin_debug_utils_label(
                cmd,
                &vk::DebugUtilsLabelEXT::default().label_name(name),
            );
        }
    }

    pub fn end_label(
        &self,
        cmd: vk::CommandBuffer,
    ) {
        unsafe {
            self.dev.cmd_end_debug_utils_label(cmd);
        }
    }

    pub fn set_name(
        &self,
        handle: impl vk::Handle,
        name: &std::ffi::CStr,
    ) -> anyhow::Result<()> {
        unsafe {
            self.dev.set_debug_utils_object_name(
                &vk::DebugUtilsObjectNameInfoEXT::default()
                    .object_handle(handle)
                    .object_name(name)
            )?;
        }

        Ok(())
    }
}

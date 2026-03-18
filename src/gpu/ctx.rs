// TODO: replace _ci, _ai naming convention everywhere with _info
use std::{ffi::c_char, mem::ManuallyDrop, sync::{Arc, Mutex}};

use ash::vk;
use gpu_allocator::{AllocationSizes, AllocatorDebugSettings, vulkan::{Allocator, AllocatorCreateDesc}};
use winit::{raw_window_handle::{RawDisplayHandle, RawWindowHandle}, window::Window};

use crate::gpu::{GPU, surf::Surface};

const DEV_EXTS: [*const c_char; 1] = [ash::khr::swapchain::NAME.as_ptr()];

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

    let mut vk12 = vk::PhysicalDeviceVulkan12Features::default()
        .buffer_device_address(true)
        .vulkan_memory_model(true);

    let mut vk13 = vk::PhysicalDeviceVulkan13Features::default()
        .synchronization2(true)
        .dynamic_rendering(true);

    let dev_ci = vk::DeviceCreateInfo::default()
        .queue_create_infos(&qu_cis)
        .enabled_extension_names(&DEV_EXTS)
        .push_next(&mut vk12)
        .push_next(&mut vk13);

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

        let app_info = vk::ApplicationInfo::default()
            .api_version(vk::API_VERSION_1_3);

        let inst_ci = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(req_exts);

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

        Ok(Self {
            gpu,
            surf,
            graphics_qu,
            compute_qu,
            transfer_qu,
            dev,
            inst,
            alloc: ManuallyDrop::new(Mutex::new(alloc)),
            entry,
        })
    }
}

impl Drop for VulkanCtx {
    fn drop(&mut self) {
        // entry is dropped by the compiler after this is done
        unsafe {
            let _ = self.dev.device_wait_idle();
            ManuallyDrop::drop(&mut self.alloc);
            self.dev.destroy_device(None);
            self.surf.destroy();
            self.inst.destroy_instance(None);
        }
    }
}

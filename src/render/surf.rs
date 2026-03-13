use std::sync::Arc;

use ash::{khr, vk};
use winit::{raw_window_handle::{RawDisplayHandle, RawWindowHandle}, window::Window};

pub struct Surface {
    pub handle: vk::SurfaceKHR,
    pub loader: khr::surface::Instance,
    win: Arc<Window>, // keep the native handles alive until we are dropped
}

impl Surface {
    pub fn new(
        entry: &ash::Entry,
        inst: &ash::Instance,
        raw_disp: &RawDisplayHandle,
        raw_win: &RawWindowHandle,
        win: Arc<Window>,
    ) -> anyhow::Result<Self> {
        let handle = unsafe {
            ash_window::create_surface(entry, inst, *raw_disp, *raw_win, None)?
        };
        let loader = khr::surface::Instance::new(entry, inst);
        Ok(Self { handle, loader, win })
    }

    pub fn destroy(&self) {
        unsafe {
            self.loader.destroy_surface(self.handle, None);
        }
    }

    pub fn physical_device_support(
        &self,
        pdev: vk::PhysicalDevice,
        queue_family: u32,
    ) -> bool {
        unsafe {
            self.loader
                .get_physical_device_surface_support(pdev, queue_family, self.handle)
                .unwrap_or(false)
        }
    }
}

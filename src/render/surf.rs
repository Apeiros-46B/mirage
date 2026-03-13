use std::sync::Arc;
use ash::{khr, vk};
use winit::window::Window;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub struct Surface {
    pub handle: vk::SurfaceKHR,
    pub loader: khr::surface::Instance,
}

impl Surface {
    pub fn new(
        entry: &ash::Entry,
        inst: &ash::Instance,
        window: &Arc<Window>,
    ) -> anyhow::Result<Self> {
        let handle = unsafe {
            ash_window::create_surface(
                entry,
                inst,
                window.display_handle()?.as_raw(),
                window.window_handle()?.as_raw(),
                None,
            )?
        };

        let loader = khr::surface::Instance::new(entry, inst);

        Ok(Self { handle, loader })
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

use std::sync::Arc;

use ash::{khr, prelude::*, vk};
use winit::window::Window;

use crate::{render::GPU, sync::RenderInit};

pub struct Surface {
    pub handle: vk::SurfaceKHR,
    pub loader: khr::surface::Instance,
    _win: Arc<Window>, // keep the native handles alive until we are dropped
}

impl Surface {
    pub fn new(
        entry: &ash::Entry,
        inst: &ash::Instance,
        init: &RenderInit,
    ) -> anyhow::Result<Self> {
        let handle = unsafe {
            ash_window::create_surface(
                entry,
                inst,
                init.raw_disp,
                init.raw_win,
                None,
            )?
        };
        let loader = khr::surface::Instance::new(entry, inst);
        Ok(Self {
            handle,
            loader,
            _win: init.win.clone(),
        })
    }

    pub fn destroy(self) {
        unsafe {
            self.loader.destroy_surface(self.handle, None);
        }
    }

    pub fn pdev_support(
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

    pub fn get_caps(&self, gpu: &GPU) -> VkResult<vk::SurfaceCapabilitiesKHR> {
        unsafe {
            self.loader.get_physical_device_surface_capabilities(
                gpu.pdev,
                self.handle,
            )
        }
    }

    pub fn get_modes(&self, gpu: &GPU) -> VkResult<Vec<vk::PresentModeKHR>> {
        unsafe {
            self.loader.get_physical_device_surface_present_modes(
                gpu.pdev,
                self.handle,
            )
        }
    }

    pub fn get_fmts(&self, gpu: &GPU) -> VkResult<Vec<vk::SurfaceFormatKHR>> {
        unsafe {
            self.loader.get_physical_device_surface_formats(
                gpu.pdev,
                self.handle,
            )
        }
    }
}

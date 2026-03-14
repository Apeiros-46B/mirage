use std::sync::Arc;

use ash::{khr, prelude::*, vk};
use winit::{raw_window_handle::{RawDisplayHandle, RawWindowHandle}, window::Window};

use crate::gpu::GPU;

pub struct Surface {
    pub handle: vk::SurfaceKHR,
    pub loader: khr::surface::Instance,
    _win: Arc<Window>, // keep the native handles alive until we are dropped
}

impl Surface {
    pub fn new(
        entry: &ash::Entry,
        inst: &ash::Instance,
        raw_disp: RawDisplayHandle,
        raw_win: RawWindowHandle,
        win: Arc<Window>,
    ) -> anyhow::Result<Self> {
        let handle = unsafe {
            ash_window::create_surface(
                entry,
                inst,
                raw_disp,
                raw_win,
                None,
            )?
        };
        let loader = khr::surface::Instance::new(entry, inst);
        Ok(Self {
            handle,
            loader,
            _win: win,
        })
    }

    pub unsafe fn destroy(&mut self) {
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

use std::sync::Arc;

use ash::{khr, vk};

use crate::render::{ctx::GPU, surf::Surface};

pub struct Swapchain {
    pub handle: vk::SwapchainKHR,
    pub loader: khr::swapchain::Device,

    surf: Surface,
    ci: vk::SwapchainCreateInfoKHR<'static>,

    img_idx: Option<u32>,
    imgs: Vec<vk::Image>,
    img_views: Vec<vk::ImageView>,
    render_sems: Vec<vk::Semaphore>, // signalled when render done
}

impl Swapchain {
    pub fn new(
        gpu: &GPU,
        dev: &ash::Device,
        surf: Arc<Surface>,
    ) -> anyhow::Result<Self> {
        let surf_caps = surf.get_caps(gpu);
        let surf_modes = surf.get_modes(gpu);
        let surf_fmts = surf.get_fmts(gpu);

        dbg!(&surf_caps);
        dbg!(&surf_modes);
        dbg!(&surf_fmts);

        anyhow::bail!("ok");
    }

    pub fn destroy(self) {

    }
}

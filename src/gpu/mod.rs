use anyhow::anyhow;
use ash::vk;

use crate::gpu::surf::Surface;

pub mod ctx;
pub mod surf;

#[derive(Debug, Clone)]
pub struct GPU {
    pub pdev: vk::PhysicalDevice,
    props: vk::PhysicalDeviceProperties,
    score: u64,

    pub graphics_idx: u32,
    pub compute_idx: u32,
    pub transfer_idx: u32,
}

impl GPU {
    pub fn uniq_idxs(&self) -> Vec<u32> {
        let mut seen = std::collections::HashSet::new();
        [self.graphics_idx, self.compute_idx, self.transfer_idx]
            .into_iter()
            .filter(|idx| seen.insert(*idx))
            .collect()
    }

    pub fn get_qus(&self, dev: &ash::Device) -> (vk::Queue, vk::Queue, vk::Queue) {
        unsafe {(
            dev.get_device_queue(self.graphics_idx, 0),
            dev.get_device_queue(self.compute_idx, 0),
            dev.get_device_queue(self.transfer_idx, 0),
        )}
    }

    // {{{ select physical device and queue families
    pub fn select(
        inst: &ash::Instance,
        surf: &Surface,
    ) -> anyhow::Result<Self> {
        unsafe { inst.enumerate_physical_devices()? }
            .into_iter()
            .filter_map(|pdev| {
                let qfams = unsafe {
                    inst.get_physical_device_queue_family_properties(pdev)
                };

                let graphics_idx = qfams
                    .iter()
                    .position(|qfam| qfam.queue_flags.contains(
                        vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE,
                    ))?
                    as u32;
                let present_idx = qfams
                    .iter()
                    .enumerate()
                    .position(|(idx, _)| surf.pdev_support(pdev, idx as u32))?
                    as u32;

                if graphics_idx != present_idx {
                    // don't support the fringe case of graphics being a different queue
                    // from presentation
                    return None;
                }

                let compute_idx = find_dedicated_qu(&qfams, vk::QueueFlags::COMPUTE)?;
                let transfer_idx = find_dedicated_qu(&qfams, vk::QueueFlags::TRANSFER)?;

                let props = unsafe { inst.get_physical_device_properties(pdev) };
                let mut score: u64 = match props.device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 100_000_000,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 10_000_000,
                    _ => 1_000_000,
                };

                let mem = unsafe { inst.get_physical_device_memory_properties(pdev) };
                let vram_mb = mem.memory_heaps[..mem.memory_heap_count as usize]
                    .iter()
                    .filter(|heap| heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
                    .map(|heap| heap.size / (1024 * 1024))
                    .sum::<vk::DeviceSize>();
                score += vram_mb;

                // reward devices that gave us dedicated queues
                if compute_idx != graphics_idx {
                    score += 50_000;
                }
                if transfer_idx != graphics_idx && transfer_idx != compute_idx {
                    score += 25_000;
                }

                Some(GPU {
                    pdev,
                    props,
                    score,
                    graphics_idx,
                    compute_idx,
                    transfer_idx
                })
            })
            .max_by_key(|gpu| gpu.score)
            .ok_or_else(|| anyhow!("no suitable device"))
    }
    // }}}
}

fn find_dedicated_qu(
    qfams: &[vk::QueueFamilyProperties],
    targ_flag: vk::QueueFlags,
) -> Option<u32> {
    let mut best_idx = None;
    let mut min_extra_flags = u32::MAX;

    for (idx, qfam) in qfams.iter().enumerate() {
        let flags = qfam.queue_flags;

        if flags.contains(targ_flag) {
            // count number of extra flags
            // SPARSE_BINDING doesn't count as extra flag
            let extra_flags = flags & !targ_flag & !vk::QueueFlags::SPARSE_BINDING;
            let extra_count = extra_flags.as_raw().count_ones();

            // perfect queue
            if extra_count == 0 {
                return Some(idx as u32);
            }

            // find queue with minimum number of extra flags
            if extra_count < min_extra_flags {
                min_extra_flags = extra_count;
                best_idx = Some(idx as u32);
            }
        }
    }

    best_idx
}

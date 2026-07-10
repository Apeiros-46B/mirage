use std::sync::Arc;

use ash::vk;

use crate::gpu::{ctx::{Destroy, VulkanCtx}, img::AllocatedImage};

pub struct DescriptorBindInfo {
    pub set: vk::DescriptorSet,
    pub layout: vk::DescriptorSetLayout,
}

// TODO: no way to free, but we don't need that yet since no dynamic resources
pub struct DescriptorAllocator {
    ctx: Arc<VulkanCtx>,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    layout: vk::DescriptorSetLayout,

    sampler_count: u32,
    sampled_img_count: u32,
    storage_img_count: u32,
}

impl Destroy for DescriptorAllocator {
    unsafe fn destroy(&mut self, ctx: &VulkanCtx) {
        unsafe {
            ctx.dev.destroy_descriptor_pool(self.pool, None);
            ctx.dev.destroy_descriptor_set_layout(self.layout, None);
        }
    }
}

const SAMPLER_BINDING: u32 = 0;
const SAMPLED_IMG_BINDING: u32 = 1;
const STORAGE_IMG_BINDING: u32 = 2;

impl DescriptorAllocator {
    pub fn new(ctx: Arc<VulkanCtx>) -> anyhow::Result<Self> {
        let pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(100_000),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(100_000),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(100_000),
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::UPDATE_AFTER_BIND)
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        let pool = unsafe { ctx.dev.create_descriptor_pool(&pool_info, None)? };

        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(SAMPLER_BINDING)
                // TODO: probably update this when we need sampling in fsh
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(100_000),
            vk::DescriptorSetLayoutBinding::default()
                .binding(SAMPLED_IMG_BINDING)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(100_000),
            vk::DescriptorSetLayoutBinding::default()
                .binding(STORAGE_IMG_BINDING)
                .stage_flags(vk::ShaderStageFlags::COMPUTE)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(100_000),
        ];
        let flags = vk::DescriptorBindingFlags::PARTIALLY_BOUND
                  | vk::DescriptorBindingFlags::UPDATE_AFTER_BIND;
        let flags_array = [flags, flags, flags];
        let mut binding_info = vk::DescriptorSetLayoutBindingFlagsCreateInfo::default()
            .binding_flags(&flags_array);
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL)
            .bindings(&bindings)
            .push_next(&mut binding_info);
        let layout = unsafe {
            ctx.dev.create_descriptor_set_layout(&layout_info, None)?
        };
        ctx.debug.set_name(layout, c"main set layout")?;

        let desc_set_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(std::slice::from_ref(&layout));
        let desc_sets = unsafe { ctx.dev.allocate_descriptor_sets(&desc_set_info)? };
        ctx.debug.set_name(desc_sets[0], c"main set")?;

        Ok(Self {
            ctx,
            pool,
            set: desc_sets[0],
            layout,

            sampler_count: 0,
            sampled_img_count: 0,
            storage_img_count: 0,
        })
    }

    pub fn get_bind_info(&self) -> DescriptorBindInfo {
        DescriptorBindInfo {
            set: self.set,
            layout: self.layout,
        }
    }

    pub fn alloc_sampler(&mut self, sampler: vk::Sampler) -> u32 {
        let idx = self.sampler_count;
        self.sampler_count += 1;

        let info = vk::DescriptorImageInfo::default()
            .sampler(sampler);
        let write = vk::WriteDescriptorSet::default()
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .dst_set(self.set)
            .dst_binding(0) // binding point for samplers
            .dst_array_element(idx)
            .image_info(std::slice::from_ref(&info));

        unsafe { self.ctx.dev.update_descriptor_sets(&[write], &[]); }

        idx
    }

    fn update_imgs<const N: usize>(
        ctx: &VulkanCtx,
        idx_start: u32,

        ty: vk::DescriptorType,
        desc_set: vk::DescriptorSet,
        binding: u32,

        layout: vk::ImageLayout,
        imgs: &[AllocatedImage; N],
    ) {
        let info: [vk::DescriptorImageInfo; N] = std::array::from_fn(|i| {
            let img = &imgs[i];
            vk::DescriptorImageInfo::default()
                .image_view(img.view)
                .image_layout(layout)
        });

        // TODO: possibly use a template
        let write = vk::WriteDescriptorSet::default()
            .descriptor_type(ty)
            .descriptor_count(N as u32)
            .dst_set(desc_set)
            .dst_binding(binding)
            .dst_array_element(idx_start)
            .image_info(&info);

        unsafe {
            ctx.dev.update_descriptor_sets(&[write], &[]);
        }
    }

    fn alloc_imgs<const N: usize>(
        ctx: &VulkanCtx,
        cursor: &mut u32,

        ty: vk::DescriptorType,
        desc_set: vk::DescriptorSet,
        binding: u32,

        layout: vk::ImageLayout,
        imgs: &[AllocatedImage; N],
    ) -> [u32; N] {
        let idx_start = *cursor;
        *cursor += N as u32;
        Self::update_imgs(ctx, idx_start, ty, desc_set, binding, layout, imgs);
        std::array::from_fn(|i| idx_start + i as u32)
    }

    pub fn alloc_sampled_imgs<const N: usize>(
        &mut self,
        imgs: &[AllocatedImage; N],
    ) -> [u32; N] {
        Self::alloc_imgs(
            &self.ctx,
            &mut self.sampled_img_count,
            vk::DescriptorType::SAMPLED_IMAGE,
            self.set,
            1, // binding point for sampled images
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            imgs,
        )
    }

    pub fn alloc_storage_imgs<const N: usize>(
        &mut self,
        imgs: &[AllocatedImage; N],
    ) -> [u32; N] {
        Self::alloc_imgs(
            &self.ctx,
            &mut self.storage_img_count,
            vk::DescriptorType::STORAGE_IMAGE,
            self.set,
            2, // binding point for storage images
            vk::ImageLayout::GENERAL,
            imgs,
        )
    }

    pub fn update_sampled_imgs<const N: usize>(
        &mut self,
        idx_start: u32,
        imgs: &[AllocatedImage; N],
    ) {
        Self::update_imgs(
            &self.ctx,
            idx_start,
            vk::DescriptorType::SAMPLED_IMAGE,
            self.set,
            1, // binding point for sampled images
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            imgs,
        )
    }

    pub fn update_storage_imgs<const N: usize>(
        &mut self,
        idx_start: u32,
        imgs: &[AllocatedImage; N],
    ) {
        Self::update_imgs(
            &self.ctx,
            idx_start,
            vk::DescriptorType::STORAGE_IMAGE,
            self.set,
            2, // binding point for storage images
            vk::ImageLayout::GENERAL,
            imgs,
        )
    }
}

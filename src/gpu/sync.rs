use ash::vk;

pub trait ImageBarrierExt {
    fn next_phase(
        self,
        new_layout: vk::ImageLayout,
        dst_stage: vk::PipelineStageFlags2,
        dst_access: vk::AccessFlags2,
    ) -> Self;

    fn dep_info(&self) -> vk::DependencyInfo<'_>;
}

impl ImageBarrierExt for vk::ImageMemoryBarrier2<'_> {
    fn next_phase(
        mut self,
        new_layout: vk::ImageLayout,
        dst_stage: vk::PipelineStageFlags2,
        dst_access: vk::AccessFlags2,
    ) -> Self {
        self.old_layout = self.new_layout;
        self.src_stage_mask = self.dst_stage_mask;
        self.src_access_mask = self.dst_access_mask;

        self.new_layout = new_layout;
        self.dst_stage_mask = dst_stage;
        self.dst_access_mask = dst_access;

        self
    }

    fn dep_info(&self) -> vk::DependencyInfo<'_> {
        vk::DependencyInfo::default()
            .image_memory_barriers(std::slice::from_ref(self))
    }
}

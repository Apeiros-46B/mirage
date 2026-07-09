use ash::vk;

pub trait ImageBarrierExt {
    fn begin(
        self,
        layout: vk::ImageLayout,
        stage: vk::PipelineStageFlags2,
        access: vk::AccessFlags2,
    ) -> Self;

    fn next_phase(
        self,
        new_layout: vk::ImageLayout,
        dst_stage: vk::PipelineStageFlags2,
        dst_access: vk::AccessFlags2,
    ) -> Self;

    fn transition(
        &mut self,
        new_layout: vk::ImageLayout,
        dst_stage: vk::PipelineStageFlags2,
        dst_access: vk::AccessFlags2,
    );

    fn dep_info(&self) -> vk::DependencyInfo<'_>;
}

impl ImageBarrierExt for vk::ImageMemoryBarrier2<'_> {
    fn begin(
        self,
        layout: vk::ImageLayout,
        stage: vk::PipelineStageFlags2,
        access: vk::AccessFlags2,
    ) -> Self {
        self
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(layout)
            .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
            .dst_stage_mask(stage)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_access_mask(access)
    }

    fn next_phase(
        mut self,
        new_layout: vk::ImageLayout,
        dst_stage: vk::PipelineStageFlags2,
        dst_access: vk::AccessFlags2,
    ) -> Self {
        self.transition(new_layout, dst_stage, dst_access);
        self
    }

    fn transition(
        &mut self,
        new_layout: vk::ImageLayout,
        dst_stage: vk::PipelineStageFlags2,
        dst_access: vk::AccessFlags2,
    ) {
        self.old_layout = self.new_layout;
        self.src_stage_mask = self.dst_stage_mask;
        self.src_access_mask = self.dst_access_mask;

        self.new_layout = new_layout;
        self.dst_stage_mask = dst_stage;
        self.dst_access_mask = dst_access;
    }

    fn dep_info(&self) -> vk::DependencyInfo<'_> {
        vk::DependencyInfo::default()
            .image_memory_barriers(std::slice::from_ref(self))
    }
}

pub trait BufferBarrierExt {
    fn dep_info(&self) -> vk::DependencyInfo<'_>;
}

impl BufferBarrierExt for vk::BufferMemoryBarrier2<'_> {
    fn dep_info(&self) -> vk::DependencyInfo<'_> {
        vk::DependencyInfo::default()
            .buffer_memory_barriers(std::slice::from_ref(self))
    }
}

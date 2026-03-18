// TODO: in the future, the streaming thread will hold its own cmd pool
// once it's done all uploads, it should send the cmd buf to render thread to dispatch?
// it should
// - take Arc<VulkanCtx> from main during init
// - send command buffer handles to the render thread to be dispatched (maybe fine if it's a separate queue, idk)
// - on shutdown, receive message from main and join so that the Arc<Ctx> is dropped properly (same as renderer)

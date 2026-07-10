// TODO: in the future, the streaming thread will hold its own cmd pool
// once it's done all uploads, it should send the cmd buf to render thread to dispatch?
// it should
// - take Arc<VulkanCtx> from main during init
// - send command buffer handles to the render thread to be dispatched (maybe fine if it's a separate queue, idk)
// - on shutdown, receive message from main and join so that the Arc<Ctx> is dropped properly (same as renderer)

use anyhow::Result;
use std::sync::Arc;

use crate::{
    gpu::{alloc::GPUArena, ctx::{Destroy, VulkanCtx}, task::Task},
    sync::{chan::SafeSend, worker::{MessageHandler, TickTiming, Worker}},
};

pub struct LoaderInit {
    pub ctx: Arc<VulkanCtx>,
}

pub struct LoaderState {
    arena: GPUArena,
    ctx: Arc<VulkanCtx>,
}

#[derive(Default)]
pub struct Loader;

// impl Worker for Loader {
//     type Init = LoaderInit;
//     type State = LoaderState;
//     type Channels = LoaderRxBundle;
//
//     fn on_resume(&self, init: Self::Init) -> Result<Self::State> {
//         let ctx = init.ctx;
//         let arena = GPUArena::new(ctx.clone(), 16 * 1024 * 1024)?;
//         Ok(LoaderState { arena, ctx })
//     }
//
//     fn on_tick(
//         this: &mut Self::State,
//         tick: &TickTiming,
//     ) -> Result<()> {
//         Ok(())
//     }
// }

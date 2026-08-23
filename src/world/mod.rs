use bytemuck::NoUninit;
use modular_bitfield::{Specifier, bitfield, prelude::*};

pub const CHUNK_WS_ID_BITS: usize = 20;
pub const REALM_WS_ID_BITS: usize = 3;
pub const PORTALS_PER_CHUNK: usize = 32;
pub const CHUNK_WS_N: usize = 1 << CHUNK_WS_ID_BITS;
pub const REALM_WS_N: usize = 1 << REALM_WS_ID_BITS;

// max 2 bits
#[derive(Debug, Specifier)]
pub enum ChunkState {
    Empty = 0,
    Loading = 1,
    Solid = 2,
    Loaded = 3,
}

#[bitfield(bits = 64)]
#[derive(Debug, Clone, Copy, NoUninit)]
#[repr(C)]
pub struct ChunkMetadata {
    pub x: B12,
    pub y: B12,
    pub z: B12,
    pub portal_heap_ofs: B18,
    pub realm_ws_id: B8,

    #[bits = 2]
    pub state: ChunkState,
}

#[bitfield(bits = 128)]
#[derive(Debug, Clone, Copy, NoUninit)]
#[repr(C)]
pub struct Node {
    pub child_mask: u64,
    payload: B32,
    pub pad: B32,
}

impl Node {
    // branch accessors (child_mask != 0)
    pub fn child_ofs(&self) -> u32 {
        self.payload()
    }
    pub fn with_child_ofs(mut self, ofs: u32) -> Self {
        self.set_payload(ofs);
        self
    }

    // leaf accessors (child_mask == 0)
    pub fn mat_id(&self) -> u16 {
        (self.payload() & 0xFFFF) as u16
    }
    pub fn with_mat_id(mut self, id: u16) -> Self {
        let p = self.payload() & 0xFFFF_0000;
        self.set_payload(p | id as u32);
        self
    }
}

#[bitfield(bits = 64)]
#[derive(Debug, Clone, Copy, NoUninit)]
#[repr(C)]
pub struct VisbufKey {
    pub chunk_id: B20,
    pub local_x: B8,
    pub local_y: B8,
    pub local_z: B8,
    pub lod: B2,
    pub face: B3,
    pub mat_class: B1,
    pub pad: B14,
}

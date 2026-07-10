use bytemuck::NoUninit;
use modular_bitfield::{Specifier, bitfield, specifiers::*};

pub const PORTALS_PER_CHUNK: usize = 32;
pub const CHUNK_WS_ID_BITS: usize = 22;
pub const REALM_WS_ID_BITS: usize = 8;

// max 2 bits
#[derive(Debug, Specifier)]
pub enum ChunkState {
    Empty = 0,
    Loading = 1,
    Solid = 2,
    Loaded = 3,
}

#[bitfield(bits = 128)]
#[derive(Debug, Clone, Copy, NoUninit)]
#[repr(C)]
pub struct Chunk {
    /// bda ptr to Node
    pub root: u64,

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
    // childPtr
    pub is_leaf: B1,
    pub child_ofs: B31,

    // material
    pub is_homogeneous: B1, // bit 0
    pub mat_id: B16,        // bits 1-16
    pub mat_pad: B15,       // (extra unused MSBs)

    pub child_mask: u64,
}

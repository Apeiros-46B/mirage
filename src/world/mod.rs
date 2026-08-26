// TODO: tree build is vibed
use bytemuck::NoUninit;
use modular_bitfield::{Specifier, bitfield, prelude::*};

pub const CHUNK_WS_ID_BITS: usize = 20;
pub const REALM_WS_ID_BITS: usize = 3;
pub const PORTALS_PER_CHUNK: usize = 32;
pub const CHUNK_WS_N: usize = 1 << CHUNK_WS_ID_BITS;
pub const REALM_WS_N: usize = 1 << REALM_WS_ID_BITS;

const CHUNK_SIDE: usize = 256;
const OCCUPANCY_WORDS: usize = CHUNK_SIDE * CHUNK_SIDE * CHUNK_SIDE / u64::BITS as usize;
const NODE_PTR_MASK: u32 = 0x7FFF_FFFF;
const NODE_LEAF_BIT: u32 = 0x8000_0000;

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
    pub fn is_leaf(&self) -> bool {
        self.payload() & NODE_LEAF_BIT != 0
    }

    pub fn with_is_leaf(mut self, is_leaf: bool) -> Self {
        let payload = if is_leaf {
            self.payload() | NODE_LEAF_BIT
        } else {
            self.payload() & !NODE_LEAF_BIT
        };
        self.set_payload(payload);
        self
    }

    // branch accessors
    pub fn child_ofs(&self) -> u32 {
        self.payload() & NODE_PTR_MASK
    }
    pub fn with_child_ofs(mut self, ofs: u32) -> Self {
        assert!(ofs <= NODE_PTR_MASK, "64-tree node offset exceeds 31 bits");
        self.set_payload(ofs);
        self
    }

    // leaf accessors
    pub fn mat_id(&self) -> u16 {
        (self.payload() & 0xFFFF) as u16
    }
    pub fn with_mat_id(mut self, id: u16) -> Self {
        let payload = self.payload() & 0xFFFF_0000;
        self.set_payload(payload | id as u32);
        self
    }
}

pub struct OccupancyGrid {
    // heap allocated, too large for stack
    bits: Box<[u64]>,
}

impl OccupancyGrid {
    fn get(&self, x: usize, y: usize, z: usize) -> bool {
        let bit_idx = x + CHUNK_SIDE * (z + CHUNK_SIDE * y);
        self.bits[bit_idx / u64::BITS as usize]
            & (1 << (bit_idx % u64::BITS as usize))
            != 0
    }
}

pub fn build_occupancy_grid<F>(mut is_occupied: F) -> OccupancyGrid
where
    F: FnMut(u8, u8, u8) -> bool,
{
    let mut bits = vec![0u64; OCCUPANCY_WORDS].into_boxed_slice();

    for y in 0..CHUNK_SIDE {
        for z in 0..CHUNK_SIDE {
            for x in 0..CHUNK_SIDE {
                if is_occupied(x as u8, y as u8, z as u8) {
                    let bit_idx = x + CHUNK_SIDE * (z + CHUNK_SIDE * y);
                    bits[bit_idx / u64::BITS as usize] |=
                        1 << (bit_idx % u64::BITS as usize);
                }
            }
        }
    }

    OccupancyGrid { bits }
}

pub fn build_64_tree(grid: &OccupancyGrid) -> Vec<Node> {
    let mut nodes = Vec::with_capacity(266_305);
    nodes.push(make_leaf(0));

    nodes[0] = match build_region(grid, &mut nodes, [0; 3], CHUNK_SIDE) {
        Region::Empty => make_leaf(0),
        Region::Solid => make_leaf(u64::MAX),
        Region::Node(node) => node,
    };
    nodes
}

#[derive(Clone, Copy)]
enum Region {
    Empty,
    Solid,
    Node(Node),
}

fn build_region(
    grid: &OccupancyGrid,
    nodes: &mut Vec<Node>,
    origin: [usize; 3],
    side: usize,
) -> Region {
    if side == 1 {
        return if grid.get(origin[0], origin[1], origin[2]) {
            Region::Solid
        } else {
            Region::Empty
        };
    }

    let child_side = side / 4;
    let mut children = [Region::Empty; 64];
    let mut child_mask = 0u64;
    let mut has_branch = false;

    for child_idx in 0..64 {
        let child_origin = [
            origin[0] + (child_idx & 3) * child_side,
            origin[1] + ((child_idx >> 4) & 3) * child_side,
            origin[2] + ((child_idx >> 2) & 3) * child_side,
        ];
        let child = build_region(grid, nodes, child_origin, child_side);

        if !matches!(child, Region::Empty) {
            child_mask |= 1 << child_idx;
        }
        has_branch |= matches!(child, Region::Node(_));
        children[child_idx] = child;
    }

    if !has_branch {
        return match child_mask {
            0 => Region::Empty,
            u64::MAX => Region::Solid,
            _ => Region::Node(make_leaf(child_mask)),
        };
    }

    let child_ofs = nodes.len();
    assert!(
        child_ofs <= NODE_PTR_MASK as usize,
        "64-tree exceeds 31-bit node offsets",
    );

    for child in children {
        match child {
            Region::Empty => {},
            Region::Solid => nodes.push(make_leaf(u64::MAX)),
            Region::Node(node) => nodes.push(node),
        }
    }

    Region::Node(Node::new()
        .with_child_mask(child_mask)
        .with_child_ofs(child_ofs as u32),
    )
}

fn make_leaf(child_mask: u64) -> Node {
    Node::new()
        .with_child_mask(child_mask)
        .with_is_leaf(true)
        .with_mat_id(u16::from(child_mask != 0))
}

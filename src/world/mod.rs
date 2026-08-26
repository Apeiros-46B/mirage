// TODO: tree build is vibed
use bytemuck::{NoUninit, Pod, Zeroable};
use modular_bitfield::{Specifier, bitfield, prelude::*};

pub const CHUNK_ID_BITS: usize = 20;
pub const REALM_ID_BITS: usize = 3;
pub const PORTALS_PER_CHUNK: usize = 32;
pub const CHUNK_N: usize = 1 << CHUNK_ID_BITS;
pub const REALM_N: usize = 1 << REALM_ID_BITS;

const CHUNK_SIDE: usize = 256;
const CHUNK_PAGE_SIDE: usize = 16;
const CHUNK_PAGE_ENTRIES: usize = CHUNK_PAGE_SIDE.pow(3);
const CHUNK_STATE_SHIFT: u32 = 30;
const OCCUPANCY_WORDS: usize = CHUNK_SIDE * CHUNK_SIDE * CHUNK_SIDE / u64::BITS as usize;
const NODE_PTR_MASK: u32 = 0x7FFF_FFFF;
const NODE_LEAF_BIT: u32 = 0x8000_0000;

// max 2 bits
#[derive(Clone, Copy, Debug, Specifier)]
pub enum ChunkState {
    Empty = 0,
    Solid = 1,
    Loaded = 2,
    _Reserved = 3,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct ChunkDirectory {
    pub pages: u64,
    pub roots: u64,
}

#[bitfield(bits = 64)]
#[derive(Debug, Clone, Copy, NoUninit)]
#[repr(C)]
pub struct ChunkMetadata {
    pub x: B12,
    pub y: B12,
    pub z: B12,
    pub portal_heap_ofs: B18,
    pub realm_id: B8,

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
    pub fn new_leaf(child_mask: u64) -> Self {
        Self::new()
            .with_child_mask(child_mask)
            .with_is_leaf(true)
            .with_mat_id(u16::from(child_mask != 0)) // TODO(material)
    }

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

// TODO(loader): this is a test, needs replacement
// {{{ CPU-side world building
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

    pub fn new<F>(mut is_occupied: F) -> Self
    where
        F: FnMut(u8, u8, u8) -> bool
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

        Self { bits }
    }

    pub fn build_tree(&self) -> Vec<Node> {
        let mut nodes = Vec::with_capacity(266_305);
        nodes.push(Node::new_leaf(0));

        nodes[0] = match Region::new(self, &mut nodes, [0; 3], CHUNK_SIDE) {
            Region::Empty => Node::new_leaf(0),
            Region::Solid => Node::new_leaf(u64::MAX),
            Region::Node(node) => node,
        };
        nodes
    }
}

#[derive(Clone, Copy)]
enum Region {
    Empty,
    Solid,
    Node(Node),
}

impl Region {
    pub fn new(
        grid: &OccupancyGrid,
        nodes: &mut Vec<Node>,
        origin: [usize; 3],
        side: usize,
    ) -> Self {
        if side == 1 {
            return if grid.get(origin[0], origin[1], origin[2]) {
                Self::Solid
            } else {
                Self::Empty
            };
        }

        let child_side = side / 4;
        let mut children = [Self::Empty; 64];
        let mut child_mask = 0u64;
        let mut has_branch = false;

        for child_idx in 0..64 {
            let child_origin = [
                origin[0] + (child_idx & 3) * child_side,
                origin[1] + ((child_idx >> 4) & 3) * child_side,
                origin[2] + ((child_idx >> 2) & 3) * child_side,
            ];
            let child = Region::new(grid, nodes, child_origin, child_side);

            if !matches!(child, Self::Empty) {
                child_mask |= 1 << child_idx;
            }
            has_branch |= matches!(child, Self::Node(_));
            children[child_idx] = child;
        }

        if !has_branch {
            return match child_mask {
                0 => Self::Empty,
                u64::MAX => Self::Solid,
                _ => Self::Node(Node::new_leaf(child_mask)),
            };
        }

        let child_ofs = nodes.len();
        assert!(
            child_ofs <= NODE_PTR_MASK as usize,
            "64-tree exceeds 31-bit node offsets",
        );

        for child in children {
            match child {
                Self::Empty => {},
                Self::Solid => nodes.push(Node::new_leaf(u64::MAX)),
                Self::Node(node) => nodes.push(node),
            }
        }

        Self::Node(Node::new()
            .with_child_mask(child_mask)
            .with_child_ofs(child_ofs as u32),
        )
    }
}

pub struct ChunkDirectoryData {
    pub pages: Vec<u32>,
    pub roots: [u32; REALM_N],
}

impl ChunkDirectoryData {
    fn new() -> Self {
        Self {
            pages: vec![0; CHUNK_PAGE_ENTRIES],
            roots: [0; REALM_N],
        }
    }

    fn insert(
        &mut self,
        realm_id: usize,
        coord: [u16; 3],
        state: ChunkState,
        chunk_id: u32,
    ) {
        assert!(realm_id < REALM_N);
        assert!(coord.into_iter().all(|component| component < 4096));
        assert!(chunk_id > 0 && chunk_id < CHUNK_N as u32);
        assert!(matches!(state, ChunkState::Solid | ChunkState::Loaded));

        let root_page = if self.roots[realm_id] == 0 {
            let page = self.allocate_page();
            self.roots[realm_id] = page;
            page
        } else {
            self.roots[realm_id]
        };
        let l2_page = self.child_page(root_page, morton_nibble(coord, 8));
        let l1_page = self.child_page(l2_page, morton_nibble(coord, 4));
        let leaf_idx = morton_nibble(coord, 0);
        let entry_idx = l1_page as usize * CHUNK_PAGE_ENTRIES + leaf_idx;

        assert_eq!(self.pages[entry_idx], 0, "duplicate chunk directory entry");
        self.pages[entry_idx] = (state as u32) << CHUNK_STATE_SHIFT | chunk_id;
    }

    fn allocate_page(&mut self) -> u32 {
        let page = self.pages.len() / CHUNK_PAGE_ENTRIES;
        self.pages.resize(self.pages.len() + CHUNK_PAGE_ENTRIES, 0);
        page.try_into().expect("chunk directory page index exceeds u32")
    }

    fn child_page(&mut self, parent_page: u32, entry: usize) -> u32 {
        let entry_idx = parent_page as usize * CHUNK_PAGE_ENTRIES + entry;
        let child_page = self.pages[entry_idx];

        if child_page != 0 {
            return child_page;
        }

        let child_page = self.allocate_page();
        self.pages[entry_idx] = child_page;
        child_page
    }
}

fn morton_nibble(coord: [u16; 3], shift: u32) -> usize {
    let x = usize::from(coord[0] >> shift & 0xF);
    let y = usize::from(coord[1] >> shift & 0xF);
    let z = usize::from(coord[2] >> shift & 0xF);
    let mut index = 0;

    for bit in 0..4 {
        index |= (x >> bit & 1) << (bit * 3);
        index |= (z >> bit & 1) << (bit * 3 + 1);
        index |= (y >> bit & 1) << (bit * 3 + 2);
    }

    index
}

pub struct BuiltWorld {
    pub nodes: Vec<Node>,
    pub chunk_root_offsets: Vec<u64>,
    pub chunk_metadata: Vec<ChunkMetadata>,
    pub chunk_directory: ChunkDirectoryData,
}

impl BuiltWorld {
    pub fn new() -> Self {
        let nodes = OccupancyGrid::new(|x, y, z| {
            let x = i32::from(x) - 128;
            let y = i32::from(y) - 128;
            let z = i32::from(z) - 128;
            x * x + y * y + z * z <= 128 * 128
        })
            .build_tree();

        // let chunks = [[0, 0, 0], [0, 1, 0]];
        // TODO(camera): imprecision
        let chunks = [[4095, 4095, 4095], [4095, 4094, 4095]];
        let mut chunk_directory = ChunkDirectoryData::new();
        let mut chunk_metadata = Vec::with_capacity(chunks.len() + 1);
        chunk_metadata.push(ChunkMetadata::new().with_state(ChunkState::Empty));

        for (chunk_idx, [x, y, z]) in chunks.into_iter().enumerate() {
            let chunk_id = chunk_idx as u32 + 1;
            chunk_directory.insert(0, [x, y, z], ChunkState::Loaded, chunk_id);
            chunk_metadata.push(
                ChunkMetadata::new()
                    .with_x(x)
                    .with_y(y)
                    .with_z(z)
                    .with_realm_id(0)
                    .with_state(ChunkState::Loaded),
            );
        }

        BuiltWorld {
            nodes,
            chunk_root_offsets: vec![0; chunks.len() + 1],
            chunk_metadata,
            chunk_directory,
        }
    }
}
// }}}

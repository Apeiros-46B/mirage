// TODO: this is unreviewed slop, probably needs to be entirely replaced
use bytemuck::{NoUninit, Pod, Zeroable};
use modular_bitfield::{Specifier, bitfield, prelude::*};

pub const CHUNK_ID_BITS: usize = 20;
pub const REALM_ID_BITS: usize = 3;
pub const PORTALS_PER_CHUNK: usize = 32;
pub const CHUNK_N: usize = 1 << CHUNK_ID_BITS;
pub const REALM_N: usize = 1 << REALM_ID_BITS;
pub const CHUNK_SIDE: usize = 256;
pub const REALM_SIDE: usize = 4096;
pub const MIN_CHUNK_COORD: i32 = -2048;
pub const MAX_CHUNK_COORD: i32 = 2047;

const CHUNK_PAGE_SIDE: usize = 16;
const CHUNK_PAGE_ENTRIES: usize = CHUNK_PAGE_SIDE.pow(3);
const CHUNK_STATE_SHIFT: u32 = 30;
const VOXEL_VALUE_OFFSET_MASK: u32 = 0x3FFF_FFFF;
const NODE_PACKED_BIT: u32 = 0x4000_0000;
const NODE_OFFSET_MASK: u32 = 0x7FFF_FFFF;
const NODE_LEAF_BIT: u32 = 0x8000_0000;

// metadata and directory keys bias signed chunk coords into unsigned u12 storage
pub fn unsign_coord(coord: [i32; 3]) -> [u16; 3] {
    coord.map(|component| {
        assert!((MIN_CHUNK_COORD..=MAX_CHUNK_COORD).contains(&component));
        (component + 2048) as u16
    })
}

// max 2 bits
#[derive(Clone, Copy, Debug, Specifier)]
pub enum ChunkState {
    Empty = 0,
    Sealed = 1, // loaded, all six outer faces are opaque
    Loaded = 2,
    _Reserved = 3,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct ChunkDirectory {
    pub pages: u64,
    pub roots: u64,
}

// see shaders/shared/world.slang::WorldView
// all u64 are bda pointers
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct WorldView {
    pub roots: u64,
    pub nodes: u64,
    pub values: u64, // u32 words containing two u16 voxel values
    pub chunk_meta: u64,
    pub chunk_dir: ChunkDirectory,
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
    pub fn new_uniform_leaf(child_mask: u64, voxel_value: u16) -> Self {
        assert!(child_mask == 0 || voxel_value != 0);
        let mut node = Self::new().with_child_mask(child_mask);
        node.set_payload(NODE_LEAF_BIT | u32::from(voxel_value));
        node
    }

    pub fn new_packed_leaf(child_mask: u64, ofs_n: u32) -> Self {
        assert!(ofs_n <= VOXEL_VALUE_OFFSET_MASK);
        let mut node = Self::new().with_child_mask(child_mask);
        node.set_payload(NODE_LEAF_BIT | NODE_PACKED_BIT | ofs_n);
        node
    }

    pub fn is_leaf(&self) -> bool {
        self.payload() & NODE_LEAF_BIT != 0
    }

    pub fn is_packed_leaf(&self) -> bool {
        self.payload() & (NODE_LEAF_BIT | NODE_PACKED_BIT)
            == NODE_LEAF_BIT | NODE_PACKED_BIT
    }

    // ofs in number of u16
    pub fn mat_ofs_n(&self) -> u32 {
        self.payload() & VOXEL_VALUE_OFFSET_MASK
    }

    // ofs in number of Node in the non-root pool
    pub fn child_ofs_n(&self) -> u32 {
        self.payload() & NODE_OFFSET_MASK
    }
    pub fn with_child_ofs_n(mut self, ofs_n: u32) -> Self {
        assert!(ofs_n <= NODE_OFFSET_MASK, "64-tree node offset exceeds 31 bits");
        self.set_payload(ofs_n);
        self
    }

    pub fn uniform_value(&self) -> u16 {
        (self.payload() & 0xFFFF) as u16
    }
}

// TODO(loader): this is a test, needs replacement
// {{{ CPU-side world building
pub struct VoxelGrid {
    // X-Z-Y order; voxel value zero means empty.
    values: Box<[u16]>,
}

impl VoxelGrid {
    fn get(&self, x: usize, y: usize, z: usize) -> u16 {
        self.values[x + CHUNK_SIDE * (z + CHUNK_SIDE * y)]
    }

    pub fn new(mut is_occupied: impl FnMut(u8, u8, u8) -> bool) -> Self {
        Self::from_voxel_values(|x, y, z| u16::from(is_occupied(x, y, z)))
    }

    pub fn from_voxel_values(mut voxel_value_at: impl FnMut(u8, u8, u8) -> u16)
        -> Self
    {
        let mut values = vec![0; CHUNK_SIDE.pow(3)].into_boxed_slice();
        for y in 0..CHUNK_SIDE {
            for z in 0..CHUNK_SIDE {
                for x in 0..CHUNK_SIDE {
                    values[x + CHUNK_SIDE * (z + CHUNK_SIDE * y)] =
                        voxel_value_at(x as u8, y as u8, z as u8);
                }
            }
        }
        Self { values }
    }

    // root, non-root nodes, voxel values
    pub fn build_tree(&self) -> (Node, Vec<Node>, Vec<u16>) {
        let mut nodes = Vec::with_capacity(266_305);
        let mut values = Vec::new();
        let root = Region::new(self, &mut nodes, &mut values, [0; 3], CHUNK_SIDE)
            .into_node();
        (root, nodes, values)
    }
}

#[derive(Clone, Copy)]
enum Region {
    Empty,
    Uniform(u16), // entirely filled with one voxel value
    Node(Node),
}

impl Region {
    fn into_node(self) -> Node {
        match self {
            Self::Empty => Node::new_uniform_leaf(0, 0),
            Self::Uniform(voxel_value) => Node::new_uniform_leaf(u64::MAX, voxel_value),
            Self::Node(node) => node,
        }
    }

    fn new(
        grid: &VoxelGrid,
        nodes: &mut Vec<Node>,
        values: &mut Vec<u16>,
        origin: [usize; 3],
        side: usize,
    ) -> Self {
        if side == 1 {
            return match grid.get(origin[0], origin[1], origin[2]) {
                0 => Self::Empty,
                voxel_value => Self::Uniform(voxel_value),
            };
        }

        let child_side = side / 4;
        let mut children = [Self::Empty; 64];
        let mut child_mask = 0u64;
        let mut uniform_value = 0;
        let mut uniform = true;

        for child_idx in 0..64 {
            let child_origin = [
                origin[0] + (child_idx & 3) * child_side,
                origin[1] + ((child_idx >> 4) & 3) * child_side,
                origin[2] + ((child_idx >> 2) & 3) * child_side,
            ];
            let child = Region::new(grid, nodes, values, child_origin, child_side);
            match child {
                Self::Empty => {},
                Self::Uniform(voxel_value) => {
                    if uniform_value == 0 { uniform_value = voxel_value; }
                    uniform &= uniform_value == voxel_value;
                },
                Self::Node(_) => uniform = false,
            }
            if !matches!(child, Self::Empty) { child_mask |= 1 << child_idx; }
            children[child_idx] = child;
        }

        if child_mask == 0 { return Self::Empty; }
        if uniform {
            return if child_mask == u64::MAX {
                Self::Uniform(uniform_value)
            } else {
                Self::Node(Node::new_uniform_leaf(child_mask, uniform_value))
            };
        }

        // only the finest leaves contain heterogeneous voxel values
        if side == 4 {
            let ofs_n = values.len();
            assert!(
                ofs_n + child_mask.count_ones() as usize <= VOXEL_VALUE_OFFSET_MASK as usize + 1,
                "voxel val pool exceeded 30b ofs",
            );
            for child in children {
                if let Self::Uniform(voxel_value) = child { values.push(voxel_value); }
            }
            return Self::Node(Node::new_packed_leaf(child_mask, ofs_n as u32));
        }

        let child_ofs_n = nodes.len();
        assert!(
            child_ofs_n <= NODE_OFFSET_MASK as usize,
            "64-tree exceeded 31b node ofs",
        );
        for child in children {
            if !matches!(child, Self::Empty) { nodes.push(child.into_node()); }
        }
        Self::Node(Node::new()
            .with_child_mask(child_mask)
            .with_child_ofs_n(child_ofs_n as u32))
    }
}

// 2 u16 voxel values per u32, low half first
fn pack_voxel_values(values: &[u16]) -> Vec<u32> {
    values.chunks(2)
        .map(|pair| {
            u32::from(pair[0]) | (u32::from(*pair.get(1).unwrap_or(&0)) << 16)
        })
        .collect()
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
        signed_coord: [i32; 3],
        state: ChunkState,
        chunk_id: u32,
    ) {
        assert!(realm_id < REALM_N);
        assert!(chunk_id > 0 && chunk_id < CHUNK_N as u32);
        assert!(matches!(state, ChunkState::Sealed | ChunkState::Loaded));
        let unsigned_coord = unsign_coord(signed_coord);

        let root_page_idx = if self.roots[realm_id] == 0 {
            let page_idx = self.allocate_page();
            self.roots[realm_id] = page_idx;
            page_idx
        } else {
            self.roots[realm_id]
        };
        let l2_page_idx = self.child_page(root_page_idx, morton_nibble(unsigned_coord, 8));
        let l1_page_idx = self.child_page(l2_page_idx, morton_nibble(unsigned_coord, 4));
        let leaf_idx = morton_nibble(unsigned_coord, 0);
        let entry_idx = l1_page_idx as usize * CHUNK_PAGE_ENTRIES + leaf_idx;

        assert_eq!(self.pages[entry_idx], 0, "duplicate chunk directory entry");
        self.pages[entry_idx] = (state as u32) << CHUNK_STATE_SHIFT | chunk_id;
    }

    fn allocate_page(&mut self) -> u32 {
        let page_idx = self.pages.len() / CHUNK_PAGE_ENTRIES;
        self.pages.resize(self.pages.len() + CHUNK_PAGE_ENTRIES, 0);
        page_idx.try_into().expect("chunk directory page index exceeds u32")
    }

    fn child_page(&mut self, parent_page_idx: u32, entry_idx_in_page: usize) -> u32 {
        let entry_idx = parent_page_idx as usize * CHUNK_PAGE_ENTRIES + entry_idx_in_page;
        let child_page_idx = self.pages[entry_idx];

        if child_page_idx != 0 {
            return child_page_idx;
        }

        let child_page_idx = self.allocate_page();
        self.pages[entry_idx] = child_page_idx;
        child_page_idx
    }
}

fn morton_nibble(unsigned_coord: [u16; 3], shift: u32) -> usize {
    let x = usize::from(unsigned_coord[0] >> shift & 0xF);
    let y = usize::from(unsigned_coord[1] >> shift & 0xF);
    let z = usize::from(unsigned_coord[2] >> shift & 0xF);
    let mut idx = 0;

    for bit in 0..4 {
        idx |= (x >> bit & 1) << (bit * 3);
        idx |= (z >> bit & 1) << (bit * 3 + 1);
        idx |= (y >> bit & 1) << (bit * 3 + 2);
    }

    idx
}

pub struct BuiltWorld {
    // branch offsets in both nodes and chunk_roots index nodes
    pub nodes: Vec<Node>, // non-root nodes only
    pub voxel_data: Vec<u32>, // pairs of u16 voxel values, offsets count # u16s
    pub chunk_roots: Vec<Node>,
    pub chunk_metadata: Vec<ChunkMetadata>,
    pub chunk_directory: ChunkDirectoryData,
}

impl BuiltWorld {
    pub fn new() -> Self {
        let (root, nodes, values) = VoxelGrid::new(|x, y, z| {
            let x = i32::from(x) - 128;
            let y = i32::from(y) - 128;
            let z = i32::from(z) - 128;
            return x * x + y * y + z * z < 128*128;

            // if x / 16 % 2 != 0 { return false }
            // if y / 16 % 2 != 0 { return false }
            // if z / 16 % 2 != 0 { return false }
            // let xi = i32::from(x % 16) - 8;
            // let yi = i32::from(y % 16) - 8;
            // let zi = i32::from(z % 16) - 8;
            // return xi * xi + yi * yi + zi * zi < 64;
        })
            .build_tree();

        let chunks = [
            [MAX_CHUNK_COORD, MAX_CHUNK_COORD, MAX_CHUNK_COORD],
        ];
        let mut chunk_directory = ChunkDirectoryData::new();
        let mut chunk_metadata = Vec::with_capacity(chunks.len() + 1);
        chunk_metadata.push(ChunkMetadata::new().with_state(ChunkState::Empty));

        for (chunk_idx, [x, y, z]) in chunks.into_iter().enumerate() {
            let chunk_id = chunk_idx as u32 + 1;
            chunk_directory.insert(0, [x, y, z], ChunkState::Loaded, chunk_id);
            let [unsigned_x, unsigned_y, unsigned_z] = unsign_coord([x, y, z]);
            chunk_metadata.push(
                ChunkMetadata::new()
                    .with_x(unsigned_x)
                    .with_y(unsigned_y)
                    .with_z(unsigned_z)
                    .with_realm_id(0)
                    .with_state(ChunkState::Loaded),
            );
        }

        BuiltWorld {
            nodes,
            voxel_data: pack_voxel_values(&values),
            // chunk id 0 means empty
            chunk_roots: std::iter::once(Node::new_uniform_leaf(0, 0))
                .chain(std::iter::repeat_n(root, chunks.len()))
                .collect(),
            chunk_metadata,
            chunk_directory,
        }
    }
}
// }}}

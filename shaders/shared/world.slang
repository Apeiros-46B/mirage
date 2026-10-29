// see crate::world
// TODO: Chunk layout can probably be made better. we probably don't need 32*2^18 portals

public static const float CHUNK_SIDE = 256.0; // in voxels
public static const int REALM_SIDE = 4096; // in chunks

static const uint CHUNK_PAGE_SIDE = 16;
static const uint CHUNK_PAGE_ENTRIES =
	CHUNK_PAGE_SIDE * CHUNK_PAGE_SIDE * CHUNK_PAGE_SIDE;

static const uint REALM_N = 1 << 3;
static const uint PORTALS_PER_CHUNK = 32;

// {{{ coordinate space stuff
// "signed coords" are [-2048,2047]
// "unsigned coords" are [0,4096)
// "local coord" is within-chunk voxel units
// "tree space" is 1 + localCoord / 256 [1,2)
static const int CHUNK_COORD_BIAS = 2048;

// signed->unsigned
uint3 unsignCoord(int3 coord) {
	return uint3(coord + CHUNK_COORD_BIAS);
}
// unsigned->signed
int3 signCoord(uint3 coord) {
	return int3(coord) - CHUNK_COORD_BIAS;
}

// A chunk coordinate; realm identity is separate.
public interface IChunkCoord {
	property int3 signedCoord { get; }
	property uint3 unsignedCoord { get; }
}

// A position split into a chunk coordinate and local voxel units.
public interface ISplitCoord : IChunkCoord {
	property float3 localCoord { get; }
}

// Split positions store signed coords; metadata decodes its unsigned coords directly.
public extension<T: ISplitCoord> T {
	public property uint3 unsignedCoord {
		get { return unsignCoord(this.signedCoord); }
	}
}

public struct RelChunkBounds {
	public float3 min;
	public float3 max;

	// bounds in voxel coords relative to the supplied split-coordinate origin
	public __init(IChunkCoord chunk, ISplitCoord origin) {
		this.min = float3(chunk.signedCoord - origin.signedCoord) * CHUNK_SIDE - origin.localCoord;
		this.max = this.min + CHUNK_SIDE;
	}

	public bool isInside() {
		return all(this.min <= 0.0) && all(this.max > 0.0);
	}
}
// }}}

// {{{ chunks and nodes
// max 2 bits
public enum ChunkState : uint {
	Empty = 0,
	Sealed = 1, // loaded, all 6 faces are filled with opaque voxels
	Loaded = 2,
	_Reserved = 3,
}

public struct ChunkMetadata : IChunkCoord {
	private uint64_t data;

	public property uint3 unsignedCoord {
		get { return uint3(this.x, this.y, this.z); }
	}
	public property int3 signedCoord {
		get { return signCoord(this.unsignedCoord); }
	}

	// bits [0,11]: unsigned coord component, realm-local x
	public property uint x {
		get { return uint(bitfieldExtract(this.data, 0, 12)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 0, 12); }
	}

	// bits [12,23]: unsigned coord component, realm-local y
	public property uint y {
		get { return uint(bitfieldExtract(this.data, 12, 12)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 12, 12); }
	}

	// bits [24,35]: unsigned coord component, realm-local z
	public property uint z {
		get { return uint(bitfieldExtract(this.data, 24, 12)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 24, 12); }
	}

	// bits [36,53]: index into portal megabuffer, points to a slice of N portals
	// owned by this chunk, where N is determined at worldgen time and never needs
	// to be read back. theoretically allows 2^18 * PORTALS_PER_CHUNK portals
	public property uint portalHeapOfs {
		get { return uint(bitfieldExtract(this.data, 36, 18)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 36, 18); }
	}

	// bits [54,61]: ephemeral ID of the realm this chunk is in.
	// theoretically allows 2^8 unique realms loaded at once
	public property uint realmId {
		get { return uint(bitfieldExtract(this.data, 54, 8)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 54, 8); }
	}

	// bits [62,63]: current state of this chunk
	public property ChunkState state {
		get { return ChunkState(bitfieldExtract(this.data, 62, 2)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 62, 2); }
	}
}

// 128-bit, branch children are packed in child-mask order
// payload: 0* = branch, 10 = uniform leaf, 11 = packed-value leaf
// packed offsets count u16 voxel values in the shared value pool (finest leaves only)
public struct Node {
	public uint64_t childMask;
	private uint payload;
	private uint pad;

	public property bool isLeaf {
		get { return bitfieldExtract(this.payload, 31, 1) != 0; }
	}

	public property bool isPackedLeaf {
		get { return bitfieldExtract(this.payload, 30, 2) == 3; }
	}

	// ofs in #Node in non-root pool
	public property uint childOfsN {
		get { return bitfieldExtract(this.payload, 0, 31); }
		set { this.payload = bitfieldExtract(newValue, 0, 31); }
	}

	// ofs in #u16 before packing two values per u32
	public property uint matOfsN {
		get { return bitfieldExtract(this.payload, 0, 30); }
	}

	// valid only for uniform leaves
	public property uint uniformValue {
		get { return bitfieldExtract(this.payload, 0, 16); }
		set { this.payload = bitfieldInsert(this.payload, newValue, 0, 16); }
	}
}
// }}}

// {{{ voxel data
// interpretation of the u16 value in a leaf
public struct Material {
	public uint id;
	public bool smooth;

	public __init(uint voxelValue) {
		this.id = voxelValue & 0xFFFFu;
		this.smooth = false;
	}
}
// }}}

// {{{ world view
// non-owning view of the world buffers
public struct WorldView {
	public Node* roots; // idx by chunk id
	public Node* nodes; // node offsets index this
	public uint* values; // values pool, each uint is 2x u16 values

	public ChunkMetadata* chunkMeta;
	public ChunkDirectory chunkDir;

	public __init(
		Node* roots,
		Node* nodes,
		uint* values,
		ChunkMetadata* chunkMeta,
		ChunkDirectory chunkDir
	) {
		this.roots = roots;
		this.nodes = nodes;
		this.values = values;
		this.chunkMeta = chunkMeta;
		this.chunkDir = chunkDir;
	}

	// requires an occupied slot in a branch, returns node and index
	public Node child(Node branch, uint slot, out uint idx) {
		idx = branch.childOfsN + countBitsBefore(branch.childMask, slot);
		return this.nodes[idx];
	}

	// requires an occupied slot in a leaf, returns complete 16-bit voxel value
	public uint leafValue(Node leaf, uint slot) {
		if (!leaf.isPackedLeaf) return leaf.uniformValue;

		// extract from shared voxel value table
		uint idx = leaf.matOfsN + countBitsBefore(leaf.childMask, slot);
		uint word = this.values[idx >> 1];
		return bitfieldExtract(word, bitfieldExtract(idx, 0, 1) * 16, 16);
	}

	private static uint countBitsBefore(uint64_t mask, uint bitIdx) {
		uint word = uint(bitfieldExtract(mask, 0, 32));
		uint count = 0;

		if (bitIdx >= 32) {
			count = countbits(word);
			word = uint(bitfieldExtract(mask, 32, 32));
		}

		uint bits = bitfieldExtract(bitIdx, 0, 5);
		return count + countbits(bitfieldExtract(word, 0, bits));
	}
}
// }}}

// {{{ toplevel chunk map
// per-traversal path through one chunk directory, invalidated if it changes
public struct ChunkPagePtr {
	uint realmId;
	uint l3PageIdx;
	uint l2PageIdx;
	uint l1PageIdx;
	int3 unsignedCoord;

	public __init(uint realmId) {
		this.realmId = realmId;
		this.l3PageIdx = ~0u; // resolve lazily, after the initial known chunk
		this.l2PageIdx = 0;
		this.l1PageIdx = 0;
		this.unsignedCoord = int3(-1);
	}
}

public struct ChunkDirectory {
	// pointer to buffer of pages, each page is 4096 uints representing a cube in space.
	// L3 pages span the entire realm, L2 pages are 256^3 chunks, L1 pages are 16^3
	//
	// in L3 and L2 pages, each entry is an index into pages buffer, index `n` points
	// to the whole page at pages[n * 4096] representing the next lvl (L3 -> L2 -> L1).
	// in L1 pages, each uint's upper 2 bits are a ChunkState. the lower 20 bits are
	// the nonzero chunk ID if ChunkState = Sealed or Loaded. chunk ID 0 means no chunk
	//
	// each page consumes one 4-bit nibble from each unsigned 12-bit realm-local
	// chunk coordinate. L3 uses bits [8, 11], L2 uses [4, 7], and L1 uses [0, 3].
	// entries are Morton ordered in X-Z-Y order: for nibble bit b, X is stored at
	// index bit 3b, Z at 3b + 1, and Y at 3b + 2. page 0 is reserved as the absent
	// page, so every L3/L2 child index and realm root is either 0 or a page index
	private uint* pages;

	// indices into pages heap to L3 (root) pages, indexed by realm ID
	private uint* roots;

	// returns 0-4095 morton idx for one page (XZY interleaved)
	// - unsignedCoord must be in [0, 4096) on every axis
	// - shift must be {8,4,0}, for L3,2,1 respectively
	private static uint mortonNibble(int3 unsignedCoord, uint shift) {
		uint3 nibble = bitfieldExtract(uint3(unsignedCoord), shift, 4);
		uint idx = 0;

		[unroll]
		for (uint bit = 0; bit < 4; bit++) {
			idx |= (nibble.x >> bit & 1) << (bit * 3);
			idx |= (nibble.z >> bit & 1) << (bit * 3 + 1);
			idx |= (nibble.y >> bit & 1) << (bit * 3 + 2);
		}

		return idx;
	}

	private uint entAt(uint pageIdx, int3 unsignedCoord, uint shift) {
		return this.pages[
			pageIdx * CHUNK_PAGE_ENTRIES + mortonNibble(unsignedCoord, shift)
		];
	}

	// resolves the cell at unsignedCoord
	// - unsignedCoord must be in [0, 4096) chunk cube of realm
	// - realmId must a valid realm id (<REALM_N)
	// skipSide is the side of empty region safe to skip in 1 step:
	// - 1 for an L1 entry
	// - the aligned region side if an L3, L2, or L1 page is absent
	public uint lookup(int3 unsignedCoord, uint realmId, out uint skipSide) {
		ChunkPagePtr path = ChunkPagePtr(realmId);
		return this.lookup(unsignedCoord, path, skipSide);
	}

	// reuse the deepest page containing the new coord, then descend from there
	// for skipSide semantics, see lookup(unsignedCoord, realmId, skipSide)
	public uint lookup(int3 unsignedCoord, inout ChunkPagePtr path, out uint skipSide) {
		if (path.l3PageIdx == ~0u) path.l3PageIdx = this.roots[path.realmId];
		if (path.l3PageIdx == 0) {
			skipSide = CHUNK_PAGE_SIDE * CHUNK_PAGE_SIDE * CHUNK_PAGE_SIDE;
			return 0;
		}

		int3 oldPathCoord = path.unsignedCoord;
		path.unsignedCoord = unsignedCoord;

		bool changedL2 = any(
			bitfieldExtract(unsignedCoord, 8, 24)
			!= bitfieldExtract(oldPathCoord, 8, 24)
		);
		if (changedL2) {
			path.l2PageIdx = this.entAt(path.l3PageIdx, unsignedCoord, 8);
			path.l1PageIdx = 0;
		}
		if (path.l2PageIdx == 0) {
			skipSide = CHUNK_PAGE_SIDE * CHUNK_PAGE_SIDE;
			return 0;
		}

		bool changedL1 = any(
			bitfieldExtract(unsignedCoord, 4, 28)
			!= bitfieldExtract(oldPathCoord, 4, 28)
		);
		if (changedL1) {
			path.l1PageIdx = this.entAt(path.l2PageIdx, unsignedCoord, 4);
		}
		if (path.l1PageIdx == 0) {
			skipSide = CHUNK_PAGE_SIDE;
			return 0;
		}

		uint ent = this.entAt(path.l1PageIdx, unsignedCoord, 0);
		uint state = bitfieldExtract(ent, 30, 2);
		if (state == uint(ChunkState.Loaded) || state == uint(ChunkState.Sealed)) {
			skipSide = 1;
			return bitfieldExtract(ent, 0, 20);
		}

		skipSide = 1;
		return 0;
	}
}
// }}}

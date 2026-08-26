// see crate::world
// TODO: Chunk layout can probably be made better. we do not need like 32*2^18 portals
import raymarch;

static const uint CHUNK_N = 1 << 20;
static const uint REALM_N = 1 << 3;
static const uint PORTALS_PER_CHUNK = 32;

static const uint CHUNK_ID_MASK = (1 << 20) - 1;
static const uint CHUNK_PAGE_ENTRIES = 16 * 16 * 16;

// {{{ chunks
// max 2 bits
enum ChunkState : uint {
	Empty = 0,
	Solid = 1, // loaded, all 6 faces are filled with opaque materials
	Loaded = 2, // loaded
	_Reserved = 3,
}

struct ChunkMetadata {
	uint64_t data;

	// bits [0,11]: realm-local x
	property uint x {
		get { return uint(bitfieldExtract(data, 0, 12)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 0, 12); }
	}

	// bits [12,23]: realm-local y
	property uint y {
		get { return uint(bitfieldExtract(data, 12, 12)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 12, 12); }
	}

	// bits [24,35]: realm-local z
	property uint z {
		get { return uint(bitfieldExtract(data, 24, 12)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 24, 12); }
	}

	// bits [36,53]: index into portal megabuffer, points to a slice of N portals
	// owned by this chunk, where N is determined at worldgen time and never needs
	// to be read back. theoretically allows 2^18 * PORTALS_PER_CHUNK portals
	property uint portalHeapOfs {
		get { return uint(bitfieldExtract(data, 36, 18)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 36, 18); }
	}

	// bits [54,61]: ephemeral ID of the realm this chunk is in.
	// theoretically allows 2^8 unique realms loaded at once
	property uint realmId {
		get { return uint(bitfieldExtract(data, 54, 8)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 54, 8); }
	}

	// bits [62,63]: current state of this chunk
	property ChunkState state {
		get { return ChunkState(bitfieldExtract(data, 62, 2)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 62, 2); }
	}
}

// 128-bit, branch children are packed in child-mask order
// payload bit 31 = leaf, leaf masks contain same-material occupied cells
struct Node {
	uint64_t childMask;
	uint32_t payload;
	uint32_t pad;

	property bool isLeaf {
		get { return (payload & 0x80000000) != 0; }
	}
	property uint childOfs {
		get { return payload & 0x7FFFFFFF; }
		set { payload = newValue & 0x7FFFFFFF; }
	}
	property uint matId {
		get { return bitfieldExtract(payload, 0, 16); }
		set { payload = bitfieldInsert(payload, newValue, 0, 16); }
	}
}
// }}}

// {{{ toplevel chunk map
struct ChunkDirectory {
	// pointer to buffer of pages, each page is 4096 uints representing a cube in space.
	// L3 pages span the entire realm, L2 pages are 256^3 chunks, L1 pages are 16^3
	//
	// in L3 and L2 pages, each entry is an index into pages buffer, index `n` points
	// to the whole page at pages[n * 4096] representing the next lvl (L3 -> L2 -> L1).
	// in L1 pages, each uint's upper 2 bits are a ChunkState. the lower 20 bits are
	// the nonzero chunk ID if ChunkState = Solid or Loaded. chunk ID 0 means no chunk
	//
	// each page consumes one 4-bit nibble from each unsigned 12-bit realm-local
	// chunk coordinate. L3 uses bits [8, 11], L2 uses [4, 7], and L1 uses [0, 3].
	// entries are Morton ordered in X-Z-Y order: for nibble bit b, X is stored at
	// index bit 3b, Z at 3b + 1, and Y at 3b + 2. page 0 is reserved as the absent
	// page, so every L3/L2 child index and realm root is either 0 or a page index
	uint* pages;

	// indices into pages heap to L3 (root) pages, indexed by realm ID
	uint* roots;

	// returns 0-4095 morton idx for one page (XZY interleaved)
	// - coord must be a >0 12-bit realm-local chunk coord
	// - shift must be {8,4,0}, for L3,2,1 respectively
	static uint _mortonNibble(int3 coord, uint shift) {
		uint3 nibble = uint3(coord) >> shift & 0xFu;
		uint index = 0;

		[unroll]
		for (uint bit = 0; bit < 4; bit++) {
			index |= (nibble.x >> bit & 1) << (bit * 3);
			index |= (nibble.z >> bit & 1) << (bit * 3 + 1);
			index |= (nibble.y >> bit & 1) << (bit * 3 + 2);
		}

		return index;
	}

	uint _entAt(uint page, int3 coord, uint shift) {
		return pages[page * CHUNK_PAGE_ENTRIES + _mortonNibble(coord, shift)];
	}

	// resolves the cell containing cur
	// - cur.coord must be in [0, 4096) chunk cube of realm
	// - cur.realmId must a valid realm id (<REALM_N)
	// skipSide is the side of empty region safe to skip in one step from this cur:
	// - 1 for an L1 entry
	// - {4096,256,16} if missing L{3,2,1}
	uint lookupChunk(ChunkCursor cur, out uint skipSide) {
		uint page = roots[cur.realmId];
		if (page == 0) {
			skipSide = 4096;
			return 0;
		}

		uint ent = _entAt(page, cur.coord, 8);
		if (ent == 0) {
			skipSide = 256;
			return 0;
		}

		page = ent;
		ent = _entAt(page, cur.coord, 4);
		if (ent == 0) {
			skipSide = 16;
			return 0;
		}

		page = ent;
		ent = _entAt(page, cur.coord, 0);
		uint state = ent >> 30;
		if (state == uint(ChunkState.Solid) || state == uint(ChunkState.Loaded)) {
			skipSide = 1;
			return ent & CHUNK_ID_MASK;
		}

		skipSide = 1;
		return 0;
	}
}
// }}}

// see crate::world
// TODO: Chunk layout can probably be made better. we do not need like 32*2^18 portals
static const uint CHUNK_WS_N = 1 << 20;
static const uint REALM_WS_N = 1 << 8;
static const uint PORTALS_PER_CHUNK = 32;

// max 2 bits
enum ChunkState : uint {
	Empty = 0,
	Loading = 1,
	Solid = 2,
	Loaded = 3,
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

	// bits [54,61]: ephemeral (working-set) ID of the realm this chunk is in.
	// theoretically allows 2^8 unique realms loaded at once
	property uint realmWsId {
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

// TODO: this doesn't really count as world
enum Face {
	PX = 0b000,
	PY = 0b001,
	PZ = 0b010,
	NX = 0b011,
	NY = 0b100,
	NZ = 0b101,

	SMOOTH = 0b111,
}
struct VisbufKey {
	uint64_t data;

	// true = per-voxel shading, false = per-face shading
	property bool smooth {
		get { return face == Face.SMOOTH; }
	}

	property bool entity {
		get { return bitfieldExtract(data, 0, 1) != 0; }
		set { data = bitfieldInsert(data, uint64_t(newValue), 0, 1); }
	}
	property Face face {
		get { return Face(bitfieldExtract(data, 1, 3)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 1, 3); }
	}
	property uint x {
		get { return uint(bitfieldExtract(data, 4, 8)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 4, 8); }
	}
	property uint y {
		get { return uint(bitfieldExtract(data, 12, 8)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 12, 8); }
	}
	property uint z {
		get { return uint(bitfieldExtract(data, 20, 8)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 20, 8); }
	}
	property uint chunkId { // entity id when entity is true
		get { return uint(bitfieldExtract(data, 28, 20)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 28, 20); }
	}
	property uint matId {
		get { return uint(bitfieldExtract(data, 48, 16)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 48, 16); }
	}
}

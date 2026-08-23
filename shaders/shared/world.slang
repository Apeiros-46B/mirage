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
	property uint portal_heap_ofs {
		get { return uint(bitfieldExtract(data, 36, 18)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 36, 18); }
	}

	// bits [54,61]: ephemeral (working-set) ID of the realm this chunk is in.
	// theoretically allows 2^8 unique realms loaded at once
	property uint realm_ws_id {
		get { return uint(bitfieldExtract(data, 54, 8)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 54, 8); }
	}

	// bits [62,63]: current state of this chunk
	property ChunkState state {
		get { return ChunkState(bitfieldExtract(data, 62, 2)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 62, 2); }
	}
}

// 128-bit, child_mask == 0 means leaf
// if leaf, payload = mat_id and oct_normal
// otherwise payload = child_ofs (index of 64-node slabs, not single nodes)
struct Node {
	uint64_t child_mask;
	uint32_t payload;
	uint32_t pad;

	property uint child_ofs {
		get { return payload; }
		set { payload = newValue; }
	}
	property uint mat_id {
		get { return bitfieldExtract(payload, 0, 16); }
		set { payload = bitfieldInsert(payload, newValue, 0, 16); }
	}
}

struct VisbufKey {
	uint64_t data;

	property uint chunkId {
		get { return uint(bitfieldExtract(data, 0, 20)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 0, 20); }
	}
	property uint x {
		get { return uint(bitfieldExtract(data, 20, 8)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 20, 8); }
	}
	property uint y {
		get { return uint(bitfieldExtract(data, 28, 8)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 28, 8); }
	}
	property uint z {
		get { return uint(bitfieldExtract(data, 36, 8)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 36, 8); }
	}
	property uint lod {
		get { return uint(bitfieldExtract(data, 44, 2)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 44, 2); }
	}
	property uint face {
		get { return uint(bitfieldExtract(data, 46, 3)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 46, 3); }
	}
	property uint matType {
		get { return uint(bitfieldExtract(data, 49, 1)); }
		set { data = bitfieldInsert(data, uint64_t(newValue), 49, 1); }
	}
}

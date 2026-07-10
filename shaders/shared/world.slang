// TODO: Chunk layout can probably be made better. we do not need like 32*2^18 portals

// max 2 bits
enum ChunkState : uint {
	Empty = 0,
	Loading = 1,
	Solid = 2,
	Loaded = 3,
}

struct Chunk {
	// if null, this chunk's data is unloaded
	Node* root;

	uint64_t packedData;

	// bits [0,11]: realm-local x
	property uint x {
		get { return uint(bitfieldExtract(packedData, 0, 12)); }
		set { packedData = bitfieldInsert(packedData, newValue, 0, 12); }
	}

	// bits [12,23]: realm-local y
	property uint y {
		get { return uint(bitfieldExtract(packedData, 12, 12)); }
		set { packedData = bitfieldInsert(packedData, newValue, 12, 12); }
	}

	// bits [24,35]: realm-local z
	property uint z {
		get { return uint(bitfieldExtract(packedData, 24, 12)); }
		set { packedData = bitfieldInsert(packedData, newValue, 24, 12); }
	}

	// bits [36,53]: index into portal megabuffer, points to a slice of N portals
	// owned by this chunk, where N is determined at worldgen time and never needs
	// to be read back. theoretically allows 2^18 * PORTALS_PER_CHUNK portals
	property uint portalHeapOfs {
		get { return uint(bitfieldExtract(packedData, 36, 18)); }
		set { packedData = bitfieldInsert(packedData, newValue, 36, 18); }
	}

	// bits [54,61]: ephemeral (working-set) ID of the realm this chunk is in.
	// theoretically allows 2^8 unique realms loaded at once
	property uint realmWsID {
		get { return uint(bitfieldExtract(packedData, 54, 8)); }
		set { packedData = bitfieldInsert(packedData, newValue, 54, 8); }
	}

	// bits [62,63]: current state of this chunk
	property ChunkState state {
		get { return ChunkState(bitfieldExtract(packedData, 62, 2)); }
		set { packedData = bitfieldInsert(packedData, uint64_t(newValue), 62, 2); }
	}
}

// yes, this can theoretically be 12-byte (merge material with childMask and put
// homogenous bit in childPtr), but I'm not sure if that's actually better.
// 16-byte aligned (now) may be faster but has a 33% memory cost, which is probably
// non-negligible given that we will have a shit ton of these nodes.
// TODO: needs profiling
// TODO: property get/set
struct Node {
	// bit 0: is_leaf (0 = branch, 1 = leaf)
	// bits 1-31: absolute offset to child nodes or leaf payload (material) array
	uint32_t childPtr;

	// bit 0: is_homogeneous (0 = heterogeneous, 1 = homogeneous)
	// bits 1-31: 16-bit material ID (extra MSBs unused)
	uint32_t material;

	// occupancy bitmask
	uint64_t childMask;
};

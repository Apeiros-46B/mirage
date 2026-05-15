// TODO: rust-side equivalents
struct Chunk {
	// if null, this chunk is unloaded (ghost full chunk)
	Node* root;

	// layout:
	// 00-11: realm-local x coordinate
	// 12-23: realm-local y coordinate
	// 24-35: realm-local z coordinate
	// 36-55: portal heap offset (2^20 unique portals loaded at once)
	// 56-63: realm ID (2^8 unique realms loaded at once)
	uint64_t packed_data;
}

// yes, this can theoretically be 12-byte (merge material with childMask and put
// homogenous bit in childPtr), but I'm not sure if that's actually better.
// 16-byte aligned (now) may be faster at a 33% memory cost, which is probably
// non-negligible given that we will have millions or more of these nodes.
// TODO: needs profiling
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

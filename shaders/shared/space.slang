// matches spatial constants in crate::world

static const float CHUNK_SIDE = 256.0; // in voxels
static const int REALM_SIDE = 4096; // in chunks

static const int CHUNK_COORD_BIAS = 2048;
static const uint CHUNK_PAGE_SIDE = 16;
static const uint CHUNK_PAGE_ENTRIES =
	CHUNK_PAGE_SIDE * CHUNK_PAGE_SIDE * CHUNK_PAGE_SIDE;

// convert u12 stored coord into signed zero-centered coord
int3 storedToLogicalChunkCoord(uint3 coord) {
	return int3(coord) - CHUNK_COORD_BIAS;
}

struct RelChunkBounds {
	float3 min;
	float3 max;

	// bounds in voxel coords relative to the supplied split-coordinate origin
	__init(int3 chunkPos, int3 originChunkPos, float3 originLocalPos) {
		this.min = float3(chunkPos - originChunkPos) * CHUNK_SIDE - originLocalPos;
		this.max = this.min + CHUNK_SIDE;
	}
}

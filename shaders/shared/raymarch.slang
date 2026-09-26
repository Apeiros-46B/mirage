import camera;
import space;

static const float FLOAT_MAX = 3.402823466e+38;

// {{{ traversal state
struct ChunkCursor {
	int3 coord;
	uint realmId;

	// move to first chunk beyond the current aligned skip region along ray
	// - ray.chunkPos is a signed logical chunk coord
	// - ray.localPos is in [0, CHUNK_SIDE) on every axis
	// - this.coord is the biased unsigned coord of the current region
	// - ray.dir may contain zero components but cannot be 0,0,0
	// - skipSide must be {1,16,256,4096} (see ChunkDirectory.lookupChunk)
	// - corresponding aligned region must contain cur.coord
	// returns false once the cursor leaves the realm cube
	[mutating]
	bool advance(Ray ray, uint skipSide) {
		int side = int(skipSide);

		int3 regionMin = this.coord & ~(side - 1);
		int3 regionMax = regionMin + side;
		float3 boundary = select(
			ray.dir > 0.0,
			float3(regionMax),
			float3(regionMin),
		);

		int3 origStoredC = ray.chunkPos + CHUNK_COORD_BIAS; // in chunk units
		float3 origLocalC = ray.localPos / CHUNK_SIDE; // in chunk units

		float3 boundaryDelta = float3(boundary - origStoredC) - origLocalC;
		float3 sideDist = boundaryDelta / ray.dir;
		if (ray.dir.x == 0.0) sideDist.x = FLOAT_MAX;
		if (ray.dir.y == 0.0) sideDist.y = FLOAT_MAX;
		if (ray.dir.z == 0.0) sideDist.z = FLOAT_MAX;

		float nextDist = min(min(sideDist.x, sideDist.y), sideDist.z);
		int3 nextCoord = origStoredC + int3(floor(origLocalC + ray.dir * nextDist));

		if (sideDist.x == nextDist) {
			nextCoord.x = ray.dir.x > 0.0 ? regionMax.x : regionMin.x - 1;
		}
		if (sideDist.y == nextDist) {
			nextCoord.y = ray.dir.y > 0.0 ? regionMax.y : regionMin.y - 1;
		}
		if (sideDist.z == nextDist) {
			nextCoord.z = ray.dir.z > 0.0 ? regionMax.z : regionMin.z - 1;
		}

		this.coord = nextCoord;

		return all(nextCoord >= 0) && all(nextCoord < REALM_SIDE);
	}
};
// }}}

// {{{ visbuf
enum Face {
	PX = 0b000,
	PY = 0b001,
	PZ = 0b010,
	NX = 0b011,
	NY = 0b100,
	NZ = 0b101,

	SMOOTH = 0b111,
}

uint faceOpposingRay(float3 dir) {
	float3 absDir = abs(dir);

	if (absDir.x >= absDir.y && absDir.x >= absDir.z) {
		return dir.x >= 0.0 ? uint(Face.NX) : uint(Face.PX);
	}
	if (absDir.y >= absDir.z) {
		return dir.y >= 0.0 ? uint(Face.NY) : uint(Face.PY);
	}
	return dir.z >= 0.0 ? uint(Face.NZ) : uint(Face.PZ);
}

uint faceCrossed(float3 distSide, float3 dir) {
	if (distSide.x <= distSide.y && distSide.x <= distSide.z) {
		return dir.x >= 0.0 ? uint(Face.NX) : uint(Face.PX);
	}
	if (distSide.y <= distSide.z) {
		return dir.y >= 0.0 ? uint(Face.NY) : uint(Face.PY);
	}
	return dir.z >= 0.0 ? uint(Face.NZ) : uint(Face.PZ);
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

	// TODO: linux nvidia crashes when compiling inlined property accessors
	[mutating]
	void clear() {
		this.data = 0;
	}
	[mutating]
	void write(float3 pos, uint face, uint matId) {
		uint3 voxel = bit_cast<uint3>(pos) >> 15 & 0xFFu;
		this.data = uint64_t(face) << 1
		     | uint64_t(voxel.x) << 4
		     | uint64_t(voxel.y) << 12
		     | uint64_t(voxel.z) << 20
		     | uint64_t(matId) << 48;
	}
	bool isHit() {
		return uint(this.data >> 48) != 0;
	}
}

// workaround for property accessor crashes
void writeVisbufKey(float3 pos, uint face, uint matId, out VisbufKey key) {
	uint3 voxel = bit_cast<uint3>(pos) >> 15 & 0xFFu;
	key.data = uint64_t(face) << 1
	         | uint64_t(voxel.x) << 4
	         | uint64_t(voxel.y) << 12
	         | uint64_t(voxel.z) << 20
	         | uint64_t(matId) << 48;
}
// }}}

// provides WorldView.traceRay extension
// (based on https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing/)
// TODO: i still don't fully understand how this works
import camera;
import world;

static const float FLOAT_MAX = 3.402823466e+38;

// {{{ ray hit info
public enum Face {
	PX = 0b000,
	PY = 0b001,
	PZ = 0b010,
	NX = 0b011,
	NY = 0b100,
	NZ = 0b101,
}

public extension Face {
	public uint axis() {
		return this % 3;
	}

	public bool positive() {
		return this < 3;
	}

	public float ofsPZ() {
		return this.positive() ? 1.0 : 0.0;
	}

	public float ofsPN() {
		return this.positive() ? 1.0 : -1.0;
	}

	public float3 cardinalNormal() {
		float3 normal = 0.0;
		normal[this.axis()] = this.ofsPN();
		return normal;
	}
}

uint faceOpposingRay(float3 dir) {
	let absDir = abs(dir);
	if (absDir.x >= absDir.y && absDir.x >= absDir.z) {
		return dir.x >= 0.0 ? uint(Face.NX) : uint(Face.PX);
	}
	if (absDir.y >= absDir.z) {
		return dir.y >= 0.0 ? uint(Face.NY) : uint(Face.PY);
	}
	return dir.z >= 0.0 ? uint(Face.NZ) : uint(Face.PZ);
}

uint faceCrossed(float3 tMax, float3 dir) {
	if (tMax.x <= tMax.y && tMax.x <= tMax.z) {
		return dir.x >= 0.0 ? uint(Face.NX) : uint(Face.PX);
	}
	if (tMax.y <= tMax.z) {
		return dir.y >= 0.0 ? uint(Face.NY) : uint(Face.PY);
	}
	return dir.z >= 0.0 ? uint(Face.NZ) : uint(Face.PZ);
}

public struct RayHit {
	public bool hit; // other fields are meaningful only when true

	public uint chunkId;
	public uint3 voxelCoord; // local integer coord of hit voxel
	public uint voxelValue;
	public Face face;

	// local intersection pos, may be subvoxel
	public float3 localCoord;
	public float3 dir;

	// hit = ray.origin + ray.dir * t (||dir|| = 1)
	public float t;

	[mutating]
	void setVoxel(uint3 voxel, uint face, uint value, float t) {
		this.hit = true;
		this.voxelCoord = voxel;
		this.voxelValue = value;
		this.face = Face(face);
		this.t = t;
	}
}
// }}}

// {{{ inter-chunk traversal state
struct ChunkCursor {
	private WorldView world;
	private uint initChunkId;

	private bool exhausted;
	private uint lookupsLeft;
	private uint skipSide;

	private ChunkPagePtr pagePtr;
	private int3 unsignedCoord;
	private Ray ray;

	// starts at a chunk on the given ray, not the ray origin
	__init(WorldView world, uint initChunkId, Ray ray) {
		this.world = world;
		this.initChunkId = initChunkId;

		this.exhausted = false;
		this.lookupsLeft = 128;
		this.skipSide = 1;

		ChunkMetadata initMeta = this.world.chunkMeta[initChunkId];
		this.pagePtr = ChunkPagePtr(initMeta.realmId);
		this.unsignedCoord = int3(initMeta.unsignedCoord);
		this.ray = ray;
	}

	// yields chunks in order of ray hits
	// returns false when no more chunks are available or lookups are exhausted
	[mutating]
	bool nextChunk(out uint chunkId) {
		chunkId = 0;

		if (this.exhausted) return false;
		if (this.initChunkId != 0) {
			chunkId = this.initChunkId;
			this.initChunkId = 0;
			return true;
		}

		[loop]
		while (this.lookupsLeft != 0) {
			if (!this.advance()) break;
			this.lookupsLeft--;

			chunkId = this.world.chunkDir.lookup(
				this.unsignedCoord,
				this.pagePtr,
				this.skipSide,
			);

			if (chunkId != 0) return true;
		}

		this.exhausted = true;
		return false;
	}

	// move to next chunk intersected by ray along skipSide
	// returns false when cursor leaves realm cube
	[mutating]
	bool advance() {
		let side = int(this.skipSide);

		int3 regionMin = this.unsignedCoord & ~(side - 1);
		int3 regionMax = regionMin + side;
		let boundary = select(this.ray.dir > 0.0,
			float3(regionMax),
			float3(regionMin),
		);

		// keep signed arithmetic so origins outside the realm remain representable
		let origUnsignedCoord = int3(this.ray.unsignedCoord);

		// origin local coordinate in chunk units
		float3 origLocalChunks = this.ray.localCoord / CHUNK_SIDE;

		float3 boundDelta = float3(boundary - origUnsignedCoord) - origLocalChunks;
		float3 tMax = boundDelta / this.ray.dir;
		if (this.ray.dir.x == 0.0) tMax.x = FLOAT_MAX;
		if (this.ray.dir.y == 0.0) tMax.y = FLOAT_MAX;
		if (this.ray.dir.z == 0.0) tMax.z = FLOAT_MAX;

		float tNext = min(min(tMax.x, tMax.y), tMax.z);
		int3 nextCoord = origUnsignedCoord
			+ int3(floor(origLocalChunks + this.ray.dir * tNext));

		if (tMax.x == tNext) {
			nextCoord.x = this.ray.dir.x > 0.0 ? regionMax.x : regionMin.x - 1;
		}
		if (tMax.y == tNext) {
			nextCoord.y = this.ray.dir.y > 0.0 ? regionMax.y : regionMin.y - 1;
		}
		if (tMax.z == tNext) {
			nextCoord.z = this.ray.dir.z > 0.0 ? regionMax.z : regionMin.z - 1;
		}

		this.unsignedCoord = nextCoord;

		return all(nextCoord >= 0) && all(nextCoord < REALM_SIDE);
	}
};
// }}}

// {{{ traversal
static const float MIN_RAY_DIR = 1e-30;

// {{{ evil bit hacks for [1,2) tree space
struct TreeSpace {
	static const int ROOT_SCALE = 21;
	static const int LVL_BITS = 2;
	static const int STK_SZ = ROOT_SCALE / LVL_BITS + 1;

	static int stkSlot(int scaleExp) {
		return scaleExp >> 1;
	}

	static float3 fromLocal(float3 localCoord) {
		return localCoord / CHUNK_SIDE + 1.0;
	}

	static float toVoxelT(float treeT) {
		return treeT * CHUNK_SIDE;
	}

	static float3 clampInside(float3 pos) {
		return clamp(pos, 1.0, 1.9999999);
	}

	static uint3 voxelCoord(float3 pos) {
		return bit_cast<uint3>(pos) >> 15 & 0xFFu;
	}

	static uint childSlot(float3 pos, int scaleExp) {
		uint3 cell = bit_cast<uint3>(pos) >> scaleExp & 3u;
		return cell.x + cell.z * 4 + cell.y * 16;
	}

	static float3 cellMin(float3 pos, int scaleExp) {
		uint mask = ~0u << scaleExp;
		return bit_cast<float3>(bit_cast<uint3>(pos) & mask);
	}

	static float cellSz(int scaleExp) {
		return bit_cast<float>((scaleExp - 23 + 127) << 23);
	}

	static float3 cellMax(float3 cellMin, int scaleExp) {
		return bit_cast<float3>(bit_cast<int3>(cellMin) + ((1 << scaleExp) - 1));
	}

	// highest changed coordinate bit rounded down to a 64-tree lvl
	// result above ROOT_SCALE means traversal left the chunk
	static int ancestorScale(float3 previous, float3 next) {
		uint3 diffBits = bit_cast<uint3>(next) ^ bit_cast<uint3>(previous);
		int scaleExp = firstbithigh(diffBits.x | diffBits.y | diffBits.z);
		if ((scaleExp & 1) == 0) scaleExp--;
		return scaleExp;
	}
}
// }}}

public extension WorldView {
	// {{{ 64-tree trace
	// origin in tree space
	RayHit traceTree(Node root, float3 origin, float3 dir, uint entryFace) {
		RayHit hit;

		// TODO: groupshared stack causes segfault on linux nvidia
		uint stk[TreeSpace.STK_SZ];
		int scaleExp = TreeSpace.ROOT_SCALE;

		var ndIdx = 0u; // unused at root scale
		Node nd = root;

		if (dir.x == 0.0) dir.x = MIN_RAY_DIR;
		if (dir.y == 0.0) dir.y = MIN_RAY_DIR;
		if (dir.z == 0.0) dir.z = MIN_RAY_DIR;

		float3 invDir = 1.0 / dir;
		float3 pos = TreeSpace.clampInside(origin);
		uint hitFace = entryFace;
		var hitT = 0.0;

		// TODO: 512?
		[loop]
		for (uint i = 0; i < 512; i++) {
			uint childIdx = TreeSpace.childSlot(pos, scaleExp);

			while (!nd.isLeaf && ((nd.childMask >> childIdx) & 1) != 0) {
				stk[TreeSpace.stkSlot(scaleExp)] = ndIdx;
				nd = this.child(nd, childIdx, ndIdx);
				scaleExp -= TreeSpace.LVL_BITS;
				childIdx = TreeSpace.childSlot(pos, scaleExp);
			}

			if (nd.isLeaf && ((nd.childMask >> childIdx) & 1) != 0) {
				hit.setVoxel(
					TreeSpace.voxelCoord(pos),
					hitFace,
					this.leafValue(nd, childIdx),
					hitT
				);
				return hit;
			}

			int advScaleExp = scaleExp;
			if (((nd.childMask >> (childIdx & 0b101010)) & 0x00330033) == 0) {
				advScaleExp++;
			}

			pos = TreeSpace.cellMin(pos, advScaleExp);
			float3 prevPos = pos;
			float scale = TreeSpace.cellSz(advScaleExp);
			// tree-space tMax and tNext
			float3 tMax = (step(0.0, dir) * scale + (pos - origin)) * invDir;
			float tNext = min(min(tMax.x, tMax.y), tMax.z);
			hitFace = faceCrossed(tMax, dir);
			hitT = tNext;

			float3 siblingMin = select(tNext == tMax,
				pos + copysign(scale, dir),
				pos,
			);
			float3 siblingMax = TreeSpace.cellMax(siblingMin, advScaleExp);
			pos = clamp(origin + dir * tNext, siblingMin, siblingMax);

			int diffExp = TreeSpace.ancestorScale(prevPos, pos);

			if (diffExp > scaleExp) {
				if (diffExp > TreeSpace.ROOT_SCALE) {
					hit.hit = false;
					return hit;
				}

				scaleExp = diffExp;
				ndIdx = stk[TreeSpace.stkSlot(scaleExp)];
				nd = scaleExp == TreeSpace.ROOT_SCALE ? root : this.nodes[ndIdx];
			}
		}

		hit.hit = false;
		return hit;
	}
	// }}}

	// {{{ intersect chunk and trace its tree
	RayHit traceChunk(Ray ray, uint chunkId) {
		RayHit hit;

		let meta = this.chunkMeta[chunkId];
		let bounds = RelChunkBounds(meta, ray);

		float3 safeDir = ray.dir;
		if (safeDir.x == 0.0) safeDir.x = MIN_RAY_DIR;
		if (safeDir.y == 0.0) safeDir.y = MIN_RAY_DIR;
		if (safeDir.z == 0.0) safeDir.z = MIN_RAY_DIR;

		float3 invDir = 1.0 / safeDir;
		float3 tPlane1 = bounds.min * invDir;
		float3 tPlane2 = bounds.max * invDir;
		float3 tNearAxis = min(tPlane1, tPlane2);
		float3 tFarAxis = max(tPlane1, tPlane2);

		float tNear = max(max(tNearAxis.x, tNearAxis.y), tNearAxis.z);
		float tEnter = max(tNear, 0.0);
		float tExit = min(min(tFarAxis.x, tFarAxis.y), tFarAxis.z);
		if (tExit < tEnter) {
			hit.hit = false;
			return hit;
		}

		float3 posEntry = ray.dir * tEnter;
		float3 posTree = TreeSpace.fromLocal(posEntry - bounds.min);
		uint entryFace = faceOpposingRay(ray.dir);

		if (tNear >= 0.0) {
			entryFace = faceCrossed(-tNearAxis, ray.dir);
		}

		hit = this.traceTree(this.roots[chunkId], posTree, ray.dir, entryFace);
		if (hit.hit) {
			hit.chunkId = chunkId;
			hit.t = tEnter + TreeSpace.toVoxelT(hit.t);
			hit.localCoord = ray.dir * hit.t - bounds.min;
			hit.dir = ray.dir;
		}
		return hit;
	}
	// }}}

	public RayHit traceRay(Ray ray, uint initChunkId) {
		ChunkCursor cursor = ChunkCursor(this, initChunkId, ray);
		RayHit hit;
		uint chunkId;

		[loop]
		while (cursor.nextChunk(chunkId)) {
			hit = this.traceChunk(ray, chunkId);
			if (hit.hit) return hit;
		}

		hit.hit = false;
		return hit;
	}
}
// }}}

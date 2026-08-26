// based on https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing/
// TODO: traversal is vibed
import shared.camera;
import shared.world;

struct PushConstants {
	CameraState* camera;
	ChunkMetadata* chunkMetadata;
	Node** chunkRoots;

	uint32_t chunkIdIdx;
	uint32_t visbufIdx;
	uint32_t extentX;
	uint32_t extentY;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(1, 0)]] Texture2D<uint> chunkIdTextures[];
[[vk::binding(2, 0)]] RWTexture2D<uint64_t> visbufTextures[];

static const float MIN_RAY_DIR = 1e-30;

uint countBitsBefore(uint64_t mask, uint bitIdx) {
	uint word = uint(mask);
	uint count = 0;

	if (bitIdx >= 32) {
		count = countbits(word);
		word = uint(mask >> 32);
	}

	uint endBit = 1u << (bitIdx & 31u);
	return count + countbits(word & (endBit - 1u));
}

uint getCellIdx(float3 pos, int scaleExp) {
	uint3 cell = bit_cast<uint3>(pos) >> scaleExp & 3u;
	return cell.x + cell.z * 4 + cell.y * 16;
}

float3 floorToScale(float3 pos, int scaleExp) {
	uint mask = ~0u << scaleExp;
	return bit_cast<float3>(bit_cast<uint3>(pos) & mask);
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

void traceTree(
	Node* nodes,
	float3 origin,
	float3 dir,
	uint entryFace,
	out VisbufKey key,
) {
	uint stack[11]; // TODO: groupshared stack causes segfault on linux nvidia
	int scaleExp = 21; // 0.25

	uint nodeIdx = 0; // root
	Node node = nodes[nodeIdx];

	if (dir.x == 0.0) dir.x = MIN_RAY_DIR;
	if (dir.y == 0.0) dir.y = MIN_RAY_DIR;
	if (dir.z == 0.0) dir.z = MIN_RAY_DIR;

	float3 invDir = 1.0 / dir;
	float3 pos = clamp(origin, 1.0, 1.9999999);
	uint hitFace = entryFace;

	// TODO: 512?
	[loop]
	for (uint i = 0; i < 512; i++) {
		uint childIdx = getCellIdx(pos, scaleExp);

		while (!node.isLeaf && ((node.childMask >> childIdx) & 1) != 0) {
			stack[scaleExp >> 1] = nodeIdx;
			nodeIdx = node.childOfs + countBitsBefore(node.childMask, childIdx);
			node = nodes[nodeIdx];
			scaleExp -= 2;
			childIdx = getCellIdx(pos, scaleExp);
		}

		if (node.isLeaf && ((node.childMask >> childIdx) & 1) != 0) {
			key.write(pos, hitFace, node.matId);
			return;
		}

		int advanceScaleExp = scaleExp;
		if (((node.childMask >> (childIdx & 0b101010)) & 0x00330033) == 0) {
			advanceScaleExp++;
		}

		pos = floorToScale(pos, advanceScaleExp);
		float3 prevPos = pos;
		float scale = bit_cast<float>((advanceScaleExp - 23 + 127) << 23);
		float3 distSide = (step(0.0, dir) * scale + (pos - origin)) * invDir;
		float tmax = min(min(distSide.x, distSide.y), distSide.z);
		hitFace = faceCrossed(distSide, dir);

		float3 siblingMin = select(
			tmax == distSide,
			pos + copysign(scale, dir),
			pos,
		);
		float3 siblingMax = bit_cast<float3>(
			bit_cast<int3>(siblingMin) + ((1 << advanceScaleExp) - 1),
		);
		pos = clamp(origin + dir * tmax, siblingMin, siblingMax);

		uint3 diffBits = bit_cast<uint3>(pos) ^ bit_cast<uint3>(prevPos);
		int diffExp = firstbithigh(diffBits.x | diffBits.y | diffBits.z);
		if ((diffExp & 1) == 0) diffExp--;

		if (diffExp > scaleExp) {
			if (diffExp > 21) {
				key.clear();
				return;
			}

			scaleExp = diffExp;
			nodeIdx = stack[scaleExp >> 1];
			node = nodes[nodeIdx];
		}
	}

	key.clear();
}

void traceChunk(Ray ray, uint chunkId, out VisbufKey key) {
	ChunkMetadata meta = push.chunkMetadata[chunkId];
	float3 chunkMin = float3(meta.x, meta.y, meta.z) * 256.0;
	float3 chunkMax = chunkMin + 256.0;

	float3 safeDir = ray.dir;

	if (safeDir.x == 0.0) safeDir.x = MIN_RAY_DIR;
	if (safeDir.y == 0.0) safeDir.y = MIN_RAY_DIR;
	if (safeDir.z == 0.0) safeDir.z = MIN_RAY_DIR;

	float3 invDir = 1.0 / safeDir;
	float3 plane1 = (chunkMin - ray.pos) * invDir;
	float3 plane2 = (chunkMax - ray.pos) * invDir;
	float3 planeNear = min(plane1, plane2);
	float3 planeFar = max(plane1, plane2);

	float distNear = max(max(planeNear.x, planeNear.y), planeNear.z);
	float distMin = max(distNear, 0.0);
	float distMax = min(min(planeFar.x, planeFar.y), planeFar.z);
	if (distMax < distMin) {
		key.clear();
		return;
	}

	float3 posEntry = ray.pos + ray.dir * distMin;
	float3 posTree = (posEntry - chunkMin) / 256.0 + 1.0;
	Node* nodes = push.chunkRoots[chunkId];
	uint entryFace = faceOpposingRay(ray.dir);

	if (distNear >= 0.0) {
		entryFace = faceCrossed(-planeNear, ray.dir);
	}

	traceTree(nodes, posTree, ray.dir, entryFace, key);
	if (key.isHit()) key.data |= uint64_t(chunkId) << 28;
}

void traceWorld(Ray ray, uint initialChunkId, out VisbufKey key) {
	uint chunkId = initialChunkId;

	[loop]
	for (uint chunkStep = 0; chunkStep < 1; chunkStep++) {
		traceChunk(ray, chunkId, key);
		if (key.isHit()) return;

		// TODO(world): select next chunk via top-level DDA
		break;
	}

	key.clear();
}

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	if (threadId.x >= push.extentX || threadId.y >= push.extentY) {
		return;
	}

	RWTexture2D<uint64_t> visbufTex = visbufTextures[push.visbufIdx];
	uint chunkId = chunkIdTextures[push.chunkIdIdx].Load(threadId);
	VisbufKey key;
	key.clear();

	if (chunkId != 0xFFFFFFFF) {
		float2 uv = (float2(threadId.xy) + 0.5) / float2(push.extentX, push.extentY);
		Ray primary = push.camera.primaryRay(uv);
		traceWorld(primary, chunkId, key);
	}

	visbufTex.Store(threadId.xy, key.data);
}

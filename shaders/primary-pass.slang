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

static const uint64_t NO_VOXEL = 0xFFFFFFFFFFFFFFFF;
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

uint64_t packVoxelPos(float3 pos) {
	uint3 voxel = bit_cast<uint3>(pos) >> 15 & 0xFFu;
	return uint64_t(voxel.x)
	     | uint64_t(voxel.y) << 8
	     | uint64_t(voxel.z) << 16;
}

uint64_t traceTree(Node* nodes, float3 origin, float3 dir) {
	uint stack[11]; // TODO: groupshared stack causes segfault on linux nvidia
	int scaleExp = 21; // 0.25

	uint nodeIdx = 0; // root
	Node node = nodes[nodeIdx];

	if (dir.x == 0.0) dir.x = MIN_RAY_DIR;
	if (dir.y == 0.0) dir.y = MIN_RAY_DIR;
	if (dir.z == 0.0) dir.z = MIN_RAY_DIR;

	float3 invDir = 1.0 / dir;
	float3 pos = clamp(origin, 1.0, 1.9999999);

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
			// TODO: return actual ID, this is just a placeholder
			return packVoxelPos(pos);
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
			if (diffExp > 21) return NO_VOXEL;

			scaleExp = diffExp;
			nodeIdx = stack[scaleExp >> 1];
			node = nodes[nodeIdx];
		}
	}

	return NO_VOXEL;
}

uint64_t traceChunk(Ray ray, uint chunk_id) {
	ChunkMetadata meta = push.chunkMetadata[chunk_id];
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

	float distMin = max(max(max(planeNear.x, planeNear.y), planeNear.z), 0.0);
	float distMax = min(min(planeFar.x, planeFar.y), planeFar.z);
	if (distMax < distMin) return NO_VOXEL;

	float3 posEntry = ray.pos + ray.dir * distMin;
	float3 posTree = (posEntry - chunkMin) / 256.0 + 1.0;
	Node* nodes = push.chunkRoots[chunk_id];

	return traceTree(nodes, posTree, ray.dir);
}

uint64_t traceWorld(Ray ray, uint initial_chunk_id) {
	uint chunk_id = initial_chunk_id;

	[loop]
	for (uint chunk_step = 0; chunk_step < 1; chunk_step++) {
		uint64_t voxel_id = traceChunk(ray, chunk_id);
		if (voxel_id != NO_VOXEL) return voxel_id;

		// TODO(world): select next chunk via top-level DDA
		break;
	}

	return NO_VOXEL;
}

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	if (threadId.x >= push.extentX || threadId.y >= push.extentY) {
		return;
	}

	RWTexture2D<uint64_t> visbufTex = visbufTextures[push.visbufIdx];
	uint chunkId = chunkIdTextures[push.chunkIdIdx].Load(threadId);

	if (chunkId == 0xFFFFFFFF) {
		// guaranteed no hit, there is no chunk on that pixel
		visbufTex.Store(threadId.xy, NO_VOXEL);
	} else {
		float2 uv = (float2(threadId.xy) + 0.5) / float2(push.extentX, push.extentY);
		Ray primary = push.camera.primaryRay(uv);
		visbufTex.Store(threadId.xy, traceWorld(primary, chunkId));
	}
}

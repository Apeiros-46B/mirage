// based on https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing/
// TODO: traversal is vibed
// TODO: move more traversal stuff to shared.raymarch, need to reuse in for shading
import shared.camera;
import shared.raymarch;
import shared.space;
import shared.world;

struct PushConstants {
	CameraState* camera;

	Node* nodes;
	Node* chunkRoots;
	ChunkMetadata* chunkMetadata;
	ChunkDirectory chunkDirectory;

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

// TODO(design): should this even take in nodes param, since the layout is expected
// to match exactly push.nodes and it's only ever used in one place?
void traceTree(
	Node root,
	Node* nodes,
	float3 origin,
	float3 dir,
	uint entryFace,
	out VisbufKey key,
) {
	uint stack[11]; // TODO: groupshared stack causes segfault on linux nvidia
	int scaleExp = 21; // 0.25

	uint nodeIdx = 0; // unused at root scale
	Node node = root;

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
			node = scaleExp == 21 ? root : nodes[nodeIdx];
		}
	}

	key.clear();
}

void traceChunk(Ray ray, uint chunkId, out VisbufKey key) {
	ChunkMetadata meta = push.chunkMetadata[chunkId];
	RelChunkBounds bounds = RelChunkBounds(
		meta.logicalPos(),
		ray.chunkPos,
		ray.localPos,
	);

	float3 safeDir = ray.dir;

	if (safeDir.x == 0.0) safeDir.x = MIN_RAY_DIR;
	if (safeDir.y == 0.0) safeDir.y = MIN_RAY_DIR;
	if (safeDir.z == 0.0) safeDir.z = MIN_RAY_DIR;

	float3 invDir = 1.0 / safeDir;
	float3 plane1 = bounds.min * invDir;
	float3 plane2 = bounds.max * invDir;
	float3 planeNear = min(plane1, plane2);
	float3 planeFar = max(plane1, plane2);

	float distNear = max(max(planeNear.x, planeNear.y), planeNear.z);
	float distMin = max(distNear, 0.0);
	float distMax = min(min(planeFar.x, planeFar.y), planeFar.z);
	if (distMax < distMin) {
		key.clear();
		return;
	}

	float3 posEntry = ray.dir * distMin;
	float3 posTree = (posEntry - bounds.min) / CHUNK_SIDE + 1.0;
	uint entryFace = faceOpposingRay(ray.dir);

	if (distNear >= 0.0) {
		entryFace = faceCrossed(-planeNear, ray.dir);
	}

	traceTree(
		push.chunkRoots[chunkId],
		push.nodes,
		posTree,
		ray.dir,
		entryFace,
		key
	);
	if (key.isHit()) key.data |= uint64_t(chunkId) << 28;
}

void traceWorld(Ray ray, uint initChunkId, out VisbufKey key) {
	ChunkMetadata initMeta = push.chunkMetadata[initChunkId];
	ChunkCursor cur;
	cur.coord = int3(initMeta.x, initMeta.y, initMeta.z);
	cur.realmId = initMeta.realmId;

	traceChunk(ray, initChunkId, key);
	if (key.isHit()) return;
	if (!cur.advance(ray, 1)) {
		key.clear();
		return;
	}

	// TODO: when to stop looping?
	[loop]
	for (uint chunkStep = 0; chunkStep < 128; chunkStep++) {
		uint skipSide;
		uint chunkId = push.chunkDirectory.lookupChunk(cur, skipSide);

		if (chunkId != 0) {
			traceChunk(ray, chunkId, key);
			if (key.isHit()) return;
		}

		if (!cur.advance(ray, skipSide)) break;
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

	if (chunkId != 0) {
		float2 uv = (float2(threadId.xy) + 0.5) / float2(push.extentX, push.extentY);
		Ray primary = push.camera.primaryRay(uv);
		traceWorld(primary, chunkId, key);
	}

	visbufTex.Store(threadId.xy, key.data);
}

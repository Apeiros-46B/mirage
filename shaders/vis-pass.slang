import shared.world;
import shared.raymarch;

struct PushConstants {
	uint32_t visbufIdx;
	uint32_t colorIdx;
	uint32_t extentX;
	uint32_t extentY;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(2, 0)]] RWTexture2D<uint64_t> visbufTextures[];
[[vk::binding(2, 0)]] RWTexture2D<float4> colorTextures[];

// #define DEBUG_POSITION
// #define DEBUG_POSITION_MOD
#define DEBUG_FACE
// #define DEBUG_MATERIAL
// #define DEBUG_CHUNK

float3 hashColor(uint state) {
	uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
	word = (word >> 22u) ^ word;
	return float3(word & 255, word >> 8 & 255, word >> 16 & 255) / 255.0;
}

float3 faceColor(Face face) {
	switch (face) {
	case Face.PX: return float3(1.0, 0.0, 0.0);
	case Face.PY: return float3(0.0, 1.0, 0.0);
	case Face.PZ: return float3(0.0, 0.0, 1.0);
	case Face.NX: return float3(0.0, 1.0, 1.0);
	case Face.NY: return float3(1.0, 0.0, 1.0);
	case Face.NZ: return float3(1.0, 1.0, 0.0);
	default: return float3(1.0);
	}
}

float3 keyToColor(VisbufKey key) {
	if (key.matId == 0) return float3(1.0);

#if defined(DEBUG_POSITION)
	return float3(key.x, key.y, key.z) / 255.0;
#elif defined(DEBUG_POSITION_MOD)
	return float3(key.x % 16, key.y % 16, key.z % 16) * 16.0 / 255.0;
#elif defined(DEBUG_FACE)
	return faceColor(key.face);
#elif defined(DEBUG_MATERIAL)
	return hashColor(key.matId);
#elif defined(DEBUG_CHUNK)
	return hashColor(key.chunkId);
#else
	return float3(1.0, 0.0, 1.0);
#endif
}

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	if (threadId.x >= push.extentX || threadId.y >= push.extentY) {
		return;
	}

	RWTexture2D<uint64_t> visbufTex = visbufTextures[push.visbufIdx];
	RWTexture2D<float4> colorTex = colorTextures[push.colorIdx];

	VisbufKey key;
	key.data = visbufTex.Load(threadId.xy);
	colorTex.Store(threadId.xy, float4(keyToColor(key), 1.0));
}

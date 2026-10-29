import shared.shading;
import shared.camera;
import shared.raymarch;
import shared.world;

#define PATH_TRACE
// #define DEBUG_POSITION
// #define DEBUG_POSITION_MOD
// #define DEBUG_FACE
// #define DEBUG_MATERIAL
// #define DEBUG_CHUNK

struct PushConstants {
	CameraState* camera;
	WorldView world;

	uint visbufIdx;
	uint colorIdx;
	uint2 extent;

	uint ptAccumIdx;
	uint sampleCount; // accumulated samples for this view, 0 on camera change
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(2, 0)]]
RWTexture2D<uint64_t> visbufTextures[];

[[vk::binding(2, 0)]]
[[vk::image_format("rgba32f")]]
RWTexture2D<float4> colorTextures[];

float3 keyToColor(uint3 threadId, VisbufKey key) {

#if defined(PATH_TRACE)

	float3 color = tracePath(
		push.world, *push.camera, key,
		threadId.xy, push.extent,
		push.sampleCount,
	);
	let ptAccum = colorTextures[push.ptAccumIdx];

	// do not load undefined contents on the first sample
	if (push.sampleCount != 0) {
		float weight = 1.0 / (float(push.sampleCount) + 1.0);
		color = lerp(ptAccum.Load(threadId.xy).rgb, color, weight);
	}

	ptAccum.Store(threadId.xy, float4(color, 1.0));
	return color;

#else

// {{{ debug
	if (!key.isHit()) return float3(1.0);
#if defined(DEBUG_POSITION)
	return float3(key.x, key.y, key.z) / 255.0;
#elif defined(DEBUG_POSITION_MOD)
	return float3(key.x % 16, key.y % 16, key.z % 16) * 16.0 / 255.0;
#elif defined(DEBUG_FACE)
	switch (key.face) {
		case Face.PX: return float3(1.0, 0.0, 0.0);
		case Face.PY: return float3(0.0, 1.0, 0.0);
		case Face.PZ: return float3(0.0, 0.0, 1.0);
		case Face.NX: return float3(0.0, 1.0, 1.0);
		case Face.NY: return float3(1.0, 0.0, 1.0);
		case Face.NZ: return float3(1.0, 1.0, 0.0);
		default: return float3(1.0);
	}
#elif defined(DEBUG_MATERIAL)
	return randColor(Material(key.voxelValue).id);
#elif defined(DEBUG_CHUNK)
	return randColor(key.chunkId);
#else
	return float3(0.9);
#endif
// }}}

#endif

}

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	if (any(threadId.xy >= push.extent)) return;

	RWTexture2D<uint64_t> visbufTex = visbufTextures[push.visbufIdx];
	RWTexture2D<float4> colorTex = colorTextures[push.colorIdx];

	VisbufKey key;
	key.data = visbufTex.Load(threadId.xy);
	colorTex.Store(threadId.xy, float4(keyToColor(threadId, key), 1.0));
}

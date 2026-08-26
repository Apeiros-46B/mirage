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

#define DEBUG_COLOR
// #define DEBUG_COLOR_MOD

float3 id_to_color(uint64_t v) {
	// no voxel hit
	if (v == 0xFFFFFFFFFFFFFFFF) return float3(1.0, 1.0, 1.0);

#if defined(DEBUG_COLOR)
	uint r = uint(v) & 255;
	uint g = uint(v >> 8) & 255;
	uint b = uint(v >> 16) & 255;
#elif defined(DEBUG_COLOR_MOD)
	uint r = (uint(v) & 255) % 16 * 16;
	uint g = (uint(v >> 8) & 255) % 16 * 16;
	uint b = (uint(v >> 16) & 255) % 16 * 16;
#else
	uint state = uint(v ^ (v >> 32));

	// PCG hash
	uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
	word = (word >> 22u) ^ word;

	uint r = word & 255;
	uint g = (word >> 8) & 255;
	uint b = (word >> 16) & 255;
#endif

	return float3(r, g, b) / 255.0;
}

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	if (threadId.x >= push.extentX || threadId.y >= push.extentY) {
		return;
	}

	RWTexture2D<uint64_t> visbufTex = visbufTextures[push.visbufIdx];
	RWTexture2D<float4> colorTex = colorTextures[push.colorIdx];

	uint64_t voxel_id = visbufTex.Load(threadId.xy);
	colorTex.Store(threadId.xy, float4(id_to_color(voxel_id), 1.0));
}

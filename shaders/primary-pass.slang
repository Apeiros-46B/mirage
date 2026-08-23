import shared.camera;
import shared.world;

struct PushConstants {
	CameraState* camera;
	uint64_t voxelDataBda;
	uint64_t chunkRootsBda;
	uint64_t chunkMetadataBda;
	uint32_t depthIdx;
	uint32_t visbufIdx;
	uint32_t extentX;
	uint32_t extentY;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(1, 0)]] Texture2D<float> depthTextures[];
[[vk::binding(2, 0)]] RWTexture2D<uint64_t> visbufTextures[];

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	if (threadId.x >= push.extentX || threadId.y >= push.extentY) {
		return;
	}

	Texture2D<float> depthTex = depthTextures[push.depthIdx];
	RWTexture2D<uint64_t> visbufTex = visbufTextures[push.visbufIdx];

	float rawDepth = depthTex.Load(threadId);

	if (rawDepth == 1.0) {
		// guaranteed no hit, there is no chunk on that pixel
		visbufTex.Store(threadId.xy, 0xFFFFFFFFFFFFFFFF);
	} else {
		float2 uv = (float2(threadId.xy) + 0.5) / float2(push.extentX, push.extentY);
		float depth = push.camera.linearizeDepth(uv, rawDepth);
		Ray primary = push.camera.primaryRay(uv);
		Ray start = primary.advanced(depth + 0.001);

		// TODO: trace rays to get actual voxel ID
		// this is just temporary for debugging ray hit pos
		int x = int(floor(start.pos.x + 0.0001)) % 16;
		int y = int(floor(start.pos.y + 0.0001)) % 16;
		int z = int(floor(start.pos.z + 0.0001)) % 16;

		uint64_t v = 0;
		v |= (uint64_t(x) & 0xFF);
		v |= (uint64_t(y) & 0xFF) << 8;
		v |= (uint64_t(z) & 0xFF) << 16;

		visbufTex.Store(threadId.xy, v);
	}
}

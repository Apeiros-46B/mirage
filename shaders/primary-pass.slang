import shared.camera;

struct PushConstants {
	CameraState* camera;
	uint64_t worldBda;
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

	float depth = depthTex.Load(threadId);

	if (depth == 1.0) {
		// guaranteed no hit
		visbufTex.Store(threadId.xy, 0);
	} else {
		// TODO: trace rays to get actual voxel ID
		visbufTex.Store(threadId.xy, 1);
	}
}

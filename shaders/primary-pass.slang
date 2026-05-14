import shared.camera;

struct PushConstants {
	CameraState* camera;
	uint64_t worldBda;
	uint32_t depthIdx;
	uint32_t colorStorageIdx;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(1, 0)]] Texture2D<float> depthTextures[];
[[vk::binding(2, 0)]] RWTexture2D<float4> colorTextures[];

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	Texture2D<float> depth = depthTextures[push.depthIdx];
	RWTexture2D<float4> color = colorTextures[push.colorStorageIdx];

	uint width, height, mipLevels;
	color.GetDimensions(0, width, height, mipLevels);

	if (threadId.x >= width || threadId.y >= height) {
		return;
	}

	// ts is ridiculous
	float value = pow(depth.Load(threadId), 1000);
	color.Store(threadId.xy, float4(value, value, value, 1.0));
}

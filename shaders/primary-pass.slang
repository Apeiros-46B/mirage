import shared.camera;
import shared.raymarch;
import shared.world;
import shared.shading;

struct PushConstants {
	CameraState* camera;
	WorldView world;

	uint chunkIdIdx;
	uint visbufIdx;
	uint2 extent;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(1, 0)]] Texture2D<uint> chunkIdTextures[];
[[vk::binding(2, 0)]] RWTexture2D<uint64_t> visbufTextures[];

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	if (any(threadId.xy >= push.extent)) {
		return;
	}

	let visbufTex = visbufTextures[push.visbufIdx];
	let chunkId = chunkIdTextures[push.chunkIdIdx].Load(threadId);

	VisbufKey key;
	key.data = 0; // default initializer does not work...

	if (chunkId != 0) {
		let primary = push.camera.primaryRay(threadId.xy, push.extent);
		let hit = push.world.traceRay(primary, chunkId);
		key.write(hit);
	}

	visbufTex.Store(threadId.xy, key.data);
}

import shared.camera;
import shared.world;

struct PushConstants {
	CameraState* camera;
	// TODO: chunkmetadata buf is not guaranteed to be contiguous, eventually
	// need some kind of collect/cull pass which builds a drawlist for us. can
	// leave it like this when we're still manually loading chunks without a
	// streaming system though.
	ChunkMetadata* chunks;
};

struct VertexOutput {
	float4 pos : SV_Position;
	nointerpolation uint chunkId : CHUNK_ID;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[shader("vertex")]
VertexOutput vsh(uint vertId : SV_VertexID, uint chunkId : SV_InstanceID) {
	const float3 CORNERS[8] = {
		float3(0, 0, 0), float3(1, 0, 0), float3(1, 1, 0), float3(0, 1, 0),
		float3(0, 0, 1), float3(1, 0, 1), float3(1, 1, 1), float3(0, 1, 1)
	};

	// ccw winding
	const uint INDICES[36] = {
		0, 1, 2, 0, 2, 3, // front (-Z)
		5, 4, 7, 5, 7, 6, // back (+Z)
		3, 2, 6, 3, 6, 7, // top (+Y)
		4, 5, 1, 4, 1, 0, // bottom (-Y)
		1, 5, 6, 1, 6, 2, // right (+X)
		4, 0, 3, 4, 3, 7  // left (-X)
	};

	ChunkMetadata meta = push.chunks[chunkId];

	float3 chunkLocalPos = CORNERS[INDICES[vertId]];
	float3 realmLocalPos = float3(
		float(meta.x),
		float(meta.y),
		float(meta.z),
	);
	float3 worldPos = (chunkLocalPos + realmLocalPos) * 256.0;

	VertexOutput out;
	out.pos = mul(push.camera.viewProj, float4(worldPos, 1.0));
	out.chunkId = chunkId;
	return out;
}

// just return the chunk ID to the render target
[shader("fragment")]
[earlydepthstencil]
uint fsh(VertexOutput in) : SV_Target0 {
	return in.chunkId;
}

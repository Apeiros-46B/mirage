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
VertexOutput vsh(uint vertId : SV_VertexID, uint chunkIdx : SV_InstanceID) {
	const float3 CORNERS[8] = {
		float3(0, 0, 0), float3(1, 0, 0), float3(1, 1, 0), float3(0, 1, 0),
		float3(0, 0, 1), float3(1, 0, 1), float3(1, 1, 1), float3(0, 1, 1)
	};

	// ccw winding
	const uint INDICES[36] = {
		0, 1, 2, 0, 2, 3, // front  (-Z)
		5, 4, 7, 5, 7, 6, // back   (+Z)
		3, 2, 6, 3, 6, 7, // top    (+Y)
		4, 5, 1, 4, 1, 0, // bottom (-Y)
		1, 5, 6, 1, 6, 2, // right  (+X)
		4, 0, 3, 4, 3, 7  // left   (-X)
	};

	// chunk id 0 is reserved as a sentinel for "no chunk"
	uint chunkId = chunkIdx + 1;
	ChunkMetadata meta = push.chunks[chunkId];

	float3 chunkMin = float3(float(meta.x), float(meta.y), float(meta.z)) * 256.0;
	float3 chunkMax = chunkMin + 256.0;

	bool cameraInside = all(push.camera.pos >= chunkMin)
	                 && all(push.camera.pos < chunkMax);

	VertexOutput vert;
	vert.chunkId = chunkId;
	if (cameraInside) {
		const float2 FULLSCREEN_TRI[3] = {
			float2(-1, -1),
			float2(-1,  3),
			float2( 3, -1),
		};

		if (vertId < 3) {
			// depth 0.0 is the near plane, occludes all other chunk proxies
			// TODO: if using reverse-Z, i think this needs to be changed to 1.0
			vert.pos = float4(FULLSCREEN_TRI[vertId], 0.0, 1.0);
		} else {
			// form remaining verts into invisible degenerate tris
			vert.pos = float4(0.0, 0.0, 0.0, 1.0);
		}
		return vert;
	}

	uint axisId = vertId / 6;
	uint faceId;
	bool faceVis;

	if (axisId == 0) {
		faceId  = push.camera.pos.x < chunkMin.x ? 5 : 4;
		faceVis = push.camera.pos.x < chunkMin.x
		       || push.camera.pos.x > chunkMax.x;
	} else if (axisId == 1) {
		faceId  = push.camera.pos.y < chunkMin.y ? 3 : 2;
		faceVis = push.camera.pos.y < chunkMin.y
		       || push.camera.pos.y > chunkMax.y;
	} else {
		faceId  = push.camera.pos.z < chunkMin.z ? 0 : 1;
		faceVis = push.camera.pos.z < chunkMin.z
		       || push.camera.pos.z > chunkMax.z;
	}

	if (!faceVis) {
		vert.pos = float4(0.0, 0.0, 0.0, 1.0);
	} else {
		uint faceVertId = vertId % 6;
		uint cornerId = INDICES[faceId * 6 + faceVertId];
		float3 relPos = (chunkMin - push.camera.pos) + CORNERS[cornerId] * 256.0;
		vert.pos = mul(push.camera.relViewProj, float4(relPos, 1.0));
	}

	return vert;
}

// just return the chunk ID to the render target
[shader("fragment")]
[earlydepthstencil]
uint fsh(VertexOutput vert) : SV_Target0 {
	return vert.chunkId;
}

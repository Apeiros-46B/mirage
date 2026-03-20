import shared.camera;

struct PushConstants {
	CameraState* camera;
	uint64_t worldBda;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[shader("vertex")]
float4 vsh(uint vertexID : SV_VertexID) : SV_Position {
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

	float3 localPos = CORNERS[INDICES[vertexID]];
	localPos *= 256.0; // chunk scale

	// do real world position later
	float3 worldPos = localPos + float3(0.0, 0.0, 0.0);
	return mul(push.camera.viewProj, float4(worldPos, 1.0));
}

[shader("fragment")]
void fsh() {}

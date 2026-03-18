struct PushConstants {
	float2 scrSz;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(0, 0)]]
Sampler2D sampler;

// egui::epaint::Vertex
struct VertexInput {
	float2 pos   : POSITION;  // 00: R32G32_SFLOAT
	float2 uv    : TEXCOORD0; // 08: R32G32_SFLOAT
	float4 color : COLOR0;    // 16: R8G8B8A8_UNORM (auto conv to [0..1])
};

struct VertexOutput {
	float4 position : SV_Position;
	float2 uv       : TEXCOORD0;
	float4 color    : COLOR0;
};

[shader("vertex")]
VertexOutput vsh(VertexInput vi) {
	VertexOutput vo;

	// transform egui pixel coordinates [0, scrSz] into vulkan clip space [-1, 1]
	// both coordinate systems are y-down
	vo.position = float4(
		2.0 * vi.pos.x / push.scrSz.x - 1.0,
		2.0 * vi.pos.y / push.scrSz.y - 1.0,
		0.0,
		1.0
	);

	vo.uv = vi.uv;
	vo.color = vi.color;

	return vo;
}

[shader("fragment")]
float4 fsh(VertexOutput vo) : SV_Target {
	// egui vert colors are premultiplied
	return vo.color * sampler.Sample(vo.uv);
}

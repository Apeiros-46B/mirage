import shared.tonemap;

struct PushConstants {
	uint colorIdx;
	uint encodeSrgb;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(2, 0)]] [[vk::image_format("rgba32f")]]
RWTexture2D<float4> colorTextures[];

[shader("vertex")]
float4 vsh(uint vertId : SV_VertexID) : SV_Position {
	// screen-sized tri
	float2 pos = float2((vertId << 1) & 2, vertId & 2);
	return float4(pos * 2.0 - 1.0, 0.0, 1.0);
}

[shader("fragment")]
float4 fsh(float4 pos : SV_Position) : SV_Target {
	float3 color = colorTextures[push.colorIdx].Load(uint2(pos.xy)).rgb;
	if (push.encodeSrgb != 0) color = linearToSrgb(color);
	return float4(color, 1.0);
}

// matches crate::workers::msgs::CameraState
struct CameraState {
	float4x4 relViewProj;
	float4x4 invProj;
	float4x4 invView;
	float3 pos;
	float _pad;

	// uv components in 0-1 range. returns ray with origin at camera
	Ray primaryRay(float2 uv) {
		// ndc xy is -1 to 1
		float4 targ = mul(this.invProj, float4(uv * 2.0 - 1.0, 1.0, 1.0));
		float3 dir = mul(this.invView, float4(normalize(targ.xyz / targ.w), 0.0)).xyz;
		return Ray(this.pos, dir);
	}
}

struct Ray {
	float3 pos;
	float3 dir;
}

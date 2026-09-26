// matches crate::workers::msgs::CameraState
struct CameraState {
	float4x4 relViewProj;
	float4x4 invProj;
	float4x4 invView;
	int3 chunkPos;
	int _chunkPad;
	float3 localPos;
	float _localPad;

	// uv components in 0-1 range. returns ray with origin at camera
	Ray primaryRay(float2 uv) {
		// ndc xy is -1 to 1
		float4 targ = mul(this.invProj, float4(uv * 2.0 - 1.0, 1.0, 1.0));
		float3 dir = mul(this.invView, float4(normalize(targ.xyz / targ.w), 0.0)).xyz;
		return Ray(this.chunkPos, this.localPos, dir);
	}
}

struct Ray {
	int3 chunkPos;
	float3 localPos;
	float3 dir;
}

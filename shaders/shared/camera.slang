// TODO: camera-relative rendering
// matches crate::workers::msgs::CameraState
struct CameraState {
	float4x4 viewProj;
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

	// uv components and raw depth in 0-1 range. returns world-space depth
	float linearizeDepth(float2 uv, float rawDepth) {
		// ndc xy is [-1,1], z is [0,1]
		float4 clipSpace = float4(uv * 2.0 - 1.0, rawDepth, 1.0);
		float4 viewSpace = mul(this.invProj, clipSpace);
		viewSpace.xyz /= viewSpace.w;
		// camera is at (0,0,0) in view space, length() gives distance
		return length(viewSpace.xyz);
	}
}

struct Ray {
	float3 pos;
	float3 dir;

	// march forward by the given distance. dist is world-space depth
	[mutating]
	void advance(float dist) {
		this.pos += dist * this.dir;
	}

	// copy marched forward by the given distance. dist is world-space depth
	Ray advanced(float dist) {
		return Ray(this.pos + dist * this.dir, this.dir);
	}
}

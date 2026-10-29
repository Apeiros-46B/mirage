// signed coord: signed chunks
// local coord: voxel units [0,256)
// RH world axes +Y up
import world;

// matches crate::workers::msgs::CameraState
public struct CameraState : ISplitCoord {
	public float4x4 relViewProj;
	private float4x4 invProj;
	private float4x4 invView;
	public int3 signedCoord;
	private int _chunkPad;
	public float3 localCoord;
	private float _localPad;

	// may need to be public later
	// pixel-center uv (top left = (0,0), bot right = (1,1))
	static float2 uv(uint2 xy, uint2 extent) {
		return (float2(xy) + 0.5) / float2(extent);
	}

	// unit ray given screen pixel and screen extent
	public Ray primaryRay(uint2 xy, uint2 extent) {
		float2 uv = this.uv(xy, extent);

		// ndc xy [-1,1], depth [0,1]
		// invProj includes proj y flip, dont flip uv again
		// invView rot into world axes without translation
		float4 targ = mul(this.invProj, float4(uv * 2.0 - 1.0, 1.0, 1.0));
		float3 dir = mul(this.invView, float4(normalize(targ.xyz / targ.w), 0.0)).xyz;

		return Ray(this.signedCoord, this.localCoord, dir);
	}
}

public struct Ray : ISplitCoord {
	public int3 signedCoord;
	public float3 localCoord;
	public float3 dir; // world axes, may not be unit, must be nonzero
}

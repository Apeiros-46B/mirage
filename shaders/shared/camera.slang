// matches crate::workers::msgs::CameraState
struct CameraState {
	float4x4 viewProj;
	float4x4 invProj;
	float4x4 invView;
	float3 pos;
	float _pad;
}

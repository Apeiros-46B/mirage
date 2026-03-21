import shared.camera;

struct PushConstants {
	CameraState* camera;
};

[[vk::push_constant]]
ConstantBuffer<PushConstants> push;

[[vk::binding(0, 0)]] Texture2D<float> depthTex;
[[vk::binding(1, 0)]] RWTexture2D<float4> outColor;

[shader("compute")]
[numthreads(8, 8, 1)]
void main(uint3 threadId : SV_DispatchThreadID) {
	uint2 pixel = threadId.xy;
	uint2 screenSz;
	outColor.GetDimensions(screenSz.x, screenSz.y);

	if (pixel.x >= screenSz.x || pixel.y >= screenSz.y) return;

	// 1. Reconstruct primary ray
	float2 uv = (float2(pixel) + 0.5) / float2(screenSz);
	float2 ndc = uv * 2.0 - 1.0;

	float4 target = mul(push.camera.invProj, float4(ndc.x, ndc.y, 1.0, 1.0));
	float3 rayDir = normalize(mul(push.camera.invView, float4(normalize(target.xyz / target.w), 0.0)).xyz);
	float3 rayPos = push.camera.pos;

	// 2. Read and linearize hardware depth to bound traversal
	float hwDepth = depthTex.Load(int3(pixel, 0)).x;

	// Un-project the depth buffer value to view space
	float4 viewPos = mul(push.camera.invProj, float4(0.0, 0.0, hwDepth, 1.0));
	float linearDepth = abs(viewPos.z / viewPos.w);
	float maxDistance = (hwDepth == 1.0) ? 10000.0 : linearDepth;

	// 3. Single-Level DDA Setup
	float3 mapPos = floor(rayPos);
	float3 deltaDist = abs(1.0 / rayDir);
	float3 raySign = sign(rayDir);
	float3 stepDir = raySign;
	float3 sideDist = (raySign * (mapPos - rayPos) + (raySign * 0.5) + 0.5) * deltaDist;

	bool hit = false;
	float t = 0.0;
	float3 normal = float3(0, 0, 0);

	float3 sphereCenter = float3(128.0, 128.0, 128.0);
	float sphereRadius = 64.0;

	// 4. Traversal Loop
	for (int i = 0; i < 512; i++) {
		// Implicit geometry check: distance from voxel center to sphere center
		float3 voxelCenter = mapPos + 0.5;
		if (distance(voxelCenter, sphereCenter) <= sphereRadius) {
			hit = true;
			break;
		}

		// Advance DDA
		if (sideDist.x < sideDist.y) {
			if (sideDist.x < sideDist.z) {
				t = sideDist.x;
				sideDist.x += deltaDist.x;
				mapPos.x += stepDir.x;
				normal = float3(-stepDir.x, 0, 0);
			} else {
				t = sideDist.z;
				sideDist.z += deltaDist.z;
				mapPos.z += stepDir.z;
				normal = float3(0, 0, -stepDir.z);
			}
		} else {
			if (sideDist.y < sideDist.z) {
				t = sideDist.y;
				sideDist.y += deltaDist.y;
				mapPos.y += stepDir.y;
				normal = float3(0, -stepDir.y, 0);
			} else {
				t = sideDist.z;
				sideDist.z += deltaDist.z;
				mapPos.z += stepDir.z;
				normal = float3(0, 0, -stepDir.z);
			}
		}

		// Stop if we hit rasterized geometry or went out of bounds
		if (t > maxDistance || t > 1000.0) {
			break;
		}
	}

	// 5. Output
	if (hit && t <= maxDistance) {
		// Simple directional lighting based on the voxel face normal
		float3 lightDir = normalize(float3(0.5, 1.0, 0.2));
		float ndotl = max(dot(normal, lightDir), 0.1);
		float3 color = float3(0.8, 0.3, 0.1) * ndotl;
		outColor[pixel] = float4(color, 1.0);
	} else {
		// Sky or Miss
		outColor[pixel] = float4(0.05, 0.05, 0.1, 1.0);
	}
}

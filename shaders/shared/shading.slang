import raymarch;
import camera;
import world;

// TODO: spirv validation error for bitfield extract...
// need to apply elsewhere as well. might be a slang bug

// default empty
public struct VisbufKey {
	public uint64_t data;

	public property bool entity {
		get { return bitfieldExtract(this.data, 0, 1) != 0; }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 0, 1); }
	}

	public property uint chunkId { // entity id when entity is true
		get { return uint(bitfieldExtract(this.data, 1, 20)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 1, 20); }
	}
	public property uint x {
		get { return uint(bitfieldExtract(this.data, 21, 8)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 21, 8); }
	}
	public property uint y {
		get { return uint(bitfieldExtract(this.data, 29, 8)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 29, 8); }
	}
	public property uint z {
		get { return uint(bitfieldExtract(this.data, 37, 8)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 37, 8); }
	}

	public property Face face {
		get { return Face(uint(bitfieldExtract(this.data, 45, 3))); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 45, 3); }
	}
	public property uint voxelValue {
		get { return uint(bitfieldExtract(this.data, 48, 16)); }
		set { this.data = bitfieldInsert(this.data, uint64_t(newValue), 48, 16); }
	}

	public bool isHit() {
		return this.voxelValue != 0;
	}

	[mutating]
	public void write(RayHit hit) {
		if (!hit.hit) {
			this.data = 0;
		} else {
			this.data = uint64_t(hit.chunkId) << 1
				| uint64_t(hit.voxelCoord.x) << 21
				| uint64_t(hit.voxelCoord.y) << 29
				| uint64_t(hit.voxelCoord.z) << 37
				| uint64_t(hit.face) << 45
				| uint64_t(hit.voxelValue & 0xFFFFu) << 48;
		}
	}
}

// PCG
public uint pcgHash(inout uint state) {
	state = state * 747796405u + 2891336453u;
	uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
	return (word >> 22u) ^ word;
}
public float randFloat(inout uint state) {
	return float(pcgHash(state) >> 8) * (1.0 / 16777216.0);
}

public float3 randColor(uint seed) {
	let r = randFloat(seed);
	let g = randFloat(seed);
	let b = randFloat(seed);
	return float3(r, g, b);
}

// {{{ naive monte carlo path tracing for nice pictures
// stuff in linear HDR radiance
static const uint MAX_BOUNCES = 2;
static const float GRAY_ALBEDO = 0.9;

static const float3 AMBIENT_COLOR = float3(0.1, 0.1, 0.1);
static const float3 SUN_COLOR = float3(1.0, 1.0, 1.0);

static const float SUN_STRENGTH = 50.0;
static const float SUN_AZIMUTH_DEG = 45.0;
static const float SUN_ELEVATION_DEG = 45.0;
static const float SUN_ANGULAR_DIAMETER_DEG = 15;

// cosine-weighted hemisphere sampling via unit sphere offset
float3 cosDir(float3 normal, inout uint state) {
	let z = randFloat(state) * 2.0 - 1.0;
	let phi = 6.28318530718 * randFloat(state);
	let r = sqrt(max(0.0, 1.0 - z * z));
	let spherePt = float3(r * cos(phi), r * sin(phi), z);
	return normalize(normal + spherePt);
}

float3 skyColor(float3 dir) {
	let azimuth = radians(SUN_AZIMUTH_DEG);
	let elevation = radians(SUN_ELEVATION_DEG);

	let sunDir = float3(
		sin(azimuth) * cos(elevation),
		sin(elevation),
		cos(azimuth) * cos(elevation)
	);
	let radius = radians(SUN_ANGULAR_DIAMETER_DEG * 0.5);

	bool hitsSun = SUN_ANGULAR_DIAMETER_DEG > 0.0 && dot(dir, sunDir) >= cos(radius);

	return AMBIENT_COLOR + (hitsSun ? SUN_COLOR * SUN_STRENGTH : float3(0.0));
}

public float3 tracePath(
	WorldView world,
	CameraState camera,
	VisbufKey key,
	uint2 pixel,
	uint2 extent,
	uint sampleCount,
) {
	var ray = camera.primaryRay(pixel, extent);
	if (!key.isHit()) return skyColor(ray.dir);

	uint state = pixel.x + pixel.y * extent.x;
	state = pcgHash(state) ^ (sampleCount * 2891336453u);

	uint chunkId = key.chunkId;
	let chunkCoord = world.chunkMeta[chunkId].signedCoord;
	uint3 voxelCoord = uint3(key.x, key.y, key.z);

	// opaque voxel containing the camera has no visible entry surface
	let inside = all(ray.signedCoord == chunkCoord)
	          && all(floor(ray.localCoord) == float3(voxelCoord));
	if (inside) return 0.0;

	Face face = key.face;
	uint axis = face.axis();

	// intersect known face plane in the hit chunk's local coords
	float3 origin = ray.localCoord + float3(ray.signedCoord - chunkCoord) * CHUNK_SIDE;
	float faceCoord = float(voxelCoord[axis]) + face.ofsPZ();
	float t = (faceCoord - origin[axis]) / ray.dir[axis];
	ray.signedCoord = chunkCoord;
	// keep roundoff at voxel edges from placing the bounce in a neighbor
	ray.localCoord = clamp(origin + ray.dir * t,
		float3(voxelCoord),
		float3(voxelCoord) + 1.0
	);

	float3 throughput = 1.0;

	for (uint i = 0; i < MAX_BOUNCES; i++) {
		float3 normal = face.cardinalNormal();

		// lambert brdf * cos / cosine-weighted pdf cancels to albedo
		throughput *= GRAY_ALBEDO;
		ray.dir = cosDir(normal, state);

		// snap crossed axis to voxel face, then offset into air
		ray.localCoord[axis] = float(voxelCoord[axis])
			+ (normal[axis] > 0.0 ? 1.0 : 0.0)
			+ normal[axis] * 0.001;

		let hit = world.traceRay(ray, chunkId);
		if (!hit.hit) return throughput * skyColor(ray.dir);

		ray.signedCoord = world.chunkMeta[hit.chunkId].signedCoord;
		ray.localCoord = hit.localCoord;

		face = hit.face;
		axis = face.axis();
		chunkId = hit.chunkId;
		voxelCoord = hit.voxelCoord;
	}

	// bounce limit reached and still occluded
	return 0.0;
}
// }}}

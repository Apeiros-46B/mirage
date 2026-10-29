public float3 linearToSrgb(float3 color) {
	return select(color <= 0.0031308,
		12.92 * color,
		1.055 * pow(max(color, 0.0), 1.0 / 2.4) - 0.055
	);
}

public float3 srgbToLinear(float3 color) {
	return select(color <= 0.04045,
		color / 12.92,
		pow((color + 0.055) / 1.055, 2.4)
	);
}

// TODO: agx or something

# mirage

Work-in-progress voxel rendering engine. Eventually I aim to use this to make some sort of
sandbox/exploration game featuring seamless portals (similar to those in *Portal*,
*Antichamber*, or *Manifold Garden*) and stylized lighting.

Currently I only have naive path tracing and no portals yet, but I'm going to implement a
proper portal-aware realtime GI solution in the future.

![a demo, rendering and shading a sphere made of voxels](https://cdn-proxy.apeiros.workers.dev/projects/mirage-big-v1.webp)

## Engine milestones

I have planned out a lot of stuff but haven't finished most of it yet.

- [X] Basic Vulkan abstractions (setup, swapchain, frame lifecycle, bindless desc alloc)
- [X] Multithreading and worker abstractions
- [X] Debug UI
- [X] Regular raymarching
- [X] Stored world (as opposed to analytically generated in shader)
- [ ] Resource lifetime cleanup
- [ ] Editable world with more than one material
- [ ] Scene persistence (entire world loaded at once)
- [ ] Portal editing, raymarching, and camera traversal
- [ ] Chunk loading/streaming (load world parts instead of the whole thing)
- [ ] Per-voxel lighting and generated normals
- [ ] Translucency
- [ ] Better GI (surfels? radiance cascades? undecided)

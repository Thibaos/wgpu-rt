# wgpu-rt

A real-time 3D voxel rendering engine. Renders sparse voxel worlds using a Tree64
acceleration structure and compute-shader ray tracing.

## Language

**World**:
The scene loaded from a baked .world file — a single sparse voxel tree covering
the full volume, plus a 256-color palette.
_Avoid_: Scene, level, map

**Tree64**:
The sparse 4³ voxel tree acceleration structure used for both GPU ray traversal
and CPU collision queries. Each node covers 4³ children; leaf nodes store material
indices.
_Avoid_: Octree, BVH

**Voxel Scale**:
1 voxel = ⅛ meter (0.125 m). All physical dimensions — player size, step height,
gravity, speeds — are expressed in meters, not voxels.
_Avoid_: Block size, grid resolution

**Palette**:
A 256-entry RGBA8 color table, sourced from the .vox file, that maps material
indices to final surface colors. GPU-side it is pre-converted to float4.
_Avoid_: Color table, LUT

## Rendering

**Chunk**:
A 256³-voxel sub-volume of the World, the unit of streaming and GPU rendering.
Each chunk owns one `texture_3d` (256³ base, 9 mip levels) bound by index, and
is drawn as a single rasterized axis-aligned proxy cube. Local voxel
coordinates are `u8` (0..=255).
_Avoid_: Block, tile, region

**DDA**:
Digital Differential Analyzer — the per-cell voxel-grid ray traversal used in
the chunk fragment shader. The fragment computes a ray from the camera through
the proxy cube, resolves the chunk AABB entry analytically, and marches the
chunk's 3D texture one cell at a time until it hits a non-empty voxel or exits.
_Avoid_: Ray marcher, tracer

**Mip level**:
One of the chunk texture's 9 progressively coarser levels (256→128→…→1), built
by 2³ occupancy downsampling. Used by the DDA as hierarchical empty-space
culling: a non-zero coarse cell means "occupied somewhere below," letting the
tracer skip large empty regions before descending to a finer level.
_Avoid_: LOD, pyramid level

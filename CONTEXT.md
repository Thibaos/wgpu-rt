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

# wgpu-rt

A real-time 3D voxel rendering engine. Renders sparse voxel worlds as chunked
3D textures, traversed by a hierarchical mip DDA (raster path) or a TLAS
ray-query (experimental hardware-RT path).

## Language

**World**:
The scene loaded from a .vox file: a sparse set of occupied voxels (a
world-coordinate HashMap) plus a 256-color palette, split into Chunks for
rendering.
_Avoid_: Scene, level, map

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
tracer skip large empty regions before descending to a finer level. Phase 2
starts at mip 5 and descends one level at a time to mip 0 inside bounded parent
cells; levels 5 through 1 provide occupancy only, while mip 0 provides the
material and hit depth.
_Avoid_: LOD, pyramid level

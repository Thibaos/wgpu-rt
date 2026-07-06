# wgpu-rt

A real-time 3D voxel rendering engine built with Rust and wgpu.

## Acceleration Structure: Tree64

Uses a sparse 4³ voxel tree (contree), as described and benchmarked in
[VoxelRT](https://github.com/dubiousconst282/VoxelRT). This was the top
performer across all tested scenes (108-182 Mrays/s on integrated GPU).

### Architecture

- **CPU**: [`tree64`](https://github.com/expenses/tree64) crate — persistent
  sparse voxel tree with 4³ branching factor, path-copying edits, undo/redo.
  Node layout: 12 bytes (u32 packed ptr/leaf flag + u64 pop_mask).
- **GPU**: WGSL compute shader using parametric ray traversal with octant
  mirroring for wave-coherent performance. Ported from VoxelRT's Tree64
  slang shader. ~20 iterations average per ray.
- **Rendering**: Compute dispatch writes to storage texture, blitted to screen.
  Camera uses a simple fly controller.

### Github

This is a solo project, don't open PRs.

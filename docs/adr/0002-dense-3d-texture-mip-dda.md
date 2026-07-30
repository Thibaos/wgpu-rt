# Dense 3D-texture mip-hierarchical DDA for GPU rendering

GPU rendering traverses each chunk's 256³ `texture_3d` with a DDA ray march
inside the chunk's fragment shader, using the texture's 9 mip levels as a
hierarchical occupancy structure to skip empty space. This replaces the
Tree64 sparse 4³-tree traversal for GPU rendering.

Rejected alternative: keep Tree64 as the GPU acceleration structure (sparse,
supports arbitrary logical volumes). Tree64 is no longer wired into the render
path (plan 010 moved to chunk 3D textures and proxy cubes), and the dense
texture gives trivial mip-based empty-space culling for free from the
rasterizer's per-chunk visibility. The trade-off: every chunk costs a full
256³ texture (memory and the power-of-two constraint) in exchange for a far
simpler GPU data path and cheap hierarchical skipping.

## Status

Accepted. Supersedes the GPU-rendering half of ADR-0001; ADR-0001's
GPU-rendering claim is obsolete (Tree64 is not used for GPU ray traversal).
The CPU-collision half of ADR-0001 is also currently unimplemented (plan 007
was rejected), so no live system relies on Tree64 dual-use today.

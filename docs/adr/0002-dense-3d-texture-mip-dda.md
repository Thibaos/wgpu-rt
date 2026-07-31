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

## Phase-2 traversal contract

Phase 2 extends the verified mip-5 DDA into a bounded hierarchical traversal.
The shader starts at mip 5 (8³ cells), then descends one level at a time through
mip 0 whenever a coarse occupancy cell is non-zero. Each refinement traverses a
local 2³ child grid bounded by the occupied parent cell. A fixed six-frame stack
preserves front-to-back sibling traversal: the parent is advanced before a child
is pushed, and a failed child branch pops back to the already-advanced parent.

Mips 5 through 1 are occupancy-only; mip 0 supplies the material, voxel entry
time, and depth. The traversal uses half-open intervals, direction-aware
negative-boundary correction, multi-axis tie advancement within `T_EPS = 1e-6`
metres, a 24-cell root cap, an 8-cell refinement cap, and a global 2048-cell
safety budget. All loops remain statically bounded for WGSL. Coarse `u8` mip
values and the existing `R8Uint` upload path are retained for possible future
LOD sampling, even though only their non-zero occupancy is used by traversal.

A test-only CPU reference with sparse explicit mip maps and an independent mip-0
DDA oracle covers descent, sibling recovery, boundary/tie behavior, and malformed
coarse occupancy. Cross-chunk occlusion remains deferred.

## Status

Accepted. Supersedes the GPU-rendering half of ADR-0001; ADR-0001's
GPU-rendering claim is obsolete (Tree64 is not used for GPU ray traversal).
The CPU-collision half of ADR-0001 is also currently unimplemented (plan 007
was rejected), so no live system relies on Tree64 dual-use today.

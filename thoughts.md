# Thoughts on the current state of the project

I've been thinking about my rendering problem. Currently, I can render tens of thousands of voxels, but with around 30 fps, or dozens of ms per frame (that order of magnitude at least). I am greatly disappointed by this result, as I have been testing other methods of rendering in the past. I've tried basic instanced rendering with simple cube meshes, and I got to millions of instances with hundreds of fps. Also, I've tried a hardware ray tracing implementation with a static TLAS containing again millions of instances, with hundreds of fps again. So in comparison, this application falls very short regarding usability for an engine.

To be totally honest, I have never been that far in the development of an engine, because this one includes a chunk system with scene loading. Rendering millions of random instances is easy, but making it fit to a tangible world and calculating lighting for example is another big challenge. Different problems appeared to me as well when working with hardware ray tracing: updating the world and maintaining the TLAS included many synchronization problems and the real bottleneck was TLAS building and memory bandwidth.

I'd like you advice on all of that though. Would you agree that the current performance of this application is not satisfactory? Should I try to optimize the current rendering algorithm thoroughly or change the approach altogether? Is there a fundamental flaw in the design that is causing the performance issues?

---

## 2026-08-02 — Design A (ray-query renderer) implemented and verified

`WGPU_RT_RAYQUERY=1` selects a compute ray-query pass (TLAS of one world BLAS
holding one AABB per chunk + the ported hierarchical mip-DDA as the procedural
intersection) writing a storage texture, blitted to the surface. The raster
chunk-proxy DDA stays the default; both are A/B-able in the bench.

Results (RTX 3070, Vulkan, 1920x1080, release): monu1 21.0ms→6.4-8.9ms GPU
(46→107-146fps wall); bistro_sm 60.7-65.0ms→16.7-17.2ms GPU (16.5→58fps wall).
Same per-pixel traversal work; the compute path schedules it ~3x more
efficiently than fragment DDA with late-Z (same 24-27M cells/frame on bistro
at ~1.6B cells/s vs ~0.5B). Correctness: monu7 hit counts match to the voxel
(196,969 vs 196,976); monu1 frame dumps 99.41% byte-exact.

Bugs fixed along the way:
- naga 24 rejects computed expressions as the `rayQueryGenerateIntersection`
  distance inside the query loop (NotInScope); bind the value to a `let`
  first. Regression-guarded in tests/shader_validate.rs.
- Per-chunk BLASes sharing an AABB buffer via `primitive_offset` drop ~60% of
  hits on multi-chunk worlds (wgpu 30; single chunk unaffected). Switched to
  one world BLAS (N AABB primitives) + one TLAS instance; chunk id comes from
  `primitive_index`. Revisit for per-chunk rebuild granularity on edits.
- Storage-texture Y orientation: `d.y = 1 - 2*uv.y` to match the raster path
  (screen row 0 = clip +1).
- Compare dumps of the post-blit surface, not the Rgba8Unorm storage target
  (sRGB encode happens at the surface, both paths).

Open edges: camera inside a chunk (TLAS candidate behavior — bench orbits
outside), tmin 0.001 vs near plane 0.1 parity, per-chunk BLAS rebuilds for the
edit path, RTGI/reflection/refraction passes on top of this primary pass.

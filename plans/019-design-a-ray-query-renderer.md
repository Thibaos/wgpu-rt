# Plan 019: Design A — TLAS-of-chunk-AABBs ray-query renderer (hardware RT primary pass)

> **History (2026-08-02)**: moved from `advisor-plans/004-design-a-ray-query-renderer.md` (improve-skill advisor batch, 2026-07-31) into the main plan index as plan 019. Terminal (executed in-tree at `f014d3b`) — see `plans/README.md` row 019.

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. Keep the diff uncommitted for review, and do NOT
> touch `plans/README.md`.
>
> **Drift check (run first)**:
> `git status --short` — expected: untracked `plans/018-*` and
> `thoughts.md` only (plan 018 was never committed). HEAD should be `bb2970d`.

## Status

- **Priority**: P0
- **Effort**: L — one new WGSL shader (ported DDA + ray-query loop + blit), one new module (`render/rayquery.rs`, BLAS/TLAS setup), env-gated wiring in `app.rs`
- **Risk**: MED — new experimental API (`EXPERIMENTAL_RAY_QUERY`, Vulkan); the rasterized DDA path stays the default and untouched, so every failure mode is confined to `WGPU_RT_RAYQUERY=1`
- **Depends on**: wgpu 30.0.0 (pinned) — ray-query + AS API shipped since v27
- **Category**: perf / architecture
- **Planned at**: 2026-08-02, continuation of the RT-direction conversation
- **Issue**: (none — see `thoughts.md` for the decision record)

## Why this matters

The current renderer (rasterized chunk proxy cubes + fragment-shader DDA) is
latency-bound: 11.8% SM throughput, 29% occupancy, work = pixels × traversal
depth, decoupled from visible surface. Design A replaces the proxy
rasterization with a fullscreen **compute** pass that traces each pixel ray
against a TLAS whose instances are the chunk AABBs, running the *existing*
mip-DDA as the procedural intersection test (`rayQueryGenerateIntersection`).

This is the migration path that keeps every dynamic-world property the user
chose textures for: chunk BLAS = the static chunk world bounds (one AABB),
**never rebuilt on edits**; edits stay async `write_texture` texel copies;
the TLAS (≤64 instances) is rebuilt only when the chunk set changes. The
hardware win is TLAS culling for effect rays (reflections/refraction/RTGI
later); the primary pass keeps the DDA per pixel but drops proxy overdraw,
front-to-back sorting, `frag_depth` late-Z, and gains explicit compute
scheduling (occupancy control, half-res + upscale later).

## Design

- Env gate: `WGPU_RT_RAYQUERY=1` (default off → rasterized DDA unchanged).
- `App::required_features` / `required_limits` become env-aware: add
  `Features::EXPERIMENTAL_RAY_QUERY` and
  `Limits::using_minimum_supported_acceleration_structure_values()` only when
  the gate is set (feature is Vulkan-only experimental; must not be requested
  on the default path).
- Per non-empty chunk, one `Blas` with a single `BlasAabbGeometry`:
  - AABB buffer: one world-space box per chunk (`GpuAabb { min, max, pad }`,
    32-byte stride), usage `BLAS_INPUT | STORAGE` (shader re-reads the hit
    chunk's bounds from the same buffer — the `ray_aabb_compute` upstream
    example pattern).
  - `primitive_offset = i * 32` so `chunk AABB` for TLAS instance `i` lives at
    `gpu_aabbs[i]`.
  - `AccelerationStructureGeometryFlags::OPAQUE` (required: naga has no
    non-opaque candidate handling).
- One `Tlas`, `max_instances = chunk_count`, instances set via
  `tlas.get_mut_single(i)` with `TlasInstance::new(&blas, identity_transform,
  i, 0xFF)` — the AABBs are already in world space, so the transform is
  identity. Built once at init (`build_acceleration_structures`).
- New compute pipeline + `assets/shaders/rayquery.wgsl`:
  - `enable wgpu_ray_query;` + existing `enable wgpu_binding_array;`.
  - `rq_main`: per pixel, rebuild the view ray from `camera.view_inv /
    proj_inv` (same math as the raster path), `rayQueryInitialize(RayDesc(0,
    0xFF, 0.001, 10000, origin, dir))`, loop `rayQueryProceed`, on
    `RAY_QUERY_INTERSECTION_AABB` run the ported `dda_chunk(origin, dir,
    c.instance_index)` and `rayQueryGenerateIntersection` on hit; commit →
    shade `palette[hit.mat]`; write to an Rgba8Unorm storage texture.
  - The DDA (constants, `TraversalFrame`, `ray_aabb`, `init_frame`,
    `advance_frame`, traversal loop) is ported **verbatim** from `chunk.wgsl`,
    minus the frag-depth/discard surface: miss → `HitResult(-1, 0)`.
  - `%%STATS_PIXEL%%` (was fragments), `%%STATS_CELLS%%`, `%%STATS_HIT%%`
    markers preserved so the profiling harness works unchanged (stats buffer
    binding moves to 3; the AS binding is at 4).
  - Blit entry points (`blit_vs_main`, `blit_fs_main`) taken verbatim from the
    upstream `ray_aabb_compute` example (fullscreen quad, proven orientation).
- `render()` branches: rayquery → `write_timestamp(0)` + compute pass +
  blit render pass + `write_timestamp(1)` (GPU time covers compute+blit, same
  query set / resolve / readback as the raster path); else the existing
  rasterize pass. Front-to-back instance sort is skipped in rayquery mode.
- `resize()` recreates the storage target + the out/blit bind groups
  (`RayQueryResources::recreate_target`).

## Changes

| File | Change |
| --- | --- |
| `assets/shaders/rayquery.wgsl` | NEW — compute ray-query + blit shader |
| `src/render/rayquery.rs` | NEW — `RayQueryResources` (BLAS/TLAS/AABB buffer, pipelines, bind groups) |
| `src/render/mod.rs` | ADD `GpuAabb`, `pub mod rayquery` |
| `src/app.rs` | env-aware features/limits, `rayquery: Option<RayQueryResources>`, render branch, resize |
| `src/framework.rs` | unchanged (uses `App::required_*` which are env-aware) |
| `src/bin/bench.rs` | unchanged (same) |

## Verification

Build/lint:
1. `cargo check` — compiles.
2. `cargo clippy --all-targets -- -D warnings` — clean under the crate's
   deny list (pedantic + nursery + unwrap_used/expect_used/indexing_slicing/
   arithmetic_side_effects/as_conversions).
3. `cargo test` — existing suite still passes (no shader refactor of the
   raster path).

A/B (bench harness, orbit camera, RTX 3070 / Vulkan; stats=1 counts work,
stats=0 is real early-Z GPU time — the ray-query path has no early-Z concept,
so use stats=1 for apples-to-apples traversal work and stats=0 for end-to-end
GPU time):

4. Baseline raster DDA:
   `WGPU_RT_STATS=1 cargo run --release --bin bench -- 900` (monu1),
   `WGPU_RT_WORLD=assets/models/bistro_sm.vox ...` (5 chunks).
5. Design A:
   `WGPU_RT_RAYQUERY=1 WGPU_RT_STATS=1 cargo run --release --bin bench -- 900`
   on both worlds. Compare gpu ms + cells/frame. Expect: comparable or faster
   primary-pass traversal work; end-to-end GPU time likely lower on
   multi-chunk scenes (no proxy overdraw/sort); latency-bound signature may
   persist (this is the honest expectation — the win is effect rays later).
6. `WGPU_RT_RAYQUERY=1 WGPU_RT_STATS=0` — real end-to-end GPU time without
   counter side effects.

## Results (RTX 3070, Vulkan, 1920x1080, release, orbit camera)

Measured after the fix below; `WGPU_RT_RAYQUERY=1` = Design A, default = raster
chunk-proxy DDA. Same binary, same camera path.

| world | path | GPU ms | wall fps |
|---|---|---|---|
| monu1 (1 chunk) | raster | 21.0-21.2 | 46 |
| monu1 | rayquery | 6.4-8.9 | 107-146 |
| bistro_sm (5 chunks) | raster | 60.7-65.0 | 16.5 |
| bistro_sm | rayquery | 16.7-17.2 | 58 |

So 2.4-3.3x faster GPU on monu1, 3.6-3.8x on bistro_sm, with identical
per-pixel traversal work (the compute path simply schedules it better than
fragment-shader DDA with late-Z: same 24-27M cells/frame on bistro at ~1.6B
cells/s vs ~0.5B in the raster path). The raster path's `hits` counter is
inflated by late-Z multi-proxy fragments (hidden chunk-B fragments still
increment), so compare `cells` and dumped images, not `hits`.

Correctness verification:
- monu7 (1 chunk): raster 196,969 hits vs rayquery 196,976 (voxel-perfect
  match, identical 19.5M cells) — the DDA port is exact.
- monu1 frame dump (post-blit surface vs raster surface): 99.41% byte-exact
  match, no flip, remaining diff = orbit pose drift between runs + silhouette
  edges. Colors match exactly (both passes round-trip the palette through the
  same sRGB encode at the surface).

## Bugs found and fixed while implementing

1. **naga 24 `rayQueryGenerateIntersection` scoping bug**: passing a computed
   expression (variable load, struct member, or arithmetic) as the generate
   distance inside the query loop fails validation with `NotInScope`;
   literals and `let`-bound values pass. Workaround: bind the distance to a
   `let` first. Regression-guarded by `minimal_rayquery_generate_variants` in
   tests/shader_validate.rs.
2. **AABB `primitive_offset` multi-instance drop**: per-chunk BLASes sharing
   one AABB buffer with `primitive_offset = i*32` dropped ~60% of expected
   hits on multi-chunk worlds (single chunk unaffected). Replaced with one
   world BLAS holding N AABB primitives + one TLAS instance; chunk id is
   recovered from the query's `primitive_index`. `gpu_aabbs[primitive_index]`
   and the texture binding array stay index-aligned because all are built in
   the same chunk order.
3. **Storage-texture Y orientation**: `d.y = 2*uv.y - 1` pointed screen row 0
   at clip BOTTOM (image vertically flipped vs raster). Fixed to
   `d.y = 1 - 2*uv.y`.
4. **sRGB comparison trap**: `rt_target` (Rgba8Unorm storage) holds palette
   values un-encoded while the raster surface is encoded — dumps must compare
   the post-blit surface, not the storage target. The blit's Linear sampler
   round-trips palette floats through the surface encode exactly like the
   raster fragment path, so colors match byte-for-byte.

## WGPU_RT_DUMP debug path

`WGPU_RT_DUMP=<dir>` writes one raw frame dump (surface, post-blit) after
frame 40 of either renderer, for pixel-level A/B: `dump_raster.bgra` vs
`dump_rayquery.bgra` (both BGRA8, 1920x1080). Requires `COPY_SRC` on the
bench's offscreen target; the interactive swapchain cannot be dumped (doc'd
in app.rs). Convert with the small python script used during development.

## Known edges / deferred

- Camera *inside* a chunk: the TLAS must still report the containing AABB as a
  candidate (DXR does); verify interactively with the player controller later
  (bench orbits outside the world).
- `GLOBAL_CELL_CAP` is per `dda_chunk` call, not per pixel (multi-chunk ray
  crossings get a fresh cap per chunk) — negligible in the 8x1x8 layout.
- Ray `tmin = 0.001` vs the raster near plane 0.1: sub-cm geometry near the
  camera renders in rayquery mode, not in raster mode — irrelevant for the
  orbit bench, note for future near-plane parity.
- Heatmap flag: carried in the uniforms struct, unused in both paths today.
- Compaction (`prepare_compaction_async`) deferred until static chunks exist.
- Per-chunk BLAS with `primitive_offset` (Design A as planned) is currently
  broken in wgpu 30 (see bug 2); revisit when the single-BLAS world BVH needs
  per-chunk rebuild granularity (edits) — the fix is likely a per-chunk AABB
  buffer or an offset-free single buffer rebuild.

## STOP conditions

- `cargo clippy` cannot be satisfied without loosening the deny list (report
  which lint; do not weaken the config unilaterally).
- Any change to the default (rasterized DDA) render path is required to make
  the ray-query path work (the two must be cleanly A/B-able).
- The bench shows the ray-query pass producing visibly wrong output vs the
  raster pass on the same camera path (black frame, flipped image, garbage
  colors) with no diagnosable cause within two fix attempts.

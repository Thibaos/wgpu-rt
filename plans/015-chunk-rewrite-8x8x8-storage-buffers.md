# Plan 015: 8³-chunk rewrite — storage-buffer voxel pool, flat-DDA ray-query primary (supersedes 014)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. A reviewer dispatches you and maintains the index,
> so SKIP any instruction to update `plans/README.md`; the reviewer updates it.
>
> **Drift check (run first)**:
> `git status --short -- src/` — expected: empty (no uncommitted source
> changes; untracked/modified files under `plans/` are expected and
> fine). HEAD must be `bde0db4` or later.
> The kickoff edits are committed: `docs/adr/` is deleted and `tree64` is
> absent from `Cargo.toml`/`Cargo.lock`.
> Plans 020 (grid-centered loading, CHUNKS_Y=8, drop diagnostics) and
> 022 (heatmap wiring, WGPU_RT_HEATMAP) have landed — executed 2026-08-03,
> commits `d3304c2` / `f90cb7f` (see plan 021) — so verify every "Current
> state" excerpt below against the live files before starting (grep
> CHUNKS_Y in src/world/chunk.rs, split_chunks in src/world/mod.rs,
> viewport_and_heatmap.z in assets/shaders/). On any mismatch, STOP and
> report the exact changed path and difference.

## Status

- **Priority:** P0
- **Effort:** XL — world/chunk data layer rewrite, flat-DDA shader, renderer consolidation (raster retirement), test rewrite, docs/ADR, measurement stage
- **Risk:** HIGH — touches every render-related module; the ray-query path is experimental (`EXPERIMENTAL_RAY_QUERY`, Vulkan) and becomes the **only** path; BLAS primitive count grows to 10⁴–10⁵ (build/traverse cost must be measured); strict clippy (pedantic+nursery deny) across a large diff
- **Drift:** plans 020 (`d3304c2`: grid-centered loading, CHUNKS_Y=8, drop diagnostics) and 022 (`f90cb7f`: heatmap reads in both shaders, WGPU_RT_HEATMAP) changed the world/chunk layer and shaders after this plan was authored — re-verify every "Current state" excerpt at dispatch (the drift check above covers this)
- **Depends on:** plan 012 orbit camera (DONE at `b0f332c`), plan 019 Design A ray-query renderer (DONE at `f014d3b`)
- **Category:** architecture / perf (primary pass)
- **Planned at:** 2026-08-02, continuation of the grill session on `docs/research-teardown-hardware-ray-tracing.md` and the no-3D-textures direction
- **Issue:** (none — see "Decision record" below)
- **Supersedes:** plan 014 (REJECTED — it instrumented/optimized the 256³ texture architecture this rewrite deletes)

## Why this matters

The Design A primary pass is traversal-bound: 24–27M DDA cells/frame on
bistro at ~1.6B cells/s ≈ the full frame, and the per-chunk 256³ textures +
9 mips exist solely to feed the hierarchical mip DDA. The Teardown research
points at a strictly simpler target:

- **Finding 3**: chunks of ≤8³ voxels with occupancy-tight AABBs and a flat
  DDA in the intersection shader — "only a few DDA steps are needed before
  hitting something or exiting"; the mipmapping concept is removed entirely.
- **Finding 6**: 4³ chunks use least VRAM but **8³ rendered faster**, and
  Teardown accepted a unified storage/render split at 8³.
- **Finding 5**: hardware-ray-traced primary beat rasterization **and** direct
  re-marching on mainstream GPUs — one ray per pixel against a TLAS of chunk
  AABBs is the target, not a stopgap.

The rewrite also deletes a whole second renderer: the raster path (proxy-cube
pipeline, depth texture, front-to-back sort, `frag_depth`, the A/B dump gate,
`enable wgpu_binding_array`, mip bake, the six-frame stack DDA). One renderer,
one shader, no textures, no mips, fewer experimental features.

## Decision record (grill session, 2026-08-02)

1. **Chunk = 8³ voxels (1 m at VOXEL_SCALE 0.125)** — the render/intersection
   unit, per the Teardown data point. No streaming exists today; the loader
   splits the world voxel set once.
2. **No 3D textures anywhere.** Voxel material data lives in one storage
   buffer (the **voxel pool**): fixed 512 B (128 u32) stride per chunk,
   local index `x + 8y + 64z`, four material u8s packed per u32 word.
3. **The chunk table is the BLAS input buffer** (`GpuAabb`, 32 B): one
   occupancy-tight AABB per chunk; chunk id = index in the coord-sorted chunk
   list = `primitive_index`. No separate table buffer is needed.
4. **Flat mip-0 DDA** in the intersection test: no stack, no mips. Semantics
   identical to the hierarchical DDA absent the safety caps (pure
   acceleration) — committed `(t, mat)` hits must match.
5. **Ray-query is THE primary.** The raster path and the `WGPU_RT_RAYQUERY`
   gate are removed. `WGPU_RT_PROFILE` / `WGPU_RT_STATS` / `WGPU_RT_DUMP` /
   `WGPU_RT_ORBIT` stay.
6. **Plan 014's transferable levers are implemented in this rewrite, not
   later**: occluded-candidate early-out (skip the DDA when the candidate
   AABB entry t ≥ current best hit) and a tmax bound on the DDA interval.
7. **Palette stays** — material u8 → palette buffer, as today. No PBR.
8. **Stats/heatmap harness preserved** (`%%STATS_PIXEL%%`/`%%STATS_CELLS%%`/
   `%%STATS_HIT%%` markers; `viewport_and_heatmap` flag).
9. **Tree64 fully removed**: docs cleaned at kickoff (CONTEXT.md, research
   docs; the whole `docs/adr/` series deleted by user decision — ADRs are
   retired, decisions live in plan files); the dead `tree64` Cargo dependency
   was already dropped at kickoff (Cargo.toml; lock regenerated in this plan).
10. **Chunk size parameterized** (`CHUNK_LOG2`, default 3) and validated by a
    post-rewrite sweep 4³/8³/16³/32³ (Stage 5) — Teardown's 8³ is the prior,
    measured in this regime before locking it in.
11. **Out of scope**: edits/streaming, BLAS refit, palette→PBR, lighting/effect
    rays, WebGPU-portable fallback (the ray-query path is Vulkan-experimental
    by nature).

## Current state (must match; drift check)

- `src/world/chunk.rs`: `CHUNK_TEXTURE_SIZE` = 256³, `CHUNK_SIZE` = 256,
  `CHUNKS_X/Y/Z` = 8×8×8 fixed grid (post-020), `to_mip_bytes`/`downsample_occupancy`
  (9 mips), `create_texture` (R8Uint D3 texture + per-mip `write_texture`).
- `src/world/mod.rs`: `MIP_LEVELS = 9`; `World::into_chunks` delegates to
  `split_chunks()` (post-020), which fills the fixed grid from the voxel
  HashMap (`div_euclid(chunk_side)` with `chunk_side: i32 = 256`) and
  returns `(Vec<Chunk>, u64)` — the u64 counts and logs dropped voxels;
  `create_palette_buffer` stays.
- `src/render/rayquery.rs`: `RayQueryParams { instances, chunk_side_world,
  palette_buf, texture_view_refs, bind_group_count, stats_buf, stats_enabled,
  camera_bind_group_layout, width, height, target_format }`; group(1)
  bindings 0 palette / 1 `binding_array<texture_3d<u32>>` (count Some) / 2
  AABBs / 3 stats? / 4 TLAS; one world BLAS, one AABB (full 32 m box) per
  chunk, TLAS with a single identity instance.
- `assets/shaders/rayquery.wgsl`: hierarchical `dda_chunk` ported verbatim
  (six-frame `array<TraversalFrame, 6>` stack, `ROOT_MIP = 5`,
  `textureLoad(voxel_textures[chunk_index], coord, top.mip)`,
  caps 24/8/2048/16384), `rayQueryGenerateIntersection` on mip-0 hit, blit.
  Post-022: `HitResult` carries `cells: u32` and both shaders read
  `camera.viewport_and_heatmap.z` to colorize by DDA work (`WGPU_RT_HEATMAP=1`
  enables it at startup) — Stage 2's flat-DDA port must preserve that read.
- `src/app.rs`: raster path is the default (`WGPU_RT_RAYQUERY=1` swaps in
  Design A); raster uses `rasterize_aabbs_pipeline`, a depth texture,
  front-to-back instance sort, and a render-pass timestamp query; `WGPU_RT_DUMP`
  dumps the raster surface or the ray-query blit.
- `tests/hierarchical_mip_dda.rs`: CPU reference + oracle for the hierarchical
  mip DDA (levels 0..=5). `tests/shader_validate.rs`: naga-validates
  `chunk.wgsl` and `rayquery.wgsl`.
- `tree64` is fully removed (0 matches in `Cargo.toml` and `Cargo.lock`) —
  already handled at kickoff; Stage 3's dependency step is now a
  verify-absent step.

## Design

### Stage 1 — Data layer (CPU): 8³ chunking, tight AABBs, voxel pool

**`src/world/chunk.rs`**
- Replace `CHUNK_TEXTURE_SIZE`/`CHUNK_SIZE` with `CHUNK_LOG2: u32 = 3` and
  `CHUNK_SIZE: u32 = 1 << CHUNK_LOG2` (8). Local coords stay `u8` (0..=7).
- Delete: `to_mip_bytes`, `downsample_occupancy`, `create_texture`, and the
  `MIP_LEVELS` import (mips are gone).
- Add: `tight_aabb(&self) -> Option<((u8,u8,u8),(u8,u8,u8))>` (min/max
  occupied local voxel, or `None` if empty) and
  `to_pool_bytes(&self) -> Vec<u8>` (exactly 512 bytes; local index
  `x + 8*y + 64*z`, zero-filled air).
- Rewrite the unit tests: 512-byte flatten + index mapping, tight AABB for
  single-voxel/sparse fills, `is_empty`. Delete the mip tests.

**`src/world/mod.rs`**
- Delete `MIP_LEVELS`.
- Rewrite `World::into_chunks`: split the voxel HashMap into occupied-only 8³
  chunks — chunk coord = `world_voxel div_euclid(8)`, local = `rem_euclid(8)`
  (i16 voxel coords ⇒ chunk coords fit i32 comfortably). Drop the fixed
  `CHUNKS_X/Y/Z` grid and its bounds checks. Return
  `Vec<(IVec3, Chunk)>` **sorted by (x, y, z)** so chunk id = list index is
  deterministic across runs (required by the dump gate).
- Add `pub struct ChunkRecord { pub origin: Vec3, pub tight: GpuAabb }` (or
  equivalent) plus a helper that converts the sorted chunk list into
  `(Vec<ChunkRecord>, Vec<u8> pool_bytes)` — pool bytes concatenated in id
  order (id `i` at byte `i * 512`, so the shader derives the offset
  `pool_word = id * 128 + local_index / 4`).

**`src/world/loader.rs`** — minimal: `center_world` currently anchors on the
  grid center (post-020; previously the single-chunk 256³ center);
  re-anchor on the voxel-bounds midpoint (offset semantics unchanged).
  Update the `CHUNK_TEXTURE_SIZE` import accordingly.

**`src/render/mod.rs`** — keep `GpuAabb` (unchanged 32 B layout). Delete the
raster-only types: `Vertex`, `INDEX_COUNT`, `InstanceRaw`, and the old
`Instance` if unused after Stage 1.

**Verify (Stage 1):** `cargo check` compiles; `cargo test chunk` passes
(rewritten suite); `cargo clippy --all-targets -- -D warnings` clean (only
if the full tree is temporarily green — otherwise defer clippy to Stage 3 and
note it).

### Stage 2 — Shader: flat DDA + early-out/tmax

**`assets/shaders/rayquery.wgsl`**
- Delete: `enable wgpu_binding_array;`, the `voxel_textures` binding array,
  and all hierarchical machinery (`ROOT_MIP`, `ROOT_GRID_SIZE`,
  `ROOT_CELL_SIZE`, `grid_size_at_mip`, `cell_size_at_mip`, the six-frame
  stack, push/pop refinement, `TRAVERSAL_BOUND`, per-frame caps, mip-aware
  `textureLoad`).
- Add: `@group(1) @binding(1) var<storage, read> voxel_pool: array<u32>;`.
  Binding 2 (`gpu_aabbs`) is now the tight chunk table.
- `dda_chunk` becomes a flat mip-0 march over the chunk's tight AABB:
  `ray_aabb` span (reuse), one `init_frame`-style entry (cell, t_max,
  t_delta, interval), then per step: `let local = cell & 7` (tight-box local
  coords), `let word = voxel_pool[chunk_index * 128 + (local.x + 8*local.y +
  64*local.z) / 4]; let mat = (word >> (8 * (local_index & 3))) & 0xFF;` —
  on `mat != 0` write `HitResult(t, mat)` and return, else advance. Bounded
  by a single step guard (tight 8³ box ⇒ ≤ 3·8+1 = 25 positive-width steps
  per ray; cap 64 with margin) and a small global cap (e.g. 256).
- Early-out + tmax (014 levers): track `var best_t: f32 = INF;` in `rq_main`;
  before running the DDA for a candidate, skip when
  `span_of(candidate) .x >= best_t`; pass `best_t` in to clamp the DDA's
  interval exit. `rayQueryGenerateIntersection` unchanged (hardware keeps the
  closest).
- Keep `%%STATS_PIXEL%%` / `%%STATS_CELLS%%` / `%%STATS_HIT%%` markers and the
  blit entry points verbatim.

**`src/render/rayquery.rs`**
- `RayQueryParams`: replace `texture_view_refs` with `voxel_pool_buf:
  &wgpu::Buffer` and `chunk_side_world` with the `&[ChunkRecord]` (tight
  AABBs); drop `bind_group_count`.
- `aabb_buf` built from tight bounds: `min = origin + tight_min * 0.125`,
  `max = origin + (tight_max + 1) * 0.125` (half-open, same voxel→world
  conversion as the shader). Usage stays `BLAS_INPUT | STORAGE`.
- Group(1) layout: 0 palette / 1 pool (plain storage read, no `count`) / 2
  AABBs / 3 stats? / 4 TLAS. Bind group entries updated; the
  `TextureViewArray` resource is gone.
- World BLAS: same single-BLAS shape, `primitive_count = chunk_count`
  (now 10⁴–10⁵ — log the build time; Stage 5 measures it).
- Stats injection (string replace) unchanged.

**Verify (Stage 2):** `cargo test --test shader_validate` — update it:
delete the `chunk.wgsl` test; keep both ray-query tests; add a test asserting
the new shader contains no `binding_array`/`textureLoad`/`voxel_textures`;
naga-validate both stats=0 and stats=1 builds. `cargo check` clean.

### Stage 3 — Retire the raster path, rewrite the tests, drop tree64

- **`src/app.rs`**: remove `rasterize_aabbs_pipeline`, the depth texture, the
  front-to-back instance sort, the raster render-pass branch and its timestamp
  query path, and the `WGPU_RT_RAYQUERY` gate — `RayQueryResources` is always
  constructed and the compute pass + blit is the only path. Keep
  `WGPU_RT_PROFILE`, `WGPU_RT_STATS`, `WGPU_RT_DUMP` (now an absolute
  determinism gate on the single renderer), `WGPU_RT_ORBIT`. Chunk texture
  creation/upload replaced by the pool buffer + records from Stage 1.
- **Delete `assets/shaders/chunk.wgsl`** and the `src/world/chunk.rs`
  texture/mip code (already gone in Stage 1).
- **`tests/hierarchical_mip_dda.rs` → delete; add `tests/flat_chunk_dda.rs`**:
  CPU mirror of the flat 8³ DDA (same coordinate conventions as the old test,
  normalized to the tight AABB) + an independent brute-force oracle (nearest
  occupied voxel along the ray). Fixtures: single voxel, full 8³, sparse
  random fills, camera-inside, negative-direction boundary correction, axis
  ties. Assert `(t, mat)` equality within the old `T_TOLERANCE`.
- **`Cargo.toml`**: `tree64` is already removed (kickoff). Run
  `cargo check`; regenerate `Cargo.lock` only if cargo reports it dirty.
- **`src/bin/bench.rs`**: refresh the header comment (raster references);
  no behavior change. `src/framework.rs` / `src/player_controller.rs` /
  `src/main.rs` unchanged.
- **Lint gate**: `cargo fmt --check` and `cargo clippy --all-targets --
  -D warnings` must pass on the whole diff (pedantic + nursery are denied).

**Verify (Stage 3):**
- `cargo test` — full suite green (rewritten chunk unit tests + flat DDA
  reference + shader_validate).
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean.
- `grep -rn "tree64\|chunk.wgsl\|binding_array\|textureLoad" src/ Cargo.toml`
  → no matches.
- Determinism: `WGPU_RT_DUMP=<dir>` twice on the same scene/orbit → identical
  bytes.

### Stage 4 — Docs & ADR

- **`CONTEXT.md`**: refresh the glossary — `Chunk` = 8³ voxels (1 m), voxel
  pool + tight AABB, one BLAS primitive per chunk; `DDA` = flat, no mips;
  delete the `Mip level` entry; `World` = split into 8³ chunks (the kickoff
  already removed Tree64 and the .world-file claims).
- **Decision record**: the `docs/adr/` series was retired at kickoff (user
  decision, 2026-08-02) — this plan's decision record stands in place of an
  ADR; no new ADR is written.
- **`docs/research-*.md`**: Tree64 references already cleaned at kickoff;
  extend the teardown doc's relevance notes if the rewrite changes any claim.

**Verify (Stage 4):** `grep -ri "tree64" CONTEXT.md docs/ plans/` → only
plans/README's historical rows and this plan's kickoff notes.

### Stage 5 — Measurement + chunk-size sweep

- Keep `CHUNK_LOG2` parameterized (default 3); the index math, guard bounds
  (`3·CHUNK_SIZE + 1` + margin), tight-AABB derivation, and stride all derive
  from it.
- Sweep `CHUNK_LOG2 ∈ {2, 3, 4, 5}` (4³/8³/16³/32³) on `monu1` and
  `bistro_sm`, outside and in-world orbits, via the bench harness
  (`WGPU_RT_PROFILE=1 WGPU_RT_STATS=1`). Record per cell: cells/frame,
  cells/ray, GPU ms, BLAS+TLAS build ms, pool bytes.
- Default stays 8³ unless data contradicts Teardown's 8³ (record the choice).
- Note (post-020): these scenes are **unclipped** — bistro_sm now spans
  the full 2048³ world (~91 chunks) and church ~111. The historical 016/019
  numbers were measured on clipped fragments (5-13 chunks) and are NOT a
  valid baseline for this sweep — record fresh pre-rewrite numbers on the
  full scenes instead. The targets below are absolute thresholds for the
  unclipped scene, not regression gates against 016/019.
- Stretch targets (data-gated, not hard gates): bistro_sm outside orbit
  **< 6 ms GPU @ 1080p**; monu1 **< 3 ms**; **avg cells/ray < 5**.

## Changes

| File | Change |
| --- | --- |
| `src/world/chunk.rs` | 8³ chunk (CHUNK_LOG2), tight AABB, 512 B pool bytes; delete mips/texture |
| `src/world/mod.rs` | MIP_LEVELS out; occupied-only dynamic chunk split; ChunkRecord + pool helper |
| `src/world/loader.rs` | center on voxel-bounds midpoint |
| `src/render/mod.rs` | GpuAabb stays; raster vertex/instance types out |
| `src/render/rayquery.rs` | pool + table bindings, tight AABBs, no binding array; early-out/tmax in shader |
| `assets/shaders/rayquery.wgsl` | flat DDA + early-out/tmax; hierarchy deleted |
| `assets/shaders/chunk.wgsl` | DELETED |
| `src/app.rs` | single ray-query renderer; raster, depth, sort, gate out |
| `tests/hierarchical_mip_dda.rs` | DELETED → `tests/flat_chunk_dda.rs` |
| `tests/shader_validate.rs` | drop chunk.wgsl; add flat-shader/no-binding-array assertions |
| `Cargo.toml` / `Cargo.lock` | `tree64` removed at kickoff; lock regenerated |
| `src/bin/bench.rs` | header comment only |
| `CONTEXT.md`, `docs/research-*`, `plans/README.md` | glossary/index updates (kickoff + Stage 4); `docs/adr/` series deleted at kickoff |

## STOP conditions

- Any fixture where the flat DDA's committed hit differs from the CPU
  reference — stop and report (the hierarchy removal must be semantics-
  preserving).
- `cargo clippy --all-targets -- -D warnings` cannot be satisfied within the
  plan's scope after two reasonable passes.
- BLAS build time at the real chunk count exceeds ~2 ms with no recorded
  decision (perf design goal; the single-BLAS world must stay build-cheap).
- Drift: any "Current state" excerpt no longer matches at plan start, or any
  step's verification fails twice with the same error.

## Risks

- **BLAS/TLAS traversal cost at 10⁴–10⁵ primitives** — tight AABBs reduce
  overlap, but hardware BVH quality matters; the early-out/tmax levers and
  the Stage-5 build-time measurement are the mitigation.
- **Naga**: the old out-pointer workaround for struct returns inside the
  ray-query loop may or may not still be needed for the flat DDA; keep the
  pattern that works and note it.
- **f32 voxel→world conversion** for tight AABBs (half-open bounds at
  0.125 m scale): use the same conversion on CPU and GPU; the CPU reference
  guards it.
- **Instance-id determinism**: sorted chunk list required for the dump gate.
- **Strict lint set** (pedantic + nursery deny, plus the deny list in
  Cargo.toml) across a large diff — budget a lint pass per stage.

## Out of scope

Edits/streaming, per-chunk BLAS rebuild granularity and refit, compaction,
palette→PBR, lighting/effect rays, half-res/upscale, WebGPU-portable
fallback, keeping the raster path in any form.

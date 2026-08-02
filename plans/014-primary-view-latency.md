# Plan 014: Primary-view latency — instrumentation, occluded-candidate early-out, tight AABBs, chunk-size matrix

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. A reviewer dispatches you and maintains the index,
> so SKIP any instruction to update `plans/README.md`; the reviewer updates it.
>
> **Drift check (run first)**:
> `git status --short` — expected: untracked `docs/research-teardown-hardware-ray-tracing.md`
> and `plans/014-primary-view-latency.md` (this plan). HEAD should be `f014d3b`.
> Then confirm the "Current state" excerpts below still match the live files.
> On any mismatch, STOP and report the exact changed path and difference.

## Status

- **Priority:** P1
- **Effort:** L — instrumentation (both paths), two ray-query shader levers, orbit preset, chunk-size parameterization + matrix, stats/dump extensions
- **Risk:** MED — ray-query path only for levers (raster baseline untouched by *behavior*), but the chunk-size parameterization touches the world loader, mip bake, and limits; the default 256³ path must stay byte-identical
- **Depends on:** plan 012 orbit camera (DONE at `b0f332c`), Design A ray-query renderer (DONE at `f014d3b`)
- **Category:** perf (primary pass) / architecture (chunk-size contract)
- **Planned at:** 2026-08-02, from the grill-me-docs session on `docs/research-teardown-hardware-ray-tracing.md`
- **Issue:** (none — see "Decision record" below)

## Why this matters

The Design A ray-query primary pass is 2.4–3.8× faster than the raster path
(monu1 21.0→6.4–8.9 ms GPU; bistro_sm 60.7→16.7–17.2 ms) but remains
**traversal-bound**: 24–27M cells/frame on bistro at ~1.6B cells/s ≈ the full
frame. The Teardown research doc's part-2 findings map 1:1 onto this pass:

- **Finding 4 (traversal-order waste)**: with hardware RT there is no control
  over which chunk's intersection shader runs first, so the DDA can be fully
  run on occluded volumes. The doc's fix was diagnostic only (clockARB
  heatmaps); wgpu-rt can do better: skip the DDA for any candidate AABB whose
  entry t is already ≥ the committed intersection (provably safe — see
  "Early-out correctness" below), and bound the DDA by the committed t as a
  tmax so it never marches past the current best hit.
- **Finding 3 (tightly-fitted AABBs)**: wgpu-rt inserts the *full 32 m chunk
  box* per chunk, so every ray sweeping past sparse geometry (a monument, a
  flagpole) still walks the root mip across the box. Tight AABBs from
  occupancy cut TLAS overlap and air-walking. monu1's ~19.5M cells at 1080p
  (~9.4 cells/ray in a single sparse chunk) is dominated by this.
- **The talk's explicit chunk-size measurement** ("4×4×4 chunks use least
  VRAM, 8×8×8 rendered faster") is the model for the last stage: parameterize
  chunk size and *measure* the matrix instead of guessing.

The phase is deliberately **primary-view only** (no lighting, no effects):
the goal is a primary pass whose latency is understood, measured in-world and
out, and gated by data at every step.

## Decision record (grill-me-docs session, 2026-08-02)

1. **Objective**: primary-view latency only. No lighting, no effect rays, no
   palette→PBR, no transparency. Everything else is out of scope.
2. **Scenario**: measure BOTH the existing outside orbit (baseline) and a new
   deterministic **in-world orbit** (camera inside chunk AABBs) — the real
   first-person regime the engine targets; also exercises the untested
   camera-inside-chunk TLAS behavior.
3. **Lever budget**: shader-level (early-out + tmax, tight AABBs) plus
   **chunk-size rethink as a first-class lever** (parameterize + measure).
   Half-res/upscale and per-cell cost are NOT in this phase.
4. **Sequencing**: levers first, chunk-size matrix last (the guard and tight
   boxes change the size optimum, so size after levers).
5. **Metric**: data-gated with stretch targets — bistro_sm outside orbit
   **<10 ms GPU** and **≥40% cells cut on both orbits** (in-world target set
   against its own new baseline); byte-exact frame-dump A/B stays green on
   both orbits.
6. **Caps**: loud + data-gated raise — cap-exceed counters + heatmap tint
   first; cap *values* unchanged until in-world data shows exceedances.
7. **Instrumentation**: heatmap (dead `viewport_and_heatmap` flag) AND
   extended stats counters, in both paths for comparability.
8. **tmin**: align the ray-query tmin to the raster near plane (0.1) for the
   phase, so the in-world byte-exact gate is clean.
9. **Raster path**: behavior untouched (plan-004 STOP condition). Additive
   stats/heatmap instrumentation in both paths is in scope; levers apply to
   the ray-query path only.
10. **Out of scope**: edits/streaming, per-chunk BLAS rebuild granularity,
    compaction, half-res, PBR, lighting, removing the A/B gate.

## Current state (verified at plan time)

- `assets/shaders/rayquery.wgsl`: `rq_main` ray-query loop + ported
  `dda_chunk(origin, dir, chunk_index, out)` (no tmax), `RayDesc(0u, 0xFFu,
  0.001, 10000.0, ...)`; `%%STATS_DECLS/CELLS/PIXEL/HIT%%` markers; stats
  binding at group(1) binding(3), AS at binding(4). `GpuAabb` = full chunk
  box (32 m).
- `assets/shaders/chunk.wgsl`: same DDA + `%%STATS_%%` markers; stats binding
  at group(1) binding(2) (no AS).
- `src/render/rayquery.rs`: `RayQueryResources::new` builds `aabb_buf`
  (full chunk boxes), one world BLAS, TLAS; stats injection via string
  replace; `tlas`/`world_blas` kept alive.
- `src/app.rs`: stats buffer 16 bytes zeroed at `:851`, readback copy 16 bytes
  at `:972`; heatmap toggle `toggle_heatmap` `:1079` (flag carried in
  uniforms, read by no shader); orbit enabled by `WGPU_RT_ORBIT=1` `:265`,
  target/radius derived from chunk instances `:266-290`;
  `WGPU_RT_DUMP=<dir>` surface dump after frame 40 `:1111-1140`;
  binding-array fatal check `:218-222`.
- `src/player_controller.rs`: `OrbitParams`, `DEFAULT_ORBIT_PARAMS` (60 s/rev,
  elevation 5..55° cos sweep), `orbit_pose(elapsed, target, radius, params)`
  `:189`, `orbit_radius_from_chunks` `:209`.
- `src/world/chunk.rs`: `CHUNK_TEXTURE_SIZE` 256³, `CHUNKS_X/Y/Z` 8/1/8,
  `TOTAL_CHUNKS` 64; `to_mip_bytes` loops `MIP_LEVELS` (9, `src/world/mod.rs`);
  texture `mip_level_count: MIP_LEVELS`; occupancy downsample.
- `src/world/loader.rs`: `SceneGraphLoader` flattens `.vox` into
  `VoxelWorldData` HashMap and chunks it. No binary bake tool in-tree.
- `src/bin/bench.rs`: forces `WGPU_RT_ORBIT=1`, overrides
  `max_binding_array_elements_per_shader_stage = TOTAL_CHUNKS.min(adapter
  limit)` `:77`.

## Design

### Step 0 — In-world orbit preset (scenario)

Add a second orbit mode selected by `WGPU_RT_ORBIT=in` (keep `=1` as today's
outside orbit; `F2` toggle unchanged):

- Target = content centroid (mean of *occupied* voxels, not chunk origins),
  radius = small enough that the camera stays inside chunk AABBs for a full
  sweep. Concretely: `radius = min(0.5 × content_half_extent, 12.0)` m,
  clamped ≥ 4 m so the sweep covers interior geometry; verify per world that
  `target ± radius` stays within the union of chunk AABBs for the whole orbit
  (assert at startup, log the pose bounds).
- Same `orbit_pose`/`DEFAULT_ORBIT_PARAMS` math — deterministic.
- The camera-inside-chunk TLAS behavior is exercised by construction; if the
  in-world image shows a systematic artifact (black band, first-chunk miss),
  that is a real bug to report, not to paper over.

### Step 1 — Instrumentation (both paths, comparability)

**Counters** — extend the Stats struct from `{fragments, processed_cells,
hits}` to add (all atomics, buffer size becomes `4 + 2*chunk_slots + 1` u32s
or as designed):

- `per_chunk_cells[slots]` — cells walked per chunk (indexed by
  chunk/instance id; the raster path already knows its instance).
- `dda_invocations[slots]` — DDA runs per chunk.
- `committed_hits[slots]` — invocations that generated a hit (in rayquery:
  `dda_chunk` returned true). Wasted work ≈ invocations − committed per
  chunk, per frame.
- `cap_exceeded` — count of `GLOBAL_CELL_CAP` early-outs (the silent-miss
  path) and, if cheap, root/refinement cap pops.

Update both shader marker blocks (`%%STATS_DECLS%%` etc. in `chunk.wgsl` via
`app.rs`, `rayquery.wgsl` via `rayquery.rs`), the zero-fill size
(`app.rs:851`), and the readback size (`app.rs:972`) together — they must
stay in lockstep. Keep the existing per-frame print contract of the bench.

**Heatmap** — implement the dead `viewport_and_heatmap` branch in both
shaders: when the flag is set, output `ramp(processed_cells)` instead of the
palette color, with a distinct tint (e.g., magenta) when the pixel's
traversal hit a cap. Per-pixel `processed_cells` already exists in both
DDA bodies — expose it to the output path (rayquery: return via `HitResult`
or an out-param; raster: via the existing fragment locals). No clock exists
in WGSL; cells-per-pixel is the proxy for the doc's clockARB heatmap.

### Step 2 — Lever 1: occluded-candidate early-out + tmax (finding 4)

In `assets/shaders/rayquery.wgsl` only:

- Track `best_t = INF` (initialized from the committed intersection once
  available; in practice keep a local `var best_t: f32 = INF` updated from
  `rayQueryGetCommittedIntersection` after each `rayQueryProceed`).
- For each `RAY_QUERY_INTERSECTION_AABB` candidate, compute the AABB span
  (reuse `ray_aabb`; move the call out of `dda_chunk` so the span is
  available in `rq_main`). If `span.x >= best_t`, skip the DDA entirely
  (`continue` the proceed loop).
- Otherwise call `dda_chunk(origin, dir, c.primitive_index, min(span.y,
  best_t), &res)` — the new `tmax` param bounds the traversal:
  `interval.y = min(span.y, tmax)` at `init_frame`, and the existing
  `at_or_after_exit` pop handles the bound with zero new special cases.
- Update `best_t` from the committed intersection as the loop proceeds.

**Early-out correctness** (why this is safe): the committed intersection
after `rayQueryProceed` is the minimum-t *generated* intersection so far. Any
hit inside candidate C lies at t ≥ span.x(C) (the AABB entry), so if
span.x(C) ≥ best_t, no hit in C can improve the committed result — skipping
is exact. If span.x(C) < best_t, the DDA bounded by tmax = min(span.y(C),
best_t) finds the same closest relevant hit as an unbounded run (hits are
monotone along the ray; the DDA returns the first hit). With no committed
hit, best_t = INF, the guard never fires, and tmax = span.y — byte-identical
to today. Regression-guard with the existing dump A/B on both orbits.

### Step 3 — Lever 2: tightly-fitted chunk AABBs (finding 3)

In the ray-query path only (the raster path builds its own proxy instances
from `InstanceRaw`; `aabb_buf` is ray-query-only):

- At load, compute each chunk's occupied AABB from its voxel HashMap
  (min/max over non-zero material coords), convert to world space, and pad by
  **exactly 1 voxel** (0.125 m) on each side.
- Padding rule: the padded box strictly contains all occupied voxels, so any
  ray whose span intersects occupied geometry behaves inside the DDA exactly
  as with the full box (same hit t/mat). Rays that pass through empty space
  only now skip the chunk entirely — the same miss the full box produced,
  at lower cost. Byte-exactness of hits is preserved by construction; verify
  with the dump A/B.
- Update `aabb_buf` construction in `RayQueryResources::new` to take the
  tight bounds instead of `[position, position + chunk_side_world]`.
- The in-shader `gpu_aabbs[chunk_index]` read (DDA bounds) stays correct —
  it reads the same buffer; the DDA now walks the tight span.

### Step 4 — Re-measure both orbits; evaluate stretch targets

Bench matrix (see Verification) after Steps 0–3. Stretch targets:
bistro_sm outside orbit **<10 ms GPU**; **≥40% cells cut** on both orbits
(in-world vs its own Step-0 baseline). If the data misses the stretch, the
plan is still complete — the size matrix (Step 5) is the remaining lever and
its outcome decides the next phase; do NOT expand scope.

### Step 5 — Chunk-size matrix (last, data-gated)

**Parameterize** (env-gated, default = today's 256³ behavior byte-identical):

- `WGPU_RT_CHUNK_SIZE` (64/128/256; default 256) drives, at load:
  `CHUNK_TEXTURE_SIZE`, `MIP_LEVELS = log2(size) + 1` (chunk.rs currently
  hardcodes both; `to_mip_bytes`, texture creation, and mip upload loops take
  the runtime values).
- Shader constants are injected via the existing `include_str` + `.replace`
  marker pattern (repo precedent: `%%STATS_*%%`): add `%%ROOT_MIP%%`
  (`= log2(size) - 3`, root grid stays 8³) and `%%CHUNK_WORLD_SIZE%%`
  (`= size × 0.125` m) to both `rayquery.wgsl` and `chunk.wgsl`. Default
  values must reproduce today's shaders exactly.
- **Content-derived chunk grid**: replace the fixed `CHUNKS_X/Y/Z` 8/1/8
  constants with a grid derived from content bounds (`ceil(extent / size)`
  per axis). This also lifts the 2048×256×2048-voxel world cap the fixed
  layout imposed.
- Binding-array: the existing fatal check (`app.rs:218-222`) and bench
  override (`bench.rs:77`) keep working from the actual chunk count; log the
  adapter max (already logged) next to the chosen count.

**Matrix** (on the Step-4 shader, both orbits × monu1 + bistro_sm):
`{256 (baseline), 128, 64}` — with 64 included only if the adapter's
`max_binding_array_elements_per_shader_stage` ≥ the chunk count at 64 (bistro
content ≈ 128 chunks; monu1 fewer). Per size, record: cells/frame,
per-chunk distribution, GPU ms (stats=0), BLAS/TLAS build time (add a
timestamp around the one-shot `build_acceleration_structures`), and peak
texture VRAM (expected size-invariant — occupied volume × 8/7 — verify, do
not use as a decision driver).

**Decision rule**: pick the size with the best cells/frame × GPU ms on
bistro_sm in-world (the discriminating regime), provided the outside orbit
does not regress >10% and the byte-exact gate stays green at that size on
both worlds. If 64 is infeasible (binding limit) or regresses, report 128 vs
256 with the data and stop — do not improvise a texture-atlas workaround.

### Verification (after each step, in order)

Build/lint/test:
1. `cargo check` — compiles.
2. `cargo clippy --all-targets -- -D warnings` — clean under the deny list.
3. `cargo test` — existing suite green (incl. orbit tests, mip-DDA reference
   tests, shader validation); add tests for the in-world preset constants and
   (if the stats layout changes) the new buffer size.

A/B correctness gates (rayquery vs raster, byte-exact dumps):
4. Outside orbit, both worlds: `WGPU_RT_RAYQUERY=1 WGPU_RT_DUMP=<dir>
   cargo run --release --bin bench -- 60` vs raster dump — byte-exact within
   the accepted orbit-drift tolerance (99.41% precedent), on both worlds.
5. In-world orbit, both worlds: same comparison. The tmin alignment (Step 0
   wiring, see below) is what makes this gate clean.

Perf baselines (record, then compare each step against the prior):
6. `WGPU_RT_ORBIT=1 WGPU_RT_RAYQUERY=1 WGPU_RT_STATS=0 cargo run --release
   --bin bench -- 900` → GPU ms both worlds (existing command).
7. `WGPU_RT_STATS=1` variant → cells/frame + new counters; in-world preset
   (`WGPU_RT_ORBIT=in`) adds the missing regime.
8. After Step 3: confirm hits identical (dump A/B) and record cells delta —
   tight AABBs must not change any committed hit.
9. Step 5 matrix: per size × orbit × world, the table in "Matrix" above;
   a `--chunk-size` passthrough env is enough, no bench-code matrix driver
   needed beyond reading `WGPU_RT_CHUNK_SIZE`.

tmin alignment (part of Step 2 or standalone before Step 4): change the
`RayDesc` tmin from `0.001` to `0.1` (raster near-plane parity), with a named
constant and a comment; verify the outside-orbit dumps are unaffected
(beyond the already-accepted silhouette tolerance) and the in-world dumps
match the raster path.

## Changes

| File | Change |
| --- | --- |
| `src/player_controller.rs` | `ORBIT_INWORLD` preset (target=content centroid, content-derived radius); tests |
| `src/world/chunk.rs` | parameterize `CHUNK_TEXTURE_SIZE`/`MIP_LEVELS` (runtime values); tight-AABB computation from voxel map |
| `src/world/mod.rs` | `MIP_LEVELS` no longer const (or derived); content-bounds helpers |
| `src/world/loader.rs` | content-derived chunk grid (replaces fixed 8/1/8); pass chunk size through |
| `src/render/rayquery.rs` | tight AABB buffer; stats layout/size; `%%ROOT_MIP%%`/`%%CHUNK_WORLD_SIZE%%` injection; BLAS build timestamp |
| `src/render/mod.rs` | `GpuAabb` unchanged (tight values in); possibly a `TightBounds` helper |
| `assets/shaders/rayquery.wgsl` | early-out + tmax in `dda_chunk`; heatmap branch; extended `%%STATS_%%` markers; `%%ROOT_MIP%%`/`%%CHUNK_WORLD_SIZE%%` |
| `assets/shaders/chunk.wgsl` | heatmap branch + extended `%%STATS_%%` markers; `%%ROOT_MIP%%`/`%%CHUNK_WORLD_SIZE%%` (additive only) |
| `src/app.rs` | orbit preset selection, stats zero/readback sizes, dump under orbit, tmin note |
| `src/bin/bench.rs` | orbit/`WGPU_RT_CHUNK_SIZE` passthrough, extended stats print |
| `tests/` | in-world orbit tests; stats-size test; shader validation for the new markers (guard against the naga `NotInScope` regression class) |

## STOP conditions

- Any change to the raster path's *behavior* (proxy geometry, depth, early-Z,
  draw order) is required to make a lever work — additive instrumentation
  only. If a lever needs raster behavior changes, stop and report.
- The byte-exact A/B gate fails on either orbit with no diagnosable cause
  within two fix attempts (black frame, flipped image, systematic first-chunk
  miss, cap-induced holes that the Step-1 counters attribute to a cap value
  rather than a lever).
- A matrix size needs a workaround for the binding-array limit (texture
  atlas, descriptor tricks) — report "infeasible" and continue with the
  feasible sizes.
- `WGPU_RT_CHUNK_SIZE=256` (the default) does not reproduce today's shaders
  byte-for-byte at the marker level (i.e., the parameterization changed the
  default path) — the default must be behavior-identical.
- The in-world orbit exposes a camera-inside-chunk TLAS artifact that looks
  structural (not cap- or tmin-related) — report it as a bug with a captured
  frame; do not continue tuning levers against a broken primary.
- `cargo clippy` cannot be satisfied without loosening the deny list (report
  which lint; do not weaken the config unilaterally).

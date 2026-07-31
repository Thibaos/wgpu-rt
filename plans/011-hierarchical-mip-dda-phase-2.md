# Plan 011: Implement bounded hierarchical mip DDA traversal

> **Executor instructions:** Follow this plan step by step. Run each verification
> command before moving to the next step. This is a shader/control-flow change;
> do not improvise around a failed boundary test or shader validation error.
> Update the Plan 011 row in `plans/README.md` to `DONE` only after every done
> criterion passes. Do not update the ADR or `CONTEXT.md`; their phase-2 design
> text is already accepted and is reproduced below.
>
> **Drift check (run first):**
> `git diff --stat 0d72007..HEAD -- assets/shaders/chunk.wgsl tests/hierarchical_mip_dda.rs tests/shader_validate.rs plans/README.md`
> Expected output is empty because the planned baseline is the current HEAD.
> Then run `git status --short`; at plan authoring time the expected pre-existing
> entries are `M CONTEXT.md`, `M docs/adr/0002-dense-3d-texture-mip-dda.md`,
> `M plans/README.md`, and `?? plans/011-hierarchical-mip-dda-phase-2.md`.
> Do not reset, rewrite, or attribute the first two documentation changes to
> this implementation. If the expected pre-existing state or any listed
> implementation file differs, compare the Current state excerpts with the live
> file; on a mismatch, stop and report before editing.

## Status

- **Priority:** P1
- **Effort:** L
- **Risk:** HIGH — shader stack control flow and ray-boundary correctness
- **Depends on:** none; phase 1 is the planned baseline commit `0d72007`
- **Category:** perf (with correctness regression coverage)
- **Planned at:** commit `0d72007`, 2026-07-31

## Why this matters

Phase 1 traverses only mip 5, so an occupied 4 m coarse cell is rendered as if
its whole volume were solid. That produces incorrect surfaces and prevents the
mip chain from providing fine-grained empty-space skipping. This plan descends
from mip 5 to mip 0, preserves front-to-back ordering, and uses only mip 0 for
material and depth. A pure Rust reference makes the difficult interval,
negative-direction, tie, and sibling-recovery rules testable without a GPU.

## Current state

The executor must confirm these facts before implementation:

- `assets/shaders/chunk.wgsl` is included directly into the runtime render
  pipeline by `src/app.rs:295-300` using `include_str!` and is the only live
  chunk shader.
- The phase-1 shader defines a single mip-5 grid:
  - `CHUNK_WORLD_SIZE = 32.0` metres (`chunk.wgsl:49`), because the CPU chunk
    texture is 256³ voxels and `VOXEL_SCALE` is 0.125 m.
  - `MIP_LEVEL = 5`, `GRID_SIZE = 8`, and `CELL_SIZE = 4.0`
    (`chunk.wgsl:50-52`).
  - `fs_main` (`chunk.wgsl:118-187`) computes a ray/AABB span (via
    `ray_aabb`, 86-115), floors the entry position into a cell, samples
    `textureLoad(voxel_textures[in.chunk_id], cell, MIP_LEVEL)`, and
    returns `origin + dir * t` projected through `camera.view_proj` on a hit
    (161-173).
  - The current shader advances only one axis per iteration using a 24-iteration
    fixed loop (`chunk.wgsl:159-187`); this is the code being replaced.
- `src/world/chunk.rs:59-72` produces nine CPU mip levels, 256→1. Each coarser
  cell contains the first non-zero child material, but only zero/non-zero is a
  valid coarse occupancy signal. `create_texture` uploads those levels as
  `R8Uint` at `src/world/chunk.rs:118-156`.
- `tests/shader_validate.rs:17-23` currently has the exact offline shader gate
  `chunk_wgsl_parses_and_validates`, which reads the live shader and calls
  `naga::front::wgsl::parse_str`. Keep this test passing; do not assume a GPU is
  available in CI.
- Existing Rust tests follow ordinary `#[cfg(test)]` modules in production files
  (for example `src/world/chunk.rs:163-240`). For this larger reference seam,
  use a separate integration test file so it remains test-only and has a clear
  independent API: `tests/hierarchical_mip_dda.rs`.

### Accepted design constraints

The accepted wording in `CONTEXT.md` and `docs/adr/0002-dense-3d-texture-mip-dda.md`
is authoritative:

- A Chunk is a 256³-voxel sub-volume spanning 32 m; local voxel coordinates are
  `u8`; `0` means air.
- Phase 2 starts at mip 5 (8³ cells, 4 m cells), descends `5 → 4 → 3 → 2 → 1
  → 0`, and uses mips 5 through 1 for occupancy only. Mip 0 supplies material,
  entry depth, and the hit voxel.
- Each refinement is a local 2³ DDA bounded by the occupied parent cell. The
  child texture origin is exactly:
  `child_tex_origin = 2 * (parent.tex_origin + parent.cell)`.
- Traversal is half-open: intervals are `[t_enter, t_exit)`. A non-zero coarse
  value is only a hint; it is never a color or a hit.
- Cross-chunk traversal, cross-chunk ordering, and occlusion culling are out of
  scope. Chunks remain independently rasterized proxy cubes.

## Commands you will need

| Purpose | Command | Expected result |
|---|---|---|
| Drift check | `git diff --stat 0d72007..HEAD -- assets/shaders/chunk.wgsl tests/hierarchical_mip_dda.rs tests/shader_validate.rs plans/README.md` | Empty at the start, or reviewed drift followed by an explicit report/stop. |
| Reference tests | `cargo test --test hierarchical_mip_dda` | The integration test binary passes all reference/oracle cases. |
| Shader validation | `cargo test --test shader_validate -- --exact chunk_wgsl_parses_and_validates` | Exactly one test passes; no WGSL parse/type error. |
| Full tests | `cargo test` | Exit 0; all unit and integration tests pass. |
| Formatting | `cargo fmt --check` | Exit 0 with no formatting diff. |
| Lint | `cargo clippy --all-targets -- -D warnings` | Exit 0 with no warnings. |
| Asset preflight | `test -f assets/models/bistro_sm.vox` | Exit 0; the smoke-test asset exists in the working tree. If using Windows PowerShell instead, run `Test-Path assets/models/bistro_sm.vox` and require `True`. |
| Runtime smoke test | `cargo run` | After the asset preflight passes, the app starts with the hardcoded `assets/models/bistro_sm.vox`, logs a loaded voxel count and non-empty chunk count, and reaches the event loop without a shader/pipeline validation failure. This is a manual, non-CI gate: observe the window for at least one rendered frame, then close it with the window close control. |

No package installation or dependency change is needed. `naga` is already a
`dev-dependency` in `Cargo.toml:19-20`.

## Git workflow

Do not reset, commit, or push the existing documentation changes. Work in the
current checkout and keep the implementation diff limited to the Scope list.
If the operator requires a commit, use a message matching the existing
imperative style (for example, `feat: chunk DDA fragment shader ...`), but do
not create it unless explicitly instructed.

## Scope

**In scope — the only implementation files to modify:**

- `assets/shaders/chunk.wgsl` — replace the phase-1 single-level DDA with the
  bounded six-frame hierarchical traversal.
- `tests/hierarchical_mip_dda.rs` — create the test-only sparse CPU reference,
  independent mip-0 oracle, and deterministic regression tests. The test file
  may contain a compact fixture enum for the logically full ray-path case, but
  ordinary explicit levels remain sparse `HashMap` grids.
- `plans/README.md` — update only Plan 011's status row after completion.

**Out of scope — do not modify:**

- `src/world/chunk.rs`: retain its existing nine-level `R8Uint` upload path.
- `src/app.rs`: retain the proxy-cube pipeline, palette binding, chunk ID, and
  runtime shader include.
- `tests/shader_validate.rs`: retain its existing parser gate unless the shader
  cannot be checked without a minimal test-only adjustment; if adjustment is
  necessary, stop and report first.
- `Cargo.toml` and `Cargo.lock`: no new dependency is required.
- `CONTEXT.md`, `docs/adr/0002-dense-3d-texture-mip-dda.md`, and any prior plan.
- Any cross-chunk ray traversal, occlusion culling, world-level ordering,
  texture format change, palette change, or CPU production renderer.

## Steps

### Step 1: Confirm the baseline and current contracts

Run the drift check and inspect the files listed in Current state. Confirm that
`0d72007` is still the baseline and that the existing shader still uses the
phase-1 constants and `fs_main` shape. Do not edit anything in this step.

**Verify:**
`git diff --stat 0d72007..HEAD -- assets/shaders/chunk.wgsl tests/hierarchical_mip_dda.rs tests/shader_validate.rs plans/README.md`
→ empty output at the planned baseline. If not empty or if the excerpts do not
match, STOP and report the exact changed path and mismatch.

### Step 2: Add the independent test-only CPU reference and oracle

Create `tests/hierarchical_mip_dda.rs`. Keep all reference types and functions
inside this integration-test file; they must not be exported from production
modules.

Implement these explicit contracts:

1. Store compact mip levels as `Vec<HashMap<IVec3, u8>>`, indexed by mip number
   (`levels[0]` is mip 0, `levels[5]` is mip 5). Missing keys are zero/air.
   Mip 0 has a logical size of 256³; mip `m` has size `256 >> m`.
2. Provide a helper that accepts explicit levels, including intentionally
   malformed coarse levels, and a separate helper that generates levels 1..5
   from sparse mip-0 material cells using the same 2³ occupancy rule as the
   CPU chunk path. The generated helper must preserve materials only as an
   occupancy witness; tests must never use a coarse material as a rendered
   material. Because materializing all 16,777,216 cells of a fully occupied
   256³ HashMap is needlessly expensive, represent the full-case test as a
   compact `Full(u8)` fixture used only by the reference lookup: for the
   fixture, *every* queried cell at *every* mip (0 through 5) returns the
   material. The lookup never receives the ray — this is safe only because
   traversal queries are confined to the ray path by construction. Name the
   test `fully_occupied_ray_path_returns_nearest_voxel`; it means every cell
   the selected test ray intersects is occupied, not that the fixture
   allocates the entire logical volume. Do not use `Full` for malformed-level,
   mapping, or hierarchy-generation tests.
3. Use normalized chunk-local coordinates in `[0, 1]³`. A mip-0 voxel
   `(x,y,z)` occupies `[x/256,(x+1)/256)` etc. The ray direction supplied to
   the reference is a unit vector in this normalized coordinate system for the
   march cases, but the reference must accept *any* direction, including the
   zero vector: treat a direction with `dot(dir, dir) <= (3.125e-10)^2` as a
   miss — this is the normalized equivalent of the shader's 1e-8 m ray-length
   threshold, and it is what makes the zero-length-ray test reachable. Return
   `Hit { material: u8, t: f32, voxel: IVec3 }`, where `t` is the normalized
   ray parameter from the ray origin to the mip-0 cell entry. A miss is `None`.
4. Implement the hierarchical reference with the same observable rules as the
   shader: root mip 5 grid 8; refinement grids 2; six frames; explicit texture
   origin; parent advanced before child push; half-open intervals; negative
   boundary correction; all-axis tie advancement; and sibling recovery. The
   reference must use sparse map lookups and must not call the shader or inspect
   GPU resources. The reference must NOT apply the shader's 24/8/2048 caps —
   those are shader safety bounds, not algorithm behavior, and a capped
   reference would spuriously diverge from the oracle. Termination is
   structural, not capped: each frame's interval shrinks with every advance, a
   ray crosses at most 4 cells of a 2³ grid and at most 22 of the 8³ root grid,
   and every descent ends at mip 0.
5. Implement a separate direct mip-0 DDA oracle. It must calculate its own
   ray/AABB entry/exit and use an analytical termination bound of
   `3 * BASE_SIZE + 1` positive-width cell-processing iterations
   (`BASE_SIZE = 256`), not the shader's 24/8/2048 caps and not the hierarchical
   implementation's helper. Zero-width intervals do not consume this bound.
   It samples only `levels[0]` and returns the first non-zero mip-0 material,
   entry `t`, and voxel coordinate. If the bound is exhausted without a hit,
   return `None`.

Use epsilon `3.125e-8` for normalized-coordinate comparisons. This is the
normalized equivalent of the shader's `1e-6` metre comparison epsilon. Epsilon
may be used for comparisons only; retain raw boundary times in returned hits.
Both implementations must skip intervals with width `<= epsilon`, reject
point-only edge/corner contacts, clamp every cell coordinate to its valid grid,
and treat a zero-length ray as a miss.

Add deterministic tests covering all of these named cases:

- `empty_chunk_is_a_miss` and
  `fully_occupied_ray_path_returns_nearest_voxel` (the compact full-path
  fixture defined above);
- nearest-hit ordering with two occupied voxels on the same ray;
- positive and negative entry boundaries, including a ray starting exactly on
  a voxel boundary;
- a multi-axis edge/corner tie that must advance every tied axis;
- coordinate mapping for low and high mip-0 coordinates, including `(0,0,0)`
  and `(255,255,255)`;
- generated valid hierarchy descent, comparing hierarchical and oracle results
  for several axis-aligned and diagonal rays;
- a false-positive non-zero coarse cell with no mip-0 descendant, which must
  produce a miss rather than a coarse hit;
- sibling recovery where an occupied child branch has no mip-0 hit and a later
  front-to-back sibling does, which must return the sibling's material;
- a ray with zero direction length, which must be a miss.

For every generated-hierarchy case, assert both result voxel/material equality
and `abs(hierarchical.t - oracle.t) <= 3.125e-8` (or a documented larger
floating-point tolerance only if the raw entry values demonstrably differ).

**Verify:** `cargo test --test hierarchical_mip_dda` → the new integration test
binary passes all named cases before shader work begins.

### Step 3: Implement the bounded WGSL traversal state

In `assets/shaders/chunk.wgsl`, retain the vertex interface, bindings,
palette lookup, `ray_aabb`, and fragment output contract. Replace the phase-1
DDA constants and body with the following state model.

Define constants with these values:

- `CHUNK_WORLD_SIZE = 32.0`;
- root `mip = 5`, root `grid_size = 8`, root cell size 4 m;
- `T_EPS = 1e-6` metres, used only in comparisons;
- `ROOT_CELL_CAP = 24`, `REFINEMENT_CELL_CAP = 8`;
- `GLOBAL_CELL_CAP = 2048`;
- a static outer traversal-loop bound of `TRAVERSAL_BOUND = 16384` (do not
  reduce it below 8195). Derivation, kept as a comment in the shader: each
  iteration either samples one positive-width cell (global cap 2048 samples;
  the 2049th discards), pops a frame, or skips a zero-width cell. Pops are
  bounded by pushes + 1, and pushes accompany mip-1..5 samples (≤ total
  samples), so pops ≤ 2049; a zero-width skip is immediately followed by a
  sample or a pop, so skips ≤ pops + samples + 1 = 4098. Total iterations
  ≤ 2048 + 2049 + 4098 = 8195; 16384 leaves margin. A naive
  `GLOBAL_CELL_CAP + stack_depth` bound is NOT sufficient — pops and skips are
  bounded by the number of samples, not by the six-frame stack depth. No WGSL
  `loop` or other unbounded control-flow construct.

Define a `TraversalFrame` containing at least:

`mip: u32`, `grid_size: i32`, `tex_origin: vec3<i32>`,
`bounds_min: vec3<f32>`, `bounds_max: vec3<f32>`, `interval: vec2<f32>`,
`cell: vec3<i32>`, `t: f32`, `t_max: vec3<f32>`, `t_delta: vec3<f32>`,
`axis_step: vec3<i32>`, and `steps_taken: i32`.

Use `array<TraversalFrame, 6>` plus an explicit `stack_len`. The frame
invariant is: `tex_origin + cell` is the texture coordinate for the current
cell at `mip`; `t` is that cell's raw ray-entry parameter; and `interval.y` is
the exclusive region exit.

Implement one `init_frame` helper that receives the ray, mip, grid size, texture
origin, world bounds, and an explicit interval. It must:

1. Compute the local cell from the interval entry point and set
   `frame.t = interval.x` — the frame's first sample uses the interval entry
   parameter, never `min(t_max)` (which is the cell exit).
2. Apply direction-aware negative-boundary correction before `floor`: when the
   ray component is negative and the normalized coordinate is within the
   comparison epsilon (`T_EPS`, 1e-6 m) of an integer boundary, decrement the
   floored cell on that axis before clamping. Positive-direction boundary
   entries select the cell on the positive side. Clamp every axis to
   `[0, grid_size - 1]`.
3. Set `t_delta[axis] = cell_size / abs(dir[axis])` and `axis_step` to -1 or
   +1, where `cell_size = (bounds_max - bounds_min) / grid_size` per axis —
   NOT `CHUNK_WORLD_SIZE / grid_size`, which is wrong for child frames (a
   mip-4 child of a root cell is 2 m, not 16 m). Parallel axes use
   `t_max = t_delta = INF` and step 0 (the `PARALLEL_EPS` threshold).
4. Compute each `t_max` as an absolute ray parameter to the next boundary in
   that axis, not as a relative duration. Preserve the unmodified/raw value.

Implement one `advance_frame` helper. It computes the raw minimum of the three
`t_max` values, sets `frame.t` to that raw minimum, then advances every axis
whose boundary is within `T_EPS` of that minimum by applying, per tied axis:
`cell[axis] += axis_step[axis]` and `t_max[axis] += t_delta[axis]`. This is
required for edge and corner ties; do not use an `else if` chain that advances
only one axis, and do not update `t` without also updating the cell and `t_max`
of every tied axis.

Initialize the root as mip 5, grid 8, `tex_origin = (0,0,0)`, bounds equal to
the chunk AABB, and the analytic ray/AABB span. Treat the span as half-open:
reject it when its width is `<= T_EPS`, and do not sample after the exclusive
exit.

### Step 4: Implement descent, caps, and hit output

In WGSL, `init_frame(frame inputs) -> TraversalFrame` must return a complete
value, and `advance_frame(frame: TraversalFrame) -> TraversalFrame` must return
an updated value. WGSL callers must assign the returned value back to
`frames[stack_len - 1]`; do not rely on mutating a by-value parameter or on an
unsupported pointer/reference feature. If the pinned WGSL front end rejects
returning or assigning a struct containing these fields, stop and report the
exact error rather than changing the six-frame design.

In the shader traversal loop, process the top frame as follows:

1. If `stack_len == 0`, discard. If the top frame has reached its local cap,
   has no positive-width current interval, or its current entry is at/after
   the exclusive interval exit within `T_EPS`, pop it. Popping a root frame
   ends the traversal; popping a child resumes the already-advanced parent.
   Guard both pop and push operations; an underflow discards, and a full stack
   treats the branch as unresolvable and continues the already-advanced parent
   rather than writing a coarse hit.
2. Compute the current cell's raw exit as the minimum of the next boundary and
   the frame's exclusive interval exit. If the width is `<= T_EPS`, do not
   sample it; advance/pop instead. Only for a positive-width interval, increment
   the global processed-cell count; if it would exceed 2048, discard. Then
   increment the current frame's `steps_taken` and sample
   `textureLoad(voxel_textures[in.chunk_id], frame.tex_origin +
   frame.cell, frame.mip)`. Thus zero-width intervals consume neither the
   per-frame nor global processed-cell budget.
3. At mip 0, a non-zero value returns its palette color and depth calculated
   exactly as phase 1: `hit_world = origin + dir * frame.t`, then
   `clip = camera.view_proj * vec4<f32>(hit_world, 1.0)` and
   `clip.z / clip.w`. The material must come only from the mip-0 sample. A zero
   value at mip 0 advances the current frame exactly as at mip 1..5 — it is not
   a miss, a discard, or a coarse hit.
4. At mip 1..5, a zero value advances the current frame. A non-zero value is
   occupancy only. Capture `parent_entry = frame.t`, `parent_next = raw next
   boundary`, and `child_exit = min(parent_next, frame.interval.y)` before
   advancing the parent. Derive the selected parent cell's world AABB from
   `frame.bounds_min`, `frame.bounds_max`, and `frame.cell`. Advance the parent
   with all-axis tie behavior. If `child_exit - parent_entry <= T_EPS`, do not
   push. Otherwise push `init_frame` with:
   - `child_mip = frame.mip - 1`;
   - `child_grid_size = 2`;
   - `child_tex_origin = 2 * (parent.tex_origin + parent.cell)`;
   - the selected parent cell's world bounds;
   - interval `[parent_entry, child_exit)`.

The stack must preserve front-to-back sibling order: the parent is advanced
before the child is pushed, so a child miss or cap exhaustion returns to the
saved parent state. Coarse values must never be used for color, depth, or an
immediate hit.

Before normalizing the ray, guard the camera-to-fragment vector against zero
length. Define `RAY_LENGTH_EPS = 1e-8` metres and discard when
`dot(delta, delta) <= RAY_LENGTH_EPS * RAY_LENGTH_EPS`, before calling
`normalize`. The CPU reference uses the corresponding normalized threshold
`3.125e-10` stated in Step 2. Preserve the existing `PARALLEL_EPS` slab
handling, but ensure no near-zero normalization can reach `ray_aabb`.

When the outer traversal loop ends without a mip-0 hit (root frame popped, or
`TRAVERSAL_BOUND` reached with no hit and no `discard` taken), the fragment
must end with `discard;`. A WGSL fragment function must terminate on every path
with `return` or `discard`, so the post-loop fallthrough is mandatory for the
shader to validate; it also preserves the phase-1 miss = discard contract.
Because `TRAVERSAL_BOUND = 16384` exceeds the derived 8195-iteration worst
case, the bound should never be the reason a hit is lost — if it ever is, that
is a bug to report, not an acceptable miss.

**Verify:** `cargo test --test shader_validate -- --exact chunk_wgsl_parses_and_validates`
→ exactly one test passes. If naga rejects a WGSL construct, stop and adapt the
implementation to an equivalent statically bounded construct; do not disable or
remove the validation test.

### Step 5: Run the complete gates and smoke test

Run the reference tests again, then all repository gates in Commands you will
need. Inspect the final diff and confirm only the in-scope implementation files
plus the Plan 011 status row changed.

Run `cargo run` using the existing hardcoded `assets/models/bistro_sm.vox`.
Observe startup logs containing a loaded voxel count, non-empty chunk count,
and successful entry into the event loop. Confirm that the rendered scene still
shows plausible geometry, that empty space is skipped rather than rendered as
solid coarse blocks, and that visible surfaces retain palette colors and depth.
This is a manual smoke gate, not a substitute for the deterministic Rust tests.

**Verify:** `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
→ all commands exit 0; then `cargo run` starts and renders without a shader
compilation/pipeline error.

## Test plan

- `tests/hierarchical_mip_dda.rs` must contain the sparse explicit-level
  reference, generated hierarchy helper, independent direct mip-0 oracle, and
  all named boundary/ordering/descent/recovery cases from Step 2.
- The reference and oracle must be separate implementations. Do not implement
  the oracle by calling the hierarchical traversal or by using shader caps.
- `tests/shader_validate.rs` remains the structural pattern for the offline
  shader gate; run its exact test name as shown above.
- Existing `src/world/chunk.rs` mip tests must continue to pass through the full
  `cargo test` gate. Do not replace them with the reference tests.

## Done criteria

All of the following must be true:

- [ ] `cargo test --test hierarchical_mip_dda` passes all named CPU reference,
      oracle, boundary, and sibling-recovery cases.
- [ ] `cargo test --test shader_validate -- --exact chunk_wgsl_parses_and_validates`
      passes exactly one test.
- [ ] `cargo test` exits 0.
- [ ] `cargo fmt --check` exits 0.
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0.
- [ ] The shader has a six-frame bounded stack, no unbounded WGSL loop, root
      cap 24, refinement cap 8, global processed-cell cap 2048, an outer
      `TRAVERSAL_BOUND = 16384` (≥ 8195) with the derivation comment, and a
      post-loop `discard;` fallthrough.
- [ ] Only mip 0 supplies material/color/depth; coarse non-zero values only
      trigger bounded descent.
- [ ] The Rust oracle and hierarchical reference agree on material, voxel, and
      entry parameter for valid generated hierarchies within the documented
      epsilon.
- [ ] `cargo run` reaches the event loop with `bistro_sm.vox` and no runtime
      shader/pipeline error; the manual smoke observations are recorded in the
      implementation result.
- [ ] Compared with the pre-existing status recorded in Step 1, only
      `assets/shaders/chunk.wgsl`, `tests/hierarchical_mip_dda.rs`, and the
      Plan 011 status row in `plans/README.md` are newly changed; the
      pre-existing `CONTEXT.md` and `docs/adr/0002-dense-3d-texture-mip-dda.md`
      modifications remain untouched.
- [ ] Plan 011's row in `plans/README.md` is updated from `TODO` to `DONE` only
      after all preceding checks pass.

## STOP conditions

Stop and report instead of improvising if:

- The drift check or Current state excerpts do not match the live repository.
- The intended frame mapping cannot satisfy
  `child_tex_origin = 2 * (parent.tex_origin + parent.cell)` without changing
  the accepted design.
- WGSL rejects arrays of `TraversalFrame`, dynamic stack indexing, or the
  chosen bounded loop under the repository's naga/wgpu versions. Report the
  exact construct and error before changing the design or dependencies.
- A boundary/tie test disagrees with the independent mip-0 oracle and the cause
  is not an obvious test-data error. Do not loosen epsilon or remove the test.
- Implementing the shader appears to require changing the CPU mip upload,
  texture format, palette binding, proxy-cube draw, or any out-of-scope file.
- Any verification command fails twice after a reasonable in-scope correction.
- The runtime smoke test cannot start because of an adapter/device/environment
  issue; report that separately from code-test results rather than weakening
  the shader tests.
- At Step 1, before any edits, the working tree contains paths other than the
  four pre-existing entries allowed by the drift check (`CONTEXT.md`,
  `docs/adr/0002-dense-3d-texture-mip-dda.md`, `plans/README.md`, and this
  untracked plan); stop and ask the operator whether to proceed rather than
  resetting or absorbing them into this plan. (After Steps 2-5 the tree
  legitimately gains `assets/shaders/chunk.wgsl` and
  `tests/hierarchical_mip_dda.rs`, so this check applies only at Step 1.) The
  existing `plans/README.md` entry is allowed to change only in Plan 011's
  status cell, from `TODO` to `DONE` after all gates pass.

## Maintenance notes

- Any future change to chunk dimensions, voxel scale, mip count, or texture
  origin mapping must update both the CPU chunk constants and the shader's
  hardcoded traversal constants, then rerun the independent oracle cases.
- Reviewers should scrutinize negative-direction entry, exact boundary starts,
  tied-axis advancement, half-open interval exits, parent advancement before
  child push, and the distinction between occupancy and material.
- The 2048 global budget is a safety bound, not a correctness oracle. The CPU
  reference intentionally uses its own analytical mip-0 bound so shader caps
  cannot make both implementations agree on an early miss.
- Cross-chunk ordering and occlusion are deliberately deferred. A later plan
  must not infer world visibility from this per-chunk traversal or from the
  proxy cube's rasterized depth.

# Plan 013: Complete plan 011 — hierarchical mip DDA CPU reference tests

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. A reviewer dispatches you and maintains the
> index, so SKIP any instruction to update `plans/README.md`; the reviewer
> updates it. Commit your work in the worktree with a message in the repo's
> imperative style (e.g. `test: hierarchical mip DDA CPU reference`).
>
> **Drift check (run first)**:
> `test ! -e tests/hierarchical_mip_dda.rs` — expected: the file does not
> exist (exit 0 when the `test !` succeeds). Then confirm the facts in
> "Current state" below against the live files. On any mismatch, STOP and
> report the exact path and difference.

## Status

- **Priority:** P1
- **Effort:** M
- **Risk:** LOW — test-only file; no production code touched
- **Depends on:** plan 011's phase-2 shader being present in the repo (the
  shader rewrite is committed before this plan runs; the full `cargo test`
  gate exercises both the new test and the existing shader validation)
- **Category:** tests (correctness regression coverage)
- **Planned at:** commit `3dcfe40`, 2026-07-31

## Why this matters

Plan 011's phase-2 shader (`assets/shaders/chunk.wgsl`) is implemented and
committed: a bounded six-frame hierarchical mip DDA (root mip 5, refinement
grids of 2, caps 24/8/2048, `TRAVERSAL_BOUND = 16384`, mip-0-only material).
It passes `cargo test --test shader_validate`, `cargo fmt --check`, and
`cargo clippy --all-targets -- -D warnings`. The one missing plan-011
deliverable is the pure-Rust sparse CPU reference and independent mip-0
oracle that make the shader's difficult rules — negative boundary correction,
tie advancement, half-open intervals, sibling recovery — testable without a
GPU. This plan writes that single file. It is the CPU mirror of the GPU
traversal; the done criterion is that the two independently-implemented CPU
algorithms agree on material, voxel, and entry parameter within epsilon.

## Current state (confirm before editing)

- `tests/hierarchical_mip_dda.rs` does not exist; `tests/` contains only
  `shader_validate.rs` (the existing offline WGSL gate, `tests/shader_validate.rs`,
  is the structural pattern for an integration test file in this repo).
- The full Step-2 spec for this file is recorded verbatim in
  `plans/011-hierarchical-mip-dda-phase-2.md` under "### Step 2" — it is
  reproduced in full below; follow the below text, not a paraphrase.
- CPU mip generation lives in `src/world/chunk.rs:59-72`: nine levels, 256→1;
  each coarser cell holds the first non-zero child material, but only
  zero/non-zero is a valid coarse occupancy signal. The test's generated-hierarchy
  helper must use the same 2³ first-non-zero occupancy rule.
- The shader constants this test mirrors: `ROOT_MIP = 5` (8³ root grid),
  refinement grid size 2, six stack frames, half-open intervals
  `[t_enter, t_exit)`, comparison epsilon 1e-6 m (normalized equivalent
  3.125e-8), ray-length threshold 1e-8 m (normalized equivalent
  (3.125e-10)² against `dot(dir,dir)`). See `assets/shaders/chunk.wgsl:49-83`.

## Scope

**In scope — the only file to create:**

- `tests/hierarchical_mip_dda.rs` — the sparse CPU reference, independent
  mip-0 oracle, and all named regression tests.

**Out of scope — do not modify:**

- `assets/shaders/chunk.wgsl`, `src/app.rs`, `src/framework.rs`,
  `src/player_controller.rs`, `src/render/*`, `src/world/chunk.rs`,
  `src/utils.rs` — no production code changes.
- `tests/shader_validate.rs` — retain the existing parser gate untouched.
- `Cargo.toml`, `Cargo.lock` — no new dependency is required (`naga` is
  already the only dev-dependency).
- `CONTEXT.md`, `docs/adr/*`, any plan file, and `plans/README.md` (the
  reviewer owns the index).

## Step 1: Confirm baseline facts

Run the drift check from the header and open `tests/shader_validate.rs` to
see the integration-test conventions (module layout, `#[test]` functions,
no `main`). Confirm the `src/world/chunk.rs:59-72` mip rule and the
`chunk.wgsl:49-83` constants listed in Current state. Do not edit anything
in this step.

**Verify:** drift check passes and the three cited code regions match the
descriptions above.

## Step 2: Create the reference and oracle test file

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
   reference must use sparse map lookups and must not call the shader or
   inspect GPU resources. The reference must NOT apply the shader's 24/8/2048
   caps — those are shader safety bounds, not algorithm behavior, and a capped
   reference would spuriously diverge from the oracle. Termination is
   structural, not capped: each frame's interval shrinks with every advance, a
   ray crosses at most 4 cells of a 2³ grid and at most 22 of the 8³ root grid,
   and every descent ends at mip 0.
5. Implement a separate direct mip-0 DDA oracle. It must calculate its own
   ray/AABB entry/exit and use an analytical termination bound of
   `3 * BASE_SIZE + 1` positive-width cell-processing iterations
   (`BASE_SIZE = 256`), not the shader's 24/8/2048 caps and not the
   hierarchical implementation's helper. Zero-width intervals do not consume
   this bound. It samples only `levels[0]` and returns the first non-zero
   mip-0 material, entry `t`, and voxel coordinate. If the bound is exhausted
   without a hit, return `None`.

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

**Verify:** `cargo test --test hierarchical_mip_dda` → the new integration
test binary compiles and passes all named cases.

## Step 3: Full gates

**Verify:** `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
→ all exit 0. The full `cargo test` must pass the new reference tests, the
existing `tests/shader_validate.rs` gate, and all unit tests in `src/world/chunk.rs`
(the existing mip tests must keep passing — do not replace them).

## Done criteria

All of the following must be true:

- [ ] `cargo test --test hierarchical_mip_dda` passes all named CPU reference,
      oracle, boundary, and sibling-recovery cases.
- [ ] `cargo test` exits 0 (includes `shader_validate` and the existing
      `chunk.rs` mip tests).
- [ ] `cargo fmt --check` exits 0.
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0.
- [ ] The reference and the oracle are separate implementations (the oracle
      does not call the hierarchical reference or share its per-cell advance
      helper; the reference does not apply the 24/8/2048 shader caps).
- [ ] The `Full(u8)` fixture is used only by
      `fully_occupied_ray_path_returns_nearest_voxel`; malformed-level,
      mapping, and hierarchy-generation tests use sparse explicit levels.
- [ ] All named tests from Step 2 exist verbatim as test names.
- [ ] The only new or changed file is `tests/hierarchical_mip_dda.rs` (plus
      the commit in the worktree).

## STOP conditions

Stop and report instead of improvising if:

- The drift check fails or any "Current state" fact does not match the live
  files.
- A boundary/tie test cannot be made to agree between the reference and the
  oracle, and the cause is not an obvious test-data error. Do not loosen
  epsilon or delete a named test.
- Implementing the spec appears to require changing `src/world/chunk.rs`,
  `assets/shaders/chunk.wgsl`, `Cargo.toml`, or any out-of-scope file.
- `cargo test` fails in a pre-existing test unrelated to this file; report the
  exact failing test and its output rather than touching the failing test.

## Maintenance notes

- The reference, oracle, and shader are three independent expressions of one
  traversal contract. When the shader changes (new caps, different root mip,
  different epsilon), update this file's mirrors in the same commit — the
  epsilons and `BASE_SIZE` are duplicated here by design.
- Watch in review: a "generated hierarchy" test that secretly hardcodes the
  expected hit instead of comparing reference vs oracle; a reference that
  imports shader caps; an oracle that delegates to the reference's advance
  helper (breaks the independence criterion).
- The manual plan-011 smoke gate (`cargo run` with `assets/models/bistro_sm.vox`
  and observing a rendered frame) is a separate, operator-side check; it is not
  part of this plan's done criteria.

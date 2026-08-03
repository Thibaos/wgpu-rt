# Plan history — execution & reconcile ledger

Chronological paper trail for the plan backlog: execution runs, reconcile
passes, measurement outcomes, and audit decisions. The **status board** lives
in `README.md` — this file is the "why" behind it. Per-plan details live in
the plan files themselves; this ledger covers cross-plan events only.

---

## 2026-08-03 — Restructure

`README.md` slimmed to a status board; all reconcile/execution history moved
into this file. Also merged since the pass-3 batch: `git merge exec-020`
(commit `d3304c2`) and `git merge exec-022` (merge `09e5981`) — both fixes
are in master at HEAD `c151709`. Local branches `exec-020`/`exec-022` remain
but are deletable; worktrees stayed in `%TEMP%`.

Fourth-pass reconcile results (current baseline):

- **Baseline at HEAD `c151709`**: `cargo check` clean; `cargo test` 55/55
  (19 bench unit + 19 main unit + 12 mip-DDA + 5 shader_validate, the 5th
  being 022's heatmap guard); `cargo fmt --check` clean; `cargo clippy -- -D
  warnings` (lib+bin) clean; `cargo clippy --all-targets -- -D warnings`
  still RED (71 in `hierarchical_mip_dda.rs` + 8 in `shader_validate.rs`,
  down from 72+9).
- **Plan 015 drift check re-verified live** — every Current-state excerpt
  matches (chunk.rs 256³ texture + 8×8×8 grid + mip funcs; mod.rs
  `MIP_LEVELS=9` + `split_chunks` + palette buffer; `RayQueryParams`
  field-for-field; rayquery.wgsl ROOT_MIP=5 + six-frame stack + heatmap
  read; app.rs raster default + `WGPU_RT_RAYQUERY` swap; tree64 0 matches;
  both test targets exist and pass). **015 is dispatchable.**
- DONE spot-checks all pass: 001 resize (`app.rs:728`), 002 acquire
  Timeout/Outdated (`framework.rs:62-70`), 008 feature narrowing
  (`framework.rs:131-148`), 009 cursor-grab fallback (`framework.rs:252-258`),
  010 chunk wiring (`app.rs:204-232`), 011 mip-DDA tests 12/12, 012 orbit
  (`app.rs:266`), 016 `WGPU_RT_PROFILE` timestamps, 019 `WGPU_RT_RAYQUERY`
  (`app.rs:117`).

## 2026-08-03 — Execution batch (plans 020/022/021)

- **020 executed** (worktree `wgpu-rt-exec-020`, branch `exec-020`, commit
  `d3304c2`) — APPROVED after one revision round: round-1 STOPPED at Step 5
  (bistro_sm dropped 3405 voxels — round-half-away offset 1025 pushed
  world-y onto the grid's exclusive bound for the exactly-2048-tall scene);
  plan revised to a `floor()` offset + a 5th regression test; re-verified
  independently (bistro_sm 5→91 chunks @ (1024,1024,1024) with zero drops,
  church 111, monu1 7 mid-grid; 54 tests; clippy/fmt clean).
- **022 executed** (worktree `wgpu-rt-exec-022`, branch `exec-022`, commit
  `f90cb7f`) — APPROVED: `HitResult.cells` + heatmap branch in both shaders
  (HEATMAP_CELL_SCALE 64), `WGPU_RT_HEATMAP=1` env, grep-guard test; verified
  via threshold dump-diff (201k differing bytes > 2%) and the ray-query leg;
  45 tests (31 unique); shader_validate clippy count unchanged at 9.
- **021 executed** in-tree (docs-only; `plans/` is the advisor's domain):
  015's drift check rewritten to expect `bde0db4`+ with 020/022 landed;
  Current-state bullets quote post-020/022 state; Stage-3 tree64 step →
  verify-absent; Stage-5 unclipped-scenes note added. Deviation from the
  branch rule: 020/022 were EXECUTED but UNMERGED when 021 ran, so 015's
  bullets were written as post-020/022.

## 2026-08-03 — Reconcile pass 3 → plans 020/021/022

- **Baseline at HEAD `bde0db4`**: `cargo check` clean; `cargo test` 30/30;
  fmt clean; **`cargo clippy --all-targets -- -D warnings` FAILS** — 72
  errors in `tests/hierarchical_mip_dda.rs`, 9 in `tests/shader_validate.rs`
  (indexing_slicing / as_conversions / arithmetic_side_effects / unwrap /
  expect / panic; the test files lack the `#[allow]` blocks in-crate test
  modules carry). The "clippy clean" claims in earlier reconcile notes no
  longer hold. Plans 020/022 gate on `cargo clippy` (lib+bin) deliberately,
  not `--all-targets`.
- **COR-01 verified live**: `WGPU_RT_WORLD=assets/models/bistro_sm.vox cargo
  run --bin bench -- 2` loads 13.8M voxels spanning world x∈[-836,1093],
  y∈[-895,1152], z∈[-155,412] but only 5/64 chunks survive (`into_chunks`
  silently drops out-of-grid voxels; loader centers at the single-chunk
  center 128, not the grid center 1024; grid is one chunk tall). church
  5/64, sponza 13/64, monu1 1/64 (fits). Plans 016/019's bistro/church
  numbers were measured on fragments. → plan 020.
- **Plan 015 drift stale**: drift check expected HEAD `f014d3b` with the
  kickoff edits uncommitted and `tree64` at `Cargo.toml:15`; actual state
  was `bde0db4` (kickoff committed), clean tree, tree64 gone → plan 021.
- **Heatmap dead**: `viewport_and_heatmap` uploaded per frame but read by no
  shader; `H` a no-op in both renderers → plan 022.
- **Adapter facts** (for 020/015 decisions): NVIDIA RTX 3070 (Vulkan),
  `max_binding_array_elements_per_shader_stage: 1048576` — CHUNKS_Y=8 (512
  chunks) is safe here.

## 2026-08-02 — Kickoff (plans 014/015 drafted; tree64 retired)

- **Tree64 fully removed** (0 matches in `Cargo.toml`/`Cargo.lock`) by user
  decision; the whole `docs/adr/` series retired (ADR-0002 removed —
  architecture decisions now live in plan files). Docs cleaned (CONTEXT.md,
  research docs).
- **014 drafted** (primary-view latency: heatmap + counters instrumentation,
  in-world orbit, occluded-candidate early-out + tmax, tight chunk AABBs,
  chunk-size matrix) then **REJECTED as superseded by 015**: the 8³
  storage-buffer chunk rewrite retires the 256³ texture architecture 014
  would have instrumented/optimized. Transferable pieces (early-out + tmax,
  stats/heatmap harness, orbit presets) were folded into 015.
- **015 drafted** from the grill session on
  `docs/research-teardown-hardware-ray-tracing.md` + the no-3D-textures
  direction; design decisions recorded in-plan.

## 2026-07-31 — Advisor batch (plans 016–019) + reconcile passes 1–2

- Plans 016–019 folded in from the deleted `advisor-plans/` directory.
  They predate 014–015 in creation order; IDs reflect merge order, not
  chronology. Their executor-instruction prose describes the old advisor
  workflow in places — treat them as historical records.
- **011 corrected**: it had been marked DONE prematurely — the phase-2
  shader (`assets/shaders/chunk.wgsl`, six-frame stack, mip-0-only material)
  passed `shader_validate`/`fmt`/`clippy`, but the pure-Rust reference test
  `tests/hierarchical_mip_dda.rs` was never written (`cargo test --test
  hierarchical_mip_dda` failed with "no test target"); shader work was
  uncommitted. Status corrected to IN PROGRESS. RESOLVED the same day: the
  shader was committed in `68a1481`; the missing reference test was found
  complete-but-uncommitted in the surviving plan-011 worktree
  `wgpu-rt-exec-011`, reviewed green, committed in `cc56462`. **013 rejected
  as redundant** (its deliverable already existed). Worktree `wgpu-rt-exec-011`
  removed.
- 011 fully verified at `b0f332c`: `cargo test` 22/22 (9 unit + 12 mip-DDA +
  1 shader_validate), fmt/clippy clean.
- **012 drift re-checked** at `b0f332c` (`git diff --stat 3dcfe40..HEAD --
  src/player_controller.rs src/app.rs src/framework.rs` empty); one stale
  note refreshed in the plan file itself.
- **012 executed** in worktree `wgpu-rt-exec-012` (branch `exec-012`): orbit
  tests 5/5, suite 27/27 (14 unit + 12 mip-DDA + 1 shader_validate),
  fmt/clippy clean. Smoke: startup target (41.6,16.0,28.8), radius 82.7 m,
  1 Hz pose log, azimuth advancing 6°/s, elevation tracking the 5..55° cos
  sweep, no shader/pipeline error; default run has zero orbit output.
  Documented deviations (all benign): `f32::abs_diff` → `.abs()` epsilon
  (older toolchain), empty-slice floor asserted with epsilon to dodge
  `clippy::float_cmp`, one `cargo fmt` pass, gitignored
  `assets/models/bistro_sm.vox` copied into the worktree for smoke runs.
  F2 keystroke delivery to the GUI window was flaky from bash (SendKeys +
  AppActivate retry succeeded; disable confirmed via log + FPS jump 18→82).
  Diff later merged to master as `be727cf`.
- **016 DONE**: instrumentation rebuilt in-tree (env-gated timestamps + DDA
  counters) with a headless bench. Release measurements: monu1 20–23 ms,
  bistro_sm 70–89 ms, church 58–105 ms GPU — GPU-bound, latency-bound
  (400–860M cells/s). Optimization A measured no gain (40.06→39.40/40.41
  ms), gated/reverted.
- **017 BLOCKED**: the Step-1 probe gate STOPPED the build — invocation
  ratio 1.09 ≤ 1.15, overdraw 1.00–1.24x px; a cross-chunk rewrite would
  save <9% of fragment work at MED-HIGH risk; probe reverted. The early-Z
  experiment (frag_depth removal + front-to-back sort) also measured no GPU
  gain — depth restored. Direction set: half-res or compute path → taken by
  plan 019.
- **019 DONE**: TLAS-of-chunk-AABBs ray-query renderer executed in-tree at
  `f014d3b`; primary pass 2.4–3.8x faster (monu1 21.0→6.4–8.9 ms,
  bistro_sm 60.7→16.7–17.2 ms GPU). Basis for plans 014/015. **Measured on
  clipped bistro/church scenes — see plan 020.**
- **018 REJECTED**: never executed — the fragment path it optimized was
  retired by plan 019.

## Before the ledger (2026-07-06 – 2026-07-30) — original backlog (plans 001–010)

- Backlog generated by the improve skill on 2026-07-06. Plans 001–010
  executed and DONE: resize-safe RT texture (001), surface acquire
  timeout/outdated handling (002), chunked world architecture (003), color
  palette (004), feature-request narrowing (008), cursor-grab error handling
  (009), chunked-world wiring into the renderer (010), Tree64 GPU bake from
  occupied voxels (006).
- **005 rejected** — dense 3D-texture DDA direction unsuitable for required
  16384³ logical scenes unless dense-volume constraints change (kept as an
  alternative direction, not a dependency of 006).
- **007 rejected** — FPS player controller; rationale was never recorded
  (decision predates the ledger; see the plan file for scope).
- Plan 006 noted as the recommended next step for the 4096→16384 bake
  regression (deliberately never benchmarked against `bistro.vox` during
  implementation).
- Plan 007 was designed via grilling session + domain modeling (2026-07-11);
  `CONTEXT.md` holds the domain glossary.

---

## Findings — resolved & rejected (audit trail)

For future audit runs: these were investigated and closed. Do not re-flag
without new evidence.

**Resolved:**

- **#4** tree64 unpinned (`Cargo.toml:15`) — tree64 fully removed at the
  2026-08-02 kickoff (0 matches in `Cargo.toml`/`Cargo.lock`).
- **#7** unused VoxelRT shaders — `assets/shaders/` holds only `chunk.wgsl`
  and `rayquery.wgsl` (removed in `f1145be`).
- **#8** fragile build.rs string replace — no `build.rs` in the tree; the
  shader loads via `include_str!("../assets/shaders/chunk.wgsl")` at
  `src/app.rs:298`.
- **#9** clippy collapsible-if in `framework.rs` — fixed in `40eee8d`.
- **Old #10** cargo fmt diff in `tree64_renderer.rs` — fixed in `40eee8d`.
  (Note: the pass-3 notes mislabeled the clippy `--all-targets` issue as
  "#7", colliding with the unused-shaders finding above; it is **#10** in
  the README findings table.)

**Considered and rejected (2026-07-31 advisor batch):**

- *Cross-chunk fullscreen traversal* — probe-rejected: overdraw 1.00–1.24x
  px, invocation_ratio 1.09 ≤ 1.15 gate; <9% fragment work at MED-HIGH risk.
- *early-Z experiment* (frag_depth removal + front-to-back sort) — measured
  no GPU gain; depth restored.
- *half-res DDA + upscale* — deferred (quality trade-off; enabled later by
  the compute path in plan 019).
- *compute-shader ray-query rewrite* — adopted as plan 019.
- *heatmap diagnostics* — folded into plan 016.

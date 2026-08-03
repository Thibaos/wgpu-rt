# Plan 022: Restore the traversal heatmap — per-pixel DDA-work coloring in both renderers

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` (row 022) unless a reviewer dispatches you and told
> you they maintain the index.
>
> **Drift check (run first)**: `git status --short -- src/ assets/ tests/` —
> expected: **empty** (no uncommitted changes to source/shader/test files;
> untracked/modified files under `plans/` are expected and fine — the plan
> batch itself dirties it). HEAD must be `bde0db4` or later
> (`git log --oneline -1`). Then spot-check the excerpts in "Current state"
> against the live files. On any mismatch, STOP and report the exact path
> and difference.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW-MED (WGSL changes; guarded by naga validation and a dump-diff smoke test)
- **Depends on**: none (plan 015's Stage 2 must preserve the heatmap read —
  see Maintenance notes)
- **Category**: bug / tooling (dead feature restored)
- **Planned at**: commit `bde0db4`, 2026-08-03
- **Issue**: none

## Why this matters

The `H` key toggles a "traversal heatmap" (`App::toggle_heatmap` →
`App::heatmap` → the `viewport_and_heatmap` uniform field, uploaded every
frame at `src/app.rs:803-807`), but **no shader reads the flag** — grep shows
`viewport_and_heatmap` only in the `CameraUniforms` struct declarations of
both `chunk.wgsl` and `rayquery.wgsl`. The toggle is a silent no-op in both
renderers. This diagnostic was a deliberate deliverable of plans 014/016
(the Teardown-derived tool for spotting occluded DDA work — see
`docs/research-teardown-hardware-ray-tracing.md`, finding 4: heatmaps showed
fully-occluded intersection-shader invocations) and it died during a shader
rewrite. Plan 015's decision record promises "Stats/heatmap harness preserved"
— this plan makes that true: per-pixel DDA-work coloring in both the raster
shader and the ray-query shader, keyed by the flag that is already uploaded.

The heatmap color encodes how many DDA cells the ray's traversal processed
before its hit: dark blue ≈ cheap (1-8 cells), red ≈ expensive (≥64
cells). Sky pixels stay black.

## Current state

- `src/app.rs:803-807` — the uniform upload (already correct, no change
  needed beyond Step 3):
  ```rust
  viewport_and_heatmap: [
      u32_to_f32(self.surface_width),
      u32_to_f32(self.surface_height),
      if self.heatmap { 1.0 } else { 0.0 },   // <-- index 2 = heatmap flag
      0.0,
  ],
  ```
- `src/app.rs:1079-1087` — `toggle_heatmap` flips `self.heatmap` (no change).
- `assets/shaders/rayquery.wgsl`:
  - `struct HitResult { t: f32, mat: u32 };` (no cells field)
  - `dda_chunk(...)` maintains `var processed_cells: i32 = 0;` locally,
    incremented per positive-width sample, but only surfaces it via the
    `%%STATS_CELLS%%` marker (stats build only).
  - `rq_main` tail:
    ```wgsl
    var res: HitResult;
    var found = false;
    ...
    let committed = rayQueryGetCommittedIntersection(&rq);
    if (committed.kind == RAY_QUERY_INTERSECTION_GENERATED && found) {
        color = palette[res.mat];
    }
    textureStore(out_color, gid.xy, color);
    ```
- `assets/shaders/chunk.wgsl` — `fs_main` tail:
  ```wgsl
  if (mat != 0u) {
      let hit_world = origin + dir * top.t;
      let clip = camera.view_proj * vec4<f32>(hit_world, 1.0);
      // %%STATS_HIT%%
      return FragmentOutput(palette[mat], clip.z / clip.w);
  }
  ```
  `fs_main` also maintains `var processed_cells: i32 = 0;` incremented at
  each positive-width sample.
- Repo conventions:
  - Shader edits must keep the `%%STATS_*%%` markers intact (they are
    string-replaced by `WGPU_RT_STATS=1` at `src/app.rs:500-518` and
    `src/render/rayquery.rs:267-283`). The markers are comments in the
    checked-in file.
  - WGSL style follows the existing files: `select`, explicit `u32`/`i32`
    casts (`f32(...)`, `u32(...)`), `T_EPS`-style named constants, no magic
    numbers without a comment.
  - `CameraUniforms.viewport_and_heatmap` is `vec4<f32>`; the heatmap flag
    is **component index 2** (`.z`) — `[width, height, heatmap, 0]`.
  - Tests use naga via the dev-dependency (`tests/shader_validate.rs`);
    in-crate test modules use the big `#[allow(clippy::...)]` block (see
    `src/world/chunk.rs:233-249`).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Compile | `cargo check` | exit 0 |
| Shader validation | `cargo test --test shader_validate` | 4 tests pass |
| Full tests | `cargo test` | all pass |
| Lint (lib+bin) | `cargo clippy` | exit 0 — **NOT `--all-targets`** (see Scope) |
| Format | `cargo fmt --check` | exit 0 |
| Dump smoke | see Step 4 | two dumps differ |

## Scope

**In scope** (the only files you should modify):
- `assets/shaders/rayquery.wgsl`
- `assets/shaders/chunk.wgsl`
- `src/app.rs` (one env-default read in `App::init`)
- `tests/shader_validate.rs` (one new no-op test — see Step 5)
- `plans/README.md` (status row, Step 6)

**Out of scope** (do NOT touch, even though they look related):
- `tests/hierarchical_mip_dda.rs` and the pre-existing clippy failures in
  the integration tests (72 + 9 errors — tracked as a separate open finding;
  this plan's new shader_validate test must add **zero** new clippy
  violations, hence the no-`unwrap` design in Step 5).
- The `%%STATS_*%%` instrumentation and `WGPU_RT_STATS` handling — unchanged.
- The palette, the color-space/tonemapping path, the blit shader.
- Plan 015's flat-DDA rewrite — this plan works on the current hierarchical
  shaders; 015 ports the heatmap read (see Maintenance notes).
- Anything outside the files listed above.

## Git workflow

- Branch: `advisor/022-heatmap` (or match the operator's workflow).
- Commit style: `feat: per-pixel DDA-work heatmap in both renderers
  (WGPU_RT_HEATMAP=1)` — match `git log --oneline`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Ray-query shader — surface the cell count and colorize

In `assets/shaders/rayquery.wgsl`:

1. Extend `HitResult` with a `cells` field:
   ```wgsl
   struct HitResult {
       t: f32,
       mat: u32,
       cells: u32,
   };
   ```
2. In `dda_chunk`, on the mip-0 hit path (the `return true;` inside
   `if (top.mip == 0u)`), write the count before returning:
   ```wgsl
   (*out).t = top.t;
   (*out).mat = mat;
   (*out).cells = u32(processed_cells);
   return true;
   ```
   (`processed_cells` is `i32` — cast with `u32(...)`; WGSL casts are
   truncating and `processed_cells` is non-negative here.)
3. In `rq_main`, replace the color assignment tail:
   ```wgsl
   let committed = rayQueryGetCommittedIntersection(&rq);
   if (committed.kind == RAY_QUERY_INTERSECTION_GENERATED && found) {
       if (camera.viewport_and_heatmap.z > 0.5) {
           let heat = clamp(f32(res.cells) / 64.0, 0.0, 1.0);
           color = vec4<f32>(
               mix(vec3<f32>(0.05, 0.0, 0.2), vec3<f32>(1.0, 0.3, 0.0), heat),
               1.0,
           );
       } else {
           color = palette[res.mat];
       }
   }
   ```
   Keep the `// %%STATS_PIXEL%%` marker line and everything else untouched.

   If naga rejects reading `res.cells` because `res` may be uninitialized
   (the existing `var res: HitResult;` is only written on hit), change the
   declaration to an initialized constructor — `var res = HitResult(0.0, 0u,
   0u);` (valid positional constructor for the t, mat, cells field order) —
   and continue. (Try the minimal change first; the read is guarded by
   `found`, and the current code already compiles with `palette[res.mat]`
   under the same guard.)

   **Caveat (pre-existing quirk, not introduced by this plan)**: `res` is
   overwritten by every `dda_chunk` call that returns `true` inside the
   `rayQueryProceed` loop, so `res.cells`/`res.mat` come from the **last**
   generated candidate, not necessarily the committed (nearest) hit. This
   already applies to `res.mat` today; the new `cells` field inherits it.
   Irrelevant for monu1 (single chunk); in multi-chunk scenes the heatmap
   shows the last candidate's cost.

**Verify**: `cargo test --test shader_validate` → 4 tests pass (naga
validates the default build and the stats build).

### Step 2: Raster shader — same branch in `fs_main`

In `assets/shaders/chunk.wgsl`, at the mip-0 hit return, insert the heatmap
branch before the normal return (keeping the existing `%%STATS_HIT%%` marker
line where it is):

```wgsl
if (mat != 0u) {
    let hit_world = origin + dir * top.t;
    let clip = camera.view_proj * vec4<f32>(hit_world, 1.0);
    // %%STATS_HIT%%
    if (camera.viewport_and_heatmap.z > 0.5) {
        let heat = clamp(f32(processed_cells) / 64.0, 0.0, 1.0);
        return FragmentOutput(
            vec4<f32>(mix(vec3<f32>(0.05, 0.0, 0.2), vec3<f32>(1.0, 0.3, 0.0), heat), 1.0),
            clip.z / clip.w,
        );
    }
    return FragmentOutput(palette[mat], clip.z / clip.w);
}
```

`processed_cells` is already in scope in `fs_main`. The `64.0` divisor (≈ one
8×8 cell row of work) is the cheap/expensive knee; keep it as a plain
literal with a comment, matching the file's style of documented constants —
or hoist it to a `const HEATMAP_CELL_SCALE: f32 = 64.0;` at the top of the
shader if you prefer; either is acceptable.

**Verify**: `cargo test --test shader_validate` → 4 tests pass.

### Step 3: Env-gated default in `App::init`

In `src/app.rs`, next to the other env-gated flags in `App::init` (the
PROFILE/STATS reads live at `src/app.rs:190-191`; the `WGPU_RT_ORBIT`
pattern to mirror is at `src/app.rs:265`):

```rust
let heatmap_enabled = std::env::var("WGPU_RT_HEATMAP").is_ok_and(|v| v == "1");
```

and set `heatmap: heatmap_enabled,` in the `Self { ... }` initializer
(currently `heatmap: false,` at line 709). The `H` key toggle stays as-is
(flips the same flag at runtime).

**Verify**: `cargo check` → exit 0. `cargo clippy` → exit 0.

### Step 4: Dump-diff smoke test

`WGPU_RT_DUMP=<dir>` writes one raw frame at frame 40 (`src/app.rs`),
`WGPU_RT_ORBIT=1` is force-set by the bench, and the default scene is
monu1. **Caveat — the orbit pose is wall-clock dependent**: `App::render`
advances the orbit by measured `delta_time` (`update_delta_time()`,
`src/app.rs:720-723`), so frame 40's pose differs slightly between any two
runs. A pose-only difference moves just silhouette-edge pixels (well under
1% of the file); the heatmap branch recolors every hit pixel (palette →
heat ramp, typically ≳10% of the file on monu1). So compare by
**differing-byte count against a threshold**, not by raw `cmp`: plain
`cmp` would report "differ" even with heatmap disabled, and would pass
for the wrong reason.

```
mkdir -p target/dump_heatmap_off target/dump_heatmap_on
WGPU_RT_DUMP=target/dump_heatmap_off cargo run --quiet --release --bin bench -- 45
WGPU_RT_HEATMAP=1 WGPU_RT_DUMP=target/dump_heatmap_on cargo run --quiet --release --bin bench -- 45
diff_bytes=$(cmp -l target/dump_heatmap_off/dump_raster.bgra target/dump_heatmap_on/dump_raster.bgra | wc -l)
echo "differing bytes: $diff_bytes"
test "$diff_bytes" -gt 165888 && echo HEATMAP_DIFF_OK
```

- `cmp -l` prints one line per differing byte; the dump is 1920×1080×4 =
  8,294,400 bytes, so 165,888 = 2% of the file. `HEATMAP_DIFF_OK` printed
  means the flag visibly changes output.
- **If `diff_bytes` is ≤ 165,888, STOP and report** — the heatmap branch
  is not taking effect.
- If the default `WGPU_RT_PROFILE=1 WGPU_RT_STATS=1` legs are too slow,
  override: `WGPU_RT_PROFILE=0 WGPU_RT_STATS=0 cargo run --quiet --release
  --bin bench -- 45` (stats are global atomics — they do not change pixels).
- Also run the ray-query leg to confirm the compute shader compiles and
  executes the new branch (compile-and-run check; no threshold needed):
  ```
  mkdir -p target/dump_heatmap_rq
  WGPU_RT_RAYQUERY=1 WGPU_RT_HEATMAP=1 WGPU_RT_DUMP=target/dump_heatmap_rq cargo run --quiet --release --bin bench -- 45
  test -s target/dump_heatmap_rq/dump_rayquery.bgra && echo RAYQUERY_DUMP_OK
  ```

**Verify**: `HEATMAP_DIFF_OK` printed (diff_bytes > 165,888); both raster
dumps exist; `RAYQUERY_DUMP_OK` printed.

### Step 5: Guard against regression

In `tests/shader_validate.rs`, add one small test that both shaders consume
the flag (prevents the dead-feature regression this plan fixes). Write it
without `expect`/`unwrap`/indexing so it adds zero clippy violations (the
integration tests currently fail `clippy --all-targets` for pre-existing
reasons; this test must not make it worse):

```rust
#[test]
fn heatmap_flag_is_consumed_by_both_shaders() {
    for name in ["chunk.wgsl", "rayquery.wgsl"] {
        let src = std::fs::read_to_string(shader_path(name));
        let Some(source) = src.as_ref().ok() else {
            continue; // shader missing: the parse tests above will fail loudly
        };
        assert!(
            source.contains("viewport_and_heatmap.z"),
            "{name} must read the heatmap flag"
        );
    }
}
```

**Verify**: `cargo test --test shader_validate` → 5 tests pass. Confirm no
new clippy errors were introduced by this file:
`cargo clippy --test shader_validate -- -D warnings 2>&1 | grep -c "^error"`
→ count unchanged from the pre-existing baseline (9) — do not fix the
pre-existing 9.

### Step 6: Full verification and index

- `cargo test` → all pass, 31 **unique** tests. (This crate is bin-only
  with no `lib.rs`: `src/main.rs` and `src/bin/bench.rs` both include the
  modules, so in-crate unit tests run twice — `cargo test` prints 14×2 +
  12 + 4 = 44 passing today = 30 unique. The one new integration test in
  `tests/shader_validate.rs` runs once: 45 printed, 31 unique. Do not look
  for a literal "31" in the printed per-target counts.)
- `cargo fmt --check` → exit 0. `cargo clippy` → exit 0 (lib+bin).
- Update `plans/README.md` row 022 → DONE with a one-line summary (note the
  WGPU_RT_HEATMAP env and the threshold dump-diff verification).

## Test plan

- New test: `heatmap_flag_is_consumed_by_both_shaders` in
  `tests/shader_validate.rs` (Step 5) — grep-style guard against the exact
  regression this plan fixes.
- Verification relies on naga validation of both builds (existing 4 tests)
  plus the Step-4 dump-diff smoke test — the only end-to-end check that the
  flag actually changes output. Because the orbit pose is wall-clock
  dependent (not deterministic between runs), the check is a differing-byte
  count against a 2%-of-file threshold, which separates the heatmap's
  whole-image recoloring from sub-1% pose-jitter edge differences. Model
  the dump usage on `src/app.rs::maybe_dump_frame` and the bench's
  WGPU_RT_DUMP handling in `src/bin/bench.rs`.
- Clippy note: the new test is written without `expect`/`unwrap`/indexing
  so it adds zero violations; the 9 pre-existing errors in
  `tests/shader_validate.rs` are out of scope (separate finding).

## Done criteria

ALL must hold:

- [ ] `cargo check` exits 0
- [ ] `cargo test` exits 0 — 31 unique tests (45 printed: 14×2 unit + 12 mip-DDA + 5 shader_validate)
- [ ] `cargo test --test shader_validate` exits 0 with 5 passing tests
- [ ] `cargo clippy` (lib+bin) exits 0; clippy error count on
      `--test shader_validate` is unchanged from 9
- [ ] `cargo fmt --check` exits 0
- [ ] Step 4 prints `HEATMAP_DIFF_OK` (diff_bytes > 165,888 of 8,294,400)
- [ ] Step 4 ray-query leg prints `RAYQUERY_DUMP_OK` (`dump_rayquery.bgra`
      non-empty, bench exit 0)
- [ ] `grep -c "viewport_and_heatmap.z" assets/shaders/chunk.wgsl
      assets/shaders/rayquery.wgsl` → ≥ 1 in each
- [ ] `src/app.rs` reads `WGPU_RT_HEATMAP`
- [ ] `git status --short -- src/ assets/ tests/` shows changes only in the
      in-scope files (pre-existing `plans/` modifications are expected)
- [ ] `plans/README.md` row 022 updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" no longer match the live files.
- Naga rejects the new WGSL and the initialization workaround (Step 1
  escape hatch) does not resolve it — report the exact naga error rather
  than restructuring the traversal.
- The Step-4 dumps are byte-identical with heatmap on vs off.
- A verification fails twice after a reasonable fix attempt.
- You discover the change requires touching an out-of-scope file.

## Maintenance notes

- **Plan 015 (8³-chunk rewrite, flat DDA)** deletes `chunk.wgsl` and rewrites
  `rayquery.wgsl`'s `dda_chunk` into a flat mip-0 march. Its Stage-2 spec
  keeps the `%%STATS_*%%` markers and the `viewport_and_heatmap` flag — the
  heatmap branch here must be ported: the flat DDA's per-pixel cell count is
  still a local `processed_cells`, so the same `HitResult.cells` + branch
  pattern carries over. Plan 021's refresh notes this in 015's Current
  state.
- **`res.cells` is the last generated candidate, not the committed hit**
  (pre-existing `res.mat` quirk the new field inherits): if 015's flat DDA
  changes to a single hit per ray, the semantics will silently improve —
  worth a comment when porting.
- **`viewport_and_heatmap.z` is index 2** — if anyone reorders the
  `viewport_and_heatmap` upload at `src/app.rs:803-807`, the shaders must
  change in lockstep; the Step-5 grep test will catch a removal but not a
  reorder.
- The `64.0` cell-scale knee is a heuristic; if real scenes show a
  different cheap/expensive split, adjust the constant in both shaders
  (keep them in sync).
- The raster path's heatmap is subject to early-Z (fragments rejected before
  the shader never write color) — the ray-query path's is not. Expect the
  two heatmaps to differ on occluded geometry; that difference is exactly
  the occluded-work signal plan 016 was after.

# Plan 020: Fix silent world clipping — grid-centered loading, CHUNKS_Y=8, drop diagnostics

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` (row 020) unless a reviewer dispatches you and told
> you they maintain the index.
>
> **Drift check (run first)**: `git status --short -- src/` — expected: **empty**
> (no uncommitted changes to source; untracked/modified files under `plans/`
> are expected and fine). HEAD must be `bde0db4` or later
> (`git log --oneline -1`). Then spot-check the cited line ranges in
> "Current state" against the live files — the line numbers are
> load-bearing; the excerpts are elided with `// ...` and annotated with
> `// <--`. On any mismatch, STOP and report the exact path and difference.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW-MED (renders scenes that previously lost geometry; VRAM rises with non-empty chunk count — see Step 2)
- **Depends on**: none
- **Category**: bug / correctness
- **Planned at**: commit `bde0db4`, 2026-08-03
- **Issue**: none

## Why this matters

The engine silently drops geometry. The world is split into a **fixed 8×1×8
chunk grid** (2048×256×2048 voxels), but the loader centers every scene at
`(128, 128, 128)` — the center of **one** chunk, not the grid — and the grid
is only **one chunk (256 voxels) tall**. Measured on this machine (see the
audit that produced this plan): bistro_sm loads 13.8M voxels spanning
x∈[-836,1093], y∈[-895,1152], z∈[-155,412] after centering, and only **5 of
64** grid chunks survive; church 5/64; sponza 13/64. The drop is a silent
`continue` — no warning, no count. Every benchmark number for those scenes
(plans 016/019's "bistro_sm 70–89 ms" etc.) was measured on fragments, and
the app itself renders wrong geometry for any model larger than ~256 voxels
in any axis.

This plan: (1) center scenes on the **grid center**, (2) raise `CHUNKS_Y`
1→8 so the grid is a 2048³ cube (safe on this machine: the RTX 3070 reports
`max_binding_array_elements_per_shader_stage: 1048576`; adapters with a
lower limit already fail loudly at `App::init`, never silently), and (3) make
any residual drop **visible** with a warning log + count. Plan 015's Stage 1
(dynamic chunking, storage-buffer pool) supersedes the fixed grid entirely;
this plan is the stopgap that makes today's app and today's measurements
correct until then.

## Current state

- `src/world/chunk.rs:24-30` — the fixed grid constants (verbatim):
  ```rust
  pub const CHUNKS_X: u32 = 8;
  pub const CHUNKS_Y: u32 = 1;
  pub const CHUNKS_Z: u32 = 8;

  pub const CHUNKS_X_INT: i32 = 8;
  pub const CHUNKS_Y_INT: i32 = 1;
  pub const CHUNKS_Z_INT: i32 = 8;
  ```
- `src/world/loader.rs:32-37` — `center_world` anchors on the **single-chunk**
  center (128), not the grid center (1024); verbatim from line 32:
  ```rust
  let (w_x, w_y, w_z) = (
      i32::try_from(CHUNK_TEXTURE_SIZE.width.div_euclid(2)).unwrap_or_default(),
      i32::try_from(CHUNK_TEXTURE_SIZE.height.div_euclid(2)).unwrap_or_default(),
      i32::try_from(CHUNK_TEXTURE_SIZE.depth_or_array_layers.div_euclid(2))
          .unwrap_or_default(),
  );
  ```
  (This predates the multi-chunk grid; it was correct when the world was one
  256³ chunk.)
- `src/world/mod.rs:54-103` — `World::into_chunks` (elided with `// ...`):
  ```rust
  pub fn into_chunks(self) -> Vec<Chunk> {
      let mut chunks: Vec<Chunk> = (0..TOTAL_CHUNKS_INT)
          .map(|i| {
              let chunk_x = i.rem_euclid(CHUNKS_X_INT);
              let chunk_z = i.div_euclid(CHUNKS_X_INT * CHUNKS_Y_INT);
              Chunk::new(glam::IVec3::new(chunk_x, 0, chunk_z))  // <-- chunk_y never decoded
          })
          .collect();
      // ... lines 67-76: chunk_side = 256, world coords via offset, div_euclid ...
      for ((x, y, z), material) in self.voxels {
          // ... lines 82-89: chunk_x/y/z + local coords computed ...
          if chunk_x < 0 || chunk_y < 0 || chunk_z < 0
              || chunk_x >= CHUNKS_X_INT || chunk_y >= CHUNKS_Y_INT || chunk_z >= CHUNKS_Z_INT
          {
              continue;  // <-- silent drop
          }
          // ... lines 91-101: index = z*CHUNKS_Y*CHUNKS_X + y*CHUNKS_X + x,
          //     usize conversion, chunks.get_mut(index).insert(...) ...
      }
      chunks
  }
  ```
  Two bugs here: the `Chunk::new(..., 0, ...)` grid position **never decodes
  chunk_y** (invisible while CHUNKS_Y=1, fatal once CHUNKS_Y>1), and the
  bounds `continue` is silent.

- Repo conventions to match:
  - Logging via the `log` crate (`log::warn!` / `log::info!`), never
    `eprintln!` in `src/world/` (there is one stray `eprintln!` at
    `src/world/mod.rs:45` — do not add more).
  - The crate denies `arithmetic_side_effects` (Cargo.toml lints): use
    `.saturating_*` / `.rem_euclid` / `.div_euclid` as the surrounding code
    does. `u32`/`i32`/`u64` conversions go through the helpers in
    `src/utils.rs` or explicit `try_from(...).unwrap_or_default()`.
  - In-crate test modules carry a big `#[allow(clippy::...)]` block — copy
    the one at `src/world/chunk.rs:233-249` (the `#[cfg(test)] mod tests`
    there) for any new test module. This is how the crate keeps
    pedantic+nursery at deny in the lib while tests stay readable.
  - `Chunk::grid_position()` is `const` and public; `Chunk::insert` /
    `is_empty` are public.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Compile | `cargo check` | exit 0 |
| Tests | `cargo test` | all pass, incl. new tests |
| Lint (lib+bin) | `cargo clippy` | exit 0 — **NOT `--all-targets`** (see Scope) |
| Format | `cargo fmt --check` | exit 0 |
| Scene smoke | `WGPU_RT_WORLD=assets/models/bistro_sm.vox cargo run --quiet --bin bench -- 2` | loads with **no** "dropped" warning; chunk count > 5 |

## Scope

**In scope**:
- `src/world/loader.rs` — centering target in `center_world` (Step 1)
- `src/world/chunk.rs` — `CHUNKS_Y` constant (Step 2)
- `src/world/mod.rs` — chunk_y decode + drop diagnostics in `into_chunks`
  (Step 3) and its unit tests (Step 4)
- `src/world/loader.rs` — new `#[cfg(test)]` module (Step 4)
- `plans/README.md` — status row (Step 6)

**Out of scope** (do NOT touch, even though they look related):
- `tests/hierarchical_mip_dda.rs` and `tests/shader_validate.rs` — they
  already fail `cargo clippy --all-targets` (72 + 9 pre-existing errors,
  tracked as a separate open finding); **do not "fix" them here**. That is
  why the lint gate below is `cargo clippy` (lib+bin targets only).
- Any shader (`assets/shaders/*.wgsl`) — unchanged.
- Replacing the fixed grid with dynamic chunking — that is plan 015's
  Stage 1; this plan deliberately keeps the fixed-grid architecture.
- `src/app.rs`, `src/framework.rs`, `src/bin/bench.rs` — unchanged.

## Git workflow

- Branch: `advisor/020-world-clipping` (or continue on `master` if that's
  your workflow — the repo has no branch convention in recent history; match
  what the operator says).
- Commit style: conventional commits, e.g. `fix: center world on the chunk
  grid and surface dropped-voxel diagnostics` (match `git log --oneline`).
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Center scenes on the grid center

In `src/world/loader.rs`, replace the `(w_x, w_y, w_z)` computation (lines
32-35) so the anchor is the **grid** center, derived from the grid constants
(so it stays correct if the grid changes):

- Extend the import at line 8:
  `use crate::world::{VoxelWorldData, World, chunk::{CHUNK_TEXTURE_SIZE, CHUNKS_X_INT, CHUNKS_Y_INT, CHUNKS_Z_INT}};`
  (run `cargo fmt` — it will re-sort the use statement; that is fine).
- Compute each axis as `grid_chunks * CHUNK_TEXTURE_SIZE / 2`. Target shape
  (adapt to the exact types; `CHUNK_TEXTURE_SIZE.width` is `u32`,
  `CHUNKS_X_INT` is `i32`):
  ```rust
  let half_side = i32::try_from(CHUNK_TEXTURE_SIZE.width).unwrap_or_default();
  let (w_x, w_y, w_z) = (
      CHUNKS_X_INT.saturating_mul(half_side).div_euclid(2),
      CHUNKS_Y_INT.saturating_mul(half_side).div_euclid(2),
      CHUNKS_Z_INT.saturating_mul(half_side).div_euclid(2),
  );
  ```
  With the current constants this yields `(1024, 1024, 1024)`.

  Keep the rest of `center_world` untouched (the offset math, the bounds log,
  the `voxels.is_empty()` early return).

**Verify**: `cargo check` → exit 0, no warnings. `cargo fmt --check` → exit 0.

### Step 2: Raise CHUNKS_Y to 8

In `src/world/chunk.rs:25-29`, change the grid to a cube:

```rust
pub const CHUNKS_Y: u32 = 8;
pub const CHUNKS_Y_INT: i32 = 8;
```

Add a comment above the constants noting: vertical extent matches x/z so the
2048³ world fits tall scenes (bistro_sm is 2047 voxels tall); the binding
array can host 512 chunk textures on this adapter (limit 1048576), and
adapters with lower limits fail loudly at `App::init` (`src/app.rs:219-224`),
never silently; plan 015 retires the fixed grid entirely.

**Verify**: `cargo check` → exit 0. (The app currently renders nothing for
y>0 chunks because of the Step-3 decode bug — fix Step 3 before smoke
testing.)

### Step 3: Decode chunk_y and surface dropped voxels

In `src/world/mod.rs`:

1. Fix the grid-position decode (lines 57-59): decode all three axes:
   ```rust
   let chunk_x = i.rem_euclid(CHUNKS_X_INT);
   let chunk_y = i.div_euclid(CHUNKS_X_INT).rem_euclid(CHUNKS_Y_INT);
   let chunk_z = i.div_euclid(CHUNKS_X_INT * CHUNKS_Y_INT);
   Chunk::new(glam::IVec3::new(chunk_x, chunk_y, chunk_z))
   ```
   (This must match the voxel-side index math at `mod.rs:91-95`:
   `index = z*CHUNKS_Y*CHUNKS_X + y*CHUNKS_X + x`, i.e. `((z*CHUNKS_Y) +
   y)*CHUNKS_X + x` — the decode above is its exact inverse for
   `i ∈ 0..TOTAL_CHUNKS_INT`.)

2. Restructure `into_chunks` so drops are counted and reported. Rename the
   body to a new method and delegate, keeping the public API:
   ```rust
   pub fn into_chunks(self) -> Vec<Chunk> {
       self.split_chunks().0
   }

   /// Splits the voxel map into grid chunks. Returns (chunks, dropped count).
   /// Logs a warning when voxels fall outside the grid (world-space clip).
   pub fn split_chunks(self) -> (Vec<Chunk>, u64) {
       // ... current body, with the additions below ...
   }
   ```
   Inside the voxel loop:
   - Capture `let total_voxels = u64::try_from(self.voxels.len()).unwrap_or_default();`
     **before** the consuming `for ... in self.voxels` loop starts (the loop
     moves `self.voxels`; reading `len()` after it is a borrow error).
   - Before the bounds check, compute the world coords as today (`wx`, `wy`,
     `wz`).
   - Track `let mut dropped: u64 = 0;` plus min/max per axis as `i32`
     (seed with the first voxel's coords via `bool` flag or `Option<i32>`,
     then `min`/`max`) — use `saturating_add`/`min`/`max` only (no plain
     `+` on counters that could overflow).
   - In the bounds-check branch, replace the bare `continue;` with:
     ```rust
     dropped = dropped.saturating_add(1);
     continue;
     ```
   - After the loop, if `dropped > 0`, log once with the names the log line
     uses (`x_min`, `x_max`, ..., `z_min`, `z_max` — reuse your tracking
     variables):
     ```rust
     log::warn!(
         "Chunk split: dropped {dropped}/{total_voxels} voxels outside the grid \
          (world x [{x_min},{x_max}] y [{y_min},{y_max}] z [{z_min},{z_max}]; \
          grid 0..{grid_side} per axis)",
     );
     ```
     where `grid_side` = `CHUNKS_X_INT * CHUNK_TEXTURE_SIZE.width` (there is
     **no** `CHUNK_TEXTURE_SIZE_side` constant in the repo — the side is
     `CHUNK_TEXTURE_SIZE.width`, which equals `height` and
     `depth_or_array_layers` = 256). No log when nothing is dropped.
   - Return `(chunks, dropped)`.

   Keep `World::into_chunks` callers (`src/app.rs:203`) working unchanged.

**Verify**: `cargo check` → exit 0. `cargo test` → all existing tests still
pass.

### Step 4: Tests

Add a `#[cfg(test)] mod tests` to `src/world/mod.rs` and one to
`src/world/loader.rs`, both with the crate's standard test allow block
(copy from `src/world/chunk.rs:195-207` — the `#[cfg(test)]`/`#[allow(
clippy::arithmetic_side_effects, ... )]`/`mod tests {` + `use super::*;`
header, **not** the test bodies that follow). Tests:

In `src/world/mod.rs` (use `offset = [0, 0, 0]` in every test that builds a
`World`):
1. `voxels_in_different_y_chunks_land_in_correct_grid_positions` — build a
   `World` with `offset = [0,0,0]`, insert voxels at `(0,0,0)`, `(0,300,0)`,
   `(0,511,0)`; call `split_chunks()`; assert `dropped == 0` and that the
   chunk holding each voxel reports grid position `(0,0,0)`, `(0,1,0)`,
   `(0,1,0)` respectively (world 300 and 511 both live in chunk y=1). This
   guards the chunk_y decode: before this plan the decode did not exist at
   all (and with CHUNKS_Y=1 the `(0,300,0)` voxel was silently dropped), so
   the test cannot pass against the old code.
2. `voxels_outside_grid_are_dropped_and_counted` — `offset = [0,0,0]`,
   voxels at `(10000,0,0)` and `(0,-5,0)`; assert `dropped == 2` and
   `chunks` contains only the in-grid voxels.
3. `index_decode_roundtrips_for_all_axes` — `offset = [0,0,0]`, voxels at
   `(256, 300, 512)`, `(511, 2047, 2047)`, `(0, 0, 0)`: assert the chunk's
   grid position equals `(wx.div_euclid(256), wy.div_euclid(256),
   wz.div_euclid(256))` — i.e. `(1,1,2)`, `(1,7,7)`, `(0,0,0)`.

In `src/world/loader.rs`:
4. `wide_tall_scene_survives_center_and_split` — the COR-01 regression:
   build a synthetic `VoxelWorldData` spanning ±800 in x, y∈[-600, 600],
   ±300 in z (e.g. a few thousand voxels spread across that range, plus a
   dense block), call **`SceneGraphLoader::center_world(voxels)`** (NOT
   `Self::center_world` — `center_world` is a private associated fn of
   `SceneGraphLoader` at `loader.rs:31`, and inside `mod tests` `Self` is
   the test module, which won't compile; `use super::*;` makes
   `SceneGraphLoader` reachable), then
   `World { voxels, palette: [[0;4];256], offset }.split_chunks()`; assert
   `dropped == 0`. This fails on the old (128,128,128) centering — world
   coords go negative — and passes with the grid-centered anchor.

(Constructing a `World` requires `palette: [[u8; 4]; 256]` — `[[0u8; 4];
256]` or copy `crate::world::create_palette_buffer`'s input shape; a
zeroed palette is fine for tests since only `offset`/`voxels` matter.)

**Verify**: `cargo test --quiet` → expect `18 passed` printed **twice**
(once for the `main` target, once for the `bench` target — this crate is
bin-only with no `lib.rs`, so in-crate unit tests run once per bin), plus
`12 passed` (hierarchical_mip_dda) and `4 passed` (shader_validate). The
pre-change baseline is `14 passed` ×2 + `12` + `4` = 44 total; post-change
is 18 ×2 + 12 + 4 = 52 total. Do NOT look for a literal "34" anywhere in
cargo's output.

### Step 5: Smoke-test real scenes

Run the headless bench on the three scenes that were clipped before:

```
WGPU_RT_WORLD=assets/models/bistro_sm.vox cargo run --quiet --bin bench -- 2
WGPU_RT_WORLD=assets/models/Church_Of_St_Sophia.vox cargo run --quiet --bin bench -- 2
WGPU_RT_WORLD=assets/models/monu1.vox cargo run --quiet --bin bench -- 2
```

**Verify** for each:
- No `Chunk split: dropped ...` warning in the log (grep for `dropped`).
- bistro_sm: `Non-empty chunks:` significantly higher than 5 (expect
  roughly 30–150; record the actual value). church: higher than 5. monu1:
  still 1.
- The `offset` log line: expect ≈ `(1024, 1024, 1024)` for bistro_sm/church
  (old code produced `(129, 129, 129)`). Derivation: the loader computes
  `offset = anchor − scene_center` then rounds (`loader.rs:38-78`), and
  bistro/church raw voxel bounds are symmetric around ~(−1,−1,−1), so
  `1024 − (−1) ≈ 1025 → 1024` after truncation. Accept a deviation of ±2 on
  any axis; beyond that, STOP and report rather than eyeballing it.

If bistro_sm/church fail with a device lost / out-of-memory error on this
machine, STOP and report (do not lower CHUNKS_Y — that reintroduces silent
clipping; the VRAM cost is ≈19 MB decimal (≈18.3 MiB) per non-empty chunk —
256³ R8Uint + 9 mip levels — and plan 015's pool is the structural fix).

**Verify**: `cargo clippy` → exit 0 (lib+bin targets only). `cargo fmt
--check` → exit 0.

### Step 6: Update the index

Update the row for plan 020 in `plans/README.md` (status → DONE with a
one-line summary incl. the measured bistro_sm chunk count) and add a
reconcile note recording: grid is now 8×8×8, centering is grid-centered,
drops are logged; plan 015's Current-state excerpts for these files are
stale until plan 021 refreshes them.

## Test plan

Covered in Step 4: 4 new tests across `src/world/mod.rs` and
`src/world/loader.rs`, modeled structurally on the test module in
`src/world/chunk.rs`. They exercise: multi-axis chunk decode (regression for
the chunk_y bug), drop counting, and the centering fix (regression for
COR-01). No integration-test files are touched (out of scope).

## Done criteria

ALL must hold:

- [ ] `cargo check` exits 0
- [ ] `cargo test --quiet` exits 0, printing `18 passed` twice (main + bench
      targets), `12 passed`, `4 passed` — total 52; baseline was 44
- [ ] `cargo clippy` (lib+bin only) exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] bench on bistro_sm: no "dropped" warning; `Non-empty chunks` > 5;
      offset ≈ (1024, 1024, 1024) ±2
- [ ] `grep -n "Chunk::new(glam::IVec3::new(chunk_x, 0, chunk_z))"
      src/world/mod.rs` → no match (chunk_y decode fixed)
- [ ] `CHUNKS_Y` / `CHUNKS_Y_INT` are 8 in `src/world/chunk.rs`
- [ ] `git status --short -- src/` shows changes only under `src/world/`
      (pre-existing `plans/` modifications are expected, not a violation)
- [ ] `plans/README.md` row 020 updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" no longer match the live files.
- A verification fails twice after a reasonable fix attempt.
- A bench run on bistro_sm/church fails with a device-lost/out-of-memory
  error (record it — do not reduce CHUNKS_Y).
- The fix appears to require touching an out-of-scope file (especially the
  integration tests or shaders).

## Maintenance notes

- **Plan 015 (8³-chunk rewrite) supersedes this stopgap**: its Stage 1
  deletes the fixed grid, `CHUNKS_X/Y/Z`, and rewrites `into_chunks` into
  dynamic occupied-only chunking. Do not deepen the fixed-grid code; keep
  this change minimal. Plan 021 refreshes 015's drift check / Current-state
  excerpts to the post-020 world — if you land 020 and 021 is not yet done,
  note that in the index.
- **`center_world` is load-bearing for the .world-format story**: plan 015
  changes it again ("re-anchor on the voxel-bounds midpoint"); the tests
  added here (especially `wide_tall_scene_survives_center_and_split`) must
  be ported to whatever the new chunking does, or they will block 015's
  lint gate.
- The `split_chunks` public method is deliberately additive (`into_chunks`
  delegates) so `src/app.rs` keeps compiling; a reviewer should confirm no
  dead-code warnings were introduced.
- Future scenes wider than 2048 voxels in any axis will still clip (with a
  warning now). The structural fix for unbounded scenes is plan 015.

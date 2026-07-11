# Plan 006: Build Tree64 GPU data from occupied voxels

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat 178741a..HEAD -- src/bin/bake.rs src/world/loader.rs src/lib.rs src/tree64_renderer.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition. The working tree already contains
> unrelated changes to `Cargo.toml`, `Cargo.lock`, `src/app.rs`, and an
> untracked dense-texture plan; do not include those changes in this work.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `178741a`, 2026-07-11

## Why this matters

Baking `assets/models/bistro.vox` currently expands a tight AABB into a
power-of-four cube. This scene crosses the `4096 -> 16384` threshold, so the
bake path presents Tree64 with a `16384³` logical model. `tree64::Tree64::new`
only receives a point-wise `VoxelModel` interface and recursively queries the
entire logical volume, making construction effectively proportional to
trillions of possible voxels instead of the occupied geometry. Before that
scan, `SparseBlocks` also allocates a dense block-pointer vector with
`1024³` entries (approximately 8 GiB on a 64-bit target).

Replace the bake-only construction path with an occupancy-driven builder that
consumes the already transformed, overlap-collapsed occupied voxels and emits
the existing `GpuTree64` node/data layout directly. This avoids both the
volume-sized Tree64 scan and the dense `SparseBlocks` allocation while keeping
the current shader, `.world` format, root coordinate convention, and runtime
Tree64 code unchanged.

## Current state

Relevant files:

- `src/bin/bake.rs` — parses the `.vox` file, extracts its palette, calls
  `SceneGraphLoader::load`, and writes the returned `WorldFile`.
- `src/world/loader.rs` — traverses the `.vox` scene graph, transforms all
  model voxels into a signed world-coordinate `HashMap`, computes a tight AABB,
  creates `SparseBlocks`, and calls `tree64::Tree64::new`.
- `src/tree64_renderer.rs` — defines the GPU-compatible `GpuNode` and
  `GpuTree64`, converts a dependency `Tree64<u8>` into GPU vectors, and
  serializes the GPU vectors. Do not change this module for this plan unless
  the live code proves a constructor/helper is unavoidable.
- `src/formats/mod.rs` — current `.world` v3 format stores a palette and one
  optional GPU tree blob. Its format should remain unchanged.
- `src/lib.rs` — exports the modules used by the bake binary and runtime.
- `C:/Users/Thiba/.cargo/git/checkouts/tree64-ddae6752ed26e2c6/aad709c/src/lib.rs`
  — read-only reference for the pinned Tree64 representation and constructor.

The current bake-specific structures are in `src/world/loader.rs:11-136`:

```rust
const BLOCK_SIZE: u32 = 16;
const BLOCK_VOXELS: usize = (BLOCK_SIZE * BLOCK_SIZE * BLOCK_SIZE) as usize;
const BLOCK_BITS: usize = BLOCK_VOXELS / 64;

struct BlockData {
    colors: [u8; BLOCK_VOXELS],
    occupied: [u64; BLOCK_BITS],
}

struct SparseBlocks {
    blocks: Vec<Option<Box<BlockData>>>,
    blocks_per_axis: u32,
    dims: [u32; 3],
}
```

`SparseBlocks::from_world_voxels` computes a dense block count and resizes the
pointer vector before inserting any voxel (`src/world/loader.rs:41-47`):

```rust
let blocks_per_axis = tree_dim / BLOCK_SIZE;
let total_blocks = (blocks_per_axis * blocks_per_axis * blocks_per_axis) as usize;
let mut blocks: Vec<Option<Box<BlockData>>> = Vec::with_capacity(total_blocks);
blocks.resize_with(total_blocks, || None);
```

The current loader borrows the parsed data and builds the old representation
(`src/world/loader.rs:147-156`):

```rust
pub fn load(vox_data: &DotVoxData, palette: [[u8; 4]; 256]) -> WorldFile {
    let instances = Self::collect_instances(vox_data);
    let all_voxels = Self::collect_all_voxels(&instances);
    let world = Self::build_world_file(all_voxels, palette);
    log::info!("Total bake time: {:.2}s", t_total.elapsed().as_secs_f32());
    world
}
```

The transformed voxel map is already unique by signed world coordinate because
`collect_all_voxels` inserts into `HashMap<(i32, i32, i32), u8>`
(`src/world/loader.rs:354-389`). The current AABB and power-of-four sizing
logic is `src/world/loader.rs:394-443`:

```rust
let aabb_min = bb_min;
let aabb_size = (bb_max - bb_min + IVec3::ONE).as_uvec3();
let max_dim = aabb_size.x.max(aabb_size.y).max(aabb_size.z);
let tree_dim = Self::round_up_pow4(max_dim);
```

The current tree construction and conversion are `src/world/loader.rs:445-472`:

```rust
let blocks = SparseBlocks::from_world_voxels(&voxels, aabb_min, tree_dim);
drop(voxels);

let tree = tree64::Tree64::new(&blocks);
let mut gpu_tree = GpuTree64::from_tree64(&tree);
gpu_tree.root_offset = [aabb_min.x, aabb_min.y, aabb_min.z];
world_file.tree = Some(gpu_tree);
```

The pinned Tree64 constructor derives a cubic power-of-four scale from
`VoxelModel::dimensions()` and calls `insert_from_model_recursive`, which loops
over all 64 child positions at every recursive level and calls
`model.access()` at every 4³ leaf cell. This is the scan to remove; do not call
`Tree64::new`, `GpuTree64::from_model`, or `Tree64::modify` from the bake path.

The GPU representation must remain compatible with the existing shader:

- `GpuNode` is three `u32`s: packed leaf/pointer plus a 64-bit occupancy mask.
- Cell ordering is `x + 4*y + 16*z`, matching `tree64` and
  `assets/shaders/tree64_compiled.wgsl:GetCellIndex_0`.
- An internal node's pointer addresses a compact contiguous range of child
  descriptors in `GpuTree64.nodes`; the child rank is the number of set mask
  bits below the requested cell.
- A leaf node's pointer addresses compact `u8` values in `leaf_data`; the
  shader treats this pointer as a byte index and divides by four when reading
  its `array<u32>` storage binding.
- Material/palette index `0` is a valid occupied voxel. Occupancy must never be
  inferred from `value != 0`.
- The root covers `tree_dim == 4^num_levels` local voxels. For Bistro's case,
  `tree_dim == 16384`, `num_levels == 7`, and `tree_scale == 14`.
- `root_offset` maps local coordinate zero back to the signed world-space AABB
  minimum. The current loader sets it to `[aabb_min.x, aabb_min.y, aabb_min.z]`.
- The root node may be at any index; the shader reads `rootNodeIndex` from the
  tree parameters.

Repository conventions to follow:

- Rust 2024 with explicit imports and inferred return types where practical.
- Use `log::info!`, `log::warn!`, and `log::error!` for bake diagnostics.
- Use `Result` for the new builder's validation errors; the existing bake
  loader may convert an impossible input error to a descriptive `expect`,
  consistent with the current CLI behavior.
- Match the existing `#[cfg(test)] mod tests` style in `src/formats/mod.rs`.
- Do not run `cargo build` or `cargo run` as part of this plan; project
  instructions prefer check/test/lint commands, and the large Bistro asset
  must not be opened during development.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Check library and bake binary | `cargo check --all-targets` | exit 0, no errors |
| Format check | `cargo fmt --check` | exit 0, no diff |
| Lint | `cargo clippy --all-targets -- -D warnings` | exit 0, no warnings |
| Tests | `cargo test` | all tests pass |
| Focused builder tests | `cargo test tree64_builder` | all builder tests pass |
| Inspect scope | `git diff --stat` | only files listed in Scope plus this plan/index are changed |

## Scope

**In scope** (the only source files to create or modify):

- `src/tree64_builder.rs` — new occupancy-driven Tree64-compatible builder and
  unit tests.
- `src/lib.rs` — register the new crate-private builder module.
- `src/world/loader.rs` — remove the bake-only `SparseBlocks`/`VoxelModel`
  adapter, convert the occupied map to local coordinates, invoke the builder,
  and optionally make the loader own/drop parsed `.vox` data earlier.
- `src/bin/bake.rs` — pass `DotVoxData` by value if the loader ownership change
  is made.
- `plans/006-occupancy-driven-tree-bake.md` — this plan.
- `plans/README.md` — add/update the plan status row and dependency notes.

**Out of scope** (do not touch):

- `src/tree64_renderer.rs` — preserve the public GPU structs, serialization,
  shader-facing layout, and existing runtime helper methods.
- `src/formats/mod.rs` — do not change `.world` v3 or `WORLD_VERSION`.
- `assets/shaders/` — no shader changes are needed.
- `Cargo.toml` or `Cargo.lock` — keep the existing Tree64 dependency; runtime
  code still compiles the existing renderer module.
- `src/app.rs`, `src/world/mod.rs`, and other runtime modules.
- `assets/models/bistro.vox` — do not read, parse, benchmark, or inspect it.
- Dense 3D texture rendering from `plans/005-dense-3d-texture-renderer.md`.
  That approach is not suitable for a required `16384³` logical scene and is
  rejected for this optimization.

## Design and interface

### Builder interface

Add a crate-private function in `src/tree64_builder.rs` with a deliberately
small interface:

```rust
pub(crate) fn build_gpu_tree<I>(
    voxels: I,
    tree_dim: u32,
    root_offset: [i32; 3],
) -> Result<GpuTree64, TreeBuildError>
where
    I: IntoIterator<Item = ([u32; 3], u8)>;
```

The builder owns all hierarchical assembly details. It must not accept
`SparseBlocks`, `HashMap`, `DotVoxData`, or a `VoxelModel`; those would couple
this seam to one storage strategy and make a later sparse/streaming adapter
needlessly invasive. The iterator contract is:

- Coordinates are root-local and must satisfy `0 <= coordinate < tree_dim` on
  all axes.
- `tree_dim` is at least 4 and is an exact power of four.
- Each coordinate occurs at most once. Duplicate coordinates are an error,
  even though the current loader's HashMap normally guarantees uniqueness.
- The `u8` value is opaque material data; zero is valid.
- Empty input returns a controlled `TreeBuildError::EmptyInput`. The loader
  handles empty worlds before invoking the builder, preserving the current
  `WorldFile::new()` behavior and palette.

Define a small error type covering at least invalid dimensions, out-of-bounds
coordinates, duplicate coordinates, empty input, and packed pointer/count
overflow. Error messages should identify the violated invariant without dumping
large input data.

### Occupancy-driven construction algorithm

1. Validate `tree_dim` using integer operations. Compute
   `num_levels = tree_dim.ilog2() / 2` only after confirming the dimension is a
   power of two with an even log2. Reject dimensions whose path key would not
   fit in the chosen key type. A `u128` path key is sufficient for all
   representable `u32` power-of-four dimensions (`6 * num_levels` bits).
2. Consume the iterator once into compact records containing `[u32; 3]`, the
   `u8` value, and a root-to-leaf path key. For each level, from the most
   significant coordinate pair to the least significant pair, compute:

   ```text
   x_digit = (x >> shift) & 3
   y_digit = (y >> shift) & 3
   z_digit = (z >> shift) & 3
   child_index = x_digit | (y_digit << 2) | (z_digit << 4)
   path_key = (path_key << 6) | child_index
   ```

   The root uses `shift = 2 * (num_levels - 1)` and the leaf uses `shift = 0`.
3. Sort records by path key, with coordinate as a deterministic tie-breaker.
   After sorting, equal coordinates/path keys must be rejected as duplicates.
   Do not create a dense array indexed by `tree_dim`, block count, or voxel
   count.
4. Recursively assemble only occupied paths using a sorted record range and a
   current depth. At an internal level, scan the range once, identify the
   contiguous groups for present child indices, recursively build those groups
   in ascending cell-index order, set the corresponding bits in the parent
   mask, then append the returned child descriptors contiguously to
   `GpuTree64.nodes`. Return a parent `GpuNode` whose non-leaf pointer is the
   start of that appended descriptor range.
5. At the final 4³ level, scan records in ascending final cell index, set the
   leaf mask, append only occupied values to `leaf_data`, and return a leaf
   `GpuNode` whose pointer is the byte offset of the appended values. Preserve
   zero-valued materials because the record's presence, not its value, sets the
   mask.
6. Append the final root descriptor to `nodes` and use its index as
   `root_node_index`. Set `tree_scale = num_levels * 2` and copy the supplied
   `root_offset` into the returned `GpuTree64`.
7. Before every pointer is packed, check that the node/data index fits the
   existing packed pointer payload and that counts fit the `.world` serializer's
   `u32` fields. Do not silently truncate. `GpuNode::new` itself does not
   validate these limits.

The returned node layout need not be byte-identical to the old constructor,
but a shader-equivalent lookup must return the same occupancy and material for
every coordinate. The implementation must not call any operation whose work is
proportional to `tree_dim³` or `(tree_dim / 16)³`.

### Why not extend Tree64 or call `modify`

The dependency does not expose its internal edit-history initialization, so a
caller cannot safely construct a `tree64::Tree64` by filling its public vectors.
Calling `Tree64::modify` once per voxel would use path-copying edits and create
unnecessary historical nodes/data. A local builder that emits the already
consumed `GpuTree64` format is the narrowest implementation for this bake-only
need. Revisit an upstream Tree64 sparse-construction API only if the project
later needs a mutable CPU tree from baked data.

## Steps

### Step 1: Add the standalone occupancy-driven builder

Create `src/tree64_builder.rs` and implement the error type, record creation,
path-key sorting, recursive grouping, leaf packing, pointer validation, and
`GpuTree64` construction described above. Keep all helper functions private
except the crate-private `build_gpu_tree` seam. Use `GpuNode::new` for node
creation so the packed layout remains centralized.

Do not import or reference `SparseBlocks`, `DotVoxData`, or the Tree64 model
constructor. The module may use `crate::tree64_renderer::{GpuNode, GpuTree64}`.

**Verify**: `cargo check --all-targets` is allowed to fail only because the
new module is not registered yet; after Step 2 it must pass. Do not run a
volume-sized test or open any asset.

### Step 2: Register the module and add builder tests

In `src/lib.rs`, add:

```rust
pub(crate) mod tree64_builder;
```

Add unit tests inside `src/tree64_builder.rs`. Write a test-only lookup helper
that follows the shader's mask/rank rules: start at `root_node_index`, derive
cell indices from coordinate bits, test the mask, rank by popcount below the
cell, follow internal child descriptors, and read leaf data by the leaf pointer
plus rank. Return `Option<u8>` based on occupancy, not material value.

Required tests:

- Empty input returns `TreeBuildError::EmptyInput`.
- A single voxel at `[0, 0, 0]` is found.
- A single voxel at `[tree_dim - 1; 3]` is found.
- Voxels on both sides of level/block boundaries, including `[3,3,3]` and
  `[4,4,4]`, resolve independently.
- A zero material value is still occupied and is returned as `Some(0)`.
- A non-cubic occupied region packed into a larger cubic root leaves padded
  coordinates empty.
- A signed root offset is preserved in the output metadata.
- Duplicate coordinates, invalid dimensions, and out-of-bounds coordinates
  return errors.
- A synthetic `tree_dim == 16384` input containing only two opposite-corner
  voxels completes without a volume loop, has `tree_scale == 14`, returns both
  values through the lookup helper, and has a small node/data footprint (use a
  generous upper bound such as fewer than 100 nodes and fewer than 10 bytes of
  leaf data rather than timing-based assertions).
- For small dimensions (`4`, `16`, and optionally `64`), compare lookup results
  against expected sparse input and, for nonzero materials, against a reference
  `tree64::Tree64::new` model. Do not compare implementation-specific node
  indices or require exact byte identity.

**Verify**: `cargo test tree64_builder` passes, and `cargo clippy --all-targets
-- -D warnings` passes. The synthetic 16384 test must complete promptly and
must not allocate an array derived from tree volume.

### Step 3: Integrate the builder into the bake loader

In `src/world/loader.rs`:

1. Remove the bake-only `BLOCK_SIZE`, `BLOCK_VOXELS`, `BLOCK_BITS`,
   `BlockData`, `SparseBlocks`, its constructor, and its `VoxelModel` adapter.
   Keep `HashMap`, Rayon, scene traversal, transformation, AABB, and
   `round_up_pow4`; those remain needed for collecting and sizing the scene.
2. Import `crate::tree64_builder::build_gpu_tree` instead of
   `crate::tree64_renderer::GpuTree64`.
3. Preserve the current AABB and `tree_dim` calculation exactly for this first
   optimization. Compute local coordinates while consuming the map:

   ```rust
   let local_voxels = voxels.into_iter().map(|((x, y, z), value)| {
       let local = [
           u32::try_from(x - aabb_min.x).expect("x coordinate outside AABB"),
           u32::try_from(y - aabb_min.y).expect("y coordinate outside AABB"),
           u32::try_from(z - aabb_min.z).expect("z coordinate outside AABB"),
       ];
       (local, value)
   });
   ```

   Prefer checked arithmetic if the live code has changed the AABB types; do
   not silently wrap signed coordinates into large unsigned values.
4. Replace the `SparseBlocks`/`Tree64::new`/`GpuTree64::from_tree64` sequence with
   one call to `build_gpu_tree(local_voxels, tree_dim, aabb_min.to_array())`.
   Convert the validated builder error to the existing bake failure style with
   a message that includes the error. Store the returned tree in
   `world_file.tree` and retain the existing tree/root diagnostics.
5. Do not add any dense block index or dense voxel volume. The map is consumed
   directly into the builder's compact records.

**Verify**: `cargo check --all-targets` passes. `rg "SparseBlocks|Tree64::new\(&blocks\)|from_tree64" src/world/loader.rs`
returns no matches. The only remaining `Tree64::new` in active source may be the
runtime/general-purpose `GpuTree64::from_model` helper in
`src/tree64_renderer.rs`, which is intentionally out of scope.

### Step 4: Release parsed `.vox` data before tree assembly

Make `SceneGraphLoader::load` take ownership of `DotVoxData` rather than a
borrow, and update the single call in `src/bin/bake.rs` accordingly. The loader
must:

1. collect borrowed instances;
2. collect the transformed voxel map;
3. explicitly drop `instances` so its model borrows end;
4. explicitly drop `vox_data` before converting/sorting the map into tree
   records;
5. build the `WorldFile` from the map and palette.

This is a peak-memory reduction and does not change scene transform or overlap
semantics. Do not change palette extraction, scene graph behavior, or the
world-file format.

**Verify**: `cargo check --all-targets` passes, and `cargo test` passes. The
loader signature has one caller (`src/bin/bake.rs`); `rg
"SceneGraphLoader::load" src` should show only the updated call.

### Step 5: Add serialization-level regression coverage

Extend the existing tests in `src/formats/mod.rs` only if needed to exercise a
builder-produced `GpuTree64`. Keep the v3 format and existing round-trip
semantics unchanged. The test should verify that a small builder result placed
in `WorldFile.tree` preserves:

- root node index;
- tree scale;
- signed root offset;
- node records;
- leaf data;
- lookup results after `WorldFile::write` and `WorldFile::read`.

If importing the crate-private builder from the formats test would create an
awkward module dependency, keep the builder lookup tests as the primary
coverage and use the existing dummy-tree round trip; do not broaden public
interfaces just for this test.

**Verify**: `cargo test` passes with all existing format tests and all builder
tests.

### Step 6: Run final checks and inspect the diff

Run the full verification commands:

```text
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test
```

Then inspect `git diff --stat` and `git diff -- src/tree64_builder.rs
src/world/loader.rs src/bin/bake.rs src/lib.rs`. Confirm no source file outside
Scope changed and no large asset was read. Do not bake Bistro as part of this
plan. A later operator may run the existing bake command manually after
reviewing the implementation.

**Verify**: every command exits 0; the diff contains no `SparseBlocks` in the
bake path, no new volume-sized allocation, no shader/format changes, and the
plan status in `plans/README.md` is updated by the executor when complete.

## Test plan

The core test surface is the builder's shader-equivalent lookup helper, not
private node-vector shape. Tests must cover:

1. Validating dimensions and coordinate bounds.
2. Empty input and duplicate input.
3. Zero-valued material occupancy.
4. Cell ordering across x/y/z and across 4³ hierarchy boundaries.
5. Non-cubic AABB padding and signed root metadata.
6. Sparse opposite-corner voxels in a synthetic `16384³` root.
7. Differential lookup against a small reference Tree64 model for nonzero
   material values.
8. World-file round-trip of builder output, without changing `WORLD_VERSION`.

Use deterministic vectors and exact lookup assertions. Avoid wall-clock
performance assertions because they are flaky; the 16384 regression test's
small output and absence of a dense allocation are the machine-checkable
regression signal.

## Done criteria

All must hold:

- [ ] `cargo fmt --check` exits 0.
- [ ] `cargo check --all-targets` exits 0.
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0.
- [ ] `cargo test` exits 0, including the builder regression tests.
- [ ] `src/tree64_builder.rs` exists and exposes only the crate-private builder
      seam plus its error type as needed.
- [ ] The builder has no loop or allocation proportional to `tree_dim³` or
      `(tree_dim / 16)³`.
- [ ] `src/world/loader.rs` no longer defines or uses `SparseBlocks`.
- [ ] The bake path no longer calls `tree64::Tree64::new` or
      `GpuTree64::from_tree64`.
- [ ] The synthetic `16384³` two-voxel test returns both values and reports
      `tree_scale == 14`.
- [ ] Existing `.world` format tests pass without a version/layout change.
- [ ] `git diff --stat` shows only files in Scope plus the plan/index.
- [ ] No Bistro or other large asset was read during implementation.
- [ ] The executor updates the 006 row in `plans/README.md` to DONE, or to
      BLOCKED with the exact reason.

## STOP conditions

Stop and report instead of improvising if:

- Any in-scope source file differs from the Current state during the drift
  check and the difference affects the planned symbols or interfaces.
- Correct GPU compatibility requires changing `GpuNode`, `GpuTree64`, the
  shader, or the `.world` format. This plan assumes the existing layout is
  sufficient; such a change needs a separate design decision.
- The pinned Tree64 representation or shader uses a different child ordering,
  pointer unit, or rank rule than documented here.
- The builder cannot preserve a zero-valued occupied material without changing
  the shader's hit semantics.
- A correct implementation appears to require `Tree64::modify` per voxel,
  a dense voxel/block array, or a full-volume `VoxelModel` scan.
- The loader's ownership change cannot drop `DotVoxData` before tree-record
  construction without changing scene transform behavior.
- Any pointer/count would overflow the existing packed `u32` or serializer
  fields for supported inputs; report the limit and do not silently truncate.
- `cargo test tree64_builder` fails after two reasonable corrections, or a
  verification command fails twice for an error unrelated to this plan.
- The executor needs to modify a file listed as out of scope.

## Maintenance notes

- The builder is intentionally a GPU-output adapter, not a replacement for the
  mutable CPU Tree64 type. If runtime editing or CPU-side queries are later
  required, design an upstream Tree64 sparse-construction API instead of
  reconstructing a CPU tree from GPU vectors.
- The direct map-to-builder path makes the old phase-2 dense block index
  unnecessary for baking. If the transformed `HashMap` is still too large for
  Bistro, address that separately with streaming or a sparse coordinate store;
  do not reintroduce `SparseBlocks` into this builder seam.
- If the root sizing policy changes, update both `round_up_pow4` and the
  builder's power-of-four validation together. The shader expects the root
  metadata and hierarchy depth to agree.
- Reviewers should scrutinize the distinction between internal node pointers
  (node-descriptor indices) and leaf pointers (byte indices into `leaf_data`),
  plus the `x + 4*y + 16*z` ordering and compact-rank calculation.
- Keep the synthetic 16384 regression test permanently. It protects against a
  future refactor accidentally routing the bake back through `VoxelModel::access`.

# Plan 003: Chunked world architecture — binary format, bake tool, and multi-chunk rendering

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat fe59d7e..HEAD -- src/ assets/ Cargo.toml`
> If any of these files changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `fe59d7e`, 2026-07-06

## Why this matters

The engine currently builds a single Tree64 from procedural Perlin noise on every
startup. For hand-built voxel scenes (authored in MagicaVoxel), this is wasteful:
building the acceleration structure from a model takes seconds at world scale.
We need a chunked architecture where a MagicaVoxel `.vox` scene is baked
offline into a compact binary file (`.world`) that maps directly to GPU buffers,
so the engine loads and renders in milliseconds. Chunking also enables editing
individual regions without rebuilding the entire world.

## What we're building

A 512m × 256m × 512m world at ⅛m voxel resolution (4,096 × 2,048 × 4,096 voxel grid)
partitioned into a **16×16 grid of chunks** (256 chunks), each 256×2,048×256 voxels.
No vertical chunk splitting — each chunk covers the full world height. Chunks that
are entirely empty (no occupied voxels) are not stored.

The three components:

1. **Binary world format** (`.world`): single file with header, chunk table of
   contents, and per-chunk GPU-ready binary blobs.
2. **Bake tool** (`src/bin/bake.rs`): CLI that reads a `.vox` file, partitions it
   into chunks, builds a `Tree64<u8>` per chunk, serializes the `GpuTree64`, and
   writes a `.world` file.
3. **Engine integration**: load `.world` at startup, create GPU buffers per chunk,
   dispatch all chunks each frame (no frustum culling in MVP).

## Current state

Key files and their roles:

- `Cargo.toml` — no workspace; single binary crate with `dot_vox` 5.2 already
  in dependencies
- `src/tree64_renderer.rs` — `GpuTree64`, `GpuNode` (3×u32, Pod+Zeroable),
  `GpuTree64Buffers`, `Tree64Params` (32B uniform). `GpuTree64::from_model()`
  builds a `Tree64<u8>` then converts to GPU-native format (lines 34–62).
  `GpuTree64::create_buffers()` uploads to GPU (lines 64–92).
- `src/world/mod.rs` — contains `TerrainModel` (procedural Perlin heightmap,
  lines 5–77) and `build_tree64()` (lines 79–94). These will be replaced.
- `src/app.rs` — `App::init()` calls `world::build_tree64()` (line 112),
  creates **one** tree bind group (lines 156–238), dispatches **one** compute
  pass per frame (lines 439–452). Holds tree buffers as fields (lines 52–54).
- `src/main.rs` — declares `mod app; mod framework; mod world; ...`,
  entry point is `framework::run()`.
- `build.rs` — Slang-to-WGSL shader compilation.

Repo conventions (match these):

- New modules declared in `src/main.rs`
- Struct constructors: `pub fn new(...)` or `pub fn from_*(...)`
- GPU data: `#[repr(C)]`, derive `Pod + Zeroable`, upload via `bytemuck::cast_slice`
- Logging: `log::info!()` / `log::warn!()` / `log::error!()`
- Error handling: `Result<_, String>` or `io::Result<_>`
- Imports: explicit at top of file, no wildcards except for enums

Tree64 crate facts (relevant for serialization):

- `tree64::Node` is `#[repr(C, packed)]` with `is_leaf_and_ptr: u32` + `pop_mask: u64`
  (12 bytes total). `GpuNode` re-encodes the same data as three u32s
  (`packed_data`, `pop_mask_lo`, `pop_mask_hi`).
- `Tree64<T>` has `serialize<W: Write>(&self, writer: W)` and
  `deserialize<R: Read>(reader: R)` that write num_levels, root_node_index,
  root_offset, node count, data count, then raw nodes + raw data.
- `VoxelModel<u8>` trait: `dimensions() -> [u32; 3]` and
  `access(coord: [usize; 3]) -> Option<u8>`.
- `FlatArray` is a built-in VoxelModel wrapper for slices.

## Binary world format specification

```
World file (.world):

HEADER (64 bytes, little-endian):
  offset  size  field
  0       4     magic: [u8; 4] = b"WRLD"
  4       4     version: u32 = 1
  8       4     chunk_count_x: u32 = 16
  12      4     chunk_count_y: u32 = 1
  16      4     chunk_count_z: u32 = 16
  20      4     chunk_voxel_x: u32 = 256
  24      4     chunk_voxel_y: u32 = 2048
  28      4     chunk_voxel_z: u32 = 256
  32      32    reserved: [u8; 32] = [0; 32]

CHUNK TABLE OF CONTENTS (chunk_count_x * chunk_count_y * chunk_count_z entries,
each 16 bytes):
  For each chunk index (y-major: index = x + z * chunk_count_x):
    offset  size  field
    0       8     byte_offset: u64  (byte offset from file start; 0 = chunk not present)
    8       8     compressed_size: u64  (bytes of chunk data)

CHUNK DATA (per present chunk, stored at `byte_offset`):
  params:      Tree64Params (32 bytes, as defined in tree64_renderer.rs)
  node_count:  u32
  node_bytes:  u32  (= node_count * 12, for sanity check)
  nodes:       [GpuNode; node_count]  (node_count * 12 bytes, Pod)
  leaf_count:  u32
  leaf_bytes:  u32  (= leaf_count, for sanity check)
  leaf_data:   [u8; leaf_count]  (raw u8 palette indices)
```

Each chunk stores its data in GPU-ready format: `GpuNode` and `leaf_data` are
byte-for-byte what gets uploaded to the GPU. `Tree64Params` is the 32-byte
uniform struct. Empty chunks (no voxels) have `byte_offset = 0` in the TOC and
no chunk data section.

## Commands you will need

| Purpose       | Command                        | Expected on success |
|---------------|--------------------------------|---------------------|
| Check         | `cargo check`                  | exit 0, no errors   |
| Check (bake)  | `cargo check --bin bake`       | exit 0, no errors   |
| Check (all)   | `cargo check --workspace`      | exit 0, no errors   |
| Test          | `cargo test`                   | all pass            |
| Clippy        | `cargo clippy -- -D warnings`  | exit 0, no warnings |
| Format        | `cargo fmt --check`            | exit 0              |

## Suggested executor toolkit

- Read `assets/shaders/tree64_compiled.wgsl` before step 7 to understand the
  shader's binding layout — it uses one set of nodes/leaf_data/params per
  dispatch. The plan already accounts for this.

## Scope

**In scope** (files you may create or modify):

- `src/formats/mod.rs` — new module: `WorldHeader`, chunk TOC, reader/writer
- `src/formats/chunk.rs` — new module: `ChunkData` serialize/deserialize
- `src/world/mod.rs` — rewrite: remove procedural generation, add `World` struct
  managing chunks. Keep `TerrainModel` if you need it for comparison, but the
  `build_tree64()` function must go.
- `src/world/chunk_manager.rs` — new module: `ChunkManager` - coordinates
  per-chunk GPU buffer creation and render dispatch
- `src/tree64_renderer.rs` — add `GpuTree64::serialize()`, `GpuTree64::deserialize()`,
  and a `GpuTree64::from_tree64(tree: &Tree64<u8>)` constructor
- `src/app.rs` — replace single-tree rendering with multi-chunk dispatch
- `src/bin/bake.rs` — new binary target: MagicaVoxel `.vox` → `.world` converter
- `Cargo.toml` — add `[[bin]]` entry for bake tool; potentially add workspace
  for the bake crate if that proves cleaner (but a bin target in the same crate
  is simpler and covered here)
- `plans/README.md` — update status row for this plan

**Out of scope** (do NOT touch):

- `src/framework.rs` — no changes needed
- `src/player_controller.rs` — no changes needed
- `src/utils.rs` — no changes needed
- `build.rs` — no changes needed
- `assets/shaders/` — no shader changes needed (one dispatch per chunk reuses
  the same pipeline)
- Frustum culling of chunks — MVP dispatches all chunks. Culling is a follow-up
- Runtime chunk editing/destruction — this plan is read-only world loading
- Compressed chunk data — chunks are stored uncompressed in MVP
- Vertical chunk splitting (chunk_count_y > 1) — the format supports it in the
  header but the bake tool hardcodes 1

## Git workflow

- Branch: `advisor/003-chunked-world-architecture`
- Commit per step or per logical unit
- Message style: `feat(<module>): <description>` — match existing commits like
  `fix: handle Option acquire result in RedrawRequested`
- Do NOT push or open a PR unless instructed.

## Steps

### Step 0: Verify checkpoint

Run the drift check and confirm no in-scope files have changed since `fe59d7e`.

**Verify**: `cargo check` → exit 0, no errors.

---

### Step 1: Add `GpuTree64` serialization and a from-tree64 constructor

In `src/tree64_renderer.rs`:

1a. Add `use std::io;` at the top.

1b. Add a second `impl GpuTree64` block (after the existing one that contains
`from_model` and `create_buffers`). Rust allows multiple impl blocks for the
same type.
This avoids the model adapter during deserialization — we build a `Tree64<u8>`
once (during bake) and serialize/deserialize the `GpuTree64` directly for the GPU.

```rust
impl GpuTree64 {
    /// Build GpuTree64 from an already-constructed Tree64<u8>.
    /// Useful when deserializing a pre-built tree (no VoxelModel needed).
    pub fn from_tree64(tree: &tree64::Tree64<u8>) -> Self {
        let root_state = tree.root_state();

        let nodes: Vec<GpuNode> = tree
            .nodes
            .iter()
            .map(|n| {
                let tree64_node = *n;
                let is_leaf = (tree64_node.is_leaf_and_ptr & 1) == 1;
                let ptr = tree64_node.is_leaf_and_ptr >> 1;
                GpuNode::new(is_leaf, ptr, tree64_node.pop_mask)
            })
            .collect();

        let leaf_data = tree.data.to_vec();

        let world_size = 4u32.pow(root_state.num_levels as u32);
        let tree_scale = world_size.ilog2();

        Self {
            nodes,
            leaf_data,
            root_node_index: root_state.index,
            tree_scale,
            root_offset: root_state.offset.to_array(),
        }
    }

    /// Serialize to a writer in the GPU-ready format.
    /// Writes: Tree64Params (32B), node_count (u32), node_bytes_sanity (u32),
    /// nodes ([GpuNode; node_count] as raw bytes), leaf_count (u32),
    /// leaf_bytes_sanity (u32), leaf_data ([u8; leaf_count]).
    pub fn serialize<W: io::Write>(&self, mut writer: W) -> io::Result<()> {
        let params = Tree64Params {
            root_node_index: self.root_node_index,
            tree_scale: self.tree_scale,
            _pad0: [0; 2],
            root_offset: self.root_offset,
            _pad1: 0,
        };
        writer.write_all(bytemuck::bytes_of(&params))?;

        let node_count = self.nodes.len() as u32;
        let node_bytes = node_count * std::mem::size_of::<GpuNode>() as u32;
        writer.write_all(&node_count.to_le_bytes())?;
        writer.write_all(&node_bytes.to_le_bytes())?;
        writer.write_all(bytemuck::cast_slice(&self.nodes))?;

        let leaf_count = self.leaf_data.len() as u32;
        writer.write_all(&leaf_count.to_le_bytes())?;
        writer.write_all(&leaf_count.to_le_bytes())?;
        writer.write_all(&self.leaf_data)?;

        Ok(())
    }

    /// Deserialize from a reader. Reads the format written by `serialize`.
    pub fn deserialize<R: io::Read>(mut reader: R) -> io::Result<Self> {
        let mut params_bytes = [0u8; std::mem::size_of::<Tree64Params>()];
        reader.read_exact(&mut params_bytes)?;
        let params: Tree64Params = *bytemuck::from_bytes(&params_bytes);

        let mut node_count_bytes = [0u8; 4];
        reader.read_exact(&mut node_count_bytes)?;
        let node_count = u32::from_le_bytes(node_count_bytes);

        let mut node_bytes_sanity = [0u8; 4];
        reader.read_exact(&mut node_bytes_sanity)?;
        // consume but ignore — used for format validation in a full impl

        let node_byte_len = node_count as usize * std::mem::size_of::<GpuNode>();
        let mut nodes: Vec<GpuNode> = vec![GpuNode {
            packed_data: 0,
            pop_mask_lo: 0,
            pop_mask_hi: 0,
        }; node_count as usize];
        reader.read_exact(bytemuck::cast_slice_mut(&mut nodes))?;

        let mut leaf_count_bytes = [0u8; 4];
        reader.read_exact(&mut leaf_count_bytes)?;
        let leaf_count = u32::from_le_bytes(leaf_count_bytes);

        let mut leaf_bytes_sanity = [0u8; 4];
        reader.read_exact(&mut leaf_bytes_sanity)?;

        let mut leaf_data = vec![0u8; leaf_count as usize];
        reader.read_exact(&mut leaf_data)?;

        Ok(Self {
            nodes,
            leaf_data,
            root_node_index: params.root_node_index,
            tree_scale: params.tree_scale,
            root_offset: params.root_offset,
        })
    }
}
```

**Verify**: `cargo check` → exit 0. The code must compile.
Expect a warning about unused variable `node_byte_len` in `deserialize` —
this is acceptable for MVP. If using `cargo clippy -- -D warnings`, add
`let _ = node_byte_len;` to suppress it.
Then run `cargo clippy -- -D warnings` → exit 0.

---

### Step 2: Create the world format module (`src/formats/`)

Create two new files.

**2a. `src/formats/mod.rs`** — world file header and TOC reader/writer:

```rust
//! Binary world format (.world) — header, chunk table of contents,
//! and per-chunk GPU-ready blobs.

pub mod chunk;

use std::io;

/// Magic bytes identifying a .world file.
pub const WORLD_MAGIC: [u8; 4] = *b"WRLD";

/// Current format version.
pub const WORLD_VERSION: u32 = 1;

/// World dimensions (immutable for this format version).
pub const CHUNK_COUNT_X: u32 = 16;
pub const CHUNK_COUNT_Y: u32 = 1;
pub const CHUNK_COUNT_Z: u32 = 16;
pub const CHUNK_VOXEL_X: u32 = 256;
pub const CHUNK_VOXEL_Y: u32 = 2048;
pub const CHUNK_VOXEL_Z: u32 = 256;
pub const TOTAL_CHUNKS: u32 = CHUNK_COUNT_X * CHUNK_COUNT_Y * CHUNK_COUNT_Z;

/// Header of a .world file (64 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WorldHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub chunk_count_x: u32,
    pub chunk_count_y: u32,
    pub chunk_count_z: u32,
    pub chunk_voxel_x: u32,
    pub chunk_voxel_y: u32,
    pub chunk_voxel_z: u32,
    pub reserved: [u8; 32],
}

impl WorldHeader {
    pub fn new() -> Self {
        Self {
            magic: WORLD_MAGIC,
            version: WORLD_VERSION,
            chunk_count_x: CHUNK_COUNT_X,
            chunk_count_y: CHUNK_COUNT_Y,
            chunk_count_z: CHUNK_COUNT_Z,
            chunk_voxel_x: CHUNK_VOXEL_X,
            chunk_voxel_y: CHUNK_VOXEL_Y,
            chunk_voxel_z: CHUNK_VOXEL_Z,
            reserved: [0; 32],
        }
    }

    pub fn read(mut reader: impl io::Read) -> io::Result<Self> {
        let mut bytes = [0u8; 64];
        reader.read_exact(&mut bytes)?;
        let header: Self = unsafe { std::ptr::read(bytes.as_ptr() as *const Self) };
        if header.magic != WORLD_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid world file magic",
            ));
        }
        if header.version != WORLD_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported world version: {}", header.version),
            ));
        }
        Ok(header)
    }

    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        let bytes: &[u8; 64] = unsafe { std::mem::transmute(self) };
        writer.write_all(bytes)
    }

    pub fn total_chunks(&self) -> u32 {
        self.chunk_count_x * self.chunk_count_y * self.chunk_count_z
    }
}

/// Single entry in the chunk table of contents (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ChunkTocEntry {
    pub byte_offset: u64,
    pub size: u64,
}

impl ChunkTocEntry {
    pub fn read(mut reader: impl io::Read) -> io::Result<Self> {
        let mut bytes = [0u8; 16];
        reader.read_exact(&mut bytes)?;
        Ok(unsafe { std::ptr::read(bytes.as_ptr() as *const Self) })
    }

    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        let bytes: &[u8; 16] = unsafe { std::mem::transmute(self) };
        writer.write_all(bytes)
    }
}

/// The full chunk table of contents (256 entries × 16 bytes = 4096 bytes).
pub struct ChunkTable {
    pub entries: Vec<ChunkTocEntry>,
}

impl ChunkTable {
    pub fn new(chunk_count: u32) -> Self {
        Self {
            entries: vec![ChunkTocEntry::default(); chunk_count as usize],
        }
    }

    pub fn read(mut reader: impl io::Read, chunk_count: u32) -> io::Result<Self> {
        let mut entries = Vec::with_capacity(chunk_count as usize);
        for _ in 0..chunk_count {
            entries.push(ChunkTocEntry::read(&mut reader)?);
        }
        Ok(Self { entries })
    }

    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        for entry in &self.entries {
            entry.write(&mut writer)?;
        }
        Ok(())
    }

    /// Convert a 3D chunk coordinate to a flat index.
    /// Layout: index = x + z * chunk_count_x  (y is always 0 since chunk_count_y = 1).
    pub fn chunk_index(x: u32, _y: u32, z: u32, chunk_count_x: u32) -> usize {
        (x + z * chunk_count_x) as usize
    }
}
```

**2b. `src/formats/chunk.rs`** — serialization wrapper for chunk data blobs:

```rust
use std::io;

use crate::tree64_renderer::GpuTree64;

/// A chunk's serialized data blob.
/// Wraps GpuTree64 serialize/deserialize for use within the world format.
pub struct ChunkData {
    pub tree: GpuTree64,
}

impl ChunkData {
    pub fn new(tree: GpuTree64) -> Self {
        Self { tree }
    }

    /// Read a chunk blob from a reader.
    /// This reads exactly the bytes written by `write`.
    pub fn read(mut reader: impl io::Read) -> io::Result<Self> {
        let tree = GpuTree64::deserialize(&mut reader)?;
        Ok(Self { tree })
    }

    /// Write the chunk blob to a writer.
    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        self.tree.serialize(&mut writer)
    }
}
```

**2c.** Register the new module in `src/main.rs`:

```rust
mod formats;
```

**Verify**: `cargo check` → exit 0. `cargo clippy -- -D warnings` → exit 0.

---

### Step 3: Add the bake binary target

Add a `[[bin]]` section to `Cargo.toml` (after `[dependencies]`, before any
`[profile]` sections if they exist):

```toml
[[bin]]
name = "bake"
path = "src/bin/bake.rs"
```

Create `src/bin/bake.rs`:

```rust
//! Bake tool: converts a MagicaVoxel .vox file into the .world binary format.
//!
//! Usage: cargo run --bin bake -- <input.vox> <output.world>
//!
//! The tool reads a .vox file, partitions the voxel grid into 16×16 chunks,
//! builds a Tree64 per chunk, and writes a .world file.

use std::env;
use std::fs::File;
use std::io::BufWriter;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input.vox> <output.world>", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    // Read .vox file — dot_vox::load takes a path string, not a reader
    let vox_data = dot_vox::load(input_path).expect("failed to parse .vox file");

    if vox_data.models.is_empty() {
        eprintln!("Error: .vox file contains no models");
        std::process::exit(1);
    }

    // Use the first model (scene with multiple models not yet supported).
    let model = &vox_data.models[0];
    let model_size = model.size;
    eprintln!(
        "Model: {}×{}×{} voxels, {} voxels total",
        model_size.x, model_size.y, model_size.z,
        model.voxels.len()
    );

    // Build world structure
    let mut world_file = wgpu_rt::formats::WorldFile::new();

    eprintln!(
        "Chunk grid: {}×{}×{}, chunk size: {}×{}×{} voxels",
        wgpu_rt::formats::CHUNK_COUNT_X,
        wgpu_rt::formats::CHUNK_COUNT_Y,
        wgpu_rt::formats::CHUNK_COUNT_Z,
        wgpu_rt::formats::CHUNK_VOXEL_X,
        wgpu_rt::formats::CHUNK_VOXEL_Y,
        wgpu_rt::formats::CHUNK_VOXEL_Z,
    );

    let mut chunks_written: u32 = 0;

    for cz in 0..wgpu_rt::formats::CHUNK_COUNT_Z {
        for cy in 0..wgpu_rt::formats::CHUNK_COUNT_Y {
            for cx in 0..wgpu_rt::formats::CHUNK_COUNT_X {
                let chunk_x_min = cx * wgpu_rt::formats::CHUNK_VOXEL_X;
                let chunk_y_min = cy * wgpu_rt::formats::CHUNK_VOXEL_Y;
                let chunk_z_min = cz * wgpu_rt::formats::CHUNK_VOXEL_Z;

                // Build a VoxelModel for this chunk's region
                let chunk_model = ChunkVoxelModel {
                    source: model,
                    offset_x: chunk_x_min,
                    offset_y: chunk_y_min,
                    offset_z: chunk_z_min,
                    chunk_size_x: wgpu_rt::formats::CHUNK_VOXEL_X,
                    chunk_size_y: wgpu_rt::formats::CHUNK_VOXEL_Y,
                    chunk_size_z: wgpu_rt::formats::CHUNK_VOXEL_Z,
                };

                // Build the Tree64
                let tree = tree64::Tree64::new(&chunk_model);

                // Skip completely empty chunks
                if tree.nodes.is_empty() || tree.root_state().index == 0 && tree.nodes[0].pop_mask == 0 {
                    continue;
                }

                // Convert to GpuTree64 and store
                let gpu_tree = wgpu_rt::tree64_renderer::GpuTree64::from_tree64(&tree);
                let index = wgpu_rt::formats::ChunkTable::chunk_index(
                    cx, cy, cz,
                    wgpu_rt::formats::CHUNK_COUNT_X,
                );

                world_file.set_chunk(index, wgpu_rt::formats::chunk::ChunkData::new(gpu_tree));
                chunks_written += 1;

                eprintln!(
                    "  Chunk ({}, {}, {}): {} nodes, {} bytes leaf data",
                    cx, cy, cz,
                    tree.nodes.len(),
                    tree.data.len(),
                );
            }
        }
    }

    eprintln!("Total non-empty chunks: {}", chunks_written);

    // Write world file
    let out_file = File::create(output_path).expect("failed to create output file");
    let writer = BufWriter::new(out_file);
    world_file.write(writer).expect("failed to write world file");

    eprintln!("Done: {} written", output_path);
}

/// A VoxelModel that wraps a dot_vox model and exposes a sub-region
/// as if it's a standalone model at origin (0,0,0).
struct ChunkVoxelModel<'a> {
    source: &'a dot_vox::Model,
    offset_x: u32,
    offset_y: u32,
    offset_z: u32,
    chunk_size_x: u32,
    chunk_size_y: u32,
    chunk_size_z: u32,
}

impl<'a> tree64::VoxelModel<u8> for &'a ChunkVoxelModel<'a> {
    fn dimensions(&self) -> [u32; 3] {
        [self.chunk_size_x, self.chunk_size_y, self.chunk_size_z]
    }

    fn access(&self, coord: [usize; 3]) -> Option<u8> {
        let (x, y, z) = (coord[0] as u32, coord[1] as u32, coord[2] as u32);

        if x >= self.chunk_size_x || y >= self.chunk_size_y || z >= self.chunk_size_z {
            return None;
        }

        let global_x = self.offset_x + x;
        let global_y = self.offset_y + y;
        let global_z = self.offset_z + z;

        if global_x >= self.source.size.x
            || global_y >= self.source.size.y
            || global_z >= self.source.size.z
        {
            return None;
        }

        // dot_vox stores voxels as a sparse list, each with its own (x, y, z, i).
        // Linear-scan to find the voxel at (global_x, global_y, global_z).
        self.source
            .voxels
            .iter()
            .find(|v| v.x == global_x as u8 && v.y == global_y as u8 && v.z == global_z as u8)
            .map(|v| v.i)
    }
}
```

The `WorldFile` struct (used above but not yet defined) will be in the formats
module. Add it in step 4.

**Verify**: `cargo check --bin bake` will fail because `WorldFile` doesn't exist yet.
That's expected — we'll fix it in the next step. Just confirm the bin target is
recognized: `cargo check --bin bake` → compilation errors about `WorldFile`, not
about missing targets.

---

### Step 4: Add `WorldFile` writer to `src/formats/mod.rs`

Add to `src/formats/mod.rs` (before the closing of the file, after the `ChunkTable`
impl block):

```rust
use crate::formats::chunk::ChunkData;

/// Complete world file: header + chunk table + chunk data blobs.
pub struct WorldFile {
    pub header: WorldHeader,
    pub table: ChunkTable,
    /// Chunk data indexed by the same flat index as the TOC.
    pub chunks: Vec<Option<ChunkData>>,
}

impl WorldFile {
    pub fn new() -> Self {
        let header = WorldHeader::new();
        let total = header.total_chunks() as usize;
        Self {
            header,
            table: ChunkTable::new(total as u32),
            chunks: (0..total).map(|_| None).collect(),
        }
    }

    /// Set chunk data for the given flat index.
    /// Also populates the TOC entry (byte_offset and size will be set during write).
    pub fn set_chunk(&mut self, index: usize, data: ChunkData) {
        self.chunks[index] = Some(data);
    }

    /// Write the complete world file.
    pub fn write(&self, mut writer: impl io::Write + io::Seek) -> io::Result<()> {
        // Write header (64 bytes)
        self.header.write(&mut writer)?;

        // Reserve space for the TOC (we'll write placeholder zeros and seek back)
        let toc_size = self.table.entries.len() * 16;
        let toc_start = writer.stream_position()?;
        let zeros = vec![0u8; toc_size];
        writer.write_all(&zeros)?;

        // Write chunk data, building TOC entries as we go
        let mut toc_entries = vec![ChunkTocEntry::default(); self.table.entries.len()];

        for (i, chunk_opt) in self.chunks.iter().enumerate() {
            if let Some(chunk) = chunk_opt {
                let offset = writer.stream_position()?;
                chunk.write(&mut writer)?;
                let end = writer.stream_position()?;
                toc_entries[i] = ChunkTocEntry {
                    byte_offset: offset,
                    size: end - offset,
                };
            }
        }

        // Seek back and write the TOC
        writer.seek(io::SeekFrom::Start(toc_start))?;
        for entry in &toc_entries {
            entry.write(&mut writer)?;
        }

        Ok(())
    }

    /// Read a complete world file.
    pub fn read(mut reader: impl io::Read + io::Seek) -> io::Result<Self> {
        let header = WorldHeader::read(&mut reader)?;
        let total = header.total_chunks() as usize;
        let table = ChunkTable::read(&mut reader, total as u32)?;

        let mut chunks: Vec<Option<ChunkData>> = Vec::with_capacity(total);

        for entry in &table.entries {
            if entry.byte_offset == 0 {
                chunks.push(None);
            } else {
                reader.seek(io::SeekFrom::Start(entry.byte_offset))?;
                let data = ChunkData::read(&mut reader)?;
                chunks.push(Some(data));
            }
        }

        Ok(Self {
            header,
            table,
            chunks,
        })
    }
}
```

Also update the import at the top of `src/formats/mod.rs`. **Replace** the
line `use std::io;` with:

```rust
use std::io::{self, Seek};
```

Do NOT add this as a second line — it must REPLACE the existing `use std::io;`.

And make `chunk` module public by changing `pub mod chunk;` if it isn't already.

**Verify**: `cargo check --bin bake` → exit 0 (the bake binary should now
compile since `WorldFile` exists). `cargo check` → exit 0.

---

### Step 5: Add a unit test for the world format round-trip

Add to `src/formats/mod.rs` (at the bottom, inside a `#[cfg(test)]` block):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::chunk::ChunkData;
    use crate::tree64_renderer::{GpuNode, GpuTree64};
    use std::io::Cursor;

    fn make_dummy_gpu_tree() -> GpuTree64 {
        GpuTree64 {
            nodes: vec![
                GpuNode::new(false, 1, 0b0001_0001_0001_0001u64),
                GpuNode::new(true, 0, 0b1111_0000_0000_0000u64),
            ],
            leaf_data: vec![1, 2, 3, 4],
            root_node_index: 0,
            tree_scale: 8,
            root_offset: [0, 0, 0],
        }
    }

    #[test]
    fn world_file_roundtrip() {
        let mut world = WorldFile::new();

        // Add a few chunks at known positions
        let chunk0 = ChunkData::new(make_dummy_gpu_tree());
        world.set_chunk(0, chunk0);

        let mut chunk5 = make_dummy_gpu_tree();
        chunk5.leaf_data = vec![5, 6, 7, 8];
        world.set_chunk(5, ChunkData::new(chunk5));

        // Write to memory
        let mut buf = Cursor::new(Vec::new());
        world.write(&mut buf).unwrap();

        // Read back
        buf.set_position(0);
        let loaded = WorldFile::read(&mut buf).unwrap();

        // Verify header
        assert_eq!(loaded.header.magic, WORLD_MAGIC);
        assert_eq!(loaded.header.version, WORLD_VERSION);
        assert_eq!(loaded.header.total_chunks(), 256);

        // Verify chunk 0
        let chunk0_loaded = loaded.chunks[0].as_ref().unwrap();
        assert_eq!(chunk0_loaded.tree.nodes.len(), 2);
        assert_eq!(chunk0_loaded.tree.leaf_data, vec![1, 2, 3, 4]);

        // Verify chunk 5
        let chunk5_loaded = loaded.chunks[5].as_ref().unwrap();
        assert_eq!(chunk5_loaded.tree.leaf_data, vec![5, 6, 7, 8]);

        // Verify empty chunks are None
        assert!(loaded.chunks[1].is_none());
        assert!(loaded.chunks[255].is_none());
    }
}
```

**Verify**: `cargo test world_file_roundtrip` → test passes.
Then `cargo test` → all tests pass (including existing ones).

---

### Step 6: Create the chunk manager

Create `src/world/chunk_manager.rs`:

```rust
use std::collections::HashMap;

use crate::tree64_renderer::{GpuTree64, GpuTree64Buffers, Tree64Params};

/// Identifies a chunk in the world grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkCoord {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// A loaded chunk with its GPU buffers.
pub struct LoadedChunk {
    pub coord: ChunkCoord,
    pub buffers: GpuTree64Buffers,
}

/// Manages the set of currently loaded chunks and their GPU resources.
pub struct ChunkManager {
    pub chunks: HashMap<ChunkCoord, LoadedChunk>,
}

impl ChunkManager {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }

    /// Load a chunk onto the GPU. Replaces any existing chunk at this coordinate.
    pub fn load_chunk(&mut self, coord: ChunkCoord, tree: GpuTree64, device: &wgpu::Device) {
        let buffers = tree.create_buffers(device);
        self.chunks.insert(
            coord,
            LoadedChunk { coord, buffers },
        );
    }

    /// Remove a chunk and drop its GPU buffers.
    pub fn unload_chunk(&mut self, coord: &ChunkCoord) {
        self.chunks.remove(coord);
    }

    /// Returns an iterator over all loaded chunks.
    pub fn loaded_chunks(&self) -> impl Iterator<Item = &LoadedChunk> {
        self.chunks.values()
    }

    /// Clear all loaded chunks.
    pub fn clear(&mut self) {
        self.chunks.clear();
    }
}
```

**Verify**: `cargo check` → exit 0.

---

### Step 7: Rewrite `src/world/mod.rs`

Replace the existing contents of `src/world/mod.rs`:

```rust
pub mod chunk_manager;

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use tree64::VoxelModel;

use crate::formats::{WorldFile, CHUNK_COUNT_X, CHUNK_COUNT_Z};
use crate::tree64_renderer::GpuTree64;

/// Loaded world state: all chunks (some may be empty) and their metadata.
pub struct World {
    /// GpuTree64 for each chunk. Index: chunks[x + z * CHUNK_COUNT_X].
    /// None means the chunk was empty (not present in the .world file).
    pub chunks: Vec<Option<GpuTree64>>,
    pub chunk_count_x: u32,
    pub chunk_count_z: u32,
    pub chunk_voxel_x: u32,
    pub chunk_voxel_z: u32,
}

impl World {
    /// Load a .world file from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let file = File::open(path.as_ref())
            .map_err(|e| format!("failed to open world file {}: {e}", path.as_ref().display()))?;
        let mut reader = BufReader::new(file);
        let world_file = WorldFile::read(&mut reader)
            .map_err(|e| format!("failed to read world file: {e}"))?;

        let total = world_file.header.total_chunks() as usize;
        let mut chunks: Vec<Option<GpuTree64>> = Vec::with_capacity(total);

        for chunk_opt in world_file.chunks {
            chunks.push(chunk_opt.map(|cd| cd.tree));
        }

        let loaded_count = chunks.iter().filter(|c| c.is_some()).count();

        log::info!(
            "Loaded world: {} chunks ({} non-empty), grid {}×{}",
            total,
            loaded_count,
            world_file.header.chunk_count_x,
            world_file.header.chunk_count_z,
        );

        Ok(Self {
            chunks,
            chunk_count_x: world_file.header.chunk_count_x,
            chunk_count_z: world_file.header.chunk_count_z,
            chunk_voxel_x: world_file.header.chunk_voxel_x,
            chunk_voxel_z: world_file.header.chunk_voxel_z,
        })
    }

    /// Get the GpuTree64 for a chunk, if present.
    pub fn get_chunk(&self, x: u32, z: u32) -> Option<&GpuTree64> {
        if x >= self.chunk_count_x || z >= self.chunk_count_z {
            return None;
        }
        let index = (x + z * self.chunk_count_x) as usize;
        self.chunks[index].as_ref()
    }
}
```

**Verify**: `cargo check` → exit 0. This removes the old `TerrainModel` and
`build_tree64()` — expect any remaining references to those in `app.rs` to
cause errors; we'll fix that next.

---

### Step 8: Update `App` for multi-chunk rendering

This is the biggest change. In `src/app.rs`:

**8a.** Add `use crate::world::chunk_manager::{ChunkCoord, ChunkManager};` to imports.
Add `use crate::world::World;`.

**8b.** Remove the single-tree buffer fields from the struct (lines 52–54):
```rust
    // REMOVE:
    // Tree buffers (needed for bind-group recreation on resize)
    tree_params_buffer: wgpu::Buffer,
    tree_nodes_buffer: wgpu::Buffer,
    tree_leaf_data_buffer: wgpu::Buffer,
```

**8c.** Replace the single `tree_bind_group` with:
```rust
    chunk_manager: ChunkManager,
```

**8d.** Replace the single `tree_bind_group_layout` with:
```rust
    chunk_bind_group_layout: wgpu::BindGroupLayout,
```
Keep `blit_bind_group_layout` (it was already present — do NOT remove it).
Add per-chunk bind groups to the struct:
```rust
    chunk_bind_groups: Vec<(ChunkCoord, wgpu::BindGroup)>,
```

**8e.** Replace the `init` body. The `init` function needs a significant rewrite.
Here's the full replacement for the body of `App::init()` (everything from
`let width = config.width;` to the closing `App { ... }`):

```rust
    pub fn init(
        config: &wgpu::SurfaceConfiguration,
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
    ) -> Self {
        let width = config.width;
        let height = config.height;
        let format = TextureFormat::Rgba8Unorm;

        let rt_texture = device.create_texture(&TextureDescriptor {
            label: Some("rt_output"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[format],
        });

        let rt_view = rt_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("rt_view"),
            format: Some(format),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });

        // Load world from a hardcoded path for now.
        // In the future, this should come from a CLI argument or config.
        let world_path = std::env::current_dir()
            .unwrap_or_default()
            .join("assets")
            .join("world.world");

        let (world, chunk_manager) = if world_path.exists() {
            let world = World::load(&world_path)
                .expect("failed to load world file");
            let mut chunk_manager = ChunkManager::new();
            for cz in 0..world.chunk_count_z {
                for cx in 0..world.chunk_count_x {
                    let coord = ChunkCoord { x: cx, y: 0, z: cz };
                    if let Some(tree) = world.get_chunk(cx, cz) {
                        chunk_manager.load_chunk(coord, tree.clone_ref(), device);
                    }
                }
            }
            (world, chunk_manager)
        } else {
            log::warn!(
                "World file not found at {:?}, using empty world. \
                 Run `cargo run --bin bake` first.",
                world_path
            );
            let world = World {
                chunks: vec![None; (CHUNK_COUNT_X * CHUNK_COUNT_Z) as usize],
                chunk_count_x: CHUNK_COUNT_X,
                chunk_count_z: CHUNK_COUNT_Z,
                chunk_voxel_x: 256,
                chunk_voxel_z: 256,
            };
            (world, ChunkManager::new())
        };

        let loaded_count = chunk_manager.loaded_chunks().count();
        log::info!("GPU chunks loaded: {}", loaded_count);

        let aspect = width as f32 / height as f32;
        let mut player_controller = PlayerController::default();
        let camera_uniforms = CameraUniforms {
            pos: [
                player_controller.translation.x,
                player_controller.translation.y,
                player_controller.translation.z,
                1.0,
            ],
            view_inv: player_controller.view().inverse().to_cols_array_2d(),
            proj_inv: glam::camera::rh::proj::vulkan::perspective(
                std::f32::consts::FRAC_PI_4,
                aspect,
                0.1,
                10000.0,
            )
            .inverse()
            .to_cols_array_2d(),
        };

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_uniforms"),
            contents: bytemuck::bytes_of(&camera_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tree64_raycast"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/tree64_compiled.wgsl"
            ))),
        });

        let chunk_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("tree64_bind_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(32),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        // Build one bind group per loaded chunk
        let chunk_bind_groups: Vec<(ChunkCoord, wgpu::BindGroup)> = chunk_manager
            .loaded_chunks()
            .map(|chunk| {
                let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!(
                        "chunk_bind_group_{}_{}",
                        chunk.coord.x, chunk.coord.z
                    )),
                    layout: &chunk_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&rt_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: chunk.buffers.params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: camera_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: chunk.buffers.nodes.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: chunk.buffers.leaf_data.as_entire_binding(),
                        },
                    ],
                });
                (chunk.coord, bg)
            })
            .collect();

        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tree64_pipeline_layout"),
                bind_group_layouts: &[Some(&chunk_bind_group_layout)],
                immediate_size: 0,
            });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("tree64_compute_pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/blit.wgsl"
            ))),
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(config.format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let blit_view_bind_group_layout = blit_pipeline.get_bind_group_layout(0);
        let blit_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit_view"),
            layout: &blit_view_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            }],
        });

        App {
            compute_pipeline,
            chunk_bind_groups,
            camera_buffer,
            player_controller,
            blit_pipeline,
            blit_view_bind_group,
            last_frame_update: Instant::now(),
            delta_time: Duration::default(),
            surface_width: width,
            surface_height: height,
            rt_texture,
            chunk_bind_group_layout,
            chunk_manager,
        }
    }
```

Note: `clone_ref()` doesn't exist on `GpuTree64`. We need to add it.
Add to `src/tree64_renderer.rs` in the `GpuTree64` impl block:

```rust
    /// Create a shallow clone that shares no ownership — copies the data.
    /// Used when loading chunks into the chunk manager.
    pub fn clone_ref(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            leaf_data: self.leaf_data.clone(),
            root_node_index: self.root_node_index,
            tree_scale: self.tree_scale,
            root_offset: self.root_offset,
        }
    }
```

**8f.** Update `recreate_render_target` in app.rs. Replace the string
`self.tree_bind_group` references. The method needs to rebuild all chunk bind
groups. Replace the body of `recreate_render_target`:

```rust
    fn recreate_render_target(&mut self, device: &wgpu::Device) {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let width = self.surface_width;
        let height = self.surface_height;

        self.rt_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rt_output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[format],
        });

        let rt_view = self.rt_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("rt_view"),
            format: Some(format),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });

        // Rebuild all chunk bind groups with the new render target view
        self.chunk_bind_groups = self
            .chunk_manager
            .loaded_chunks()
            .map(|chunk| {
                let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!(
                        "chunk_bind_group_{}_{}",
                        chunk.coord.x, chunk.coord.z
                    )),
                    layout: &self.chunk_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&rt_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: chunk.buffers.params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.camera_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: chunk.buffers.nodes.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: chunk.buffers.leaf_data.as_entire_binding(),
                        },
                    ],
                });
                (chunk.coord, bg)
            })
            .collect();

        self.blit_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit_view"),
            layout: &self.blit_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            }],
        });
    }
```

**8g.** Update `render()` — replace the single compute pass dispatch with a loop
over all chunk bind groups:

Find this block in `render()`:
```rust
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("tree64_compute_pass"),
                timestamp_writes: None,
            });

            cpass.set_pipeline(&self.compute_pipeline);
            cpass.set_bind_group(0, &self.tree_bind_group, &[]);

            let workgroup_x = self.surface_width.div_ceil(8);
            let workgroup_y = self.surface_height.div_ceil(8);
            cpass.dispatch_workgroups(workgroup_x, workgroup_y, 1);
        }
```

Replace with:
```rust
        {
            let workgroup_x = self.surface_width.div_ceil(8);
            let workgroup_y = self.surface_height.div_ceil(8);

            for (_coord, bind_group) in &self.chunk_bind_groups {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("tree64_compute_pass"),
                    timestamp_writes: None,
                });
                cpass.set_pipeline(&self.compute_pipeline);
                cpass.set_bind_group(0, bind_group, &[]);
                cpass.dispatch_workgroups(workgroup_x, workgroup_y, 1);
            }
        }
```

**8h.** Update the `App` struct definition. Remove `tree_bind_group: wgpu::BindGroup,`
and replace with these fields. Also ensure `blit_bind_group_layout` is still present:
```rust
    chunk_bind_groups: Vec<(crate::world::chunk_manager::ChunkCoord, wgpu::BindGroup)>,
    chunk_manager: crate::world::chunk_manager::ChunkManager,
```
The existing `blit_bind_group_layout: wgpu::BindGroupLayout` field must remain —
do NOT remove it. If you see it missing from the struct after your edits, add it back.

**8i.** Add the missing imports for constants. In the imports section at the top of
`src/app.rs`, add:
```rust
use crate::formats::{CHUNK_COUNT_X, CHUNK_COUNT_Z};
```

**Verify**: `cargo check` → exit 0, no errors. Expect several iterations of fixing
compilation errors — the struct field changes cascade. Work through each
compiler error systematically.

Then `cargo clippy -- -D warnings` → exit 0.

---

### Step 9: Integration test — bake a test scene and run

**9a.** This step requires a `.vox` file. If you have one, place it at
`assets/test_scene.vox`. If you don't have one, skip to step 9d — the empty-world
fallback verifies the engine doesn't crash.

To create a minimal `.vox` programmatically, you can use this Python script
(requires `pip install pyvox`):
```python
# save as scripts/make_test_vox.py
from pyvox.models import Vox, Model
from pyvox.writer import VoxWriter
model = Model(size=(8, 8, 8))
for x in range(2, 6):
    for y in range(2, 6):
        for z in range(2, 6):
            model.voxels.append(Vox(x=x, y=y, z=z, i=1))
vox = Vox(models=[model])
with open('assets/test_scene.vox', 'wb') as f:
    VoxWriter(f).write(vox)
```
Run: `python scripts/make_test_vox.py`

**9b.** Once a `.vox` file is available at `assets/test_scene.vox`, run:

```
cargo run --bin bake -- assets/test_scene.vox assets/world.world
```

Expected output: logs showing chunk counts, node counts, "Done" message.
The file `assets/world.world` should exist and be non-zero size.

**9d.** If no `.vox` file is available, confirm the empty-world fallback works:
```
cargo run
```
Expected: log shows "World file not found...", window opens, black screen,
no crash. Move the camera — no panic, no error.

**Verify**: The engine starts and renders a window without crashing.

---

### Step 10: Clean up and final verification

- Remove the old procedural generation code entirely (should already be gone
  from step 7, but double-check: `grep -rn "TerrainModel\|build_tree64\|Perlin" src/`
  should return no matches).
- Run `cargo fmt` if any formatting issues remain.
- Run `cargo clippy -- -D warnings` → exit 0.
- Run `cargo test` → all tests pass.

**Verify**: `cargo check --workspace` → exit 0. `cargo test` → all pass.
`cargo fmt --check` → exit 0. `cargo clippy -- -D warnings` → exit 0.

---

## Test plan

- **`world_file_roundtrip`** (new, in `src/formats/mod.rs`): writes a WorldFile
  with 2 chunks to a Cursor, reads back, verifies header, TOC, and chunk data
  match. Tests empty chunk handling (offset=0 → None).
- **`gpu_tree_serialize_roundtrip`** (new, in `src/tree64_renderer.rs`): serialize
  a GpuTree64, deserialize, verify fields match.
- Existing tests from tree64 crate continue to pass as before.

## Done criteria

ALL must hold:

- [ ] `cargo check --workspace` exits 0
- [ ] `cargo check --bin bake` exits 0
- [ ] `cargo test` exits 0; `world_file_roundtrip` test exists and passes
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -- -D warnings` exits 0
- [ ] `grep -rn "TerrainModel\|build_tree64\|Perlin" src/` returns no matches
- [ ] `grep -rn "tree_bind_group\b" src/app.rs` returns no matches (replaced by chunk_bind_groups)
- [ ] `assets/world.world` can be produced by `cargo run --bin bake` with a test .vox
- [ ] Engine starts with `cargo run` and renders the world
- [ ] No files outside the in-scope list are modified (`git diff --stat` only shows
  files listed in Scope)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- `cargo check` fails with an error that can't be resolved within the scope of
  files listed above — the codebase may have drifted from commit `fe59d7e`.
- A step's verification fails twice after a reasonable fix attempt.
- The fix appears to require touching `src/framework.rs`, `build.rs`, or any
  shader file.
- `ChunkVoxelModel::access` returns incorrect results — dot_vox stores voxels
  as a sparse `Vec<Voxel>` where each `Voxel` has explicit `x: u8, y: u8,
  z: u8, i: u8` fields. The access method must scan the list for matching
  coordinates, NOT index as if it were a dense array. If the line-scan
  approach is too slow for large models, consider building a
  `HashMap<(u8,u8,u8), u8>` during `ChunkVoxelModel` construction.
- The shader expects exactly the bind group layout defined in step 8e. If
  `tree64_compiled.wgsl` has different bindings, STOP — this plan was written
  against the bindings observed in commit `fe59d7e` (binding 0=output texture,
  1=TreeParams uniform, 2=Camera uniform, 3=treeNodes storage, 4=leafData storage).

## Maintenance notes

- The chunk bind group layout (step 8e) is duplicated in `init` and
  `recreate_render_target`. If bindings change, both must be updated.
- `World::load()` expects the file path to be `assets/world.world` relative to
  the working directory. This should become a CLI argument or config in a
  follow-up plan.
- The MVP dispatches ALL chunks every frame. With 256 chunks, expect frame time
  to scale linearly with chunk count. Frustum culling (next plan) will bring
  this down to ~20–50 dispatches.
- The `GpuTree64::clone_ref()` method clones all data; for very large worlds,
  consider a zero-copy approach (shared Arc<Vec<GpuNode>>) in a follow-up.
- The bake tool currently reads the first model from the .vox file. Multi-model
  scenes and palette color extraction are out of scope.
- The `#[allow(unused)]` on struct field `_pad0` in `Tree64Params` may cause a
  new clippy warning when adding `clone_ref` — suppress with `#[allow(dead_code)]`
  if needed.
- When implementing runtime chunk editing in a future plan, the `ChunkManager`
  will need a `reload_chunk()` method that updates the per-chunk bind group.
- The `node_bytes_sanity` and `leaf_bytes_sanity` fields in
  `GpuTree64::deserialize` are read but not validated. A follow-up should
  add assertion checks comparing them against the actual byte counts.
- `ChunkVoxelModel::access` uses a linear scan over `model.voxels`. For large
  models (100K+ voxels), this will be slow during baking. A future optimization
  should build a `HashMap<(u8,u8,u8), u8>` on construction or sort the voxel
  list and use binary search.

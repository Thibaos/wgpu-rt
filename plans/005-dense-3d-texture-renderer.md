# Plan 005: Replace tree64 with dense 3D-texture DDA ray marcher

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 178741a..HEAD -- src/ assets/shaders/ Cargo.toml build.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED (full renderer swap — if something goes wrong the app shows black)
- **Depends on**: none (all prior plans are DONE)
- **Category**: direction
- **Planned at**: commit `178741a`, 2026-07-09

## Why this matters

The tree64 sparse tree is fast for empty-space skipping but complex (~300 lines
of compiled WGSL generated from Slang + GLSL patterns). The 3D-texture approach
replaces it with a straightforward Amanatides-Woo DDA march through an R8Uint
volume texture. The shader is ~80 lines of hand-written WGSL. This trades
peak performance for radical simplicity — the entire renderer becomes
understandable in one reading, and the codebase sheds the `tree64` dependency,
the slangc build step, and the brittle Slang→WGSL compilation pipeline.

## Current state

The app renders voxels via a tree64 compute shader. Relevant files:

- `src/tree64_renderer.rs` — `GpuTree64`, `GpuTree64Buffers`, `Tree64Params`, `create_palette_buffer`.
  Builds GPU buffers from a tree64, serializes them to the `.world` file.
- `src/world/mod.rs` — `World { tree: Option<GpuTree64>, palette }`.
  Loads a `.world` file.
- `src/formats/mod.rs` — `.world` binary format v3: header (64 B), palette
  (1024 B), then a tree blob serialized by `GpuTree64::serialize`.
  Contains `WorldHeader::read` which rejects `version > WORLD_VERSION` but
  currently has no explicit `version < WORLD_VERSION` check (v3 is the minimum).
- `src/app.rs` — `App::init(config, _adapter, device)` creates 6-slot bind group
  (output texture, tree params uniform, camera uniform, nodes storage, leaf data
  storage, palette storage). Uses `TEXTURE_BINDING_ARRAY | SHADER_INT64` features.
  Calls `crate::tree64_renderer::create_palette_buffer(device, &world.palette)`.
- `src/bin/bake.rs` — loads `.vox`, calls `SceneGraphLoader::load`, writes `.world`.
- `src/framework.rs` — calls `App::init(surface.config(), &context.adapter, &context.device)`.
  `context` also has `context.queue` available at that point.
- `assets/shaders/tree64_compiled.wgsl` — Slang-generated WGSL (~250 lines, not human-authored).
  Ray setup: NDC → `viewInv`/`projInv` → world-space ray, then Y/Z axis swap
  (`-rayOrigin.z, rayOrigin.y` for origin, `-rayDir.z, rayDir.y` for direction).
- `Cargo.toml` — depends on `tree64 = { git = "...", version = "0.1.0" }`.
- `build.rs` — invokes `slangc` to compile `.slang` → `.wgsl`.

Repo conventions:
- GPU resources use `device.create_buffer_init` / `device.create_texture` with
  label strings. Bind groups use the `wgpu::BindGroupDescriptor` pattern seen in
  `src/app.rs`. Match the existing naming (snake_case fields, `Some("label")` strings).
- `WorldFile` serialization: fixed-size header, then palette, then payload.
  Use `bytemuck::cast` for array transmutes, `io::Read::read_exact` for I/O.
- Error handling: `Result<_, String>` in world module, `io::Result<_>` in formats.
- Shaders: `include_str!(...)` from `src/app.rs`, path relative to `src/`.

## Commands you will need

| Purpose   | Command               | Expected on success |
|-----------|-----------------------|---------------------|
| Check     | `cargo check`         | exit 0              |
| Build     | `cargo build`         | exit 0              |
| Clippy    | `cargo clippy -- -D warnings` | exit 0     |
| Fmt check | `cargo fmt --check`   | exit 0 (or "no changes") |
| Bake      | `cargo run --bin bake -- assets/castle.vox assets/castle.world` | exit 0, writes file |
| Run (manual test) | `cargo run`  | window opens, renders voxels |

## Scope

**In scope** (the only files you should modify or create):
- `assets/shaders/dda_raycast.wgsl` — **CREATE** new compute shader
- `src/app.rs` — replace tree64 bind groups with 3D texture bind groups,
  inline `create_palette_buffer`, add `queue` parameter to `init`
- `src/world/mod.rs` — store dense `Vec<u8>` instead of `Option<GpuTree64>`
- `src/world/loader.rs` — produce dense array, remove SparseBlocks/tree64 usage
- `src/formats/mod.rs` — new v4 binary format with explicit v3 rejection
- `src/bin/bake.rs` — produce dense array instead of tree64
- `src/lib.rs` — remove `pub mod tree64_renderer`
- `src/framework.rs` — pass `queue` to `App::init` (one-line call-site change)
- `Cargo.toml` — remove `tree64` dependency

**Out of scope** (do NOT touch):
- `src/player_controller.rs` — unchanged
- `src/utils.rs` — unchanged
- `build.rs` — no longer needed for shader compilation, but leave in place;
  it becomes a no-op when slangc is absent (already handled)
- `assets/shaders/tree64_compiled.wgsl`, `tree64.slang`, `tree64_compute.slang`,
  `BackendCommon.slang`, `PerfCounters.slang`, `PerfCounters.h.slang` —
  leave in place (reference files, not loaded at runtime after app.rs changes)
- `src/tree64_renderer.rs` — leave in place for reference, remove from lib.rs

## Git workflow

- Branch: work on current branch (no naming convention enforced in this repo)
- Commit per step or per logical unit; message style matches existing commits
  (imperative present tense, e.g. `add DDA raycast shader`, `switch world format
  to dense voxel array`). Example from `git log`: `"feat(render): add color palette
  from .vox to world format and GPU rendering"`.
- Do NOT push or open a PR.

## Steps

### Step 1: Write the DDA raycast WGSL shader

Create `assets/shaders/dda_raycast.wgsl`. This is a hand-written WGSL shader
(no Slang, no build.rs compilation). It uses a DDA (Amanatides-Woo) grid march
through a `texture_3d<u32>` (`R8Uint` format).

The shader structure:

```wgsl
struct VolumeParams {
    dims: vec3<u32>,         // width, height, depth
    world_origin: vec3<i32>, // AABB min in world space
}

struct CameraUniforms {
    pos: vec4<f32>,
    view_inv: mat4x4<f32>,
    proj_inv: mat4x4<f32>,
}

@group(0) @binding(0) var output_tex : texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(1) var<uniform> camera : CameraUniforms;
@group(0) @binding(2) var voxel_tex : texture_3d<u32>;
@group(0) @binding(3) var<uniform> volume : VolumeParams;
@group(0) @binding(4) var<storage, read> palette : array<vec4<f32>>;
```

The ray-setup logic in `main`:
- Compute NDC from `global_invocation_id` and output texture dimensions
- Build ray in view space: `ndc → projInv → normalize` (this gives the **unnormalized** direction — keep it as `ray_dir_unnorm` for DDA)
- Transform to world space via `viewInv`
- The world-space ray origin is `(viewInv * vec4(0,0,0,1)).xyz`
- Apply the same Y/Z axis swap as the existing shader:
  - Origin: `vec3(rayOrigin.x, -rayOrigin.z, rayOrigin.y)`
  - Direction: `vec3(rayDir.x, -rayDir.z, rayDir.y)`
- **Normalize** the swapped direction to get `ray_dir` for hit-position computation
- Keep a separate `ray_dir_unnorm` (the unnormalized swapped direction) for the DDA — DDA math requires the raw direction vector

The `dda_raycast` function signature and logic:

```
fn dda_raycast(
    ray_origin: vec3<f32>,       // world-space (already axis-swapped)
    ray_dir: vec3<f32>,          // normalized direction (for hit pos)
    ray_dir_unnorm: vec3<f32>,   // unnormalized direction (for DDA step distances)
    volume: VolumeParams,
    voxel_tex: texture_3d<u32>,
    palette: ptr<function, array<vec4<f32>>>,
) -> vec4<f32>
```

1. Convert ray origin to local volume space:
   `local_origin = ray_origin - vec3<f32>(volume.world_origin)`
   `dims_f = vec3<f32>(volume.dims)`

2. Handle rays starting outside the volume: if `local_origin` is outside
   `[0, dims_f)`, advance the ray to the first intersection with the volume
   AABB using a slab test:

   ```
   // Inverse direction for slab test (handle zero components)
   let inv_dir = 1.0 / ray_dir_unnorm;  // may be ±inf, which is fine
   let t0 = (0.0 - local_origin) * inv_dir;
   let t1 = (dims_f - local_origin) * inv_dir;
   let t_min = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
   let t_max = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
   // Miss: t_max < max(t_min, 0.0) → return vec4(0,0,0,1)
   // Entry t = max(t_min, 0.0)
   local_origin = local_origin + ray_dir_unnorm * t_entry;
   // Clamp to [0, dims_f) to avoid fp edge issues
   local_origin = clamp(local_origin, vec3(0.0), dims_f - vec3(0.001));
   ```

3. Initialize DDA state:
   - `voxel = vec3<i32>(floor(local_origin))`
   - `step = vec3<i32>(sign(ray_dir_unnorm))`
   - `t_delta = abs(1.0 / ray_dir_unnorm)` — distance between voxel planes
     (if a direction component is 0, use a large value like 1e30 to avoid infinity)
   - `t_max`: distance to first voxel boundary in each axis:
     ```
     for i in 0..3:
         if ray_dir_unnorm[i] > 0.0:
             t_max[i] = (f32(voxel[i] + 1) - local_origin[i]) * t_delta[i]
         else:
             t_max[i] = (local_origin[i] - f32(voxel[i])) * t_delta[i]
     ```

4. March loop (max 4096 iterations):
   - Bounds check: if `any(voxel < vec3(0)) || voxel.x >= i32(volume.dims.x) || voxel.y >= i32(volume.dims.y) || voxel.z >= i32(volume.dims.z)`, break → sky
   - **CRITICAL**: `textureLoad` on `texture_3d<u32>` returns `vec4<u32>`, not `u32`.
     Extract the red channel: `let mat = textureLoad(voxel_tex, vec3<u32>(voxel), 0).r;`
   - If `mat != 0u`: hit! Compute hit position and normal:
     ```
     let t_hit = min(min(t_max.x, t_max.y), t_max.z);
     let hit_pos = ray_origin + ray_dir * t_hit;  // use normalized dir
     // Normal: the axis with the minimum t_max
     var normal = vec3<f32>(0.0);
     if t_max.x <= t_max.y && t_max.x <= t_max.z { normal.x = -f32(step.x); }
     else if t_max.y <= t_max.x && t_max.y <= t_max.z { normal.y = -f32(step.y); }
     else { normal.z = -f32(step.z); }
     let light_dir = normalize(vec3<f32>(0.5, 1.0, 0.3));
     let lambert = max(dot(normal, light_dir), 0.1);
     let color = palette[mat].rgb * lambert;
     return vec4<f32>(color, 1.0);
     ```
   - Advance to next voxel: find axis with smallest `t_max`, step in that axis, add `t_delta`:
     ```
     if t_max.x < t_max.y {
         if t_max.x < t_max.z {
             voxel.x += step.x; t_max.x += t_delta.x;
         } else {
             voxel.z += step.z; t_max.z += t_delta.z;
         }
     } else {
         if t_max.y < t_max.z {
             voxel.y += step.y; t_max.y += t_delta.y;
         } else {
             voxel.z += step.z; t_max.z += t_delta.z;
         }
     }
     ```

5. After loop: return `vec4<f32>(0.0, 0.0, 0.0, 1.0)` (sky/black).

The final `main` function:
```wgsl
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let target_size = vec2<f32>(textureDimensions(output_tex));
    let ndc = (vec2<f32>(gid.xy) + 0.5) / target_size * 2.0 - 1.0;

    let ray_clip = vec4<f32>(ndc.x, ndc.y, 1.0, 1.0);
    var ray_dir_view = (camera.proj_inv * ray_clip).xyz;
    let ray_dir_world_unnorm = (camera.view_inv * vec4<f32>(ray_dir_view, 0.0)).xyz;
    let ray_origin_world = (camera.view_inv * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;

    // Y/Z axis swap (matching existing shader convention)
    let ray_origin = vec3<f32>(ray_origin_world.x, -ray_origin_world.z, ray_origin_world.y);
    let ray_dir_unnorm = vec3<f32>(ray_dir_world_unnorm.x, -ray_dir_world_unnorm.z, ray_dir_world_unnorm.y);
    let ray_dir = normalize(ray_dir_unnorm);

    let color = dda_raycast(ray_origin, ray_dir, ray_dir_unnorm, volume, voxel_tex, &palette);
    textureStore(output_tex, gid.xy, color);
}
```

Write the complete shader to `assets/shaders/dda_raycast.wgsl`.

**Verify**: `cargo check` — should pass (the shader is not yet referenced).

### Step 2: Change World/WorldFile to store a dense voxel array

In `src/world/mod.rs`:

- Replace `pub tree: Option<GpuTree64>` with:
  ```rust
  pub voxels: Vec<u8>,
  pub dims: [u32; 3],
  pub world_origin: [i32; 3],
  ```
- Remove `use crate::tree64_renderer::GpuTree64;`.
- Update `World::load` to map from the new `WorldFile` fields:
  ```rust
  Ok(Self {
      voxels: world_file.voxels,
      dims: world_file.dims,
      world_origin: world_file.world_origin,
      palette: world_file.palette,
  })
  ```
  (the old code mapped `tree` and `palette`; replace with the above).

In `src/formats/mod.rs`:

- Bump `WORLD_VERSION` from `3` to `4`.
- **Add an explicit v3 rejection** in `WorldHeader::read`, right after the
  `version > WORLD_VERSION` check. Insert:
  ```rust
  if header.version < 4 {
      return Err(io::Error::new(
          io::ErrorKind::InvalidData,
          "world file is v3 (tree64 format); re-bake with `cargo run --bin bake`",
      ));
  }
  ```
  Then remove the existing `if header.version < 3` block (the old v3 check is
  now redundant — any version < 4 is caught above).

- Replace `pub tree: Option<GpuTree64>` with:
  ```rust
  pub voxels: Vec<u8>,
  pub dims: [u32; 3],
  pub world_origin: [i32; 3],
  ```
  Also remove `use crate::tree64_renderer::GpuTree64;` from the top of the file.

- Update `WorldFile::new()`:
  ```rust
  pub fn new() -> Self {
      Self {
          header: WorldHeader::new(false),
          palette: [[0u8; 4]; 256],
          voxels: Vec::new(),
          dims: [0; 3],
          world_origin: [0; 3],
      }
  }
  ```

- Update `WorldFile::write`:
  - Change `WorldHeader::new(self.tree.is_some())` to `WorldHeader::new(!self.voxels.is_empty())`.
  - After `writer.write_all(&palette_bytes)?;`, add:
    ```rust
    writer.write_all(&self.dims[0].to_le_bytes())?;
    writer.write_all(&self.dims[1].to_le_bytes())?;
    writer.write_all(&self.dims[2].to_le_bytes())?;
    writer.write_all(&self.world_origin[0].to_le_bytes())?;
    writer.write_all(&self.world_origin[1].to_le_bytes())?;
    writer.write_all(&self.world_origin[2].to_le_bytes())?;
    writer.write_all(&self.voxels)?;
    ```
  - Remove the entire `if let Some(ref tree) = self.tree { ... }` block.
  - Remove the `use crate::tree64_renderer::GpuTree64;` import.

- Update `WorldFile::read`:
  - After reading palette (1024 B), read dims and world_origin:
    ```rust
    let mut dims_buf = [0u8; 12];
    reader.read_exact(&mut dims_buf)?;
    let dims: [u32; 3] = bytemuck::cast(dims_buf);

    let mut origin_buf = [0u8; 12];
    reader.read_exact(&mut origin_buf)?;
    let world_origin: [i32; 3] = bytemuck::cast(origin_buf);

    let mut voxels = Vec::new();
    reader.read_to_end(&mut voxels)?;
    ```
  - Remove the `if header.tree_present != 0 { ... }` block entirely.

- Remove the entire `#[cfg(test)] mod tests` block from the bottom of the file
  (the existing tests reference `GpuTree64` which no longer exists). The test
  plan section below provides replacement unit tests.

**Verify**: `cargo check` — will fail because `src/bin/bake.rs` and
`src/app.rs` still reference the old types. That's expected, proceed to step 3.

### Step 3: Update the bake binary to produce dense voxel arrays

In `src/world/loader.rs`, replace the tree64-based `build_world_file` with
a dense-array version. Remove everything tree64-related. Here's exactly what
to change:

**Remove these items entirely** (they exist only to feed tree64):
- The constants at the top: `BLOCK_SIZE`, `BLOCK_VOXELS`, `BLOCK_BITS`.
- The `BlockData` struct and the `SparseBlocks` struct and its entire
  `impl SparseBlocks { fn from_world_voxels(...) -> Self { ... } }` block.
- The `impl tree64::VoxelModel<u8> for &SparseBlocks { ... }` block (including
  `fn dimensions` and `fn access`).
- The imports: `use tree64::{Tree64, VoxelModel};` and
  `use crate::tree64_renderer::GpuTree64;`.
  Keep only: `use std::collections::HashMap;`, `use std::time::Instant;`,
  `use dot_vox::...;`, `use glam::...;`, `use rayon::prelude::*;`,
  `use crate::formats::WorldFile;`.

**Replace `build_world_file`** with this version:

```rust
fn build_world_file(
    voxels: HashMap<(i32, i32, i32), u8>,
    palette: [[u8; 4]; 256],
) -> WorldFile {
    if voxels.is_empty() {
        log::warn!("No voxels in world — output will be empty.");
        return WorldFile::new();
    }

    // Compute tight AABB
    let t_aabb = Instant::now();
    let mut bb_min = IVec3::splat(i32::MAX);
    let mut bb_max = IVec3::splat(i32::MIN);
    for &(x, y, z) in voxels.keys() {
        bb_min.x = bb_min.x.min(x);
        bb_min.y = bb_min.y.min(y);
        bb_min.z = bb_min.z.min(z);
        bb_max.x = bb_max.x.max(x);
        bb_max.y = bb_max.y.max(y);
        bb_max.z = bb_max.z.max(z);
    }
    let sx = (bb_max.x - bb_min.x + 1) as u32;
    let sy = (bb_max.y - bb_min.y + 1) as u32;
    let sz = (bb_max.z - bb_min.z + 1) as u32;

    // Round up to powers of two, capped at GPU limits
    let dx = sx.next_power_of_two().min(2048);
    let dy = sy.next_power_of_two().min(2048);
    let dz = sz.next_power_of_two().min(512);

    if sx > 2048 || sy > 2048 || sz > 512 {
        panic!(
            "world dimensions ({sx}, {sy}, {sz}) exceed max (2048, 2048, 512)"
        );
    }

    // Fill dense array
    let t_fill = Instant::now();
    let total = (dx * dy * dz) as usize;
    let mut data = vec![0u8; total];
    for ((x, y, z), color) in &voxels {
        let lx = (x - bb_min.x) as u32;
        let ly = (y - bb_min.y) as u32;
        let lz = (z - bb_min.z) as u32;
        let idx = (lx + ly * dx + lz * dx * dy) as usize;
        data[idx] = *color;
    }

    log::info!(
        "Dense volume: {}×{}×{} = {} voxels, {} MB ({:.2}s AABB + {:.2}s fill)",
        dx, dy, dz,
        voxels.len(),
        total / (1024 * 1024),
        t_aabb.elapsed().as_secs_f32(),
        t_fill.elapsed().as_secs_f32(),
    );

    WorldFile {
        header: crate::formats::WorldHeader::new(true),
        palette,
        dims: [dx, dy, dz],
        world_origin: [bb_min.x, bb_min.y, bb_min.z],
        voxels: data,
    }
}
```

All other functions in `loader.rs` (`collect_instances`, `traverse_recursive`,
`to_transform`, `collect_all_voxels`, `round_up_pow4`) stay as-is — they don't
reference tree64.

In `src/bin/bake.rs`, update the log output after building the world file:

```rust
eprintln!(
    "Dense volume: {}×{}×{} = {} voxels ({} MB)",
    world_file.dims[0], world_file.dims[1], world_file.dims[2],
    world_file.voxels.len(),
    world_file.voxels.len() / (1024 * 1024),
);
```

**Verify**: `cargo check` — still fails (app.rs references tree64). Proceed.

### Step 4: Remove tree64 dependency and module

In `Cargo.toml`, remove the line:
```
tree64 = { git = "https://github.com/expenses/tree64", version = "0.1.0" }
```

In `src/lib.rs`, remove the line:
```rust
pub mod tree64_renderer;
```

Do NOT delete `src/tree64_renderer.rs` — leave it on disk for reference.

**Verify**: `cargo check` — now the only errors should be in `src/app.rs` (it
still imports `GpuTree64Buffers` and calls `create_palette_buffer` from the
now-removed module). Proceed.

### Step 5: Rewrite App::init for 3D texture rendering

In `src/app.rs`, this is the largest change. Replace the tree64 bind group
machinery with a 3D-texture-based bind group.

**5a. Change `App::init` signature to accept `queue`**

The new init needs `queue` for `write_texture`. Change the signature from:
```rust
pub fn init(
    config: &wgpu::SurfaceConfiguration,
    _adapter: &wgpu::Adapter,
    device: &wgpu::Device,
) -> Self {
```
To:
```rust
pub fn init(
    config: &wgpu::SurfaceConfiguration,
    _adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Self {
```

Then in `src/framework.rs`, update the one call site. Find:
```rust
let app = App::init(surface.config(), &context.adapter, &context.device);
```
Replace with:
```rust
let app = App::init(surface.config(), &context.adapter, &context.device, &context.queue);
```

**5b. Remove tree64 imports and fields**

Remove:
```rust
use crate::tree64_renderer::GpuTree64Buffers;
```

Remove these fields from the `App` struct:
- `tree_bind_group: Option<wgpu::BindGroup>`
- `tree_buffers: Option<GpuTree64Buffers>`
- `tree_bind_group_layout: wgpu::BindGroupLayout`

**5c. Add new fields to App struct**

```rust
voxel_texture: wgpu::Texture,
voxel_bind_group: Option<wgpu::BindGroup>,
voxel_bind_group_layout: wgpu::BindGroupLayout,
volume_buffer: wgpu::Buffer,
```

**5d. Inline `create_palette_buffer`**

Since `tree64_renderer` was removed from `lib.rs` in step 4, the function
`create_palette_buffer` no longer exists as an importable path. Add it as
a free function at the top of `src/app.rs`, right after the `use` statements
and before the `CameraUniforms` struct:

```rust
fn create_palette_buffer(device: &wgpu::Device, palette: &[[u8; 4]; 256]) -> wgpu::Buffer {
    let float_palette: [[f32; 4]; 256] = std::array::from_fn(|i| {
        let [r, g, b, a] = palette[i];
        [
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]
    });
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("palette"),
        contents: bytemuck::cast_slice(&float_palette),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}
```

Then change the call site in `App::init` from:
```rust
let palette_buffer = crate::tree64_renderer::create_palette_buffer(device, &world.palette);
```
To:
```rust
let palette_buffer = create_palette_buffer(device, &world.palette);
```

**5e. Add `VolumeParams` struct**

Add above `CameraUniforms`:
```rust
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VolumeParams {
    dims: [u32; 3],        // offset 0,  size 12
    _pad0: u32,            // offset 12, size 4  (std140: vec3<u32> alignment = 16)
    world_origin: [i32; 3], // offset 16, size 12
    _pad1: u32,            // offset 28, size 4  (trailing padding to reach 32)
}
```

The `_pad0` between `dims` and `world_origin` is **critical** — in WGSL uniform
buffers, `vec3<u32>` has alignment 16, so the second `vec3` starts at byte 16,
not 12. Without this padding the GPU reads `world_origin` from the wrong offset.

**5f. Change required features**

```rust
pub fn required_features() -> wgpu::Features {
    wgpu::Features::empty()
}
```

**5g. Create the 3D texture and upload data**

In `App::init`, after creating the palette buffer, replace the entire
`let (tree_buffers, tree_bind_group) = ...` block (the ~30 lines that create
tree buffers and the tree bind group) with:

```rust
let (voxel_texture, voxel_bind_group, volume_buffer) = if !world.voxels.is_empty() {
    let dims = world.dims;
    let size_mb = world.voxels.len() / (1024 * 1024);
    log::info!(
        "Creating 3D voxel texture: {}×{}×{} ({} MB)",
        dims[0], dims[1], dims[2], size_mb,
    );

    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("voxel_volume"),
        size: wgpu::Extent3d {
            width: dims[0],
            height: dims[1],
            depth_or_array_layers: dims[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &world.voxels,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(dims[0]),
            rows_per_image: Some(dims[1]),
        },
        wgpu::Extent3d {
            width: dims[0],
            height: dims[1],
            depth_or_array_layers: dims[2],
        },
    );

    let vol_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("volume_params"),
        contents: bytemuck::bytes_of(&VolumeParams {
            dims: [dims[0], dims[1], dims[2]],
            _pad0: 0,
            world_origin: world.world_origin,
            _pad1: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("voxel_bind_group"),
        layout: &voxel_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: camera_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(
                    &tex.create_view(&wgpu::TextureViewDescriptor {
                        label: Some("voxel_volume_view"),
                        format: Some(wgpu::TextureFormat::R8Uint),
                        dimension: Some(wgpu::TextureViewDimension::D3),
                        ..Default::default()
                    }),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: vol_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: palette_buffer.as_entire_binding(),
            },
        ],
    });

    (tex, Some(bg), vol_buf)
} else {
    log::warn!("No voxels in world — rendering will be blank.");
    // Create a dummy 1×1×1 texture and buffer so the bind group layout
    // is valid even with empty worlds.
    let dummy_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("voxel_volume_dummy"),
        size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let dummy_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("volume_params_dummy"),
        contents: bytemuck::bytes_of(&VolumeParams {
            dims: [1, 1, 1],
            _pad0: 0,
            world_origin: [0, 0, 0],
            _pad1: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("voxel_bind_group_dummy"),
        layout: &voxel_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&rt_view) },
            wgpu::BindGroupEntry { binding: 1, resource: camera_buffer.as_entire_binding() },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(
                    &dummy_tex.create_view(&wgpu::TextureViewDescriptor {
                        label: Some("voxel_volume_view_dummy"),
                        format: Some(wgpu::TextureFormat::R8Uint),
                        dimension: Some(wgpu::TextureViewDimension::D3),
                        ..Default::default()
                    }),
                ),
            },
            wgpu::BindGroupEntry { binding: 3, resource: dummy_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: palette_buffer.as_entire_binding() },
        ],
    });
    (dummy_tex, Some(bg), dummy_buf)
};
```

Note: both branches now return a concrete `wgpu::Texture` (not `Option`).
The `voxel_texture` field stays as `wgpu::Texture`.

**5h. Replace the bind group layout**

Replace the existing 6-entry `tree_bind_group_layout` with a 5-entry layout:

```rust
let voxel_bind_group_layout =
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("voxel_bind_layout"),
        entries: &[
            // binding 0: output storage texture
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
            // binding 1: camera uniform
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<CameraUniforms>() as u64,
                    ),
                },
                count: None,
            },
            // binding 2: 3D voxel texture (R8Uint, read-only)
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            // binding 3: volume params uniform
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<VolumeParams>() as u64,
                    ),
                },
                count: None,
            },
            // binding 4: palette storage buffer
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        (256 * 4 * std::mem::size_of::<f32>()) as u64,
                    ),
                },
                count: None,
            },
        ],
    });
```

**5i. Replace the shader module**

```rust
let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("dda_raycast"),
    source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
        "../assets/shaders/dda_raycast.wgsl"
    ))),
});
```

**5j. Update pipeline layout**

```rust
let compute_pipeline_layout =
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("dda_pipeline_layout"),
        bind_group_layouts: &[Some(&voxel_bind_group_layout)],
        immediate_size: 0,
    });

let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
    label: Some("dda_compute_pipeline"),
    layout: Some(&compute_pipeline_layout),
    module: &compute_shader,
    entry_point: Some("main"),
    compilation_options: Default::default(),
    cache: None,
});
```

**5k. Update `recreate_render_target`**

Replace the existing tree-buffer bind group recreation with voxel bind group
recreation. The new code creates a new `rt_view` from the new `rt_texture`,
then rebuilds `voxel_bind_group` using the existing volume texture, volume
buffer, camera buffer, and palette buffer:

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

    // Rebuild the voxel bind group with the new render target view
    let voxel_view = self.voxel_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("voxel_volume_view"),
        format: Some(wgpu::TextureFormat::R8Uint),
        dimension: Some(wgpu::TextureViewDimension::D3),
        ..Default::default()
    });

    self.voxel_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("voxel_bind_group"),
        layout: &self.voxel_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: self.camera_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&voxel_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: self.volume_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: self.palette_buffer.as_entire_binding(),
            },
        ],
    }));

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

**5l. Update the `App` struct construction at the end of `init`**

```rust
App {
    compute_pipeline,
    voxel_texture,
    voxel_bind_group,
    voxel_bind_group_layout,
    volume_buffer,
    camera_buffer,
    player_controller,
    blit_pipeline,
    blit_view_bind_group,
    last_frame_update: Instant::now(),
    delta_time: Duration::default(),
    surface_width: width,
    surface_height: height,
    rt_texture,
    blit_bind_group_layout: blit_view_bind_group_layout,
    palette_buffer,
}
```

Note: `voxel_texture` is a concrete `wgpu::Texture` from step 5g (both branches
return one), so no `unwrap_or` needed.

**5m. Update the render method**

In `App::render`, change the compute pass to use `voxel_bind_group`:

```rust
if let Some(ref bind_group) = self.voxel_bind_group {
    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("dda_compute_pass"),
        timestamp_writes: None,
    });
    cpass.set_pipeline(&self.compute_pipeline);
    cpass.set_bind_group(0, bind_group, &[]);
    cpass.dispatch_workgroups(workgroup_x, workgroup_y, 1);
}
```

(Replace `self.tree_bind_group` with `self.voxel_bind_group`.)

### Step 6: Build and fix compilation errors

Run `cargo check`. Fix any remaining errors. Common issues:
- Unused import `rayon` in `loader.rs` if block-processing was only used
  by `SparseBlocks::from_world_voxels` — it's still used by `collect_all_voxels`,
  so it should stay.
- `HashMap` import may become unused in `loader.rs` now that `SparseBlocks` is gone —
  no, `collect_all_voxels` still returns `HashMap`.
- References to old field names (`tree`, `tree_buffers`, `tree_bind_group_layout`).

After `cargo check` passes:
- `cargo clippy -- -D warnings` → exit 0
- `cargo fmt --check` → exit 0

### Step 7: Re-bake and run

You need a `.vox` file to bake. The `assets/castle.vox` path referenced in
this plan does **not** exist in the repository — you must source one (e.g.
from a MagicaVoxel export, or a downloaded `.vox` model). Place it at
`assets/castle.vox` (or update the command path below).

```bash
cargo run --bin bake -- assets/castle.vox assets/castle.world
```

Verify the output shows dimensions and size, e.g.:
```
Dense volume: 64×64×32 = 131072 voxels, 0 MB
```

Then:
```bash
cargo run
```

The window should open and render voxels using the DDA march. Fly around (`ZQSD`,
Space/Ctrl, Esc to toggle mouse grab) and verify:
- Voxels are visible
- Colors match the palette
- No visual artifacts (flickering, missing voxels)
- Performance is acceptable (FPS logged to console)

## Test plan

No test suite exists in the repo (see finding #3 in `plans/README.md`). Basic
manual verification:

1. **Empty world**: Delete `assets/castle.world`, run `cargo run`. Should start
   with a blank screen, no crash. Check console for "No voxels in world" warning.
2. **Baking**: `cargo run --bin bake -- assets/castle.vox assets/castle.world`
   should succeed and print dimension info.
3. **Rendering**: Run `cargo run`, confirm voxels render with correct colors.
4. **Clippy/fmt**: Both pass clean.

Add unit tests for the new v4 format roundtrip. In `src/formats/mod.rs`,
at the bottom of the file (after removing the old tests in step 2), add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn world_file_v4_roundtrip() {
        let mut world = WorldFile::new();
        world.dims = [4, 4, 2];
        world.world_origin = [10, 20, 30];
        world.voxels = vec![0u8; 32];
        world.voxels[1 + 2 * 4 + 1 * 4 * 4] = 42; // voxel at (1,2,1) in 4×4×2
        world.header = WorldHeader::new(true);

        let mut buf = Cursor::new(Vec::new());
        world.write(&mut buf).unwrap();
        buf.set_position(0);
        let loaded = WorldFile::read(&mut buf).unwrap();

        assert_eq!(loaded.dims, [4, 4, 2]);
        assert_eq!(loaded.world_origin, [10, 20, 30]);
        assert_eq!(loaded.voxels.len(), 32);
        assert_eq!(loaded.voxels[1 + 2 * 4 + 1 * 4 * 4], 42);
    }

    #[test]
    fn world_file_v4_empty_roundtrip() {
        let world = WorldFile::new();
        let mut buf = Cursor::new(Vec::new());
        world.write(&mut buf).unwrap();
        buf.set_position(0);
        let loaded = WorldFile::read(&mut buf).unwrap();
        assert_eq!(loaded.dims, [0; 3]);
        assert_eq!(loaded.voxels.len(), 0);
    }
}
```

**Verify**: `cargo test` → all tests pass.

## Done criteria

- [ ] `cargo check` exits 0
- [ ] `cargo clippy -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo test` exits 0 (all tests pass, including new v4 roundtrip)
- [ ] `cargo run --bin bake -- <your.vox> assets/castle.world` succeeds
- [ ] `cargo run` renders voxels (visual check)
- [ ] `grep -rn "tree64" src/ Cargo.toml` returns no matches in active code
  (reference files in `assets/shaders/` and `src/tree64_renderer.rs` are OK)
- [ ] `grep -rn "GpuTree" src/` returns no matches in active modules
- [ ] `plans/README.md` status row updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:

- The code at the locations in "Current state" doesn't match the excerpts
  (the codebase has drifted since this plan was written).
- `cargo check` fails after step 6 with an error that is NOT on the
  "expected compilation errors" list for that step.
- `cargo run` starts but the window shows a solid black screen for a known
  non-empty world file (check: the `VolumeParams` padding is wrong and the GPU
  is reading garbage world_origin → use `renderdoc` or add debug logging).
- The shader loop runs past 4096 iterations without a hit (DDA stuck — check
  if `t_delta` is producing NaN from a zero direction component; verify the
  slab test doesn't produce `t_entry = NaN`).
- The 3D texture upload fails with an OOM or size error — the world dimensions
  may exceed GPU limits. Check the logged max 3D texture size from startup.
- Baking fails with a `Vec<u8>` allocation error (world exceeds system RAM).

## Maintenance notes

- **Performance**: The DDA march has no empty-space skipping. A ray through the
  diagonal of a 2048³ volume takes ~3500 steps. If FPS is too low, a natural
  next step is to add a low-res occupancy grid (e.g. 128³) to skip large empty
  regions before doing the fine DDA march.
- **Dynamic updates**: The `write_texture` path works for one-shot uploads. If
  you add runtime voxel editing, switch to `queue.write_texture` on sub-regions
  to avoid re-uploading the entire 2 GB volume.
- **World format v4 is NOT backward compatible** with v3. Old `.world` files
  will fail with "world file is v3 (tree64 format)" — the error message tells
  users to re-bake. No v3 compatibility path is needed.
- **The `slangc`/`build.rs` pipeline** is now dead code — the new shader is
  hand-written WGSL. You can remove `build.rs`, the `.slang` files, and the
  compiled `.wgsl` in a follow-up cleanup plan.
- **`tree64_renderer.rs`** is left on disk but no longer compiled. Delete it
  in a follow-up cleanup once the new renderer is stable.
- **`VolumeParams` std140 layout**: the `_pad0` field is load-bearing. If you
  ever change the WGSL struct to reorder fields, you must update the Rust
  struct's padding to match. A mismatch silently corrupts `world_origin`.

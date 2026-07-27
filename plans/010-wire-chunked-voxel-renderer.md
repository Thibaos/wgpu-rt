# Plan 010: Wire the loaded chunked world into the voxel renderer

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Precondition**: Do not start this plan until plans 008 and 009 both show
> `DONE` in `plans/README.md` and their commits are present in the current
> branch. They are not optional: this plan changes the same `App`/framework
> startup and input path.
>
> **Drift check (run first, after the dependencies are DONE)**:
> `git diff --stat 36b178e..HEAD -- src/app.rs src/render/mod.rs src/world/mod.rs src/world/chunk.rs src/world/loader.rs assets/shaders/aabb_texture.wgsl src/framework.rs`
>
> This plan was written against commit `36b178e`. The exact `experimental_features`
> and Escape-handler changes from plans 008 and 009 are expected drift; preserve
> them. Refresh this plan's SHA and excerpts if either dependency changes any
> other in-scope behavior before execution. Any other change to an excerpt below
> is a STOP condition until the plan is refreshed.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: plans/008-gate-experimental-features.md,
  plans/009-cursor-grab-error-handling.md
- **Category**: bug
- **Planned at**: commit `36b178e`, 2026-07-27

## Why this matters

The repository has a loader, chunk serializer, palette buffer helper, and a
voxel raycast shader, but the running application never calls any of them.
`App::init` currently creates 64 placeholder cube instances, and
`aabb_texture.wgsl` returns a UV gradient from the cube faces. Consequently the
application does not display the loaded `.vox` world at all.

This plan makes the smallest coherent end-to-end renderer: load a small `.vox`
world, apply its centering offset, partition its voxels into the existing
255³ chunks, upload only non-empty chunks as `R8Uint` 3D textures, bind those
textures through the already-requested `TEXTURE_BINDING_ARRAY` feature, and do a
flat voxel DDA in the fragment shader. It keeps the existing AABB-instancing
experiment rather than replacing it with the rejected dense-global-volume
approach or prematurely rebuilding the Tree64 renderer.

## Current state

### Repository vocabulary and constraints

`CONTEXT.md` defines the following terms and invariants:

- A **World** is the loaded voxel world plus its 256-entry palette.
- **Tree64** is the sparse 4-cubed structure used by the planned production
  renderer and collision system; do not rename it to Octree or BVH.
- **Voxel Scale** is `1 voxel = 1/8 meter`; physical dimensions are meters.
- The **Palette** is the 256-entry RGBA8 table converted to GPU `float4` values.

This plan is a rasterizer prototype, not a replacement for the documented
Tree64 production direction. It must use `VOXEL_SCALE = 0.125` for world-space
chunk placement so the camera and voxel renderer agree on units.

### Application pipeline is currently placeholder-only

`src/app.rs` (`App::init`, around lines 65–270) currently:

- takes `config`, `_adapter`, and `device`, but not `queue`;
- requests `TEXTURE_BINDING_ARRAY | SHADER_INT64`; `SHADER_INT64` is not used
  by this raster prototype, while a single-draw texture lookup by flat instance
  ID requires the stable `SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING`
  feature as well as `TEXTURE_BINDING_ARRAY` on wgpu 30;
- creates one uniform containing only `proj * view`;
- creates a bind group with only that uniform;
- creates 64 `Instance`s at positions spaced by `CHUNK_TEXTURE_SIZE * 2`;
- never calls `World::load`, `Chunk::create_texture`, or
  `create_palette_buffer`;
- draws the 64 cubes with `aabb_texture.wgsl`.

The framework calls it here (`src/framework.rs`, inside `resumed`):

```rust
        let app = App::init(surface.config(), &context.adapter, &context.device);
```

`RenderContext::init` currently requests
`App::required_limits().using_resolution(adapter.limits())`. The default
`max_binding_array_elements_per_shader_stage` is zero, but
`using_resolution` does not copy that field from the adapter. Therefore the
device request must be changed in this plan to request a nonzero binding-array
limit for the fixed prototype, capped by the adapter's reported limit. In wgpu
30, a texture binding array contributes to
`max_binding_array_elements_per_shader_stage`, not to the separate
sampled-texture count; do not incorrectly require 64 ordinary sampled-texture
bindings.

The render loop currently updates only a 4x4 matrix:

```rust
        let mx_total = proj_mat * view_mat;
        let mx_ref: &[f32; 16] = mx_total.as_ref();
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(mx_ref));
```

### Chunk representation has two integration defects

`src/world/chunk.rs` currently defines 255³ chunks and `Chunk::to_bytes`, but
`flatten_voxels` adds `position * CHUNK_TEXTURE_SIZE` to every voxel key. That
only works for a global-coordinate map and is wrong once a chunk owns local
coordinates. It also creates a five-mip texture while uploading only mip 0:

```rust
        let desc = wgpu::TextureDescriptor {
            // ...
            mip_level_count: MIP_LEVELS as u32,
            // ...
        };

        let data = self.to_bytes();
        device.create_texture_with_data(queue, &desc, wgpu::wgt::TextureDataOrder::default(), &data)
```

For this MVP use one mip level. `wgpu`'s `create_texture_with_data` utility
accepts tightly packed source mip data and constructs the per-row upload layout;
do not manually add row padding unless the API version in the checkout reports
otherwise.

### World loading computes, but does not apply, its offset

`src/world/loader.rs` returns:

```rust
let (world_offset, voxels) = Self::center_world(voxels);
World { voxels, palette, world_offset }
```

The coordinates in `voxels` are not shifted by `world_offset`. The chunking
step in this plan must add that offset before calculating chunk coordinates.
Use signed Euclidean division (`div_euclid` / `rem_euclid`) so boundaries are
well-defined even if a future loader produces negative coordinates.

`World::load` currently loads a `.vox` path despite the project's `.world`
terminology. For this prototype, keep the existing loader and use
`assets/models/monu1.vox` (small and fast to iterate); do not make the 224 MB
`bistro.vox` file the default smoke-test asset.

### Current instance format

`src/render/mod.rs` currently has a 64-byte matrix only:

```rust
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct InstanceRaw {
    model: [[f32; 4]; 4],
}
```

The vertex buffer uses locations 2–5 for the four matrix columns. The cube
vertices are in `[-1, 1]`, so a scale of half the chunk side is required when a
chunk should occupy exactly one chunk side.

### Current shader

`assets/shaders/aabb_texture.wgsl` has only a camera matrix binding and returns
an unlit debug gradient:

```wgsl
@group(0) @binding(0) var<uniform> camera_transform: mat4x4<f32>;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.tex_coord.x, in.tex_coord.y, 0.0, 1.0);
}
```

The existing `texture_raycast.wgsl` contains a larger mip-DDA, but it targets a
single 1024³ compute texture and cannot be copied unchanged. The MVP here uses
a bounded level-0 DDA against one selected 255³ chunk texture.

### Relevant wgpu API facts

- `TEXTURE_BINDING_ARRAY` is already in `App::required_features()`.
- `SHADER_INT64` is currently requested but no source or shader uses it; remove
  it when updating the feature set.
- A single draw with a flat per-instance chunk ID is a dynamically non-uniform
  texture-array access across overlapping primitives. Add
  `SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING` to
  `App::required_features()` alongside `TEXTURE_BINDING_ARRAY`; stop if the
  adapter does not expose it. This is a source change in `src/app.rs`, not a
  Cargo dependency change.
- A layout entry with `count: Some(NonZeroU32)` declares an array of texture
  bindings.
- In wgpu 30, the bind group resource type is
  `wgpu::BindingResource::TextureViewArray(&texture_view_refs)`, where
  `texture_view_refs` is a slice of references (`&[&wgpu::TextureView]`), not a
  slice of owned views.
- WGSL declares it as
  `binding_array<texture_3d<u32>>` and indexes it with a flat chunk ID. Add
  `enable wgpu_binding_array;` at the top of the WGSL file, as required by
  naga 30 for this native extension.
- Keep `R8Uint`, `TextureSampleType::Uint`, `TextureViewDimension::D3`, and
  `textureLoad` consistent. No sampler is needed.
- `create_texture_with_data` implicitly adds `COPY_DST` and accepts tightly
  packed mip data. For 255-wide `R8Uint` rows, do not pass a hand-padded buffer
  to this helper.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format check | `cargo fmt --check` | exit 0 |
| Typecheck | `cargo check` | exit 0 |
| Lint | `cargo clippy -- -D warnings` | exit 0 |
| CPU tests | `cargo test` | all tests pass |
| Smoke run | `cargo run` | window launches without a validation panic |

Do not use `cargo build` or a development server as a substitute for these
checks. The project instructions prohibit build/dev commands unless explicitly
needed; `cargo check`, clippy, tests, and the short smoke run are sufficient.

## Scope

**In scope** (the only source files this plan may modify):

- `src/world/chunk.rs` — local chunk storage, one-mip texture upload, accessors.
- `src/world/mod.rs` — world-to-chunk partitioning and its CPU tests.
- `src/render/mod.rs` — instance origin data and camera-uniform POD type.
- `src/app.rs` — load the world, create texture views/palette/camera bindings,
  update camera data, and draw compact non-empty instances.
- `src/framework.rs` — pass `queue` to the changed `App::init` signature and
  request the binding-array limits needed by the fixed prototype.
- `assets/shaders/aabb_texture.wgsl` — AABB ray reconstruction, per-chunk DDA,
  palette lookup, and hit depth.
- `plans/README.md` — status row only.

**Out of scope** (do NOT touch):

- `src/player_controller.rs` physics, gravity, collision, or speed tuning;
  those belong to plan 007. The renderer will use the existing fly controller.
- Tree64 data structures, `.world` serialization, or the Tree64 ADR.
- `assets/shaders/texture_raycast.wgsl` and `assets/shaders/blit.wgsl`.
- `Cargo.toml` or dependency versions. The required wgpu feature set in
  `src/app.rs` is in scope only to replace the unused `SHADER_INT64` feature and
  add the explicitly required non-uniform texture-array feature.
- Any framework behavior unrelated to the device limit request, queue plumbing,
  and the plan-009 cursor handling.
- The `.vox` assets themselves.
- A compute renderer, global dense 3D volume, texture atlas, streaming, frustum
  culling, mip-DDA, lighting improvements, or greedy meshing.

## Design to implement

### 1. Partition into compact local chunks

Add this exact public method in `src/world/mod.rs`:

```rust
pub fn into_chunks(self) -> Result<Vec<Chunk>, String>
```

It must return exactly `TOTAL_CHUNKS as usize` chunk slots on success, in the
order already used by the app. On an out-of-range coordinate it must return an
error containing the signed world coordinate, the computed chunk coordinate,
its fixed grid bounds, and no partially populated chunks may be used by the
caller. The caller may use `.expect("world does not fit chunk grid")` only after
this method has completed.

The fixed-grid index is:

```text
index = (chunk_z * CHUNKS_Y + chunk_y) * CHUNKS_X + chunk_x
```

For each `(x, y, z), material` in `self.voxels`:

1. Convert to `i32` and add the corresponding component of `self.world_offset`.
2. Compute `chunk_x = x.div_euclid(255)`, `local_x = x.rem_euclid(255)`;
   repeat for y and z.
3. If the chunk coordinate is outside `0..CHUNKS_X`,
   `0..CHUNKS_Y`, or `0..CHUNKS_Z`, return an error rather than silently
   rendering a clipped World. Use a project-consistent `Result` error string.
4. Insert `(local_x as u8, local_y as u8, local_z as u8) -> material` into
   that chunk.

Add `Chunk::insert`, `Chunk::is_empty`, and a position accessor as needed.
Keep the chunk's stored voxel keys local; `Chunk::to_bytes` must index directly:

```text
index = (local_z * 255 + local_y) * 255 + local_x
```

Do not add the chunk position a second time during flattening.

The `World::into_chunks` method should consume the World so the palette and
chunked data cannot accidentally diverge. In `App::init`, save the palette
before consuming the World:

```rust
let world = World::load("assets/models/monu1.vox")
    .expect("failed to load voxel world");
let palette = world.palette;
let chunks = world.into_chunks().expect("world does not fit chunk grid");
```

Only non-empty chunks should become GPU textures and instances. Creating 64
255³ textures would allocate roughly 1 GiB before overhead, even when most are
empty. Keep the compact list order identical for texture views and instances;
then `@builtin(instance_index)` is the texture-array index.

### 2. Make chunk texture upload valid for the MVP

Change `Chunk::create_texture` to use:

- `mip_level_count: 1`;
- `TextureFormat::R8Uint`;
- `TextureDimension::D3`;
- `TextureUsages::TEXTURE_BINDING` (the helper supplies `COPY_DST`);
- `self.to_bytes()` as the one tightly packed mip payload.

Leave `to_mip_bytes` in place only if it remains used or is intentionally kept
for a later plan; do not claim that the MVP has mip traversal.

Add a CPU test proving a voxel at a known local coordinate appears at the
expected flattened index and that an empty chunk produces exactly
`255 * 255 * 255` bytes. Do not create a GPU device in unit tests.

### 3. Extend instance and camera data

In `src/render/mod.rs`:

- Add `chunk_origin: [f32; 4]` to `InstanceRaw` after the model matrix.
- Add the corresponding `origin` field to `Instance`.
- In `Instance::to_raw`, scale the `[-1,1]` cube by
  `Vec3::splat(CHUNK_SIZE.x * VOXEL_SCALE * 0.5)` and translate it to
  `origin + half_chunk_side`. This makes each cube's world-space AABB exactly
  one 255-voxel chunk wide.
- Preserve the existing column-major matrix layout.

Add a `#[repr(C)]`, `Pod`, `Zeroable` camera struct with the following five
WGSL-compatible fields (the matrices are laid out as four `vec4` columns), for
example:

```rust
pub(crate) struct CameraUniforms {
    pub camera_pos: [f32; 4],
    pub view_inv: [[f32; 4]; 4],
    pub proj_inv: [[f32; 4]; 4],
    pub view_proj: [[f32; 4]; 4],
    pub viewport_and_heatmap: [f32; 4], // width, height, heatmap flag, unused
}
```

The Rust struct and WGSL struct must have the same field order and compatible
alignment. Store the heatmap flag as `0.0` or `1.0` in this MVP; no integer
packing is needed.

Update the instance vertex layout in `src/app.rs` with location 6:

```rust
wgpu::VertexAttribute {
    format: wgpu::VertexFormat::Float32x4,
    offset: 4 * 16,
    shader_location: 6,
}
```

The new stride must be `size_of::<InstanceRaw>()`, not a hard-coded 64.

### 4. Load GPU resources and bind them together

Change `App::init` to accept `queue: &wgpu::Queue`, and update the call in
`src/framework.rs` accordingly.

Use `std::num::NonZeroU32` for the binding-array count. Before creating any
chunk texture, compare the required binding count (at least 1 for the dummy
case, otherwise the non-empty chunk count) with
`adapter.limits().max_binding_array_elements_per_shader_stage`. If that limit
is smaller, stop with a diagnostic containing the limit and required count; do
not silently clip chunks or create a fallback renderer. The ordinary sampled
texture limit need not equal the array length because this layout has one
texture binding-array entry. Pass the adapter to `App::init` under its real name
for this check.

Build a compact `Vec<Chunk>` of non-empty chunks, then for each chunk:

1. Create its 255³ one-mip texture with `chunk.create_texture(device, queue)`.
2. Create a default 3D texture view and retain the texture/view in `App` fields
   for clear ownership and diagnostics.
3. Compute the chunk origin in meters:
   `chunk_grid_position * 255.0 * 0.125`.
4. Create an `Instance` whose origin is that value and whose center is origin
   plus half the chunk side.

If the loaded World is empty, create one zero-filled dummy texture and bind it,
but set `instances` to an empty vector so no draw occurs. The layout still needs
a count of at least one; the dummy prevents a zero-length binding array. The
same minimum-one count applies after filtering, and the preflight limit check
must use that count.

Replace the one-entry bind-group layout with these entries:

- binding 0, vertex + fragment visibility: uniform camera buffer, with
  `min_binding_size` equal to `size_of::<CameraUniforms>()`;
- binding 1, fragment visibility: read-only storage buffer for the 256
  `vec4<f32>` palette;
- binding 2, fragment visibility: `Texture` with `sample_type: Uint`,
  `view_dimension: D3`, `multisampled: false`, and
  `count: Some(NonZeroU32::new(texture_views.len() as u32).unwrap())`.

Create the palette buffer through the existing
`world::create_palette_buffer(device, &palette)` helper. Create one bind group at binding 2 using wgpu 30's actual resource type:
`BindingResource::TextureViewArray(&texture_view_refs)`, where
`texture_view_refs` is a local `Vec<&wgpu::TextureView>` collected from the
owned `Vec<wgpu::TextureView>`. The owned texture views and textures must be
retained in `App` for the entire bind-group lifetime; the local reference vector
only needs to live through `create_bind_group`.

Do not bind the old one-matrix uniform at the same layout; the shader and layout
must be changed as one step.

### 5. Replace the debug shader with a bounded per-chunk DDA

Rewrite `assets/shaders/aabb_texture.wgsl` around this interface:

```wgsl
struct CameraUniforms {
    camera_pos: vec4<f32>,
    view_inv: mat4x4<f32>,
    proj_inv: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    viewport_and_heatmap: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) chunk_id: u32,
    @location(1) @interpolate(flat) chunk_origin: vec3<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<storage, read> palette: array<vec4<f32>>;
@group(0) @binding(2) var voxel_textures: binding_array<texture_3d<u32>>;
```

The vertex shader must:

- reconstruct the matrix from locations 2–5;
- transform the cube position with `camera.view_proj * model_matrix`;
- forward `@builtin(instance_index)` as a flat `u32` chunk ID;
- forward the per-instance origin from location 6 as a flat `vec3<f32>`.

The fragment shader must:

1. Reconstruct a DirectX ray from `@builtin(position).xy`, viewport width and
   height, `camera.proj_inv`, and `camera.view_inv`. Use `camera.camera_pos`
   as the origin. Use these exact equations (pixel origin is the rasterizer's
   top-left origin): `uv = (pixel_xy + vec2(0.5)) / viewport`, `ndc.x =
   uv.x * 2.0 - 1.0`, `ndc.y = 1.0 - uv.y * 2.0`, `clip = vec4(ndc.x,
   ndc.y, 1.0, 1.0)`, `view = camera.proj_inv * clip`, and
   `ray_dir = normalize((camera.view_inv * vec4(view.xyz / view.w, 0.0)).xyz)`.
   Do not swap axes; the rasterizer and player controller use the same glam
   coordinate system. The center pixel must point along the camera's negative-Z
   forward direction when yaw and pitch are zero.
2. Intersect the ray with the chunk AABB
   `[chunk_origin, chunk_origin + vec3(255.0 * 0.125)]`, clamping the near
   value to zero. Handle zero direction components without producing a NaN:
   for each axis, branch on `ray_dir.axis == 0.0`; reject the ray if the origin
   is outside that slab, otherwise use explicitly declared finite sentinels
   `const INF: f32 = 1e30;` and `-INF` for that axis. Only compute
   `(bound - origin) / direction` in the nonzero branch. Do not rely on
   `select` to protect a division because WGSL evaluates both select operands.
3. Convert the entry point to local voxel coordinates and run a standard
   3D Amanatides–Woo DDA through integer coordinates `[0, 254]`. Use a small
   positive entry epsilon (for example `1e-4` voxel) before `floor`, clamp the
   resulting coordinate to `[0, 254]`, and handle a ray starting exactly on a
   chunk boundary according to its direction so a negative-direction ray does
   not re-enter the cell it just left. A point on the exclusive upper AABB face
   must never produce coordinate `255`; treat it as an AABB miss or clamp it to
   the last cell only when the ray is entering from inside.
4. For each visited cell, load
   `textureLoad(voxel_textures[in.chunk_id], voxel, 0).r`.
   Material `0u` is empty, matching the current renderer convention.
5. On the first nonzero material, read `palette[material]`, apply a small
   directional diffuse term, and return it. Keep a bounded maximum step count
   (for example 768) so a malformed ray cannot loop indefinitely.
6. On a miss, execute `discard`; a miss must not write color or depth and must
   allow another overlapping chunk to contribute.
7. Define a fragment output containing both color and `@builtin(frag_depth)`.
   Project the hit world position with `camera.view_proj`, divide by `w`, and
   use the resulting DirectX depth in `[0, 1]`. This is necessary because the
   rasterized AABB depth is not the voxel hit depth. Keep the pipeline depth
   comparison `Less` and depth writes enabled.
8. If `viewport_and_heatmap.z != 0.0`, output a simple step-count heatmap;
   otherwise output the palette color. This makes the existing H toggle affect
   the renderer rather than remaining dead state.

Use `max` and explicit finite checks where supported by naga 30; branch rather
than using `select` around any potentially infinite reciprocal. The shader may
use the declared `INF = 1e30` sentinel and must reject any `isNan` result before
DDA setup. Keep the shader level-0; do not copy the 1024³ mip stack from
`texture_raycast.wgsl`. If the selected naga version does not expose the named
finite-check built-ins, stop with the exact validation error and do not replace
them with an unguarded reciprocal.
Use `@interpolate(flat)` for both the integer ID and origin. If the selected
wgpu/naga version rejects the binding-array indexing or interpolation syntax,
stop and report the exact validation error rather than switching to a different
resource architecture inside this plan.

### 6. Update the pipeline and render loop

In `src/app.rs`:

- Set the instance stride and attributes for the expanded `InstanceRaw`.
- Use `cull_mode: None` while validating the ray/AABB path. Front-face culling
  can hide the relevant projected faces and is not a safe default for a camera
  that may enter a chunk.
- Keep the `Depth32Float` attachment with `CompareFunction::Less` and depth
  writes enabled.
- Update the camera uniform every frame with the current player position,
  inverse view, inverse projection, view-projection, viewport dimensions, and
  `self.heatmap`.
- Continue using the existing projection near/far values (`0.1` and `10000.0`)
  and DirectX projection helper.
- Bind the single combined bind group, expanded instance buffer, and draw
  `self.instances.len()` instances. If it is zero, skip the draw call.

The app currently calls `player_controller.view()` from a mutable controller;
keep that borrowing pattern and do not redesign the controller in this plan.

## Steps

### Step 0: Confirm dependency and feature preconditions

Before changing source, confirm plans 008 and 009 are DONE in
`plans/README.md` and their commits are present. Do not begin on the current
checkout while either dependency is TODO. Plan 008's older statement that
`SHADER_INT64` is needed is superseded by this plan's source audit; do not
preserve that unused feature. Also confirm from the wgpu 30 sources
(or `cargo check` after the feature edit) that
`SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING` is the exact
feature name. The final `App::required_features()` must contain
`TEXTURE_BINDING_ARRAY | SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING`
and must not contain unused `SHADER_INT64`.

The material-0 and asset-size preflight happens immediately after
`World::load` in Step 3, before GPU textures are created: log
`world.voxels.len()`, the count of `world.voxels.values()` equal to `0`, and the
non-empty chunk count. Under this repository's representation, a voxel entry
with material index `0` is always treated as empty (the shader and existing DDA
use `0u` as the empty sentinel), so any nonzero material-0 count is an
objective STOP condition; report that count plus a few loaded coordinates. Do
not invent a separate parser, shift indices, or commit a temporary inspection
program.

**Verify**: `rg -n "Status|008|009" plans/README.md` → both dependency rows
say `DONE`; `rg -n "SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING" C:/Users/Thiba/.cargo/registry/src -g '*.rs'` → the wgpu 30 feature exists.

### Step 1: Implement and test CPU chunk partitioning

Modify `src/world/chunk.rs` and `src/world/mod.rs` as described in Design
sections 1 and 2. Add pure CPU tests for chunk boundaries and flattening.
Tests should cover at least:

- local `(0, 0, 0)` maps to byte index 0;
- local `(254, 254, 254)` maps to the final byte;
- global coordinate 255 maps to chunk coordinate 1 and local coordinate 0;
- negative coordinate `-1` uses Euclidean division (chunk -1/local 254),
  then is rejected by the fixed nonnegative chunk grid;
- a material survives partitioning with its value unchanged.

**Verify**: `cargo test` → all tests pass. `cargo check` → exit 0.

### Step 2: Extend render data types

Modify `src/render/mod.rs` and the instance vertex layout in `src/app.rs`.
Make the Rust camera struct match the WGSL layout before creating the new bind
group. Do not leave the old 64-byte instance stride or old matrix-only uniform
in place.

**Verify**: `cargo check` → exit 0. `cargo fmt --check` → exit 0.

### Step 3: Wire World loading and GPU resources

Change the framework device-request preparation so the requested limits
preserve capacity for `TOTAL_CHUNKS` texture-array elements: after
`App::required_limits().using_resolution(adapter.limits())`, set
`max_binding_array_elements_per_shader_stage` to
`TOTAL_CHUNKS.min(adapter.limits().max_binding_array_elements_per_shader_stage)`.
Do not raise any other limit. If the adapter limit is below one, stop. Then
change `App::init` to accept the queue and a non-underscored adapter reference,
update `framework.rs`, and load
`assets/models/monu1.vox`, partition it, create compact chunk textures/views,
create the palette buffer, create the camera buffer, and create the three-entry
bind group. Retain texture resources and views in `App` fields. Build the
instance buffer from only non-empty chunks.

Log the loaded voxel count, non-empty chunk count, and texture count. If a
texture creation or bind-group creation fails, stop at the validation error;
do not fall back to the old UV renderer.

**Verify**: `cargo check` → exit 0. `cargo clippy -- -D warnings` → exit 0.

### Step 4: Replace the WGSL debug shader

Replace `assets/shaders/aabb_texture.wgsl` with the interface and bounded DDA
specified above. Compile it through the normal `cargo check` shader inclusion
path and resolve only errors directly caused by the specified interface.

**Verify**: `cargo check` → exit 0. This only verifies Rust compilation and
WGSL inclusion, not necessarily runtime naga/backend validation. The bounded
`cargo run` in Step 6 is mandatory and must exercise pipeline creation; record
whether shader validation succeeded or the exact first validation error.

### Step 5: Update camera uploads and pipeline state

Update the per-frame camera data, expanded instance buffer layout, pipeline
culling, and draw path. Ensure the H toggle writes the heatmap flag into the
uniform rather than only logging it.

**Verify**: `cargo fmt --check` → exit 0; `cargo clippy -- -D warnings` → exit 0;
`cargo test` → all tests pass.

### Step 6: Run the visual smoke test

Run:

```text
cargo run
```

Expected log/output:

- the adapter initializes without a missing-feature panic and logs the selected
  backend;
- the model loader reports a nonzero voxel count for `monu1.vox`;
- the app reports the material-0 count, non-empty chunk count, texture count,
  and the binding-array element limit;
- the window shows colored voxel geometry, not the old red/green UV gradient;
- moving with the existing Z/Q/S/D controls changes the view/position;
- pressing H changes the voxel colors to/from the traversal heatmap;
- pressing Escape continues to work without panicking after plan 009.

Run the smoke test for a bounded interval, then close the window and record the
backend, counts, and any first validation error. If the window opens but is
black, capture the first wgpu validation/backend error and stop. Do not increase the camera speed, move the model arbitrarily,
or replace the shader with a different renderer as a workaround. A black frame
can mean a coordinate-system, texture-array, or depth bug that needs diagnosis.

## Test plan

Add CPU-only unit tests in `src/world/mod.rs` and/or `src/world/chunk.rs` using
the project's current inline `#[cfg(test)]` convention (there are no existing
integration tests). Cover chunk boundary mapping, negative Euclidean division,
local flattening, empty chunks, and material preservation. Include a test that
constructs a `World` with a known voxel and a nonzero `world_offset`, then
asserts the exact resulting chunk coordinate, local coordinate, and flattened
byte index. This test must fail if the offset is omitted or applied twice. Do
not add GPU tests; portable adapter/device setup is not currently established
in this repository.

Verification commands:

- `cargo test` → all CPU tests pass;
- `cargo fmt --check` → exit 0;
- `cargo clippy -- -D warnings` → exit 0;
- `cargo run` → visual smoke check described in Step 6.

## Done criteria

- [ ] `World::load("assets/models/monu1.vox")` is reached by `App::init`.
- [ ] `world_offset` is applied exactly once before chunk partitioning.
- [ ] Chunk coordinates and local coordinates use `div_euclid`/`rem_euclid`.
- [ ] `Chunk::to_bytes` stores local coordinates without adding position again.
- [ ] MVP chunk textures have `mip_level_count: 1` and upload exactly one mip.
- [ ] Only non-empty chunks receive textures and draw instances; empty worlds
      use one dummy binding but zero draw instances.
- [ ] `InstanceRaw` contains the model matrix and chunk origin, and the vertex
      stride/layout matches its actual size.
- [ ] The bind group contains camera uniform, palette storage buffer, and a
      fixed-size 3D texture binding array.
- [ ] The shader no longer returns the UV debug gradient.
- [ ] Fragment DDA samples the texture selected by flat instance ID and uses
      the palette buffer for the final color.
- [ ] Hit depth is written from the voxel hit position, not the AABB surface.
- [ ] `cargo test`, `cargo check`, `cargo clippy -- -D warnings`, and
      `cargo fmt --check` succeed.
- [ ] `App::required_features()` requests exactly the two texture-array
      features needed by this plan and no unused `SHADER_INT64` feature.
- [ ] No files outside the Scope list are modified.
- [ ] `plans/README.md` status row is updated.

## STOP conditions

Stop and report without improvising if:

- Any Current state excerpt differs for a reason other than the exact planned
  changes from plans 008 or 009.
- `monu1.vox` cannot be loaded or `World::into_chunks` reports that the model
  exceeds the fixed 8×1×8 chunk grid. Do not silently clip the World; report the
  bounds and make a separate sizing decision.
- The loaded world contains any voxel with material index 0. Under the fixed
  sentinel convention this is a data-format mismatch; report the count and
  representative loaded coordinates before changing the representation.
- `TEXTURE_BINDING_ARRAY` or
  `SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING` is unavailable
  on the selected adapter, the device request fails, or wgpu rejects the 3D
  texture binding array. Do not add a fallback renderer or remove either
  required feature in this plan.
- WGSL validation rejects dynamic indexing of `binding_array<texture_3d<u32>>`,
  flat interpolation, or `frag_depth`. Capture the exact error and stop.
- A texture upload validation error indicates a different `wgpu 30` layout API
  than the one described here. Report the exact API/error rather than guessing
  at row padding or changing the dependency.
- Any step requires modifying Tree64, player physics, the `.world` format,
  `Cargo.toml`, or an out-of-scope shader.
- The smoke run is black or triggers a wgpu validation error. Record the first
  error and the adapter/backend; do not mask it by returning a debug color.
- The adapter's `max_binding_array_elements_per_shader_stage` is smaller than
  the required texture count. Report the limit and count rather than clipping
  the World.
- The asset preflight finds any material-0 voxel intended to be visible.
- The final feature request would require `SHADER_INT64`, blanket experimental
  features, or a feature not exposed by the selected adapter.

## Maintenance notes

- This is deliberately a correctness prototype. A 255³ level-0 DDA per
  projected chunk can be expensive, especially when chunk AABBs overlap. The
  next renderer decision should measure it before adding mip traversal or
  switching to a storage-buffer pool.
- The fixed 8×1×8 grid is not a scalable World format. A future chunk-manager
  plan should make grid dimensions data-driven and stream only visible chunks.
- The texture binding array count is fixed when the bind group layout is
  created. Streaming or changing the non-empty chunk set requires rebuilding
  the layout/bind group; do not mutate the texture vector and assume the GPU
  binding changes automatically.
- The shader treats material 0 as empty and indexes the palette directly. A
  future format plan must document whether `.vox` material indices are shifted
  or whether zero is reserved.
- The documented production architecture still favors Tree64 for sparse GPU
  traversal and CPU collision. This rasterized dense-chunk path should not be
  presented as replacing that ADR without a new decision record.
- Reviewers should scrutinize coordinate units, `world_offset` application,
  binding-array indexing, texture lifetime, and fragment depth. These are the
  places where a visually plausible but incorrect renderer is most likely.

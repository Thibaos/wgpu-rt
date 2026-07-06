# Plan 001: Recreate RT output texture on window resize

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat d31e828..HEAD -- src/app.rs src/tree64_renderer.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `d31e828`, 2026-07-06

## Why this matters

After any window resize, the compute shader dispatch dimensions (updated in
`render()` from `surface_width`/`surface_height`) mismatch the storage texture
dimensions (created once in `init()` at original window size). Each compute
thread writes to `textureStore(output, globalId.xy, ...)` using coordinates up
to the new surface size, but the storage texture still has the original size.
wgpu validates this mismatch — on some backends it crashes with a validation
error; on others it writes out-of-bounds silently. Either way, the app is
effectively non-resizable.

## Current state

- `src/app.rs:328-332` — `App::resize()` only stashes `surface_width`/`surface_height`. Does not recreate `rt_texture`, `rt_view`, or the two bind groups that reference `rt_view`.
- `src/app.rs:80-93` — `App::init()` creates `rt_texture` at initial window size from `config.width`/`config.height`, then `rt_view` from it.
- `src/app.rs:202-230` — `tree_bind_group` (binding 0 = `rt_view`, bindings 1-4 = tree/camera buffers).
- `src/app.rs:277-284` — `blit_view_bind_group` (binding 0 = `rt_view`).
- `src/app.rs:364-366` — Compute dispatch uses current `surface_width`/`surface_height` for workgroup count.
- `src/app.rs:383` — Blit pass binds `blit_view_bind_group` (stale `rt_view`).

The `tree_bind_group_layout`, `compute_pipeline_layout`, `compute_pipeline`,
and `blit_pipeline` do NOT depend on the texture size and do not need to be
recreated. Only the texture, its view, and the two bind groups must be rebuilt.

The format (`Rgba8Unorm`) does not change on resize and stays a constant
(`TextureFormat::Rgba8Unorm`).

## Commands you will need

| Purpose  | Command                       | Expected on success   |
|----------|-------------------------------|-----------------------|
| Build    | `cargo build`                 | exit 0, "Finished"    |
| Format   | `cargo fmt --check`           | exit 0, no diff       |
| Lint     | `cargo clippy -- -D warnings` | exit 0, no warnings   |
| Test     | `cargo test`                  | exit 0, 0 tests run   |

## Scope

**In scope** (the only files you should modify):
- `src/app.rs`

**Out of scope** (do NOT touch, even though they look related):
- `src/tree64_renderer.rs` — tree buffers and `GpuTree64Buffers` are correct, don't need resize.
- `src/framework.rs` — `SurfaceWrapper::resize` is correct; the surface config is updated there.
- `assets/shaders/` — shaders don't reference texture dimensions.
- Any change to the `TextureFormat` or addition of mipmaps.

## Git workflow

- Branch: `advisor/001-resize-rt-texture`
- Commit per step; message style: `fix: <description>` (matching observed style from `d31e828 fix voxel axis lookup`)
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Extract a helper to rebuild resize-dependent GPU resources

In `src/app.rs`, add a private method `recreate_render_target` that creates
the `rt_texture`, `rt_view`, `tree_bind_group`, and `blit_view_bind_group`
from the current `surface_width` and `surface_height`. This avoids
duplicating the texture/bind-group creation logic between `init` and `resize`.

The method signature:

```rust
fn recreate_render_target(
    &mut self,
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    tree_buffers: &crate::tree64_renderer::GpuTree64Buffers,
) {
    // create rt_texture, rt_view, tree_bind_group, blit_view_bind_group
    // assign to self.rt_texture etc. — but we'd need to add these fields to App
}
```

Wait — `App` doesn't currently store the `rt_texture` as a field. It stores
`tree_bind_group` and `blit_view_bind_group` (which own ref-counted references).
We need to also store `rt_texture` so it stays alive, and store the
`tree_bind_group_layout` so we can recreate the bind groups.

So first, add these fields to `App`:

```rust
pub struct App {
    // existing fields ...
    compute_pipeline: wgpu::ComputePipeline,
    tree_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    pub player_controller: PlayerController,
    blit_pipeline: wgpu::RenderPipeline,
    blit_view_bind_group: wgpu::BindGroup,
    last_frame_update: Instant,
    delta_time: Duration,
    surface_width: u32,
    surface_height: u32,

    // NEW fields:
    rt_texture: wgpu::Texture,
    tree_bind_group_layout: wgpu::BindGroupLayout,
    blit_bind_group_layout: wgpu::BindGroupLayout,
}
```

In `init()`, after creating each of these resources, store them as fields.
The `compute_pipeline_layout` does NOT need to be stored since the layout
never changes (it only depends on `tree_bind_group_layout` which is invariant
w.r.t. size).

Then add the helper method. It re-creates only the size-dependent resources:
`rt_texture`, `rt_view`, `tree_bind_group`, `blit_view_bind_group`. The
`tree_bind_group_layout` and `blit_bind_group_layout` are reused from `self`.

**Important**: `rt_texture` must be dropped before the new one is created.
In Rust, assigning to `self.rt_texture = device.create_texture(...)` will drop
the old texture automatically. Same for bind groups.

The helper (`recreate_render_target`):

```rust
fn recreate_render_target(&mut self, device: &wgpu::Device) {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let width = self.surface_width;
    let height = self.surface_height;

    // Recreate output texture at new size
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

    // Recreate tree bind group (new rt_view, same tree/camera buffers)
    self.tree_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tree64_bind_group"),
        layout: &self.tree_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            },
            // Bindings 1-4 are stored in self — but wait, tree_buffers and
            // camera_buffer aren't fields of App. We need them.
        ],
    });

    // Recreate blit bind group
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

**Problem**: bindings 1-4 (tree params, camera, tree nodes, leaf data) are
created as `tree_buffers` in `init()` but not stored in `App`. We need to
store them. Add these fields:

```rust
tree_params_buffer: wgpu::Buffer,
tree_nodes_buffer: wgpu::Buffer,
tree_leaf_data_buffer: wgpu::Buffer,
```

Then in `init()`, extract them from `tree_buffers`:

```rust
let tree_buffers = gpu_tree.create_buffers(device);

self.tree_params_buffer = tree_buffers.params;
self.tree_nodes_buffer = tree_buffers.nodes;
self.tree_leaf_data_buffer = tree_buffers.leaf_data;
```

Now the `tree_bind_group` entries for bindings 1-4 can reference `self.tree_params_buffer.as_entire_binding()` etc.

**Verify**: `cargo build` → exit 0, "Finished"

### Step 2: Call `init()` with the new field assignments

Update `App::init()`:
- Store `tree_bind_group_layout`, `blit_bind_group_layout` as `self` fields
- Store `rt_texture` as `self.rt_texture`
- Store `tree_params_buffer`, `tree_nodes_buffer`, `tree_leaf_data_buffer` as `self` fields
- Build `tree_bind_group` and `blit_view_bind_group` using the stored layouts and buffers
- Remove the old code that creates these in the return expression

The `App` struct return at the end of `init()` becomes:

```rust
App {
    compute_pipeline,
    tree_bind_group,
    camera_buffer,
    player_controller,
    blit_pipeline,
    blit_view_bind_group,
    last_frame_update: Instant::now(),
    delta_time: Duration::default(),
    surface_width: width,
    surface_height: height,
    rt_texture,
    tree_bind_group_layout,
    blit_bind_group_layout,
    tree_params_buffer,
    tree_nodes_buffer,
    tree_leaf_data_buffer,
}
```

The `tree_bind_group` and `blit_view_bind_group` creation code in `init()` now
references `self.tree_bind_group_layout` etc. — but since `self` doesn't exist
yet (we're inside `init()` returning `Self`), create them with local variables
first, then assign:

```rust
// ... create layouts, buffers, pipelines as locals ...

// Build bind groups using local references to the layouts
let tree_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("tree64_bind_group"),
    layout: &tree_bind_group_layout,
    entries: &[
        // binding 0: rt_view (local)
        // bindings 1-4: buffers (locals, stored for later)
    ],
});

// ... same for blit ...

App {
    // ...
    tree_bind_group,
    blit_view_bind_group,
    rt_texture,
    tree_bind_group_layout,
    blit_bind_group_layout,
    tree_params_buffer: tree_buffers.params,
    tree_nodes_buffer: tree_buffers.nodes,
    tree_leaf_data_buffer: tree_buffers.leaf_data,
}
```

**Verify**: `cargo build` → exit 0, "Finished"

### Step 3: Implement `recreate_render_target` helper

Add the helper method to `impl App`. It:

1. Creates a new `rt_texture` at `self.surface_width` × `self.surface_height`.
2. Creates a new `rt_view` from it.
3. Creates a new `tree_bind_group` using `self.tree_bind_group_layout` with the new `rt_view` at binding 0 and the stored tree buffers at bindings 1-4.
4. Creates a new `blit_view_bind_group` using `self.blit_bind_group_layout` with the new `rt_view` at binding 0.
5. Assigns all four to `self` fields.

Full method:

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

    self.tree_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tree64_bind_group"),
        layout: &self.tree_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: self.tree_params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: self.camera_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: self.tree_nodes_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: self.tree_leaf_data_buffer.as_entire_binding(),
            },
        ],
    });

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

**Verify**: `cargo build` → exit 0, "Finished"

### Step 4: Call `recreate_render_target` in `resize()`

In `App::resize()`, after updating `self.surface_width` and `self.surface_height`, call:

```rust
pub fn resize(
    &mut self,
    config: &wgpu::SurfaceConfiguration,
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
) {
    self.surface_width = config.width;
    self.surface_height = config.height;
    self.recreate_render_target(device);
}
```

**Verify**: `cargo build` → exit 0, "Finished"

### Step 5: Final check

Run the full verification suite:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo build
cargo test
```

All must exit 0.

## Test plan

- No new automated tests (GPU-requiring integration tests for resize are impractical in CI).
- Manual verification: launch `cargo run`, resize the window. The rendered content should scale correctly without crashes or artifacts.
- The resize codepath is exercised by `winit` on every OS-level window resize event, so any glaring issue surfaces immediately.

## Done criteria

- [ ] `cargo build` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -- -D warnings` exits 0
- [ ] `cargo test` exits 0
- [ ] `grep -rn "rt_texture\|rt_view\|tree_bind_group\|blit_view_bind_group" src/app.rs` shows they are all now fields of `App` and used in both `init()` and `recreate_render_target()`
- [ ] `grep "recreate_render_target" src/app.rs` finds the method definition and a call in `resize()`
- [ ] No files outside `src/app.rs` modified (`git diff --stat` shows only `src/app.rs`)
- [ ] `plans/README.md` status row updated to DONE

## STOP conditions

Stop and report back (do not improvise) if:

- The code at the locations in "Current state" doesn't match the excerpts
  (the codebase has drifted since this plan was written).
- A step's `cargo build` fails twice after a reasonable fix attempt.
- Storing the bind group layout in `App` causes a lifetime issue (the layout
  borrows from the device, but the device outlives App — this should work,
  but if it doesn't, report the exact compiler error).
- The fix appears to require touching `src/framework.rs` or `src/tree64_renderer.rs`.

## Maintenance notes

- If additional bindings are added to the compute shader (e.g., a second
  storage texture for normals or depth), the `recreate_render_target` helper
  must be updated to include them.
- If the texture format changes from `Rgba8Unorm`, update the `format` local
  in `recreate_render_target` — or promote it to a constant on `App`.
- The `tree_bind_group_layout` and `blit_bind_group_layout` fields are stored
  because `wgpu::BindGroupLayout` is cheap to clone (ref-counted). No perf
  concern.
- The old texture and bind groups are automatically dropped on reassignment;
  no explicit cleanup needed.

# Plan 003: Depth-exit — terminate the chunk DDA at the previous frame's nearest surface

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. Save safety copies before editing
> (`cp` the three files to `/tmp/`), keep the diff uncommitted for review, and
> do NOT touch `plans/README.md` or `advisor-plans/README.md`.
>
> **Drift check (run first)**:
> `git diff --stat 1817d7d..HEAD -- assets/shaders/chunk.wgsl src/app.rs src/render/mod.rs src/framework.rs`
> Expected output is empty (the tree should be at `1817d7d`, the TraversalFrame
> compaction, plus nothing else). Then compare the "Current state" excerpts
> below against the live files. On any mismatch, STOP and report the exact
> changed path and difference.

## Status

- **Priority**: P1
- **Effort**: M — one uniform extension, one bind-group pair, ping-pong depth swap, ~30 lines of shader math
- **Risk**: MED — touches the render path's per-pixel behavior at occlusion boundaries; the traversal algorithm is untouched (only the root frame's interval exit is shortened), and every gate is A/B-measured and revertible
- **Depends on**: `1817d7d` (TraversalFrame compaction) and the profiling harness (`e628648`)
- **Category**: perf
- **Planned at**: 2026-08-01, session after handoff `wgpu-rt-handoff-2026-08-01.md`
- **Issue**: (none)

## Why this matters

Nsight on the compacted shader (RTX 3070, release): SM throughput 11.8%,
occupancy 29%, and launch stalls split ~equal thirds — register allocation
28.8%, **OOO warp completion 27.3%**, slot allocation 22.8%. The dominant
remaining inefficiency is warp divergence: a warp's 32 lanes mix short rays
(sky, ~1-2 cells) and long rays (grazing descents through occupied structure,
15+ cells, up to the caps), so stragglers block slot reuse. Locality is dead
(L2 83.9%); registers are largely spent; the remaining lever is making warp
durations uniform.

**Depth-exit** does that: the previous frame's depth buffer is a per-pixel
upper bound on the ray. The chunk traversal's root interval exit is clamped to
the distance of the nearest surface seen last frame at that pixel (with a
reprojection consistency check for the orbit camera). A ray that would spend
tens of cells grazing through structure behind a surface now stops the moment
it reaches that surface's distance — every terminated pixel costs ~0-1 cells,
uniform with sky. Terminated pixels discard; in the multi-instance front-to-
back draw the earlier (nearer) instance's depth+color already covers them, so
the image is correct where a surface is actually visible.

Measured prize (from the handoff, release bench): bistro_sm (5 chunks) is the
divergence-dominated scene at 58.54 ms GPU mean (17 fps); monu1 (1 chunk) at
9.7-10.3 ms (93 fps) is the latency-sensitive sanity case — the depth sample
added to every fragment's critical path could regress it slightly, which is
explicitly allowed and reported (gate below).

## Current state

All excerpts are from the working tree at `1817d7d`. Verify each before
editing.

- `assets/shaders/chunk.wgsl` — the compacted shader. `CameraUniforms`
  (lines 6-14) is `camera_pos, view_inv, proj_inv, view_proj,
  viewport_and_heatmap`. `VertexOutput` already carries
  `@builtin(position) position` (line 25), so the fragment has
  `in.position.xy` = screen pixel coords. Bindings: group 0 = camera,
  group 1 = `palette` (0), `voxel_textures` binding array (1). No sampler
  exists anywhere in the app today (the DDA uses `textureLoad` only).
- `fs_main` (lines ~255-270): after the `dot(delta,delta)` guard it computes
  `span = ray_aabb(origin, dir, bmin, bmax)`, discards on `span[1] < 0.0`
  and on `span[1] - span[0] <= T_EPS`, then at ~line 280:
  ```wgsl
  frames[0] = init_frame(origin, dir, chunk_origin, ROOT_MIP, vec3<i32>(0), span);
  ```
  This is the ONLY call site whose interval must change. `init_frame` and the
  traversal loop (`advance_frame`, push logic, caps, `TRAVERSAL_BOUND`) are
  NOT modified — the bound only shortens the root frame's `interval.y`, and
  children inherit `child_exit = min(parent_next, top.interval.y)`.
- `src/app.rs`:
  - `create_depth_texture` (lines 72-88): `Depth32Float`,
    `usage: RENDER_ATTACHMENT` (add `TEXTURE_BINDING`), `view_formats:
    &[Depth32Float]`. Called from `init` (~line 526) and `resize` (~line 633);
    `depth_texture` field at line 44.
  - Render pass (lines 745-775): depth attachment = `self.depth_texture`
    view, `depth_ops: Clear(1.0) / Store`. Timestamp queries unchanged.
  - Camera uniforms built at lines 687-700: `view_proj = proj_mat * view_mat`,
    then `view_inv`, `proj_inv`; `viewport_and_heatmap =
    [surface_width, surface_height, heatmap, 0.0]`. Uploaded via
    `queue.write_buffer` at line 704. The uniform buffer is
    `size_of::<CameraUniforms>()` bytes (line 252).
  - Resource bind group layout at lines 300-341: entries 0 (palette storage),
    1 (texture array), then `if stats_enabled { binding 2 (stats storage) }`.
    The matching bind group at lines 344-394 pushes `palette`, texture views,
    and (stats) the stats buffer; it is created ONCE in `init` and stored as
    a field. **This plan leaves it untouched** and adds a THIRD bind group
    (group 2, depth-only) — see Step 2.2 item 5.
  - Instance sort comment at lines 710-714 ("With the frag_depth write
    removed...") is STALE (the early-Z experiment was reverted; the shader
    still writes frag_depth). Do not trust it; leave it alone (see
    Maintenance notes).
- `src/render/mod.rs` — `CameraUniforms` (lines 45-51):
  ```rust
  pub(crate) struct CameraUniforms {
      pub camera_pos: [f32; 4],
      pub view_inv: [[f32; 4]; 4],
      pub proj_inv: [[f32; 4]; 4],
      pub view_proj: [[f32; 4]; 4],
      pub viewport_and_heatmap: [f32; 4],
  }
  ```
- Bench protocol (from the handoff): `cargo build --release --bin bench` then
  `WGPU_RT_WORLD=assets/models/<scene>.vox WGPU_RT_STATS=0|1
  ./target/release/bench.exe <frames>`. `WGPU_RT_PROFILE=1` (default in the
  bench) logs per-second lines with gpu ms, frags, cells, hits plus
  `Orbit: t=...` lines (the orbit is a pure function of time, az_period 60 s).
  The bench is GPU-paced (blocking Wait poll per frame), so wall time ≈
  frames × frame time.

### Design decisions (locked in by the user on 2026-08-01)

1. **Ping-pong depth textures** — two `Depth32Float` textures; the pass
   writes `depth_cur`, the shader samples `depth_prev`; swap after the pass.
   Zero copy-engine traffic (the handoff flagged CE at 100% in the debug
   trace). Costs +8.3 MB VRAM at 1080p and `TEXTURE_BINDING` usage.
2. **Depth-only, discard** — a pixel whose DDA is terminated by the bound
   discards (background shows; in multi-instance draws, nearer instances
   already wrote depth+color there). No color feedback texture in this plan.
   Known acceptable artifact: a ray slipping through a sub-voxel gap shows
   background instead of geometry behind the gap (grazing angles only) — the
   smoke gate judges visibility; a color-feedback follow-up is the fallback.
3. **Consistency check** — project-reproject in pixel space: unproject the
   previous frame's depth at this pixel to a world point, project it into the
   current view, and accept the bound only if it lands within
   `REPROJECT_PX` pixels of this pixel (plus `t > 0` and `w > 0` guards).
   This makes the bound self-invalidating on reveals/camera cuts — those
   pixels fall back to full traversal.

### Repo conventions to follow

- **Tests**: `cargo test` must pass — `chunk_wgsl_parses_and_validates`
  (naga) and `hierarchical_mip_dda` (CPU reference, untouched). No new Rust
  unit tests; correctness evidence = naga + CPU reference + visual smoke +
  stats-leg cells/hits comparison.
- **Logging**: `log::info!` plain messages.
- **Code style**: `cargo fmt`; `glam` on the CPU; WGSL matches the existing
  comment style in `chunk.wgsl`.
- **Vocabulary** (CONTEXT.md): "DDA", "Mip level", "Chunk", "Voxel Scale".
- **Git workflow**: leave the implementation diff uncommitted for review.
  Do not commit; do not touch `plans/README.md` or `advisor-plans/README.md`.

## Commands you will need

| Purpose | Command | Expected result |
|---|---|---|
| Drift check | `git diff --stat 1817d7d..HEAD -- assets/shaders/chunk.wgsl src/app.rs src/render/mod.rs src/framework.rs` | Empty |
| Check | `cargo check` | exit 0, no warnings |
| Build (debug) | `cargo build` | exit 0 |
| Build (bench) | `cargo build --release --bin bench` | exit 0 |
| Shader gate | `cargo test --test shader_validate` | exactly 1 test passes |
| DDA reference | `cargo test --test hierarchical_mip_dda` | all pass |
| Full gates | `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings` | all exit 0 |
| Perf capture | `WGPU_RT_WORLD=assets/models/<scene>.vox WGPU_RT_STATS=0 ./target/release/bench.exe <frames> > /tmp/bench_<tag>.log 2>&1` | profile + Orbit lines; wall ≈ frames × frame ms |
| Stats capture | same with `WGPU_RT_STATS=1` | cells/frag + hits/frame (disables early-Z) |
| Smoke run | `WGPU_RT_ORBIT=1 cargo run > /tmp/smoke_<tag>.log 2>&1 &` then watch the window ~60 s, then `kill <pid>` | window opens, orbit logs 1 Hz |

**Capture hygiene (learned from plans 001-002)**: run captures sequentially,
never in parallel; close/minimize other windows so the display shows only the
test window; always kill the bench (it never exits on its own — it runs a
fixed frame count and does exit, so just wait). If a capture is aborted
early, re-run it — do not reuse partial logs.

**Frame counts**: the bench is GPU-paced, so to cover one full 60 s orbit at
a given frame time use `frames = 70_000 / expected_frame_ms` (rounded up),
and verify the log's last `Orbit: t=` is ≥ 55 s (if not, rerun with more
frames). Table: monu1 baseline ≈ 10 ms → 7000; bistro baseline ≈ 58 ms →
1200; after depth-exit use the measured baseline of the leg (bistro may fall
to ~35 ms → 2000, monu1 may fall to ~8 ms → 9000). Both A/B legs must each
cover ≥ 55 s of orbit time; the first profile sample (startup frame) is
excluded from parsing (same as plan 001).

## Scope

**In scope — the only files to modify:**

- `assets/shaders/chunk.wgsl` — `CameraUniforms` +2 fields, a new group-2
  depth bind group (`texture_depth_2d` at binding 0, sampler at binding 1),
  one helper `depth_exit_bound`, and the root frame's interval clamp in
  `fs_main`. The traversal algorithm (constants, `init_frame`,
  `advance_frame`, the loop, caps) stays byte-for-byte.
- `src/app.rs` — depth ping-pong (`depth_textures: [Texture; 2]` +
  `depth_write_index`), `create_depth_texture` usage +1, a NEW depth-only
  bind group + layout (group 2, pre-created once per texture, selected by
  write index), `prev_view_proj_inv` retention, uniform upload of the new
  fields.
- `src/render/mod.rs` — `CameraUniforms` +2 fields (must match the WGSL
  struct exactly).

**Out of scope — do not touch:**

- `src/world/*`, `src/framework.rs`, `src/main.rs`, `src/bin/bench.rs`,
  `Cargo.toml`/`Cargo.lock` (no new dependencies), `tests/*`, `plans/`,
  `docs/`, `CONTEXT.md`, assets/models.
- The traversal constants, caps, and the ADR-0002 traversal contract.
- Half-resolution DDA + upscale and the compute-shader path — deferred (this
  plan stacks under both: fewer/cheaper fragments make each of them better).
- Color feedback for terminated pixels — deferred to a follow-up plan gated
  on this plan's smoke results.

## Steps

### Step 1: Baseline captures (anchor the A/B)

Goal: fresh numbers at `1817d7d` with the same protocol the depth-exit legs
will use, so the A/B is comparable on this machine.

1. `cargo build --release --bin bench`.
2. For each scene (monu1, bistro_sm), run stats=0 and stats=1 legs with the
   frame counts from the table above. E.g.:
   ```
   WGPU_RT_WORLD=assets/models/monu1.vox  WGPU_RT_STATS=0 ./target/release/bench.exe 7000 > /tmp/bench_base_monu1_s0.log 2>&1
   WGPU_RT_WORLD=assets/models/monu1.vox  WGPU_RT_STATS=1 ./target/release/bench.exe 7000 > /tmp/bench_base_monu1_s1.log 2>&1
   WGPU_RT_WORLD=assets/models/bistro_sm.vox WGPU_RT_STATS=0 ./target/release/bench.exe 1200 > /tmp/bench_base_bistro_s0.log 2>&1
   WGPU_RT_WORLD=assets/models/bistro_sm.vox WGPU_RT_STATS=1 ./target/release/bench.exe 1200 > /tmp/bench_base_bistro_s1.log 2>&1
   ```
3. Parse stats=0 legs (GPU ms mean) with the plan-001 command:
   ```bash
   grep -oE "GPU render pass: [0-9.]+ms" /tmp/bench_base_monu1_s0.log | grep -oE "[0-9.]+" | awk '{s+=$1; n++; if(n==1||$1<m)m=$1; if($1>M)M=$1} END{printf "GPU ms n=%d min=%.2f max=%.2f avg=%.2f\n", n, m, M, s/n}'
   ```
   (repeat for bistro). From the stats=1 logs, extract cells/frag and
   hits/frame (profile lines carry frags, cells, hits; cells/frag =
   cells/frags, hits/frame = hits per profile line).
4. **Sanity**: monu1 avg GPU ms ≈ 9.7-10.3, bistro ≈ 58-59, matching the
   handoff within ±10%. If bistro drifts >15% from 58.54 ms, STOP and report
   (machine/environment drift — the A/B would not be comparable).
5. Record all numbers in the report table (Step 5).

### Step 2: Implement depth-exit

Save safety copies: `cp assets/shaders/chunk.wgsl /tmp/chunk_before_003.wgsl`,
`cp src/app.rs /tmp/app_before_003.rs`,
`cp src/render/mod.rs /tmp/render_before_003.rs`.

#### 2.1 `src/render/mod.rs` — extend `CameraUniforms`

Append after `viewport_and_heatmap` (order MUST match the WGSL struct):

```rust
    pub prev_view_proj_inv: [[f32; 4]; 4], // inverse of LAST frame's view_proj
    pub prev_depth_valid: f32,             // 0.0 until a previous frame exists
```

#### 2.2 `src/app.rs` — ping-pong depth, uniforms, bindings

1. `create_depth_texture` (line ~72): change usage to
   `wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING`.
   Do not change format, size, or view_formats.
2. Replace the `depth_texture: wgpu::Texture` field (line 44) with:
   ```rust
   depth_textures: [wgpu::Texture; 2], // ping-pong: cur written, prev sampled
   depth_write_index: usize,
   prev_view_proj_inv: Option<glam::Mat4>, // None until a second frame exists
   ```
   In `init` (~line 526) create both textures:
   ```rust
   let depth_textures = [
       create_depth_texture(device, width, height),
       create_depth_texture(device, width, height),
   ];
   ```
   and initialize `depth_write_index: 0, prev_view_proj_inv: None` in the
   constructor. In `resize` (~line 633) recreate both textures and set
   `self.prev_view_proj_inv = None` (the fresh textures are garbage — the
   next frame must not sample them).
3. In `render`, where the camera uniforms are built (lines ~687-700), add:
   ```rust
   let (prev_view_proj_inv, prev_depth_valid) = match self.prev_view_proj_inv {
       Some(m) => (m.to_cols_array_2d(), 1.0),
       None => (glam::Mat4::IDENTITY.to_cols_array_2d(), 0.0),
   };
   let camera_uniforms = CameraUniforms {
       /* existing fields */
       prev_view_proj_inv,
       prev_depth_valid,
   };
   ```
   and after `queue.write_buffer(...)`, before the frame's matrices are
   overwritten next frame, store the current inverse:
   ```rust
   self.prev_view_proj_inv = Some(view_proj.inverse());
   ```
4. The render pass (line ~745): use the current texture's view:
   ```rust
   let depth_view = self.depth_textures[self.depth_write_index]
       .create_view(&wgpu::TextureViewDescriptor::default());
   ```
   and after the pass (after the `{ ... }` block that owns `rpass`), swap:
   ```rust
   self.depth_write_index = 1 - self.depth_write_index;
   ```
   Place the swap AFTER the timestamp resolve so the write index and the
   sampled texture are consistent with the frame's own camera uniform.
5. **NEW depth-only bind group (group 2)** — do NOT touch the existing
   resource bind group construction (it is created once in `init`, lines
   ~344-394, and rebuilding it per frame would churn the palette/texture/
   stats entries). Instead, add a third bind group layout + two pre-created
   bind groups, one per depth texture, and select by write index in the
   pass. This keeps the existing structure intact and guarantees the shader
   samples the OTHER texture than the one being written, with zero per-frame
   allocation:
   ```rust
   // After resource_bind_group_layout creation:
   let depth_bind_group_layout = device.create_bind_group_layout(
       &wgpu::BindGroupLayoutDescriptor {
           label: Some("depth_bind_group_layout"),
           entries: &[
               wgpu::BindGroupLayoutEntry {
                   binding: 0,
                   visibility: wgpu::ShaderStages::FRAGMENT,
                   ty: wgpu::BindingType::Texture {
                       sample_type: wgpu::TextureSampleType::Depth,
                       view_dimension: wgpu::TextureViewDimension::D2,
                       multisampled: false,
                   },
                   count: None,
               },
               wgpu::BindGroupLayoutEntry {
                   binding: 1,
                   visibility: wgpu::ShaderStages::FRAGMENT,
                   ty: wgpu::BindingType::Sampler(
                       wgpu::SamplerBindingType::NonFiltering,
                   ),
                   count: None,
               },
           ],
       },
   );
   let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
       label: Some("depth_prev_sampler"),
       address_mode_u: wgpu::AddressMode::ClampToEdge,
       address_mode_v: wgpu::AddressMode::ClampToEdge,
       address_mode_w: wgpu::AddressMode::ClampToEdge,
       mag_filter: wgpu::FilterMode::Nearest,
       min_filter: wgpu::FilterMode::Nearest,
       mipmap_filter: wgpu::FilterMode::Nearest,
       ..Default::default()
   });
   // depth_bind_groups[i] samples depth_textures[1 - i] (the PREVIOUS
   // frame's texture). Views are immutable; create once here.
   let depth_bind_groups = [0usize, 1].map(|i| {
       device.create_bind_group(&wgpu::BindGroupDescriptor {
           layout: &depth_bind_group_layout,
           entries: &[
               wgpu::BindGroupEntry {
                   binding: 0,
                   resource: wgpu::BindingResource::TextureView(
                       &depth_textures[1 - i].create_view(
                           &wgpu::TextureViewDescriptor::default(),
                       ),
                   ),
               },
               wgpu::BindGroupEntry {
                   binding: 1,
                   resource: wgpu::BindingResource::Sampler(&depth_sampler),
               },
           ],
           label: Some("depth_bind_group"),
       })
   });
   ```
   Store `depth_bind_group_layout`, `depth_bind_groups`, `depth_sampler` as
   fields (the pipeline layout needs the layout; the pass needs the groups).
   Extend the pipeline layout (lines ~396-402) with the new layout:
   ```rust
   bind_group_layouts: &[
       Some(&camera_bind_group_layout),
       Some(&resource_bind_group_layout),
       Some(&depth_bind_group_layout),
   ],
   ```
   and in the render pass, after `set_bind_group(1, ...)`, add:
   ```rust
   rpass.set_bind_group(
       2,
       &self.depth_bind_groups[self.depth_write_index],
       &[],
   );
   ```
   **First-frame behavior**: frame 0 has `prev_depth_valid = 0.0` so the
   shader never samples; frame 1 writes `depth_textures[1]` while sampling
   `depth_textures[0]` (frame 0's real depth) — correct by construction.

#### 2.3 `assets/shaders/chunk.wgsl` — the bound

1. Extend `CameraUniforms` to match the CPU struct (order matters):
   ```wgsl
   struct CameraUniforms {
       camera_pos: vec4<f32>,
       view_inv: mat4x4<f32>,
       proj_inv: mat4x4<f32>,
       view_proj: mat4x4<f32>,
       viewport_and_heatmap: vec4<f32>,
       prev_view_proj_inv: mat4x4<f32>,
       prev_depth_valid: f32,
   };
   ```
2. Add a NEW bind group 2 after the existing group-1 declarations (line
   ~21) — group 1's conditional stats binding at 2 is untouched, and the
   depth bindings live in their own group so the existing resource bind
   group is not rebuilt:
   ```wgsl
   // Previous frame's depth buffer (ping-pong partner of the depth attachment
   // being written — WebGPU forbids sampling the attachment in the pass that
   // writes it). Sampled with a NON-filtering sampler: exact depth values,
   // no interpolation across silhouette edges.
   @group(2) @binding(0) var depth_prev: texture_depth_2d;
   @group(2) @binding(1) var depth_sampler: sampler;
   ```
   and a constant near the others:
   ```wgsl
   // Consistency tolerance (screen pixels): how close the reprojected
   // previous-frame surface must land to this pixel for the depth bound to be
   // trusted. Tuned for the 60 s orbit (~2-3 px/frame of apparent motion at
   // screen center); raise if smoke shows bound rejection on stable geometry.
   const REPROJECT_PX: f32 = 3.0;
   ```
3. Add the helper after `advance_frame`:
   ```wgsl
   // Upper bound on the DDA ray parameter: the distance to the nearest surface
   // seen by the PREVIOUS frame along this pixel's ray. The bound is only
   // trusted when the reprojected surface lies on the current ray (project-
   // reproject check in pixel space), so reveals/camera cuts self-invalidate
   // and fall back to full traversal. Returns INF (no bound) when there is no
   // previous frame, the previous depth is sky, or the check fails.
   fn depth_exit_bound(origin: vec3<f32>, dir: vec3<f32>, frag_xy: vec2<f32>) -> f32 {
       if (camera.prev_depth_valid < 0.5) {
           return INF;
       }
       let uv = frag_xy / camera.viewport_and_heatmap.xy;
       let z = textureSample(depth_prev, depth_sampler, uv);
       // Cleared far depth: unprojecting z = 1.0 would divide by w = 0 (NaN).
       if (z >= 0.9995) {
           return INF;
       }
       let ndc = vec4<f32>(uv * 2.0 - 1.0, z, 1.0);
       let world_h = camera.prev_view_proj_inv * ndc;
       let world = world_h.xyz / world_h.w;
       let to_surface = world - origin;
       let t_reproj = dot(to_surface, dir);
       if (t_reproj <= 0.0) {
           return INF;
       }
       let now_h = camera.view_proj * vec4<f32>(world, 1.0);
       if (now_h.w <= 0.0) {
           return INF;
       }
       let ndc_now = now_h.xy / now_h.w;
       let px_now = (ndc_now * 0.5 + 0.5) * camera.viewport_and_heatmap.xy;
       if (distance(px_now, frag_xy) > REPROJECT_PX) {
           return INF;
       }
       return t_reproj;
   }
   ```
4. In `fs_main`, replace the span handling + root init (lines ~258-280):
   ```wgsl
   let span = ray_aabb(origin, dir, bmin, bmax);
   if (span[1] < 0.0) {
       discard;
   }
   // Depth-exit: clamp the traversal's exit to the previous frame's nearest
   // surface along this ray. The root frame's interval is shortened; the
   // march pops at the bound and discards (background, or the nearer
   // front-to-back instance already written to this pixel). When the bound is
   // INF this is exactly the old span.
   let bound = depth_exit_bound(origin, dir, in.position.xy);
   let exit_t = min(span[1], bound);
   if (exit_t - span[0] <= T_EPS) {
       discard;
   }
   ```
   and change the root init call to
   `frames[0] = init_frame(origin, dir, chunk_origin, ROOT_MIP, vec3<i32>(0),
   vec2<f32>(span[0], exit_t));`
   (delete the old standalone `if (span[1] - span[0] <= T_EPS) { discard; }`
   — it is subsumed by the `exit_t` check).

**Verify (before any capture)**:
- `cargo check` → exit 0, no warnings.
- `cargo test --test shader_validate` → exactly 1 passes (naga catches
  struct/binding errors).
- `cargo test --test hierarchical_mip_dda` → all pass.
- `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
  → all exit 0.

### Step 3: Correctness smoke

1. `WGPU_RT_ORBIT=1 cargo run > /tmp/smoke_dep.txt 2>&1 &`, wait ~60 s, kill.
   Watch the window the whole time (or a screen recording).
2. **Must be unchanged** (baseline behavior):
   - Geometry, palette colors, and chunk seams identical to pre-change on
     stable interior pixels (walls, floors, sky pixels that miss the chunks).
   - No cracks, holes, or white/black NaN flashes anywhere (a NaN flash would
     mean the depth unproject or the `w` guard misfired).
   - The first ~1 s after startup (no previous frame) renders exactly like
     baseline.
   - Heatmap (`H`) still toggles; terminated regions show GREEN (low
     processed_cells) where baseline showed red — that is the mechanism
     working, not an error.
3. **Expected and reportable** (the design's known artifact): at grazing
   silhouette edges, a thin band of pixels may show background where a
   sub-voxel gap lets the ray through. Judge: if it is a stable 1-2 px band,
   note it and proceed (acceptable); if it flickers or widens, STOP and
   report with the observation.
4. If the smoke fails any "must be unchanged" item, restore the three safety
   copies, rerun the Step-1 gates, and STOP and report (do not tune
   REPROJECT_PX blindly to hide a real bug).

### Step 4: A/B gate

1. `cargo build --release --bin bench`. Run the four legs again with the
   depth-exit build, using the per-leg frame counts from the table (bistro
   may now need ~2000 frames, monu1 ~9000 — compute from the depth-exit
   build's measured frame time so each leg still covers ≥ 55 s of orbit).
2. Parse GPU ms mean (stats=0) and cells/frag + hits/frame (stats=1) exactly
   as in Step 1.
3. **Gate (keep-or-revert)**: depth-exit is KEPT only if ALL of:
   - bistro_sm avg GPU ms (stats=0) ≤ 0.90 × baseline bistro avg GPU ms
     (≥10% better — the divergence scene is the target);
   - monu1 avg GPU ms ≤ 1.05 × baseline monu1 (allow ≤5% regression — the
     added depth sample sits on every fragment's critical path; report it);
   - stats=1 cells/frag dropped meaningfully on both scenes (mechanism
     proof; expect a large drop on bistro);
   - stats=1 hits/frame within 10% of baseline (correctness proxy — a bigger
     drop means the bound is clipping genuinely visible geometry, which
     should have shown in smoke as missing walls).
   If any fails after ONE re-run of the offending leg (to rule out noise),
   restore the safety copies, rerun the full gate suite, and STOP and report
   the measured numbers (a color-feedback follow-up or REPROJECT_PX tuning is
   the next move — do not improvise it here).
4. Re-run `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`.

### Step 5: Final gates and report

1. Full gate suite → all exit 0.
2. One final stats=0 capture per scene (same protocol) into the report.
3. `git status --short` → only `assets/shaders/chunk.wgsl`, `src/app.rs`,
   `src/render/mod.rs` (plus this plan file) changed.
4. Fill the report table below in the plan file (append under "Report"):
   | run | scene | avg GPU ms | avg FPS | cells/frag | hits/frame | verdict |
   |---|---|---|---|---|---|---|
   | baseline (Step 1) | monu1 | | | | | |
   | depth-exit (Step 4) | monu1 | | | | | |
   | baseline (Step 1) | bistro_sm | | | | | |
   | depth-exit (Step 4) | bistro_sm | | | | | |
   Also record: the smoke observations (edge artifact extent), the stats-leg
   cells/frag delta, and any tuning knobs touched (REPROJECT_PX).
5. Do NOT update `advisor-plans/README.md` or `plans/README.md`; leave the
   diff uncommitted for review.

## Test plan

- No new Rust unit tests: the shader has no CPU seam; `chunk_wgsl_parses_and_
  validates` (naga) is the compile gate and `hierarchical_mip_dda` guards the
  (untouched) traversal contract. The bound math is verified by visual smoke
  (the reveal/gap cases are exactly the failure modes a unit test cannot
  express) plus the stats-leg hits/frame proxy.
- Perf evidence: the four-leg table, from GPU-paced release bench runs with
  the deterministic time-based orbit (az_period 60 s), both legs ≥ 55 s of
  orbit time.

## Done criteria

- [ ] `cargo test` exits 0; `cargo fmt --check` and
      `cargo clippy --all-targets -- -D warnings` exit 0.
- [ ] Step-1 baseline matches the handoff within ±10% (monu1 ~10 ms,
      bistro ~58 ms); otherwise STOP was reported.
- [ ] Smoke: no cracks/holes/NaN at stable pixels, first frame == baseline,
      heatmap works; only the expected grazing-edge band differs (reported).
- [ ] Gate passed: bistro ≥10% faster, monu1 ≤5% slower, cells/frag dropped,
      hits/frame within 10%.
- [ ] Report table filled with all four rows + smoke/knob notes.
- [ ] `git status --short` shows only the four in-scope files.

## STOP conditions

Stop and report (do not improvise) if:

- Drift check or Current-state excerpts do not match the live tree.
- Step-1 baseline drifts >15% from the handoff (environment drift; A/B not
  comparable).
- Smoke shows cracks/holes/NaN flashes, or first-frame rendering differs from
  baseline (the `prev_depth_valid` guard or the ping-pong swap is wrong).
- The Step-4 gate fails after one re-run; restore the safety copies and
  report the measured numbers (do not loosen the gate or stack unmeasured
  changes).
- An edit turns out to require touching an out-of-scope file.
- The depth textures can't be sampled as `texture_depth_2d` with a
  NonFiltering sampler on this adapter (feature/validation surprise) — report
  the exact error instead of switching to a workaround silently.

## Maintenance notes

- **The stale early-Z comment** in `src/app.rs` (~line 710, "With the
  frag_depth write removed...") is from the reverted plan-002 experiment and
  is now wrong (the shader still writes frag_depth). It is out of this plan's
  scope; a drive-by doc fix is welcome in a future plan.
- **Ping-pong semantics**: the sampled texture is the PREVIOUS frame's
  completed depth (the queue serializes the two frames, so there is no
  in-flight hazard). The group-2 bind groups are pre-created with immutable
  views, one per texture, and selected by `depth_write_index` — the pass
  always samples the texture it is NOT writing. If a future edit rebuilds
  the depth bind groups, this invariant is the thing to re-check.
- **REPROJECT_PX and the sampler** are the tuning knobs: raise REPROJECT_PX
  if stable geometry gets bound-rejected (full traversal = no depth-exit
  benefit); a Filtering (bilinear) sampler softens depth discontinuities at
  the cost of edge blur — not the default.
- **Depth precision**: `z >= 0.9995` is the sky guard; the orbit keeps the
  scene well under the far plane (0.1..10000 near/far), so real surfaces are
  far below it. If a future plan changes the near/far planes, revisit.
- **Interactions with deferred directions**: depth-exit stacks under both
  half-res (fewer, cheaper fragments) and the compute path (the Tree64
  traversal can consume the same bound). The color-feedback variant (render
  terminated pixels with last frame's surface color) is the natural follow-up
  if the grazing-edge band turns out visible on a real scene.
- **Correctness proxy caveat**: the old "stats=1 cells/frag identical between
  baseline and new" proxy intentionally BREAKS here — cells/frag dropping is
  the point. The new proxies are hits/frame (within 10%) + smoke. Record this
  in the final report.

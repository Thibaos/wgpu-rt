# Plan 002: Replace per-proxy fragment DDA with a single cross-chunk fullscreen pass

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise.
>
> **Drift check (run first)**:
> `git diff --stat 1881c54..HEAD -- assets/shaders/chunk.wgsl src/app.rs src/framework.rs src/render/mod.rs tests/shader_validate.rs`
> The working tree should be at `1881c54` **plus** the plan-001 instrumentation
> diff (see Step 0). Compare the "Current state" excerpts below against the
> live files. On any mismatch, STOP and report the exact changed path and
> difference.

## Status

- **Priority**: P1
- **Effort**: L — new render path (fullscreen pipeline + shader), gated by a temporary diagnostic probe and an A/B capture
- **Risk**: MED-HIGH — replaces the proxy-cube rasterizer; the shader's traversal algorithm is preserved byte-for-byte, but the per-pixel ray changes source (interpolated proxy face → camera math), so a visual smoke gate is mandatory
- **Depends on**: plan 001's GPU timestamp instrumentation (Step 0; uncommitted diff exists in worktree `/tmp/wgpu-rt-exec`)
- **Category**: perf
- **Planned at**: commit `1881c54`, 2026-07-31
- **Issue**: (none)

## Why this matters

Plan 001 measured the render pass at **40.06 ms avg GPU time, ~97% of the
41.1 ms frame** (avg 24.3 FPS, orbit protocol) and **falsified the ALU
hypothesis**: removing ~9 divisions per stack push and the duplicated `min`
changed nothing (39.40 / 40.41 ms vs 40.06 ms baseline — pure noise). The
remaining structural suspect is **per-pixel redundancy across overlapping
chunk proxies**: the shader writes `@builtin(frag_depth)`, which disables
early-Z, so a pixel covered by *k* proxy cubes runs the full six-frame DDA *k*
times — once per covering proxy — and the last-drawn proxy wins even when it
is farther (a known ordering artifact deferred by ADR-0002).

This plan (1) **measures** that redundancy with a temporary invocation/sample
probe, then (2) **replaces** the proxy-cube rasterizer with a single fullscreen
pass that traces every chunk per pixel, nearest-hit wins, with a front-to-back
early exit so a hit in the first chunk skips the rest. Each pixel then runs at
most one DDA instead of up to *k*, and the nearest-hit rule fixes the
far-proxy-wins artifact for free. The whole change is gated: probe first (if
redundancy is tiny, STOP with evidence), A/B capture second (keep only on
measured gain, else revert).

## Current state

All excerpts are from the working tree at `1881c54` (+ plan-001 instrumentation
when Step 0 says so). Verify each before editing.

- `assets/shaders/chunk.wgsl` — the only live shader. `fs_main` (line 255)
  derives the ray from the interpolated proxy-face position
  (`in.world_position`), runs `ray_aabb` over `chunk_origin..+CHUNK_WORLD_SIZE`,
  then the bounded six-frame mip DDA (constants at 49-84, `TraversalFrame` at
  85-99, `init_frame` at 175, `advance_frame` at 241, mip-0 hit at 326-334).
  `CameraUniforms` (lines 7-14) already carries `view_inv` and `proj_inv` —
  the fullscreen pass computes `dir` from those plus the interpolated UV.
- `src/app.rs`:
  - `optional_features` (line 88-90 after 001) = `TIMESTAMP_QUERY`; the
    timestamp query set / resolve / staging buffers and the every-30-frames
    readback log `GPU render pass: X.XXms` (from plan 001 — keep them).
  - `resource_bind_group_layout` at lines 270-298: binding 0 = `palette`
    storage (read), binding 1 = `binding_array<texture_3d<u32>>` with
    `count: Some(bind_group_count)`; `resource_bind_group` built at 308-320.
    Texture index == instance index (`voxel_textures[in.chunk_id]` in the
    shader, `chunk_id = instance_index` in `vs_main`).
  - Instance data: `instances: Vec<Instance>` built in `init` (~line 180-200);
    `Instance::to_raw` → `InstanceRaw { model, chunk_origin }` written into
    `instance_buffer` (lines 224-245, created with `create_buffer_init`).
    `chunk_origin = position` (chunk min corner, world space).
  - Pipeline at 395-432: vertex buffer 0 = 24-vertex cube, buffer 1 =
    instances, `cull_mode: Some(Front)`, depth_stencil with `compare: Less`,
    target `config.view_formats[0]`. The render pass at 564-596 sets bind
    groups 0/1, vertex buffers, `draw_indexed` (cube × instance count).
  - App fields: `instance_buffer` at line 42; `vertex_buf`/`index_buf` created
    at ~228-235 (names to confirm).
- `src/render/mod.rs` — `Vertex`, `Instance`, `InstanceRaw`, `to_raw`,
  `create_vertices`, `INDEX_COUNT`, `VOXEL_SCALE` (104 lines). If the
  proxy-cube path is removed, the now-unused items here trigger `dead_code`
  under `clippy -D warnings` — this file is in scope for the removal.
- `tests/shader_validate.rs` — one test `chunk_wgsl_parses_and_validates`
  parses `chunk.wgsl` with naga offline. The new shader gets its own test.

### ADR-0002 contract that must be preserved (from plan 011/012)

Phase 2 starts at mip 5 (8³ cells, 4 m), descends 5→0, mips 5-1 occupancy-only
and mip 0 supplies material; caps `ROOT_CELL_CAP=24`, `REFINEMENT_CELL_CAP=8`,
`GLOBAL_CELL_CAP=2048`, `TRAVERSAL_BOUND=16384`; `child_tex_origin =
2*(parent.tex_origin + parent.cell)`; half-open intervals `[t_enter, t_exit)`;
tie and negative-boundary rules as coded. The traversal **algorithm** is copied
verbatim into the new shader — only the *caller* (which chunk, which span, how
many DDAs per pixel) changes. ADR-0002 deferred cross-chunk traversal; this
plan implements it with measured justification (plan 001 + Step 1 probe).

### Repo conventions to follow

- **Tests**: `cargo test` must pass — `chunk_wgsl_parses_and_validates`,
  the new `chunk_cross_wgsl_parses_and_validates`, and
  `tests/hierarchical_mip_dda.rs` (CPU reference, untouched). Rust unit tests
  inline under `#[cfg(test)]`.
- **Logging**: `log::info!` plain messages (exemplar: `src/app.rs:119-122`).
- **Code style**: `cargo fmt`; `glam` for CPU math; WGSL matches the comment
  style in `chunk.wgsl`.
- **Vocabulary** (from `CONTEXT.md`): "DDA", "Mip level", "Chunk",
  "Voxel Scale" (1 voxel = ⅛ m). `CHUNK_WORLD_SIZE = 32.0` stays hardcoded
  equal to `CHUNK_TEXTURE_SIZE.width * VOXEL_SCALE`.
- **Git workflow**: leave implementation diffs uncommitted for review. Do not
  commit; do not touch `plans/README.md` (project-owned) or
  `advisor-plans/README.md` (reviewer-owned).

## Commands you will need

| Purpose | Command | Expected result |
|---|---|---|
| Drift check | `git diff --stat 1881c54..HEAD -- assets/shaders/chunk.wgsl src/app.rs src/framework.rs src/render/mod.rs tests/shader_validate.rs` | Empty (before Step 0) |
| Check | `cargo check` | exit 0, no warnings |
| Build | `cargo build` | exit 0 |
| Shader gates | `cargo test --test shader_validate` | exactly 2 tests pass |
| DDA reference | `cargo test --test hierarchical_mip_dda` | all pass |
| Full tests | `cargo test` | exit 0 |
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --all-targets -- -D warnings` | exit 0 |
| Perf capture | `WGPU_RT_ORBIT=1 ./target/debug/wgpu-rt.exe > /tmp/orbit_<tag>.log 2>&1 &` then wait, then `kill <pid>` | orbit logs 1 Hz, FPS + GPU-ms lines |

**Display hygiene (important — learned from plan 001's run)**: the app opens
on the user's live desktop and the session captures the screen during runs.
Before every capture, close/minimize other applications so the display shows
only the test window. Run captures sequentially, never in parallel, and always
`kill` the PID (the app never exits on its own). Expected total runtime for
this plan: ~35-50 min (1-2 cold/ incremental builds + ~6 × ~90 s captures +
probe runs). If a capture is aborted early, re-run it — do not reuse partial
logs.

## Scope

**In scope — the only files to modify:**

- `assets/shaders/chunk_cross.wgsl` — **new file**: fullscreen vertex +
  cross-chunk fragment. Traversal code copied verbatim from `chunk.wgsl`.
- `assets/shaders/chunk.wgsl` — Step 1 only (temporary probe counters),
  restored before Step 3. The file stays in the repo (naga-validated
  reference for the traversal core).
- `src/app.rs` — Steps 1, 3, 4 (probe, chunk-desc buffer, bind group layout
  +1 binding, new pipeline, render-pass swap). Keep the plan-001 timestamp
  instrumentation untouched.
- `src/render/mod.rs` — removal of now-unused proxy-cube types (or a minimal
  `ChunkDesc` addition here; see Step 3.5). No changes to `CameraUniforms`.
- `tests/shader_validate.rs` — add `chunk_cross_wgsl_parses_and_validates`
  (refactor `shader_path` to take a filename).

**Out of scope — do not touch:**

- `src/world/*` — no chunk/mip/texture-upload changes.
- `src/framework.rs` — no edit expected (feature request already flows through
  `App::optional_features`; it is in the drift check only as a tripwire).
- The traversal constants and the ADR-0002 traversal contract (see above).
- `src/main.rs`, `Cargo.toml`/`Cargo.lock` (no new dependencies), `plans/`,
  `docs/`, `CONTEXT.md`, assets/models.
- Half-resolution DDA + upscale, and the compute-shader path — deliberately
  deferred (see Maintenance notes); they stack *on top of* this change.

## Steps

### Step 0: Ensure the plan-001 instrumentation is in the tree

The A/B gate needs GPU timestamps. If `src/app.rs` does not yet contain
`GPU render pass:` logging and `TIMESTAMP_QUERY`:

1. Apply the plan-001 instrumentation diff if the worktree still exists:
   `cd /tmp/wgpu-rt-exec && git diff > /tmp/p001.diff` then
   `cd <your-tree> && git apply /tmp/p001.diff`.
   If that worktree is gone, re-implement plan 001 Step 1 exactly as written in
   `advisor-plans/001-dda-gpu-timing-and-shader-optimization.md` (the full
   code is there), including the **ungated** resolve+copy (16 B/frame) with the
   every-30-frames map/readback, and the heatmap wiring (Step 2 of plan 001).
2. Verify: `grep -c "GPU render pass:" /tmp/orbit_instr.log` ≥ 1 on a short
   run (~20 s), and `cargo test` passes.

If the instrumentation is already present, skip to Step 1.

### Step 1: Probe — measure per-pixel redundancy before building anything

Goal: quantify the lever. In the **current** pipeline, count (a) fragment
invocations per frame and (b) total `processed_cells` per frame, read back
every 30 frames, log the ratio. Temporary diagnostic code — removed before
Step 3, so it never contaminates the A/B.

1. In `src/app.rs`, add a probe storage buffer (2 × `atomic<u32>`), created
   gated on `timestamps_enabled` (reuse the timestamp plumbing pattern):
   ```rust
   // In App struct (temporary):
   probe_buf: Option<wgpu::Buffer>,
   // In init, next to the timestamp buffers:
   let probe_buf = device.create_buffer(&wgpu::BufferDescriptor {
       label: Some("traversal_probe"),
       size: 8,
       usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
       mapped_at_creation: false,
   });
   ```
   Add binding 2 to `resource_bind_group_layout`:
   ```rust
   wgpu::BindGroupLayoutEntry {
       binding: 2,
       visibility: wgpu::ShaderStages::FRAGMENT,
       ty: wgpu::BindingType::Buffer {
           ty: wgpu::BufferBindingType::Storage { read_only: false },
           has_dynamic_offset: false,
           min_binding_size: None,
       },
       count: None,
   },
   ```
   and bind `probe_buf` into `resource_bind_group`. In `App::render`, before
   the pass, zero it each frame (or every 30): `queue.write_buffer(... &[0u8;8])`.
2. In `assets/shaders/chunk.wgsl`, add
   `@group(1) @binding(2) var<storage, read_write> probe: array<atomic<u32>>;`
   and increment at the very top of `fs_main` (`atomicAdd(&probe[0], 1u);`)
   and at the positive-width sample site (the line
   `processed_cells = processed_cells + 1;`) (`atomicAdd(&probe[1], 1u);`).
3. In `App::render`, next to the timestamp readback (same every-30-frames
   guard), read the probe back and log:
   ```rust
   log::info!(
       "probe: invocations={} samples={} px={}x{}",
       probe[0], probe[1], width, height
   );
   ```
   (atomic buffers read back as u32s; reuse the map pattern from plan 001).
4. `cargo build`, one orbit capture (~70 s) into `/tmp/orbit_probe.log`.
   From the last readback rows, compute:
   - `invocation_ratio = invocations / (width * height)` — average number of
     proxy fragments per pixel. **This is the multiplicity.**
   - `samples_per_fragment = samples / invocations` — DDA depth per proxy.
   - `covered_ratio = invocations / (width * height * n_chunks)` where
     `n_chunks` = 5 — what fraction of pixels any proxy covers.
5. **Gate (STOP or proceed)**:
   - If `invocation_ratio ≤ 1.15` (essentially no overlap in the orbit
     sweep), the redundancy lever is small — **STOP and report** the measured
     ratio and recommend half-resolution DDA + upscale (or the compute path)
     as the next plan instead. Do not build the fullscreen pass on a tiny
     lever.
   - If `invocation_ratio ≥ 1.25`, the lever is real — proceed to Step 2.
     Record the numbers; they belong in the final report.
6. Revert the probe (restore `chunk.wgsl` and the app.rs probe additions;
   keep the plan-001 instrumentation!). `cargo test` must pass and
   `grep -c probe` in the shader must be 0.

### Step 2: Baseline capture (fresh numbers to beat)

Same protocol as plan 001 Step 3, with the probe removed and the
instrumentation in place:

`cargo build`, then `WGPU_RT_ORBIT=1 ./target/debug/wgpu-rt.exe >
/tmp/orbit_base2.log 2>&1 &`, wait ~70 s, `kill`. Parse:

```bash
grep -oE "\([0-9]+\.[0-9]+ FPS\)" /tmp/orbit_base2.log | grep -oE "[0-9]+\.[0-9]+" | tail -n +2 | sort -n | awk '{s+=$1; a[NR]=$1} END{printf "FPS n=%d min=%.1f max=%.1f avg=%.1f median=%.1f\n", NR, a[1], a[NR], s/NR, a[int((NR+1)/2)]}'
grep -oE "GPU render pass: [0-9.]+ms" /tmp/orbit_base2.log | grep -oE "[0-9.]+" | awk '{s+=$1; n++; if(n==1||$1<m)m=$1; if($1>M)M=$1} END{printf "GPU ms n=%d min=%.2f max=%.2f avg=%.2f\n", n, m, M, s/n}'
```

Sanity: avg GPU ms ≈ 40 ms (1000/FPS), avg FPS ~23-25. If GPU ms has drifted
>15% from plan 001's 40.06 ms, STOP and report (the machine/build changed
underneath you; the A/B is not comparable).

### Step 3: Implement the cross-chunk fullscreen pass

Save safety copies first: `cp assets/shaders/chunk.wgsl /tmp/chunk_before_002.wgsl`,
`cp src/app.rs /tmp/app_before_002.rs`, `cp src/render/mod.rs /tmp/render_before_002.rs`.

#### 3.1 New shader `assets/shaders/chunk_cross.wgsl`

Structure (copy the traversal core verbatim from `chunk.wgsl` — constants,
`TraversalFrame`, `init_frame`, `advance_frame`, `ray_aabb`, `heat_color`):

```wgsl
enable wgpu_binding_array;

struct CameraUniforms { /* identical to chunk.wgsl */ };

struct ChunkDesc {
    origin: vec3<f32>,   // chunk min corner, world space (== chunk_origin)
    texture_index: u32,  // index into voxel_textures binding array
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct TraceResult {
    hit: bool,
    t: f32,
    color: vec4<f32>,
};

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(1) @binding(0) var<storage, read> palette: array<vec4<f32>>;
@group(1) @binding(1) var voxel_textures: binding_array<texture_3d<u32>>;
@group(1) @binding(2) var<storage, read> chunk_descs: array<ChunkDesc>;

const CHUNK_CAP: i32 = 16; // upper bound on chunks traced per pixel

// Fullscreen triangle (no vertex buffers needed).
@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0),
    )[vi];
    var out: VertexOutput;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = pos * 0.5 + 0.5;
    return out;
}

// The plan-011/012 traversal, extracted unchanged from chunk.wgsl's fs_main
// body (everything from `var frames: array<TraversalFrame, 6>;` through the
// mip-0 hit/advance logic), parameterized by chunk and span. Returns
// hit=false when the traversal ends with no mip-0 hit (the old discards).
// The heatmap behavior (heat_color by processed_cells) is preserved.
fn trace_chunk(
    origin: vec3<f32>,
    dir: vec3<f32>,
    chunk_origin: vec3<f32>,
    span: vec2<f32>,
    texture_index: u32,
) -> TraceResult { /* traversal body from chunk.wgsl, with the mip-0 hit
                      returning TraceResult(true, top.t, palette[mat]) and
                      every discard path returning TraceResult(false, 0.0,
                      vec4<f32>(0.0)) */ }

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    let origin = camera.camera_pos.xyz;

    // Ray from camera math instead of an interpolated proxy face.
    let ndc = vec4<f32>(in.uv * 2.0 - 1.0, 1.0, 1.0);
    let view_pt = camera.proj_inv * ndc;          // view space
    let world_pt = camera.view_inv * view_pt;     // world space
    let delta = world_pt.xyz - origin;
    if (dot(delta, delta) <= RAY_LENGTH_EPS * RAY_LENGTH_EPS) { discard; }
    let dir = normalize(delta);

    let heatmap_on = camera.viewport_and_heatmap.z >= 0.5;

    let n = min(i32(arrayLength(&chunk_descs)), CHUNK_CAP);

    // Per-chunk spans (ray vs 32 m AABB), then process front-to-back with
    // early exit: once a hit is closer than the next chunk's entry, stop.
    var spans: array<vec2<f32>, CHUNK_CAP>;
    for (var i: i32 = 0; i < n; i = i + 1) {
        let bmin = chunk_descs[i].origin;
        spans[i] = ray_aabb(origin, dir, bmin, bmin + vec3<f32>(CHUNK_WORLD_SIZE));
    }

    var best_t = INF;
    var best_color = vec4<f32>(0.0);
    var hit = false;
    var processed_cells_total: i32 = 0;

    for (var k: i32 = 0; k < n; k = k + 1) {
        // Select the unprocessed chunk with the smallest entry t (O(n^2) with
        // n <= 5-16; each round is a few compares, negligible vs one DDA).
        var sel: i32 = -1;
        var sel_entry = INF;
        for (var i: i32 = 0; i < n; i = i + 1) {
            if (spans[i].x >= 0.0 && spans[i].x < sel_entry) {
                sel_entry = spans[i].x;
                sel = i;
            }
        }
        if (sel < 0) { break; }              // no chunk in front
        spans[sel].x = -1.0;                 // mark consumed
        if (sel_entry >= best_t) { break; }  // everything left is behind the hit

        // Clamp the DDA span to the current best hit.
        let span = vec2<f32>(spans[sel].y >= 0.0 ? spans[sel].x : -1.0, 0.0);
        let trace_span = vec2<f32>(
            sel_entry,
            min(spans[sel].y, best_t),
        );
        if (trace_span.y - trace_span.x <= T_EPS) { continue; }

        let res = trace_chunk(
            origin, dir,
            chunk_descs[sel].origin,
            trace_span,
            chunk_descs[sel].texture_index,
        );
        if (res.hit && res.t < best_t) {
            best_t = res.t;
            best_color = res.color;
            hit = true;
        }
    }

    if (hit) {
        let clip = camera.view_proj * vec4<f32>(origin + dir * best_t, 1.0);
        if (heatmap_on) {
            return FragmentOutput(heat_color(processed_cells_total), clip.z / clip.w);
        }
        return FragmentOutput(best_color, clip.z / clip.w);
    }
    if (heatmap_on) {
        return FragmentOutput(heat_color(processed_cells_total), 0.999);
    }
    discard;
}
```

Notes for the executor:
- `trace_chunk` returns the raw ray parameter `t` (not the hit world
  position); `fs_main` computes `clip` from `origin + dir * best_t` at the
  end. The depth written is therefore the **nearest** hit's depth — this
  replaces the old per-proxy depth and makes overlapping proxies render
  nearest-wins (an improvement; see the smoke gate in Step 4).
- If `trace_chunk`'s body accumulates `processed_cells` locally, return it
  too (add a `samples: i32` field to `TraceResult`) so the heatmap and the
  probe's "samples" concept survive. Keep the per-chunk caps identical.
- The O(n²) selection with `spans[sel].x = -1.0` marking needs the span's x
  restored for nothing later — it is consumed once, so a plain `let`
  overwrite is fine. Do not "unmark".
- `ray_aabb` and all constants are copied verbatim; do not re-derive
  `CHUNK_WORLD_SIZE` from anything.
- If naga rejects `arrayLength` on the top-level runtime array (it should
  not), fall back to a `uniform` chunk count in `CameraUniforms` — but prefer
  `arrayLength`; do not change `CameraUniforms` layout without noting it.

#### 3.2 `src/app.rs` render-path swap

1. Build `chunk_descs` in `init` from the same source as `instance_buffer`
   (the `instances`/`origins` vec used at lines ~180-245). A `ChunkDesc` is
   16 bytes (`origin: vec3`, `texture_index: u32`); reuse the `InstanceRaw`
   layout pattern with `bytemuck::cast_slice`. `texture_index = i` (instance
   order == texture order, as today). Create it with
   `create_buffer_init`, usage `STORAGE | COPY_DST`.
2. `resource_bind_group_layout`: add binding 2 (`Storage { read_only: true }`,
   fragment visibility, `min_binding_size: None`) and bind the new buffer in
   `resource_bind_group`. **Remove the Step-1 probe binding if still present.**
3. New pipeline `cross_chunk_pipeline`:
   - shader module from `chunk_cross.wgsl`,
   - vertex buffers: none,
   - `primitive: TriangleList` (3 vertices),
   - `cull_mode: None`,
   - `depth_stencil` identical to the current one (`Less`, depth write on),
   - same targets, same bind group layouts (0 = camera, 1 = resources).
   Keep the old pipeline struct field for now (it is the revert path), but
   the render pass uses the new one.
4. In `App::render`, replace the cube draw with:
   ```rust
   rpass.set_pipeline(&self.cross_chunk_pipeline);
   rpass.set_bind_group(0, &self.camera_bind_group, &[]);
   rpass.set_bind_group(1, &self.resource_bind_group, &[]);
   rpass.draw(0..3, 0..1);
   ```
   Remove the `set_vertex_buffer` / `set_index_buffer` / `draw_indexed` calls.
5. If the cube pipeline, `vertex_buf`, `index_buf`, `instance_buffer` fields,
   and the render/mod.rs types become unused, remove them (clippy
   `-D warnings` fails on `dead_code`). Keep `Instance`/`chunk_origin`
   semantics available for `chunk_descs` construction in `init`.

#### 3.3 `tests/shader_validate.rs`

Refactor `shader_path()` to take a file name and add:
```rust
#[test]
fn chunk_cross_wgsl_parses_and_validates() {
    let source = std::fs::read_to_string(shader_path("chunk_cross.wgsl"))
        .expect("chunk_cross.wgsl must exist");
    naga::front::wgsl::parse_str(&source).expect("chunk_cross.wgsl failed WGSL validation");
}
```

**Verify (before any capture)**:
- `cargo test --test shader_validate` → exactly 2 pass.
- `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
  → all exit 0.
- Short run (~20 s) into `/tmp/orbit_cross_smoke.log`: window opens, orbit
  logs, GPU-ms lines appear. Kill it.

### Step 4: A/B gate for the fullscreen pass

1. **Visual smoke (mandatory)**: `cargo run` (or a short orbit capture), press
   `H` to toggle heatmap, inspect several angles. Expected: the scene looks
   correct; colors may differ **only at pixels where multiple proxies overlap
   and the old far-proxy-wins artifact hid the nearer chunk** — that change is
   the fix working, and nearest-wins should look more correct (closer chunk
   occludes farther). What must NOT change: cracks, holes, wrong palette
   colors, chunk-border seams where no overlap exists, or a flipped/mirrored
   image. The heatmap must still show cost and toggle cleanly. If the executor
   cannot press keys, use the plan-001 trick: temporarily default `heatmap` to
   `true`, capture, revert.
2. Orbit capture ~70 s into `/tmp/orbit_cross.log`, parse with the Step-2 awk
   commands.
3. **Gate (keep-or-revert)**: avg GPU ms for the fullscreen pass must be
   **≤ 0.95 × baseline avg GPU ms** (≥5% better). If it is, keep it. If it is
   not, or smoke shows artifacts: restore `/tmp/chunk_before_002.wgsl` /
   `/tmp/app_before_002.rs` / `/tmp/render_before_002.rs`, delete
   `chunk_cross.wgsl` and the new test, rerun the gates, and STOP and report
   the measured numbers (the probe's multiplicity + the A/B delta tell the
   next plan where to go).
4. Re-run `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`.

### Step 5: Final gates and report

1. Full gate suite (`cargo test`, fmt, clippy) → all exit 0.
2. One final orbit capture `/tmp/orbit_final2.log`, parse FPS + GPU ms.
3. `git status --short` → only in-scope files changed: `chunk_cross.wgsl`,
   `chunk.wgsl` (only if probe leftovers — should be clean), `src/app.rs`,
   `src/render/mod.rs`, `tests/shader_validate.rs`.
4. Report the table (baseline2 / cross-chunk / final) plus the Step-1 probe
   numbers (multiplicity, samples per fragment, covered ratio) and the
   visual-smoke verdict (what changed at overlaps, nearest-wins behavior).
   Do NOT update `advisor-plans/README.md` (reviewer-owned).

## Test plan

- No new Rust unit tests for the shader (no CPU seam; the traversal core is
  copied byte-for-byte and gate-corrected by naga validation + visual smoke).
  The new `chunk_cross_wgsl_parses_and_validates` covers parse/type/validation.
  `tests/hierarchical_mip_dda.rs` (CPU reference) must keep passing — it
  guards the traversal contract the copied core relies on.
- Perf evidence: Step-1 probe (multiplicity) + Step-2/4/5 orbit A/B captures,
  same deterministic ~70 s window as plan 001.

## Done criteria

- [ ] `cargo test` exits 0 (3 shader/CPU gates + app tests).
- [ ] `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` exit 0.
- [ ] Step-1 probe measured and reported; gate decision recorded (proceed or
      STOP with numbers). Probe code fully reverted.
- [ ] Baseline2 within ±15% of plan 001's 40.06 ms avg GPU.
- [ ] Visual smoke: no cracks/holes/wrong colors; overlap pixels show
      nearest-wins; heatmap works.
- [ ] Fullscreen pass passed its ≥5% gate (avg GPU ms ≤ 0.95 × baseline2);
      if not, reverted and STOP reported.
- [ ] Final capture improves over baseline2; report table has all runs.
- [ ] `git status --short` shows only in-scope files.

## STOP conditions

Stop and report (do not improvise) if:

- Drift check or Current-state excerpts do not match the live tree.
- Step-1 probe shows `invocation_ratio ≤ 1.15` (the redundancy lever is
  small) — report the numbers and recommend half-res/compute instead.
- Baseline2 GPU ms drifts >15% from plan 001's 40.06 ms (environment drift;
  A/B not comparable).
- Step-4 gate fails after one re-run to rule out noise — revert the fullscreen
  pass and report (do not loosen the gate or stack unmeasured changes).
- Visual smoke shows cracks, holes, mirrored/flipped output, or wrong colors
  at non-overlap pixels.
- An edit turns out to require touching an out-of-scope file.

## Maintenance notes

- **Nearest-wins is a behavior change at overlaps** (ADR-0002's deferred
  ordering/occlusion, implemented with measured justification). If the user
  preferred the old far-proxy-wins look, that is a product decision — the
  probe + A/B numbers are the evidence to decide with.
- **The `CHUNK_CAP = 16` bound**: the shader traces at most 16 chunks per
  pixel. The scene has 5 today; if a future scene exceeds 16 non-empty chunks
  per ray, the cap must be revisited (and `spans` array grows registers).
- **Early-Z is still disabled** (frag_depth write), but with one fullscreen
  pass there is no overdraw left to early-Z — each pixel runs exactly one
  traversal loop. This removes the original reason early-Z mattered.
- **Deferred directions** (do not implement here):
  - *Half-resolution DDA + upscale*: multiplies with this change (~4× fewer
    pixels × single DDA). Next-best lever after cross-chunk.
  - *Compute-shader path*: fullscreen pass is a stepping stone; a compute pass
    gets explicit occupancy control and shared-memory tricks.
  - *Release-profile perf gate*: debug build keeps CPU overhead in the mix;
    a `--release` run separates it for future plans.
- The plan-001 instrumentation (timestamps + heatmap) is the durable
  measurement asset; keep it across all of the above.

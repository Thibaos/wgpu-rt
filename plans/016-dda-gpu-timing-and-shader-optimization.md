# Plan 016: Measure the DDA render pass with GPU timestamps, then apply the two highest-leverage shader optimizations

> **History (2026-08-02)**: moved from `advisor-plans/001-dda-gpu-timing-and-shader-optimization.md` (improve-skill advisor batch, 2026-07-31) into the main plan index as plan 016. Terminal — see `plans/README.md` row 016.

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` (row 016 — reviewer-owned; the former advisor index
> was folded into the repo's plan index on 2026-08-02).
>
> **Drift check (run first)**:
> `git diff --stat 1881c54..HEAD -- assets/shaders/chunk.wgsl src/app.rs src/framework.rs`
> Expected output is empty. Then compare the "Current state" excerpts below
> against the live files. On any mismatch, STOP and report the exact changed
> path and difference.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED — shader arithmetic refactor; each optimization step is gated by its own A/B measurement and is revertible
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `1881c54`, 2026-07-31
- **Issue**: (none)
- **Execution outcome** (2026-07-31, verdict BLOCKED): Steps 1-3 executed and verified by review; the measurement harness (GPU timestamps + wired heatmap) is the durable asset — the uncommitted diff lives in worktree `/tmp/wgpu-rt-exec` (only `assets/shaders/chunk.wgsl` + `src/app.rs`). **Optimization A was measured and rejected**: baseline avg GPU 40.06 ms vs A 39.40/40.41 ms — within run-to-run noise, the ≥5% gate failed honestly, and A was reverted per the plan. **Optimization B was not run** — per the plan's own STOP gate (do not stack unmeasured changes after a failed gate). The falsification is the result: init_frame's divisions and the duplicated min are NOT the bottleneck. Measured facts to carry forward: render pass is 97% of frame time (~41 ms), GPU-bound; heatmap shows cost concentrated on dense geometry (per-proxy traversal work). Any future optimization plan should target the deferred directions (per-pixel redundancy across overlapping proxies / cross-chunk traversal, half-res, compute path) and reuse the timestamp harness rather than shader arithmetic micro-optimizations.

## Why this matters

The app renders a 13.85 M-voxel scene (5 chunk instances) at 1920×1080 with a
per-pixel hierarchical mip DDA fragment shader. On an RTX 3070 (Vulkan) the
measured steady-state performance is **23.1 FPS average (44.2 ms/frame),
min 16.9, max 29.8** — and the FPS curve tracks the orbit camera's view angle
(grazing views are slowest), which suggests the DDA traversal itself is the
bottleneck. But this is **unmeasured inference**: the only metric today is
wall-clock FPS, the render pass records no GPU timestamps, and the numbers
came from a debug build. Before optimizing blind, this plan (1) adds GPU
timestamp queries around the render pass so GPU time vs CPU time is known,
(2) wires the currently-dead traversal heatmap so the executor can *see* where
sample cost concentrates, then (3) applies two staged, measured, revertible
shader optimizations — hoisted inverse direction (removes ~9 divisions per
stack push) and a compacted traversal stack (removes 10 of 27 scalar fields
per frame, cutting the six-frame stack's register footprint ~37%) — with a
deterministic orbit-camera A/B protocol that keeps or reverts each step on
measured evidence.

If the instrumentation shows the render pass is NOT GPU-bound (GPU ms well
under the frame time), the "slow DDA" premise is wrong and the plan stops and
reports instead of churning the shader (see STOP conditions).

## Current state

All excerpts are from the working tree at `1881c54`. Verify each before editing.

- `assets/shaders/chunk.wgsl` — the only live chunk shader, included at runtime
  via `include_str!` (`src/app.rs:295-300`). The hierarchical traversal is
  documented in the header comment (lines 28–48) and in
  `docs/adr/0002-dense-3d-texture-mip-dda.md` (accepted design — do not change
  the algorithm, mip choices, caps, or interval semantics; only their
  arithmetic encoding).
- `src/app.rs` — owns the pipeline, bind groups, and `App::render`, which
  builds the render pass (`src/app.rs:564-583`; the descriptor currently has
  `timestamp_writes: None` at line 583).
- `src/framework.rs` — the winit event loop. Device creation combines features
  at `src/framework.rs:137-140`:
  ```rust
  required_features: (App::optional_features() & adapter.features())
      | App::required_features(),
  ```
  No framework change is needed for instrumentation beyond what flows from
  `App::optional_features`; the `FrameCounter` (`framework.rs:185-211`) already
  logs `Frame time X.XXms (Y.Y FPS)` about once per second — that is the CPU
  wall-clock side of the CPU-vs-GPU comparison.
- The `TraversalFrame` struct (`chunk.wgsl:85-99`) — 27 scalar fields × a
  6-frame stack:
  ```wgsl
  struct TraversalFrame {
      mip: u32,
      grid_size: i32,
      tex_origin: vec3<i32>,
      bounds_min: vec3<f32>,
      bounds_max: vec3<f32>,
      interval: vec2<f32>,
      cell: vec3<i32>,
      t: f32,
      t_max: vec3<f32>,
      t_delta: vec3<f32>,
      axis_step: vec3<i32>,
      steps_taken: i32,
  };
  ```
- `init_frame` (`chunk.wgsl:175-237`) performs vec3 divisions and per-axis
  divisions:
  ```wgsl
  let cell_size = (bounds_max - bounds_min) / vec3<f32>(f32(grid_size));   // line 194
  let entry = origin + dir * interval.x;
  let local = (entry - bounds_min) / cell_size;                            // line 196
  ...
  } else {
      axis_step[i] = select(-1, 1, dir[i] > 0.0);
      t_delta[i] = cell_size[i] / abs(dir[i]);                             // line 218
      let boundary = select(
          bounds_min[i] + f32(c) * cell_size[i],
          bounds_min[i] + f32(c + 1) * cell_size[i],
          dir[i] > 0.0,
      );
      t_max[i] = (boundary - origin[i]) / dir[i];                          // line 225
  }
  ```
  It is called once for the root frame plus once per coarse-occupied cell
  pushed during descent (`chunk.wgsl:277-284` and the push at lines 358-366).
- `advance_frame` (`chunk.wgsl:241-252`) recomputes the minimum the caller
  already computed:
  ```wgsl
  fn advance_frame(frame: TraversalFrame) -> TraversalFrame {
      var f = frame;
      let min_t = min(min(f.t_max.x, f.t_max.y), f.t_max.z);
      f.t = min_t;
      for (var i: i32 = 0; i < 3; i = i + 1) {
          if (f.t_max[i] - min_t <= T_EPS) {
              f.cell[i] = f.cell[i] + f.axis_step[i];
              f.t_max[i] = f.t_max[i] + f.t_delta[i];
          }
      }
      return f;
  }
  ```
  Its four call sites are at `chunk.wgsl:315, 337, 357, 375`; each call site's
  scope already contains the just-computed
  `let next_boundary = min(min(top.t_max.x, top.t_max.y), top.t_max.z);`
  (line 301) — the value to pass in.
- The root frame initialization (`chunk.wgsl:277-284`):
  ```wgsl
  var frames: array<TraversalFrame, 6>;
  var stack_len: i32 = 0;
  var processed_cells: i32 = 0;

  frames[0] = init_frame(origin, dir, ROOT_MIP, ROOT_GRID_SIZE, vec3<i32>(0), bmin, bmax, span);
  stack_len = 1;
  ```
- The mip-0 hit and the coarse-descend branch (`chunk.wgsl:326-366`):
  ```wgsl
  if (top.mip == 0u) {
      if (mat != 0u) {
          let hit_world = origin + dir * top.t;
          let clip = camera.view_proj * vec4<f32>(hit_world, 1.0);
          return FragmentOutput(palette[mat], clip.z / clip.w);
      }
      frames[top_idx] = advance_frame(top);
      continue;
  }

  if (mat != 0u) {
      let parent_entry = top.t;
      let parent_next = next_boundary;
      let child_exit = min(parent_next, top.interval.y);
      let cell_size = (top.bounds_max - top.bounds_min) / vec3<f32>(f32(top.grid_size)); // line 349
      let cell_bmin = top.bounds_min + vec3<f32>(top.cell) * cell_size;
      let cell_bmax = cell_bmin + cell_size;
      let child_tex_origin = 2 * (top.tex_origin + top.cell);
      let child_mip = top.mip - 1u;

      frames[top_idx] = advance_frame(top);

      if (child_exit - parent_entry > T_EPS && stack_len < 6) {
          frames[stack_len] = init_frame(
              origin,
              dir,
              child_mip,
              2,
              child_tex_origin,
              cell_bmin,
              cell_bmax,
              vec2<f32>(parent_entry, child_exit),
          );
          stack_len = stack_len + 1;
      }
  } else {
      frames[top_idx] = advance_frame(top);
  }
  ```
- The `viewport_and_heatmap` uniform is declared (`chunk.wgsl:8`) but never
  read; `App::toggle_heatmap` (`src/app.rs:600-610`) flips `heatmap` and
  `App::render` packs it into the uniform's z component (`src/app.rs:522-524`).
  The heatmap is dead code today.
- Key constants: `ROOT_MIP = 5u`, `ROOT_GRID_SIZE = 8`, `T_EPS = 1e-6`,
  `PARALLEL_EPS = 1e-8`, `ROOT_CELL_CAP = 24`, `REFINEMENT_CELL_CAP = 8`,
  `GLOBAL_CELL_CAP = 2048`, `TRAVERSAL_BOUND = 16384` (`chunk.wgsl:49-84`).
  Do not change any of them.
- `App::optional_features` (`src/app.rs:82-83`) is currently
  `wgpu::Features::empty()`.

### Repo conventions to follow

- **Tests**: `cargo test` must pass — including the offline shader gate
  `tests/shader_validate.rs` (parses `chunk.wgsl` with naga) and the DDA CPU
  reference `tests/hierarchical_mip_dda.rs`. Those tests exercise a *separate
  CPU reference implementation*, not the shader, so shader edits do not need
  new Rust tests — but they must keep the shader validating and the rendered
  output visually unchanged. Rust unit tests live inline under `#[cfg(test)]`
  (exemplar: `src/world/chunk.rs:163-240`).
- **Logging**: `log::info!` with plain messages (exemplar: `src/app.rs:119-122`).
- **Code style**: `cargo fmt`; `glam` for CPU math; WGSL shader matches the
  existing comment style in `chunk.wgsl`.
- **Vocabulary** (from `CONTEXT.md`): use "DDA", "Mip level", "Chunk",
  "Voxel Scale" (1 voxel = ⅛ m). A chunk spans 32 m; `CHUNK_WORLD_SIZE = 32.0`
  is hardcoded in the shader and must stay equal to
  `CHUNK_TEXTURE_SIZE.width * VOXEL_SCALE`.
- **Git workflow**: the repo's plans leave implementation diffs uncommitted for
  review. Do not commit unless explicitly instructed; do not touch
  `plans/README.md` (project-owned, reviewer-maintained).

## Commands you will need

| Purpose | Command | Expected result |
|---|---|---|
| Drift check | `git diff --stat 1881c54..HEAD -- assets/shaders/chunk.wgsl src/app.rs src/framework.rs` | Empty |
| Check | `cargo check` | exit 0, no warnings |
| Build | `cargo build` | exit 0 |
| Shader gate | `cargo test --test shader_validate -- --exact chunk_wgsl_parses_and_validates` | exactly 1 test passes |
| DDA reference | `cargo test --test hierarchical_mip_dda` | all pass |
| Full tests | `cargo test` | exit 0 |
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --all-targets -- -D warnings` | exit 0 |
| Perf capture | `WGPU_RT_ORBIT=1 ./target/debug/wgpu-rt.exe > /tmp/orbit_<tag>.log 2>&1 &` then wait ~70 s, then `kill <pid>` | window opens, orbit logs 1 Hz, FPS + GPU-ms lines appear |

All perf captures run from the repo root with the app window open on the
display (the orbit camera needs no input). Build with `cargo build` before each
capture so the binary matches the current source.

## Scope

**In scope — the only files to modify:**

- `assets/shaders/chunk.wgsl` — Steps 2, 4, 5 (heatmap read, arithmetic
  refactor, stack compaction). The traversal *algorithm* (mip choice, caps,
  half-open intervals, tie/negative-boundary rules) stays byte-for-byte
  equivalent in behavior.
- `src/app.rs` — Steps 1, 2 (timestamp instrumentation, heatmap already wired
  CPU-side; only shader side needed).
- `src/framework.rs` — no edit required (feature request flows through
  `App::optional_features`), but it is in the drift check in case the executor
  finds a framework-side need (e.g. poll timing) — any such edit must be
  minimal and reported.

**Out of scope — do not touch:**

- `src/world/*` — no chunk/mip/texture-upload changes.
- `src/render/mod.rs` — no pipeline/vertex/instance changes.
- The traversal constants (`ROOT_MIP`, caps, `TRAVERSAL_BOUND`, `T_EPS`,
  `PARALLEL_EPS`) and the accepted ADR-0002 traversal contract (mip 5 start,
  2³ child grids, six frames, parent-advance-before-push, occupancy-only coarse
  mips). This plan only changes how the same values are *computed*.
- `src/main.rs`, `Cargo.toml`/`Cargo.lock` (no new dependencies — `bytemuck`
  is already present), `plans/`, `docs/`, `CONTEXT.md`, assets/models.
- Cross-chunk traversal, occlusion culling, half-resolution rendering, or a
  compute-shader rewrite — all deliberately deferred (see Maintenance notes).

## Suggested executor toolkit

- Use `context7_docs` with library id `/websites/rs_wgpu` if any wgpu 30 API
  shape in Step 1 needs confirmation (the APIs below were verified against
  wgpu 30 docs on 2026-07-31: `Features::TIMESTAMP_QUERY`,
  `QuerySetDescriptor`, `RenderPassTimestampWrites`,
  `CommandEncoder::resolve_query_set`, `Queue::get_timestamp_period`,
  `Buffer::map_async`).

## Steps

### Step 1: Add GPU timestamp instrumentation around the render pass

Goal: log the GPU duration of the chunk render pass (`GPU render pass: X.XXms`)
every ~30 frames, next to the existing wall-clock FPS line, so CPU vs GPU time
is comparable.

In `src/app.rs`:

1. Change `optional_features` (line 82-83) to:
   ```rust
   pub fn optional_features() -> wgpu::Features {
       wgpu::Features::TIMESTAMP_QUERY
   }
   ```
   `framework.rs:137-140` already ANDs this with adapter features, so adapters
   without the feature simply don't request it.
2. Add five fields to the `App` struct, after the orbit fields (lines 46-48):
   ```rust
   // GPU timing instrumentation
   timestamps_enabled: bool,
   timestamp_query_set: Option<wgpu::QuerySet>,
   timestamp_resolve_buf: Option<wgpu::Buffer>,
   timestamp_staging_buf: Option<wgpu::Buffer>,
   frame_index: u64,
   last_gpu_log_frame: u64,
   ```
3. In `App::init`, after `let depth_texture = create_depth_texture(...)` and
   before the `App { ... }` literal, create the query resources (gated on the
   device feature) and initialize all six fields in the constructor:
   ```rust
   let timestamps_enabled = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
   let (timestamp_query_set, timestamp_resolve_buf, timestamp_staging_buf) = if timestamps_enabled {
       log::info!(
           "GPU timestamp queries enabled (period {} ns/tick)",
           queue.get_timestamp_period()
       );
       let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
           label: Some("dda_pass_timestamps"),
           ty: wgpu::QueryType::Timestamp,
           count: 2,
       });
       let resolve_buf = device.create_buffer(&wgpu::BufferDescriptor {
           label: Some("dda_pass_timestamp_resolve"),
           size: 16, // 2 x u64 timestamps
           usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
           mapped_at_creation: false,
       });
       let staging_buf = device.create_buffer(&wgpu::BufferDescriptor {
           label: Some("dda_pass_timestamp_staging"),
           size: 16,
           usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
           mapped_at_creation: false,
       });
       (Some(query_set), Some(resolve_buf), Some(staging_buf))
   } else {
       log::warn!("TIMESTAMP_QUERY unavailable; GPU timing disabled, FPS-only A/B");
       (None, None, None)
   };
   ```
   In the constructor literal, add:
   ```rust
   timestamps_enabled,
   timestamp_query_set,
   timestamp_resolve_buf,
   timestamp_staging_buf,
   frame_index: 0,
   last_gpu_log_frame: 0,
   ```
4. In `App::render`, inside the render-pass descriptor (line 583), replace
   `timestamp_writes: None,` with:
   ```rust
   timestamp_writes: self.timestamp_query_set.as_ref().map(|qs| wgpu::RenderPassTimestampWrites {
       query_set: qs,
       beginning_of_pass_write_index: Some(0),
       end_of_pass_write_index: Some(1),
   }),
   ```
   (All uses of `self` inside this block are immutable borrows, so this
   compiles; the query set reference lives only as long as
   `begin_render_pass` needs it.)
5. After the `{ ... }` block that owns `rpass` and before
   `queue.submit(Some(encoder.finish()));` (line 594), add the resolve and the
   periodic copy-to-staging:
   ```rust
   if self.timestamps_enabled {
       let query_set = self.timestamp_query_set.as_ref().unwrap();
       let resolve_buf = self.timestamp_resolve_buf.as_ref().unwrap();
       encoder.resolve_query_set(query_set, 0..2, resolve_buf, 0);
       if self.frame_index - self.last_gpu_log_frame >= 30 {
           let staging = self.timestamp_staging_buf.as_ref().unwrap();
           encoder.copy_buffer_to_buffer(resolve_buf, 0, staging, 0, 16);
       }
   }
   ```
6. After `queue.submit(...)` and before the closing brace of `render`, add the
   readback (stalls the GPU once per 30 frames — acceptable for a diagnostic):
   ```rust
   self.frame_index += 1;
   if self.timestamps_enabled && self.frame_index - self.last_gpu_log_frame >= 30 {
       self.last_gpu_log_frame = self.frame_index;
       let staging = self.timestamp_staging_buf.as_ref().unwrap();
       let slice = staging.slice(..);
       let (tx, rx) = std::sync::mpsc::channel();
       slice.map_async(wgpu::MapMode::Read, move |res| {
           let _ = tx.send(res);
       });
       device.poll(wgpu::Maintain::Wait);
       rx.recv()
           .expect("map callback never fired")
           .expect("buffer map failed");
       let data = slice.get_mapped_range();
       let ticks: &[u64] = bytemuck::cast_slice(&data);
       let period_ns = queue.get_timestamp_period();
       let ms = (ticks[1].saturating_sub(ticks[0])) as f64 * period_ns as f64 / 1e6;
       log::info!("GPU render pass: {ms:.2}ms");
       drop(data);
       staging.unmap();
   }
   ```
   If `render` does not currently receive `queue` (it does: `queue: &wgpu::Queue`
   in the signature at `src/app.rs:482-487`), no signature change is needed.

**Verify**:
- `cargo check` → exit 0, no warnings.
- `cargo build` → exit 0.
- Short run: `WGPU_RT_ORBIT=1 ./target/debug/wgpu-rt.exe > /tmp/orbit_instr.log 2>&1 &` ;
  wait ~20 s (long enough to pass the ~17 s startup), kill it, then:
  `grep -c "GPU render pass:" /tmp/orbit_instr.log` → at least 1, and
  `grep "GPU timestamp queries enabled" /tmp/orbit_instr.log` → present.
  If the timestamp-enabled line is absent and instead the
  `TIMESTAMP_QUERY unavailable` warning appears, STOP and report — proceed to
  Step 3 with FPS-only A/B (Steps 4-5 still apply, measured on FPS).

### Step 2: Wire the dead heatmap to traversal cost

Goal: pressing `H` colors each pixel by its `processed_cells` count (traversal
cost), so the A/B runs in Steps 4-5 are visually verifiable.

In `assets/shaders/chunk.wgsl`:

1. Add a helper after the constants block (after `TRAVERSAL_BOUND`, line 84):
   ```wgsl
   // Diagnostic: traversal-cost heat color (green -> yellow -> red).
   fn heat_color(processed_cells: i32) -> vec4<f32> {
       let t = clamp(f32(processed_cells) / 64.0, 0.0, 1.0);
       return vec4<f32>(t, 1.0 - t, 0.0, 1.0);
   }
   ```
2. At the top of `fs_main`, after `let dir = normalize(delta);` (line 267), add:
   ```wgsl
   let heatmap_on = camera.viewport_and_heatmap.z >= 0.5;
   ```
3. Mip-0 hit (lines 330-333): wrap the existing return so heatmap mode
   substitutes the heat color:
   ```wgsl
   if (mat != 0u) {
       let hit_world = origin + dir * top.t;
       let clip = camera.view_proj * vec4<f32>(hit_world, 1.0);
       if (heatmap_on) {
           return FragmentOutput(heat_color(processed_cells), clip.z / clip.w);
       }
       return FragmentOutput(palette[mat], clip.z / clip.w);
   }
   ```
4. The two discard exits that represent *misses* — the stack-empty exit
   (`if (stack_len == 0) { discard; }` at the top of the traversal loop, line
   289) and the final fallthrough `discard;` at the end of `fs_main` (line
   382) — become, when heatmap is on, a far-depth heat pixel so expensive
   miss pixels are visible:
   ```wgsl
   if (stack_len == 0) {
       if (heatmap_on) {
           return FragmentOutput(heat_color(processed_cells), 1.0);
       }
       discard;
   }
   ```
   and respectively the final fallthrough:
   ```wgsl
   if (heatmap_on) {
       return FragmentOutput(heat_color(processed_cells), 1.0);
   }
   discard;
   ```
   Leave the mid-loop `GLOBAL_CELL_CAP` discard (line 322) untouched.

**Verify**:
- `cargo test --test shader_validate -- --exact chunk_wgsl_parses_and_validates`
  → exactly 1 test passes.
- `cargo run` → window opens; press `H` once: the scene turns into a
  green-yellow-red cost view (red = ≥64 samples); press `H` again to restore
  palette colors; geometry edges are crisp in both modes (no cracks or holes
  that were not there before). Record what you observe (where the red regions
  are — this informs Steps 4-5).

### Step 3: Baseline measurement with the orbit protocol

Goal: capture the pre-optimization numbers with the new instrumentation, and
confirm the "GPU-bound" premise before touching the traversal arithmetic.

1. `cargo build` then:
   `WGPU_RT_ORBIT=1 ./target/debug/wgpu-rt.exe > /tmp/orbit_base.log 2>&1 &`
   Note the PID. Wait ~70 s (the app needs ~17 s to load the scene before
   frames start), then `kill <pid>`. The orbit makes the sweep deterministic:
   the same 70 s window covers the same poses every run, so A/B is comparable.
2. Parse with these commands (from the repo root; the first FPS sample is the
   ~16.5 s startup frame — exclude it):
   ```bash
   grep -oE "\([0-9]+\.[0-9]+ FPS\)" /tmp/orbit_base.log | grep -oE "[0-9]+\.[0-9]+" | tail -n +2 | sort -n | awk '{s+=$1; a[NR]=$1} END{printf "FPS n=%d min=%.1f max=%.1f avg=%.1f median=%.1f\n", NR, a[1], a[NR], s/NR, a[int((NR+1)/2)]}'
   grep -oE "GPU render pass: [0-9.]+ms" /tmp/orbit_base.log | grep -oE "[0-9.]+" | awk '{s+=$1; n++; if(n==1||$1<m)m=$1; if($1>M)M=$1} END{printf "GPU ms n=%d min=%.2f max=%.2f avg=%.2f\n", n, m, M, s/n}'
   ```
3. Record the numbers in your final report. Sanity checks:
   - Avg GPU ms should be close to the wall-clock frame time
     (`1000 / avg FPS`, ~40 ms for ~23 FPS) and the FPS spread should still be
     ~17-30 FPS. If instead **avg GPU ms < ~60% of the wall-clock frame time**
     (e.g. GPU ~10 ms but frames at 44 ms), the render pass is NOT the
     bottleneck — STOP and report the measured split (the plan's premise is
     wrong; the fix is elsewhere, e.g. CPU/debug-build overhead or surface
     pacing, and churning the shader would be wasted work).
   - The heatmap (Step 2) should show red/mid-cost regions in the directions
     that were slowest in the prior FPS curve (grazing angles over dense
     geometry).

**Verify**: both parse commands above produce output; the numbers match the
prior-session baseline within ±10% (avg FPS 23.1, avg frame ~44 ms).

### Step 4: Optimization A — hoisted inverse direction and scalar cell size

Goal: replace ~9 divisions per `init_frame` call with one hoisted reciprocal
plus multiplies, and make `advance_frame` take the minimum it would otherwise
recompute. This is a pure arithmetic re-encoding of the exact same values:
the parallel-axis guard branch (`abs(dir[i]) < PARALLEL_EPS`) is preserved
exactly, so `inv_dir` is never read on a parallel axis (where it would be
infinite). Keep a safety copy of the current shader before editing:
`cp assets/shaders/chunk.wgsl /tmp/chunk_before_A.wgsl`.

In `assets/shaders/chunk.wgsl`:

1. In `fs_main`, immediately after `let dir = normalize(delta);` (line 267),
   add the hoisted reciprocal:
   ```wgsl
   let inv_dir = 1.0 / dir;
   ```
   (IEEE: a zero component gives infinity, but the parallel-axis guard means
   that component is never consumed.)
2. Change `init_frame`'s signature (lines 175-188) to take `inv_dir` after
   `dir`, and rewrite the body's arithmetic:
   ```wgsl
   fn init_frame(
       origin: vec3<f32>,
       dir: vec3<f32>,
       inv_dir: vec3<f32>,
       mip: u32,
       grid_size: i32,
       tex_origin: vec3<i32>,
       bounds_min: vec3<f32>,
       bounds_max: vec3<f32>,
       interval: vec2<f32>,
   ) -> TraversalFrame {
       var frame: TraversalFrame;
       frame.mip = mip;
       frame.grid_size = grid_size;
       frame.tex_origin = tex_origin;
       frame.bounds_min = bounds_min;
       frame.bounds_max = bounds_max;
       frame.interval = interval;
       frame.steps_taken = 0;

       // AABB and parent-cell AABBs are cubes: one scalar cell size per frame.
       let cell_size = (bounds_max.x - bounds_min.x) / f32(grid_size);
       let inv_cell_size = 1.0 / cell_size;
       let entry = origin + dir * interval.x;
       let local = (entry - bounds_min) * inv_cell_size;

       var cell = vec3<i32>(0);
       var t_max = vec3<f32>(INF);
       var t_delta = vec3<f32>(INF);
       var axis_step = vec3<i32>(0);

       for (var i: i32 = 0; i < 3; i = i + 1) {
           var c = i32(floor(local[i]));
           let near_boundary = abs(local[i] - round(local[i])) * cell_size <= T_EPS;
           if (dir[i] < 0.0 && near_boundary) {
               c = c - 1;
           }
           c = clamp(c, 0, grid_size - 1);
           cell[i] = c;

           if (abs(dir[i]) < PARALLEL_EPS) {
               t_max[i] = INF;
               t_delta[i] = INF;
               axis_step[i] = 0;
           } else {
               axis_step[i] = select(-1, 1, dir[i] > 0.0);
               t_delta[i] = cell_size * abs(inv_dir[i]);
               // Absolute ray parameter of the next boundary on this axis.
               let boundary = select(
                   bounds_min[i] + f32(c) * cell_size,
                   bounds_min[i] + f32(c + 1) * cell_size,
                   dir[i] > 0.0,
               );
               t_max[i] = (boundary - origin[i]) * inv_dir[i];
           }
       }

       frame.cell = cell;
       frame.t = interval.x;
       frame.t_max = t_max;
       frame.t_delta = t_delta;
       frame.axis_step = axis_step;
       return frame;
   }
   ```
3. Change `advance_frame` (lines 241-252) to take the precomputed minimum:
   ```wgsl
   fn advance_frame(frame: TraversalFrame, min_t: f32) -> TraversalFrame {
       var f = frame;
       f.t = min_t;
       for (var i: i32 = 0; i < 3; i = i + 1) {
           if (f.t_max[i] - min_t <= T_EPS) {
               f.cell[i] = f.cell[i] + f.axis_step[i];
               f.t_max[i] = f.t_max[i] + f.t_delta[i];
           }
       }
       return f;
   }
   ```
4. Update the two `init_frame` call sites to pass `inv_dir`: the root frame
   (line 280) and the child push (lines 358-366). The child push becomes:
   ```wgsl
   if (child_exit - parent_entry > T_EPS && stack_len < 6) {
       frames[stack_len] = init_frame(
           origin,
           dir,
           inv_dir,
           child_mip,
           2,
           child_tex_origin,
           cell_bmin,
           cell_bmax,
           vec2<f32>(parent_entry, child_exit),
       );
       stack_len = stack_len + 1;
   }
   ```
5. Update all four `advance_frame` call sites (lines 315, 337, 357, 375) to
   pass `next_boundary` — it is in scope at each of them (computed at line
   301 at the top of the loop iteration):
   `frames[top_idx] = advance_frame(top, next_boundary);`

**Verify**:
- `cargo test --test shader_validate -- --exact chunk_wgsl_parses_and_validates`
  → 1 test passes.
- `cargo build`, then run the orbit capture exactly as in Step 3 into
  `/tmp/orbit_A.log`; parse with the same two awk commands.
- **Gate (keep-or-revert)**: avg GPU ms for A must be **≤ 0.95 × baseline avg
  GPU ms** (≥5% better). If it is, keep A and record both numbers. If it is
  not — or the visual smoke (run `cargo run`, look at the scene from several
  angles; geometry must look identical to before: no cracks, holes, or wrong
  colors) shows artifacts — restore `/tmp/chunk_before_A.wgsl` over
  `assets/shaders/chunk.wgsl`, rerun the gates, and STOP and report that A
  did not help (with the measured numbers), so a future plan can target
  something else.
- `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
  → all exit 0.

### Step 5: Optimization B — compact the traversal stack

Goal: remove the 10 redundant scalar fields (`grid_size`, `bounds_min`,
`bounds_max`, `axis_step`) from `TraversalFrame`. They are all derivable:
`grid_size = select(2, ROOT_GRID_SIZE, mip == ROOT_MIP)`; the frame's world
bounds are `chunk_origin + vec3<f32>(tex_origin) * cell_size_m` where
`cell_size_m = exp2(f32(mip) - 3.0)` metres (mip 5 → 4.0, mip 4 → 2.0, ...,
mip 0 → 0.125 — matches `VOXEL_SCALE`); and `axis_step` is a function of the
fragment's `dir` only, identical for every frame, so it is hoisted and passed
as a parameter. This cuts the six-frame stack from 27 to 17 scalars per frame
(−37%), which is the measured-risk reduction for the register-pressure /
occupancy hypothesis. Save a safety copy first:
`cp assets/shaders/chunk.wgsl /tmp/chunk_before_B.wgsl`.

In `assets/shaders/chunk.wgsl`:

1. Remove `grid_size`, `bounds_min`, `bounds_max`, `axis_step` from the
   `TraversalFrame` struct (lines 88-99). The struct keeps only:
   `mip: u32, tex_origin: vec3<i32>, interval: vec2<f32>, cell: vec3<i32>,
   t: f32, t_max: vec3<f32>, t_delta: vec3<f32>, steps_taken: i32`.
2. Rewrite `init_frame` (which currently reads `frame.grid_size = ...`,
   `frame.bounds_min = ...`, etc. from Step 4) to take `chunk_origin` instead
   of `bounds_min`/`bounds_max` and to drop the `grid_size` parameter, deriving
   everything:
   ```wgsl
   fn init_frame(
       origin: vec3<f32>,
       dir: vec3<f32>,
       inv_dir: vec3<f32>,
       mip: u32,
       tex_origin: vec3<i32>,
       chunk_origin: vec3<f32>,
       interval: vec2<f32>,
   ) -> TraversalFrame {
       var frame: TraversalFrame;
       frame.mip = mip;
       frame.tex_origin = tex_origin;
       frame.interval = interval;
       frame.steps_taken = 0;

       let grid_size = select(2, ROOT_GRID_SIZE, mip == ROOT_MIP);
       let cell_size = exp2(f32(mip) - 3.0); // metres: mip5=4.0, mip0=0.125
       let inv_cell_size = 1.0 / cell_size;
       let bounds_min = chunk_origin + vec3<f32>(tex_origin) * cell_size;
       let entry = origin + dir * interval.x;
       let local = (entry - bounds_min) * inv_cell_size;

       var cell = vec3<i32>(0);
       var t_max = vec3<f32>(INF);
       var t_delta = vec3<f32>(INF);

       for (var i: i32 = 0; i < 3; i = i + 1) {
           var c = i32(floor(local[i]));
           let near_boundary = abs(local[i] - round(local[i])) * cell_size <= T_EPS;
           if (dir[i] < 0.0 && near_boundary) {
               c = c - 1;
           }
           c = clamp(c, 0, grid_size - 1);
           cell[i] = c;

           if (abs(dir[i]) < PARALLEL_EPS) {
               t_max[i] = INF;
               t_delta[i] = INF;
           } else {
               t_delta[i] = cell_size * abs(inv_dir[i]);
               let boundary = select(
                   bounds_min[i] + f32(c) * cell_size,
                   bounds_min[i] + f32(c + 1) * cell_size,
                   dir[i] > 0.0,
               );
               t_max[i] = (boundary - origin[i]) * inv_dir[i];
           }
       }

       frame.cell = cell;
       frame.t = interval.x;
       frame.t_max = t_max;
       frame.t_delta = t_delta;
       return frame;
   }
   ```
   Note: the boundary/tie arithmetic is unchanged; the derived `bounds_min` is
   a different float expression than the old stored one (same mathematical
   value, possibly a few ULPs off). That is acceptable — the `T_EPS` guard and
   the negative-boundary correction absorb it — but it is exactly why the
   visual smoke gate in this step exists.
3. Hoist `axis_step` once in `fs_main`, near `inv_dir` (after line 267):
   ```wgsl
   let axis_step = vec3<i32>(
       select(-1, 1, dir.x > 0.0),
       select(-1, 1, dir.y > 0.0),
       select(-1, 1, dir.z > 0.0),
   );
   ```
4. Change `advance_frame` to take `axis_step` as a parameter:
   ```wgsl
   fn advance_frame(frame: TraversalFrame, min_t: f32, axis_step: vec3<i32>) -> TraversalFrame {
       var f = frame;
       f.t = min_t;
       for (var i: i32 = 0; i < 3; i = i + 1) {
           if (f.t_max[i] - min_t <= T_EPS) {
               f.cell[i] = f.cell[i] + axis_step[i];
               f.t_max[i] = f.t_max[i] + f.t_delta[i];
           }
       }
       return f;
   }
   ```
   Update all four call sites to
   `frames[top_idx] = advance_frame(top, next_boundary, axis_step);`.
5. Update the root-frame init (line 280) to the new signature:
   ```wgsl
   frames[0] = init_frame(origin, dir, inv_dir, ROOT_MIP, vec3<i32>(0), chunk_origin, span);
   ```
   (`chunk_origin` is `in.chunk_origin`, already in scope as `chunk_origin` at
   line 257.)
6. Update the coarse-descend branch (lines 346-366): the derived-bounds lines
   go away entirely — the child frame derives its own bounds from
   `child_tex_origin` + `child_mip` + `chunk_origin`. The branch becomes:
   ```wgsl
   if (mat != 0u) {
       let parent_entry = top.t;
       let parent_next = next_boundary;
       let child_exit = min(parent_next, top.interval.y);
       let child_tex_origin = 2 * (top.tex_origin + top.cell);
       let child_mip = top.mip - 1u;

       frames[top_idx] = advance_frame(top, next_boundary, axis_step);

       if (child_exit - parent_entry > T_EPS && stack_len < 6) {
           frames[stack_len] = init_frame(
               origin,
               dir,
               inv_dir,
               child_mip,
               child_tex_origin,
               chunk_origin,
               vec2<f32>(parent_entry, child_exit),
           );
           stack_len = stack_len + 1;
       }
   } else {
       frames[top_idx] = advance_frame(top, next_boundary, axis_step);
   }
   ```
   (`cell_size`, `cell_bmin`, `cell_bmax`, and the old 4-argument
   `init_frame` call are gone.)

**Verify**:
- `cargo test --test shader_validate -- --exact chunk_wgsl_parses_and_validates`
  → 1 test passes.
- `cargo test --test hierarchical_mip_dda` → all pass (the CPU reference is
  untouched; this confirms the contract still matches the shader's intent).
- `cargo build`, orbit capture into `/tmp/orbit_B.log`, parse with the same
  awk commands.
- **Gate (keep-or-revert)**: avg GPU ms for B must be **≤ avg GPU ms after A**
  (strict improvement, no regression). If it is, keep B. If it is not, or the
  visual smoke (`cargo run`; inspect several angles incl. the negative-boundary
  and edge cases from the plan-011 tests) shows cracks/holes/color changes,
  restore `/tmp/chunk_before_B.wgsl`, rerun the gates, and STOP and report B
  as rejected with the measured numbers (A stays).
- `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
  → all exit 0.

### Step 6: Final gates and report

1. Run the full gate suite: `cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings` → all exit 0.
2. One final orbit capture into `/tmp/orbit_final.log` (same protocol) and
   parse FPS + GPU ms.
3. Confirm scope: `git status --short` → only `assets/shaders/chunk.wgsl`,
   `src/app.rs` (and `src/framework.rs` only if the executor needed a minimal
   edit there, which must be reported) plus `plans/` files.
4. Update this plan's row in `plans/README.md` to `DONE` and fill in
   the report table below in the plan file under "Report" (append it):
   | run | avg FPS | min/max FPS | avg GPU ms | verdict |
   |---|---|---|---|---|
   | baseline (Step 3) | | | | |
   | after A (Step 4) | | | | keep/revert |
   | after B (Step 5) | | | | keep/revert |
   | final (Step 6) | | | | |
   Also record: the CPU-vs-GPU split conclusion, what the heatmap showed, and
   any artifact observations.

**Verify**: all gates exit 0; the report table is filled; the row in
`plans/README.md` is `DONE`.

## Test plan

- No new Rust unit tests: the shader has no CPU test seam (the DDA CPU
  reference in `tests/hierarchical_mip_dda.rs` is a separate implementation by
  design — plan 011 built it that way) and the repo gates it via
  `tests/shader_validate.rs` (naga parse). The correctness evidence for Steps
  4-5 is: (a) `chunk_wgsl_parses_and_validates` passes, (b) the CPU reference
  still passes (guards the contract), (c) visual smoke shows no geometric
  change, (d) the deterministic orbit A/B quantifies the perf delta. This
  limitation is deliberate and stated here so the executor does not invent a
  shader-executing test that cannot run headless.
- Perf evidence: the four-row table in Step 6, from `WGPU_RT_ORBIT=1` runs of
  the same ~70 s window (orbit is a pure function of elapsed time — plan 012 —
  so poses match across runs).

## Done criteria

All of the following must be true:

- [ ] `cargo test` exits 0 (includes `chunk_wgsl_parses_and_validates` and
      `hierarchical_mip_dda`).
- [ ] `cargo fmt --check` exits 0.
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0.
- [ ] Baseline (Step 3) confirms the render pass is GPU-bound: avg GPU ms ≥
      ~60% of wall-clock frame time, and FPS/GPU-ms numbers are within ±10% of
      the prior-session baseline (avg FPS 23.1, avg frame ~44 ms).
- [ ] `H` heatmap shows traversal cost and toggles cleanly; default rendering
      is unchanged (palette colors, crisp geometry).
- [ ] Optimization A passed its ≥5% gate (avg GPU ms ≤ 0.95 × baseline); if it
      did not, it was reverted and the STOP condition was reported instead.
- [ ] Optimization B passed its no-regression gate (avg GPU ms ≤ after-A); if
      it did not, it was reverted and reported (A remains).
- [ ] The final capture (Step 6) shows improvement over baseline, and the
      report table in this plan records all four runs.
- [ ] `git status --short` shows only in-scope files changed (`chunk.wgsl`,
      `app.rs`, optionally a reported minimal `framework.rs` edit, plus
      `plans/`).
- [ ] `plans/README.md` row is `DONE` with the measured numbers.

## STOP conditions

Stop and report back (do not improvise) if:

- The drift check or Current state excerpts do not match the live repository.
- Step 1 finds `TIMESTAMP_QUERY` unavailable AND a reasonable gating fix does
  not work; fall back to FPS-only A/B and say so in the report (do not fake GPU
  numbers).
- Step 3 shows the render pass is NOT GPU-bound (avg GPU ms < ~60% of
  wall-clock frame time): the premise is wrong; report the measured CPU/GPU
  split instead of proceeding to shader edits.
- A Step-4/5 gate fails (no ≥5% improvement / regression) after one re-run of
  the capture to rule out noise; revert that step and report the numbers. Do
  not loosen the gate or stack unmeasured changes.
- The visual smoke shows cracks, holes, or changed colors after Step 4 or 5
  (a T_EPS-boundary or derived-bounds regression); revert that step and report.
- An edit turns out to require touching an out-of-scope file (e.g. the CPU mip
  upload, texture format, palette, or the traversal constants).

## Maintenance notes

- The derived-bounds math in Step 5 (`exp2(f32(mip) - 3.0)`) encodes the
  voxel scale (0.125 m) and chunk size (32 m) in the shader — the same
  hardcoding the shader already had (`CHUNK_WORLD_SIZE`, `ROOT_MIP`). If a
  future plan changes `VOXEL_SCALE` or `CHUNK_TEXTURE_SIZE`, the
  `cell_size`/`CHUNK_WORLD_SIZE` constants here must be revisited and the
  plan-011 CPU reference re-run.
- The perf gate thresholds (5% for A, no-regression for B) are deliberately
  conservative; a future plan can relax them once the measurement harness is
  stable. The harness itself (GPU timestamps, Step 1) is the durable asset —
  keep it even if a later plan reworks the shader.
- **Deferred directions** (do not implement here, but worth a future plan):
  - *Per-pixel redundancy across overlapping proxies*: the shader writes
    `@builtin(frag_depth)`, which disables early-Z, so a pixel covered by k
    proxy cubes runs the full DDA k times. Fixing this needs cross-chunk
    traversal (one ray per pixel across all chunks) — explicitly deferred by
    ADR-0002. It is the largest remaining lever and should be measured once the
    present instrumentation is in place.
  - *Half-resolution DDA + upscale*: ~4× fewer fragments, visual trade-off.
  - *Compute-shader path*: better occupancy control and shared-memory tricks,
    full rewrite.
  - *Release-profile perf gate*: the baseline is a debug build; a `--release`
    run (and a `[profile.release]` in Cargo.toml) would separate CPU overhead
    from GPU cost more cleanly for future plans.

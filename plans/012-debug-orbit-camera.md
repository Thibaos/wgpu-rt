# Plan 012: Add a deterministic debug orbit camera for reliable scene coverage

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. A reviewer dispatches you and maintains the index,
> so SKIP any instruction to update `plans/README.md`; the reviewer updates it.
>
> **Drift check (run first)**:
> `git diff --stat 3dcfe40..HEAD -- src/player_controller.rs src/app.rs src/framework.rs`
> Expected output is empty. Then confirm the "Current state" excerpts below
> still match the live files. On any mismatch, STOP and report the exact
> changed path and difference.

## Status

- **Priority:** P1
- **Effort:** S
- **Risk:** LOW — additive debug camera mode; default behavior unchanged
- **Depends on:** none (builds on plan 011, already DONE at the same HEAD)
- **Category:** dx (test tooling / debug camera)
- **Planned at:** commit `3dcfe40`, 2026-07-31

## Why this matters

The plan-011 smoke test rendered only the default camera pose — `PlayerController`
starts at world `(32, 5, 32)` looking down `-Z` (`src/player_controller.rs:31-40`)
— while the loaded scene spans roughly ±150 m across 5 chunk instances
(13.8 M voxels; see the runtime logs in the plan-011 result). A single static
view therefore exercises only a thin slice of the hierarchical mip DDA shader:
most rays never touch the far chunks, and the negative-direction, boundary-tie,
and deep-descent paths are barely covered. This plan adds a **deterministic,
input-free orbit camera** (debug-only, off by default) whose target and radius
are derived from the actually-rendered chunk instances, so a smoke/perf run can
sweep every azimuth and a range of elevations over all geometry for a full
revolution. The orbit math is a pure function of elapsed time — same run,
same poses — which makes the coverage repeatable and the results comparable
across runs. It is explicitly NOT the plan-007 FPS controller (which was
rejected); it is a test camera and must not grow game-play semantics.

## Current state

- `src/player_controller.rs` — the flying noclip camera. `PlayerController`
  holds `translation: Vec3`, `yaw`, `pitch`, a cached `view: Mat4`, and
  `needs_view_update`. `Default` starts at `Vec3::new(32.0, 5.0, 32.0)` with
  yaw/pitch 0 (looks along `-Z`). Public API used by `App`:
  `fly_movement(&mut self, delta_time, keys)`, `rotate(delta)`,
  `camera_position()`, `view()`. The file ends with `compute_view()` /
  `handle_speed_change()`; it has no `#[cfg(test)]` module yet.
- `src/app.rs` — owns `pub player_controller: PlayerController` (line 18).
  Chunk instances are built in `App::init`:
  ```rust
  let chunk_side_world = CHUNK_TEXTURE_SIZE.width as f32 * VOXEL_SCALE; // 32.0 m
  ...
  let gp = chunk.grid_position();
  let position = glam::Vec3::new(
      gp.x as f32 * chunk_side_world,
      gp.y as f32 * chunk_side_world,
      gp.z as f32 * chunk_side_world,
  );
  instances.push(Instance { position });        // app.rs ~145-165
  ```
  `Instance { position: Vec3 }` is defined in `src/render/mod.rs:23-24`; the
  position is the chunk's world-space origin (each chunk is a 32 m cube).
  The camera is driven in `App::render` (app.rs:438-462):
  ```rust
  self.update_delta_time();
  self.player_controller.fly_movement(self.delta_time, keys);
  let aspect = ...;
  let proj_mat = glam::camera::rh::proj::directx::perspective(FRAC_PI_4, aspect, 0.1, 10000.0);
  let view_mat = self.player_controller.view();
  let view_proj = proj_mat * view_mat;
  ...
  let camera_pos = self.player_controller.camera_position();
  let camera_uniforms = CameraUniforms { camera_pos: [...], view_inv, proj_inv, view_proj, viewport_and_heatmap };
  queue.write_buffer(&self.camera_uniform_buf, 0, bytemuck::cast_slice(&[camera_uniforms]));
  ```
  `App` ends with `toggle_heatmap()` and
  `pub fn update_look_position(&mut self, delta: (f64, f64)) { self.player_controller.rotate(delta); }`.
  The `App` struct (app.rs:17-48) has fields `last_frame_update: Instant`,
  `delta_time: Duration`, `surface_width/height: u32`, `instances: Vec<Instance>`,
  and `heatmap: bool` near the end.
- `src/framework.rs` — winit event loop. The heatmap toggle is the exact
  pattern to mirror (framework.rs:327-333):
  ```rust
  if physical_key
      == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyH)
  {
      app.toggle_heatmap();
  }
  self.pressed_keys.insert(logical_key);
  ```
  `Escape` toggles cursor grab (framework.rs:316-326). Mouse movement calls
  `app.update_look_position(delta)` (framework.rs ~382).
- Repo conventions to follow:
  - Rust unit tests live inline in production files under `#[cfg(test)]` —
    see `src/world/chunk.rs:163-240` as the exemplar. Match that style.
  - Logging uses `log::info!` with plain messages, e.g.
    `log::info!("Non-empty chunks: {}, texture binding count: {}", ...)`
    (app.rs:119-122).
  - Code style: `cargo fmt` formatting, `glam` for math (`Vec3`, `Mat4`,
    `vec3`, `Quat`). `f32` math helpers come from `std::f32::consts`
    (the file already imports `FRAC_PI_2` and `TAU` in player_controller.rs).
- The coordinate space for the orbit is world space (same space as
  `player_controller.translation` and `Instance.position`). Voxel-space scene
  bounds are not needed; everything derives from the rendered chunk origins.

## Commands you will need

| Purpose | Command | Expected result |
|---|---|---|
| Drift check | `git diff --stat 3dcfe40..HEAD -- src/player_controller.rs src/app.rs src/framework.rs` | Empty |
| Unit tests | `cargo test --bin wgpu-rt orbit` | All five new orbit tests pass (this package is binary-only — `cargo test --lib` fails with "no library targets found in package `wgpu-rt`"; the orbit tests live in the `wgpu-rt` binary crate) |
| Full tests | `cargo test` | Exit 0; all unit + integration tests pass (incl. `tests/shader_validate.rs`) |
| Formatting | `cargo fmt --check` | Exit 0 |
| Lint | `cargo clippy --all-targets -- -D warnings` | Exit 0 |
| Default-run unchanged | `cargo run` (no env var) | Camera at (32,5,32); fly keys + mouse look behave as before |
| Orbit smoke | `WGPU_RT_ORBIT=1 cargo run` | Orbit params logged at startup; 1 Hz pose log with changing azimuth; geometry visible from many angles; no shader/pipeline error |

No package installation or dependency change is needed.

## Scope

**In scope — the only files to modify:**

- `src/player_controller.rs` — add `OrbitParams`, `DEFAULT_ORBIT_PARAMS`,
  `orbit_pose`, `orbit_radius_from_chunks`, and the inline `#[cfg(test)]`
  module with the named tests below. Do not alter `PlayerController`'s
  existing fly/look behavior.
- `src/app.rs` — add orbit state fields, compute target/radius from
  `instances` in `init`, read the `WGPU_RT_ORBIT` env var, branch the camera
  transform in `render`, add `toggle_orbit_camera()`, and no-op mouse look
  while orbiting.
- `src/framework.rs` — add the `F2` physical-key handler mirroring the `KeyH`
  block exactly.

**Out of scope — do not touch:**

- `src/main.rs` — no argv/CLI plumbing; the env var is read inside `App::init`.
- `src/world/*`, `assets/shaders/chunk.wgsl`, `src/render/*` — no world,
  shader, pipeline, or instance changes.
- `assets/models/*` and any `.vox`/`.world` data.
- Plan 007's FPS controller/collision semantics — the orbit camera is a
  debug/test camera only; do not add gravity, input movement, or collision.
- Changing the default camera pose or any default-run behavior.

## Git workflow

Work in the current checkout. Do not create a git commit unless explicitly
instructed; leave the diff uncommitted for the reviewer. Keep the diff limited
to the Scope list.

## Steps

### Step 1: Confirm baseline and current contracts

Run the drift check and read the excerpts in Current state. Confirm the shader
is untouched (plan 011 is DONE at HEAD) and `PlayerController`/`App`/
`Framework` match the excerpts. Do not edit anything in this step.

**Verify:** `git diff --stat 3dcfe40..HEAD -- src/player_controller.rs src/app.rs src/framework.rs` → empty output.

### Step 2: Add the pure orbit math to `src/player_controller.rs`

Append the following (public, so `App` can use it; tests inline at the bottom
of the file). Keep all existing `PlayerController` code unchanged.

```rust
// --- Debug orbit camera (plan 012) ---------------------------------------
//
// A deterministic, input-free camera for smoke/perf runs. The pose is a pure
// function of elapsed time: same elapsed -> same pose. The orbit target and
// radius are derived from the rendered chunk instances (world space), so the
// sweep covers the actual geometry. This is a TEST camera, not gameplay.

/// Orbit parameters. Angles in radians, periods in seconds.
pub struct OrbitParams {
    pub az_period: f32,   // seconds per full azimuth revolution (0..2*PI)
    pub elev_min: f32,    // minimum elevation above the horizon
    pub elev_max: f32,    // maximum elevation
    pub elev_period: f32, // seconds per full elevation sweep (min -> max -> min)
}

/// Default orbit: one revolution per 60 s, elevation sweeping 5..55 degrees
/// every 30 s. Chosen so rays hit geometry from grazing to steep angles.
pub const DEFAULT_ORBIT_PARAMS: OrbitParams = OrbitParams {
    az_period: 60.0,
    elev_min: 5.0 * std::f32::consts::PI / 180.0,
    elev_max: 55.0 * std::f32::consts::PI / 180.0,
    elev_period: 30.0,
};

/// Returns `(position, look_target)` of the orbit camera at `elapsed`
/// seconds. The elevation is a smooth `cos` sweep that never leaves
/// `[elev_min, elev_max]`; the azimuth advances monotonically. The camera is
/// always exactly `radius` from `target` and looks at it; up is +Y.
pub fn orbit_pose(
    elapsed: f32,
    target: Vec3,
    radius: f32,
    params: &OrbitParams,
) -> (Vec3, Vec3) {
    let azimuth = std::f32::consts::TAU * elapsed / params.az_period;
    let elev = params.elev_min
        + (params.elev_max - params.elev_min)
            * 0.5
            * (1.0 - (std::f32::consts::TAU * elapsed / params.elev_period).cos());
    let (sin_e, cos_e) = elev.sin_cos();
    let (sin_a, cos_a) = azimuth.sin_cos();
    let pos = target
        + radius * Vec3::new(cos_e * cos_a, sin_e, cos_e * sin_a);
    (pos, target)
}

/// Returns an orbit radius (metres) that frames every chunk: the distance from
/// the chunk-centroid `target` to the farthest corner of any chunk AABB
/// `[origin, origin + side]^3`, times `margin`, floored at `chunk_side_world`.
/// `chunk_origins` are the world-space origins of the non-empty chunks.
pub fn orbit_radius_from_chunks(
    chunk_origins: &[Vec3],
    chunk_side_world: f32,
    margin: f32,
) -> f32 {
    let half = chunk_side_world * 0.5;
    let target = chunk_origins
        .iter()
        .fold(Vec3::ZERO, |acc, o| acc + *o)
        / chunk_origins.len().max(1) as f32
        + Vec3::splat(half);
    let mut max_dist = 0.0f32;
    for o in chunk_origins {
        for x in [o.x, o.x + chunk_side_world] {
            for y in [o.y, o.y + chunk_side_world] {
                for z in [o.z, o.z + chunk_side_world] {
                    max_dist = max_dist.max(Vec3::new(x, y, z).distance(target));
                }
            }
        }
    }
    (max_dist * margin).max(chunk_side_world)
}
```

Add a `#[cfg(test)] mod tests` at the bottom of `src/player_controller.rs`
(mirror the style of `src/world/chunk.rs:163-240`) with these named tests:

- `orbit_pose_is_deterministic` — call `orbit_pose` twice with identical
  arguments; assert the two `(pos, target)` tuples are exactly equal.
- `orbit_pose_is_antipodal_after_half_revolution` — with
  `elev_min == elev_max` (fixed nonzero elevation, e.g. 30° in radians), the
  poses at `t` and `t + az_period/2` are opposite points on the same latitude
  circle: the azimuth advances by exactly π, so the horizontal offsets from
  `target` flip sign while `y` is unchanged. Assert
  `((pos_b.x - target.x) + (pos_a.x - target.x)).abs() < 1e-3`, the same for
  `z`, and `(pos_b.y - pos_a.y).abs() < 1e-4`. IMPORTANT: do NOT assert
  `pos_b == 2 * target - pos_a` componentwise — that only holds at elevation
  0; at nonzero elevation both y components are `target.y + radius * sin(elev)`
  (equal, not negated). Use epsilon (`<`) comparisons, never `==` on f32
  (bare `==` trips `clippy::float_cmp` under the `-D warnings` gate). Use
  modest coordinates (order 1–100) to avoid float cancellation, e.g.
  `target = vec3(10.0, -5.0, 3.0)`, `radius = 50.0`,
  `elev_min = elev_max = 30.0 * PI / 180.0`, `az_period = 60.0`,
  `elev_period = 30.0`, `t = 7.5`.
- `orbit_elevation_stays_within_bounds` — with `DEFAULT_ORBIT_PARAMS`, sample
  `elapsed` in `[0, 2 * elev_period)` in 200 steps; derive elevation from the
  returned position: `sin(elev) = (pos.y - target.y) / radius`; assert
  `elev` is within `[elev_min - 1e-4, elev_max + 1e-4]` for every sample.
- `orbit_pose_distance_is_radius` — for several samples across two azimuth
  periods, assert `(pos - target).length() == radius` within `1e-4`.
- `orbit_radius_frames_all_chunks` — synthetic `chunk_origins` (e.g.
  `[vec3(0.0, 0.0, 0.0), vec3(64.0, 0.0, 64.0), vec3(-96.0, 32.0, -48.0)]`,
  `chunk_side_world = 32.0`, `margin = 1.3`): compute `target` the same way as
  the helper (centroid + half side) and assert the returned radius is
  `>= max corner distance` and `>= chunk_side_world`. Also assert an empty
  slice returns `chunk_side_world` (the floor).

Use `glam::{Vec3, vec3}` — the file already imports `Vec3, vec3`.

**Verify:** `cargo test --bin wgpu-rt orbit` → all five new tests pass, and no
existing unit test regresses (`cargo test --bin wgpu-rt` → exit 0). NOTE: this
package has no library target (`cargo test --lib` errors with "no library
targets found in package `wgpu-rt`") — the orbit tests live in the `wgpu-rt`
binary crate, so filter with `--bin wgpu-rt`.

### Step 3: Wire the orbit camera into `App`

In `src/app.rs`:

1. Add fields to the `App` struct (near the other state fields, e.g. after
   `heatmap: bool`):
   ```rust
   // Debug orbit camera (plan 012)
   orbit_enabled: bool,
   orbit_elapsed: Duration,
   orbit_target: glam::Vec3, // app.rs does not import glam types; use the fully-qualified path
   orbit_radius: f32,
   last_orbit_log_secs: u64,
   ```
2. In `App::init`, AFTER the `instances` vector is fully built (after the
   `if non_empty_chunks.is_empty() { ... } else { ... }` block that pushes
   `instances`, i.e. right before or after the `"Created {} chunk textures..."`
   log), add:
   ```rust
   let orbit_enabled = std::env::var("WGPU_RT_ORBIT").map(|v| v == "1").unwrap_or(false);
   let (orbit_target, orbit_radius) = if orbit_enabled {
       if instances.is_empty() {
           log::info!("Orbit camera: no chunks; falling back to target (0,0,0) radius 64.0");
           (glam::Vec3::ZERO, chunk_side_world * 2.0)
       } else {
           let origins: Vec<glam::Vec3> = instances.iter().map(|i| i.position).collect();
           let target = origins.iter().fold(glam::Vec3::ZERO, |a, o| a + *o)
               / origins.len() as f32
               + glam::Vec3::splat(chunk_side_world * 0.5);
           let radius = crate::player_controller::orbit_radius_from_chunks(
               &origins,
               chunk_side_world,
               1.3,
           );
           log::info!(
               "Orbit camera: target=({:.1},{:.1},{:.1}) radius={:.1}m chunks={} (azimuth 60s, elevation 5..55 deg)",
               target.x, target.y, target.z, radius, origins.len(),
           );
           (target, radius)
       }
   } else {
       (glam::Vec3::ZERO, chunk_side_world)
   };
   ```
   Initialize ALL FIVE new fields in the `App { ... }` constructor literal:
   `orbit_enabled,` `orbit_target,` `orbit_radius,` (shorthand — the locals
   computed above have exactly these names), `orbit_elapsed: Duration::ZERO,`,
   and `last_orbit_log_secs: 0,`. Omitting any of the five is a compile error
   (`E0063 missing fields ...`).
3. In `App::render`, replace EXACTLY THREE statements with the single branch
   below: (a) the `self.player_controller.fly_movement(self.delta_time, keys);`
   call, (b) the `let view_mat = self.player_controller.view();` line, and (c)
   the `let camera_pos = self.player_controller.camera_position();` line.
   Statement (a) is replaced IN PLACE by the whole branch; statements (b) and
   (c) are removed. The `let aspect = ...;`, `let proj_mat = ...;`,
   `let view_proj = ...;`, `let view_inv = ...;`, and `let proj_inv = ...;`
   statements that currently sit between them stay in place, unchanged, in
   their current order.
   ```rust
   let (view_mat, camera_pos) = if self.orbit_enabled {
       self.orbit_elapsed += self.delta_time;
       let orbit_params = crate::player_controller::DEFAULT_ORBIT_PARAMS;
       let (pos, target) = crate::player_controller::orbit_pose(
           self.orbit_elapsed.as_secs_f32(),
           self.orbit_target,
           self.orbit_radius,
           &orbit_params,
       );
       let view_mat = glam::camera::rh::view::look_at_mat4(pos, target, glam::Vec3::Y);
       let secs = self.orbit_elapsed.as_secs();
       if secs != self.last_orbit_log_secs {
           self.last_orbit_log_secs = secs;
           log::info!(
               "Orbit: t={:.1}s az={:.1} deg elev={:.1} deg pos=({:.1},{:.1},{:.1})",
               secs,
               (std::f32::consts::TAU * secs as f32 / orbit_params.az_period)
                   .to_degrees()
                   .rem_euclid(360.0),
               ((pos.y - target.y) / self.orbit_radius).asin().to_degrees(),
               pos.x, pos.y, pos.z,
           );
       }
       (view_mat, pos)
   } else {
       self.player_controller.fly_movement(self.delta_time, keys);
       let view_mat = self.player_controller.view();
       let camera_pos = self.player_controller.camera_position();
       (view_mat, camera_pos)
   };
   ```
   Everything downstream of the branch (`camera_uniforms`, `write_buffer`, the
   render pass) is unchanged — `view_mat` and `camera_pos` now come from the
   branch; `view_proj`/`view_inv`/`proj_inv` are computed from `view_mat` /
   `proj_mat` exactly as before. The projection (`FRAC_PI_4`, 0.1, 10000.0)
   stays as-is.
4. Add a toggle method next to `toggle_heatmap`:
   ```rust
   pub fn toggle_orbit_camera(&mut self) {
       self.orbit_enabled = !self.orbit_enabled;
       if self.orbit_enabled {
           self.orbit_elapsed = Duration::ZERO;
           self.last_orbit_log_secs = 0;
           log::info!(
               "Orbit camera enabled: target=({:.1},{:.1},{:.1}) radius={:.1}m",
               self.orbit_target.x, self.orbit_target.y, self.orbit_target.z, self.orbit_radius,
           );
       } else {
           log::info!("Orbit camera disabled");
       }
   }
   ```
5. Make mouse look inert while orbiting:
   ```rust
   pub fn update_look_position(&mut self, delta: (f64, f64)) {
       if self.orbit_enabled {
           return;
       }
       self.player_controller.rotate(delta);
   }
   ```

**Verify:** `cargo check` → exit 0 with no warnings. `cargo test` → exit 0.

### Step 4: Add the F2 toggle to `src/framework.rs`

Mirror the `KeyH` block exactly (framework.rs:327-333): immediately after the
`KeyH` block, inside the same `WindowEvent::KeyboardInput` `Pressed` arm, add:

```rust
if physical_key
    == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F2)
{
    app.toggle_orbit_camera();
}
```

Do not touch the `Escape` cursor-grab handling or the released-key arm.

**Verify:** `cargo check` → exit 0.

### Step 5: Run the complete gates and smoke tests

Run all repository gates, then two smoke runs:

1. Default run (no env var, no F2): `cargo run` → camera at `(32,5,32)`, fly
   keys (ZQSD/space/ctrl) and mouse look work as before; no orbit log lines;
   no shader error. Close with Escape/close control.
2. Orbit run: `WGPU_RT_ORBIT=1 cargo run` → the startup log prints the orbit
   target/radius/chunks; the 1 Hz `Orbit:` log shows azimuth advancing (past
   360° after ~60 s) and elevation within 5..55 deg; the window shows geometry
   from many angles (chunks visible from all sides); FPS stays within a steady
   band; press `F2` mid-run to confirm it disables orbit (orbit logs stop,
   fly/mouse controls return) and `F2` again to re-enable (orbit restarts at
   t=0); no shader/pipeline/validation error at any point. Run at least 65 s
   total so at least one full revolution is observed; record the FPS spread and
   first/last azimuth in the result report.

**Verify:**
`cargo test && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
→ all exit 0; both smoke runs above behave as described.

## Test plan

- New inline `#[cfg(test)]` module in `src/player_controller.rs` (pattern:
  `src/world/chunk.rs:163-240`) with the five named tests from Step 2:
  determinism, half-revolution antipodal symmetry, elevation bounds across a
  full sweep, distance == radius, and radius-frames-all-chunks (including the
  empty-slice floor).
- These tests need no GPU: `cargo test --bin wgpu-rt orbit` runs them
  headlessly.
- Existing tests (`tests/shader_validate.rs`, and the chunk mip tests inline
  in `src/world/chunk.rs`) must continue to pass through the full `cargo test`
  gate — the orbit change must not touch them. (Note: `tests/hierarchical_mip_dda.rs`
  referenced by plan 011 does NOT exist in the tree — plan 011's README row
  says DONE but that file was never committed. Do not create it as part of
  this plan.)

## Done criteria

All of the following must be true:

- [ ] `cargo test --bin wgpu-rt orbit` passes all five new orbit tests.
- [ ] `cargo test` exits 0 (all existing tests still pass).
- [ ] `cargo fmt --check` exits 0.
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0.
- [ ] Default run (`cargo run`, no env var) is unchanged: camera at `(32,5,32)`,
      fly keys and mouse look work; no orbit log lines.
- [ ] `WGPU_RT_ORBIT=1 cargo run` logs orbit target/radius/chunk count at
      startup, logs a 1 Hz pose line with azimuth advancing through ≥ 360°,
      shows geometry from multiple azimuths in the window, and runs ≥ 60 s
      with no shader/pipeline error.
- [ ] `F2` toggles orbit on/off and orbit restarts at `t=0` on re-enable;
      `Escape` cursor-grab and `H` heatmap are unaffected.
- [ ] Only `src/player_controller.rs`, `src/app.rs`, and `src/framework.rs`
      are modified (`git status` shows no other changes).
- [ ] Plan 012's row in `plans/README.md` shows `DONE` (reviewer maintains the
      index — do not edit it yourself).

## STOP conditions

Stop and report back (do not improvise) if:

- The drift check or Current state excerpts do not match the live repository.
- `orbit_pose`/`orbit_radius_from_chunks` cannot be made pure
  deterministic functions without extra state.
- The `render` refactor cannot keep the existing default camera path
  byte-for-byte equivalent (fly movement + mouse look) when orbit is disabled.
- `winit::keyboard::KeyCode::F2` does not exist in this winit version (report
  the exact compile error; the equivalent `NamedKey::F2` is an acceptable
  fallback only if you also report which you used).
- Any verification command fails twice after a reasonable in-scope correction.
- Implementing this appears to require touching an out-of-scope file
  (`src/main.rs`, `src/world/*`, `src/render/*`, shaders, models, or the
  rejected plan-007 controller).

## Maintenance notes

- The orbit constants (`DEFAULT_ORBIT_PARAMS`) are hardcoded test parameters;
  if a future change alters chunk size or world scale, re-verify the
  `orbit_radius_from_chunks` floor and margin still frame the chunks (the unit
  tests cover the math, the smoke run covers reality).
- The 1 Hz pose log is test evidence; keep it terse (one line) so it does not
  drown the frame-time logger in long orbit runs.
- If a future plan adds camera persistence, screenshots, or a headless frame
  capture, the orbit mode is the natural place to drive it — but do not add
  those here.
- Plan 007 (FPS controller) was rejected; if it is ever revived, the orbit
  camera must remain an explicitly separate, toggleable debug mode.

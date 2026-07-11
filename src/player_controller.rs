use core::f32;
use std::{
    collections::HashSet,
    f32::consts::{FRAC_PI_2, TAU},
    time::Duration,
};

use glam::{Mat4, Quat, Vec3, vec3};
use winit::keyboard::{Key, NamedKey, SmolStr};

use crate::tree64::query::{self, CollisionResult};
use crate::tree64::renderer::GpuTree64;

const FORWARD: Key = Key::Character(SmolStr::new_static("z"));
const LEFT: Key = Key::Character(SmolStr::new_static("q"));
const BACKWARD: Key = Key::Character(SmolStr::new_static("s"));
const RIGHT: Key = Key::Character(SmolStr::new_static("d"));

const UP: Key = Key::Named(NamedKey::Space);
const CONTROL: Key = Key::Named(NamedKey::Control);

/// Key used for jumping in FPS mode.
const JUMP_KEY: Key = Key::Named(NamedKey::Space);

/// Control mode for the player controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlMode {
    Fly,
    Fps,
}

/// Physics constants for FPS mode.
pub const TICK_DURATION: Duration = Duration::from_nanos(16_666_667); // 60 Hz
const GRAVITY: f32 = 30.0;
const PLAYER_HALF_WIDTH: f32 = 0.3; // AABB half-width in XZ (total width = 0.6 m)
const PLAYER_HEIGHT: f32 = 1.8;
const EYE_OFFSET: f32 = 1.65;
const GROUND_SPEED: f32 = 6.0;
const STEP_HEIGHT: f32 = 0.5;
const TERMINAL_DOWN: f32 = 50.0;
const TERMINAL_UP: f32 = 30.0;
const JUMP_IMPULSE: f32 = 8.0;

/// Compute the player's body AABB min given the foot position.
fn player_body_min(feet: Vec3) -> Vec3 {
    Vec3::new(feet.x - PLAYER_HALF_WIDTH, feet.y, feet.z - PLAYER_HALF_WIDTH)
}

/// Compute the player's body AABB max given the foot position.
fn player_body_max(feet: Vec3) -> Vec3 {
    Vec3::new(
        feet.x + PLAYER_HALF_WIDTH,
        feet.y + PLAYER_HEIGHT,
        feet.z + PLAYER_HALF_WIDTH,
    )
}

pub struct PlayerController {
    pub speed: f32,
    pub sensitivity: f64,
    pub translation: Vec3,
    pub control_mode: ControlMode,

    /// FPS physics state
    pub velocity: Vec3,
    pub is_grounded: bool,

    yaw: f32,
    pitch: f32,

    view: Mat4,
    needs_view_update: bool,
}

impl Default for PlayerController {
    fn default() -> Self {
        let translation = Vec3::new(32.0, 5.0, 32.0);

        Self {
            speed: 32.0,
            sensitivity: 0.001,
            translation,
            control_mode: ControlMode::Fps,
            velocity: Vec3::ZERO,
            is_grounded: false,
            yaw: 0.0,
            pitch: 0.0,
            view: glam::camera::rh::view::look_at_mat4(
                translation,
                Vec3::new(32.0, 4.0, 31.0),
                Vec3::Y,
            ),
            needs_view_update: true,
        }
    }
}

impl PlayerController {
    const MAX_PITCH: f32 = FRAC_PI_2 - 0.01;
    const MIN_PITCH: f32 = -Self::MAX_PITCH;

    /// Camera position in world space.
    pub fn camera_position(&self) -> Vec3 {
        match self.control_mode {
            ControlMode::Fly => self.translation,
            ControlMode::Fps => self.translation + Vec3::new(0.0, EYE_OFFSET, 0.0),
        }
    }

    pub fn view(&mut self) -> Mat4 {
        if self.needs_view_update {
            self.compute_view();
        }
        self.view
    }

    /// Run one physics tick (60 Hz) for FPS mode.
    ///
    /// Handles gravity, WASD input, collision resolution (Y→X→Z priority
    /// with swept steps ≤ 0.3 m), step-up for obstacles ≤ 0.5 m, and ground
    /// detection.
    pub fn physics_tick(&mut self, tree: Option<&GpuTree64>, keys: &HashSet<Key<SmolStr>>) {
        if self.control_mode != ControlMode::Fps {
            return;
        }

        // ---- gravity ----
        if !self.is_grounded {
            self.velocity.y -= GRAVITY * TICK_DURATION.as_secs_f32();
        }

        // ---- terminal velocity ----
        self.velocity.y = self.velocity.y.clamp(-TERMINAL_DOWN, TERMINAL_UP);

        // ---- jump ----
        if self.is_grounded && keys.contains(&JUMP_KEY) {
            self.velocity.y = JUMP_IMPULSE;
            self.is_grounded = false;
        }

        // ---- WASD horizontal input ----
        let view_inv = self.view().inverse();
        let abs_forward = view_inv.transform_vector3(-Vec3::Z);
        let forward = vec3(abs_forward.x, 0.0, abs_forward.z).normalize_or_zero();
        let right = view_inv.transform_vector3(Vec3::X);

        if self.is_grounded {
            // Ground: instant 6 m/s.
            if keys.contains(&FORWARD) {
                self.velocity.x = forward.x * GROUND_SPEED;
                self.velocity.z = forward.z * GROUND_SPEED;
            } else if keys.contains(&BACKWARD) {
                self.velocity.x = -forward.x * GROUND_SPEED;
                self.velocity.z = -forward.z * GROUND_SPEED;
            } else if keys.contains(&RIGHT) {
                self.velocity.x = right.x * GROUND_SPEED;
                self.velocity.z = right.z * GROUND_SPEED;
            } else if keys.contains(&LEFT) {
                self.velocity.x = -right.x * GROUND_SPEED;
                self.velocity.z = -right.z * GROUND_SPEED;
            } else {
                self.velocity.x = 0.0;
                self.velocity.z = 0.0;
            }
        } else {
            // Air: 50% acceleration, no friction.
            let air_accel = GROUND_SPEED * 0.5 * TICK_DURATION.as_secs_f32();
            if keys.contains(&FORWARD) {
                self.velocity.x += forward.x * air_accel;
                self.velocity.z += forward.z * air_accel;
            }
            if keys.contains(&BACKWARD) {
                self.velocity.x -= forward.x * air_accel;
                self.velocity.z -= forward.z * air_accel;
            }
            if keys.contains(&RIGHT) {
                self.velocity.x += right.x * air_accel;
                self.velocity.z += right.z * air_accel;
            }
            if keys.contains(&LEFT) {
                self.velocity.x -= right.x * air_accel;
                self.velocity.z -= right.z * air_accel;
            }
        }

        // ---- integrate with swept collision resolution ----
        let dt = TICK_DURATION.as_secs_f32();
        let total_disp = self.velocity * dt;

        // Sweep at most 0.3 m per step.
        const MAX_STEP: f32 = 0.3;
        let steps = (total_disp.length() / MAX_STEP).ceil() as usize;
        let step_disp = if steps > 0 {
            total_disp / steps as f32
        } else {
            total_disp
        };

        let mut pos = self.translation;

        for _ in 0..steps {
            let candidate = pos + step_disp;

            let body_min = player_body_min(candidate);
            let body_max = player_body_max(candidate);

            let collision = tree.and_then(|t| match query::aabb_collides(t, body_min, body_max) {
                CollisionResult::Blocked {
                    penetration_x,
                    penetration_neg_x,
                    penetration_y,
                    penetration_neg_y,
                    penetration_z,
                    penetration_neg_z,
                } => Some((penetration_x, penetration_neg_x, penetration_y, penetration_neg_y, penetration_z, penetration_neg_z)),
                CollisionResult::Clear => None,
            });

            let Some((px, nx, py, ny, pz, nz)) = collision else {
                // No collision — accept the step.
                pos = candidate;
                continue;
            };

            // ---- resolve: smallest-penetration depenetration ----
            let mut resolved = candidate;

            if let Some(t) = tree {
                let is_clear = |p: Vec3| -> bool {
                    matches!(
                        query::aabb_collides(t, player_body_min(p), player_body_max(p)),
                        CollisionResult::Clear
                    )
                };

                // Find the smallest penetration and resolve along that axis.
                let mut best_dirs = [(py, 0u8), (ny, 1), (px, 2), (nx, 3), (pz, 4), (nz, 5)];
                best_dirs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                for &(dist, dir) in &best_dirs {
                    let mut test = resolved;
                    match dir {
                        0 => test.y += dist, // +Y
                        1 => test.y -= dist, // -Y
                        2 => test.x += dist, // +X
                        3 => test.x -= dist, // -X
                        4 => test.z += dist, // +Z
                        5 => test.z -= dist, // -Z
                        _ => unreachable!(),
                    }
                    if is_clear(test) {
                        resolved = test;
                        break;
                    }
                }

                // Step-up: if still blocked, try raising and re-resolving.
                if !is_clear(resolved) {
                    let su = Vec3::new(resolved.x, resolved.y + STEP_HEIGHT, resolved.z);
                    if is_clear(su) {
                        // Try horizontal movement from elevated position.
                        let su_fwd = Vec3::new(su.x + step_disp.x, su.y, su.z + step_disp.z);
                        if is_clear(su_fwd) {
                            resolved = su_fwd;
                        } else {
                            let su_x = Vec3::new(su.x + step_disp.x, su.y, su.z);
                            if is_clear(su_x) {
                                resolved = su_x;
                            } else {
                                resolved = su;
                            }
                        }
                    }
                }
            }

            pos = resolved;
        }

        // ---- ground detection at final position ----

        // Clamp to tree bounds.
        if let Some(t) = tree {
            let world_size = (1u64 << t.tree_scale) as f32;
            let ox = t.root_offset[0] as f32;
            let oy = t.root_offset[1] as f32;
            let oz = t.root_offset[2] as f32;
            pos.x = pos.x.clamp(ox + PLAYER_HALF_WIDTH, ox + world_size - PLAYER_HALF_WIDTH);
            pos.y = pos.y.clamp(oy, oy + world_size - PLAYER_HEIGHT);
            pos.z = pos.z.clamp(oz + PLAYER_HALF_WIDTH, oz + world_size - PLAYER_HALF_WIDTH);
        }

        let body_min = player_body_min(pos);
        let body_max = player_body_max(pos);

        let ground_hit = tree.is_some_and(|t| {
            let slab_min = Vec3::new(body_min.x, body_min.y - 1.0, body_min.z);
            let slab_max = Vec3::new(body_max.x, body_min.y, body_max.z);
            matches!(
                query::aabb_collides(t, slab_min, slab_max),
                CollisionResult::Blocked { .. }
            )
        });

        if ground_hit {
            self.is_grounded = true;
            self.velocity.y = 0.0;

            // Snap to surface.
            if let Some(t) = tree {
                match query::aabb_collides(t, body_min, body_max) {
                    CollisionResult::Blocked { penetration_y, .. } => {
                        // Body intersects: push up.
                        pos.y += penetration_y;
                    }
                    CollisionResult::Clear => {
                        // Body is clear but slab detects ground. Snap down
                        // to the surface: test body shifted 1 voxel lower.
                        let low_y = pos.y - 1.0;
                        let low_min = Vec3::new(body_min.x, low_y, body_min.z);
                        let low_max = Vec3::new(body_max.x, low_y + PLAYER_HEIGHT, body_max.z);
                        if let CollisionResult::Blocked { penetration_y, .. } =
                            query::aabb_collides(t, low_min, low_max)
                        {
                            pos.y = low_y + penetration_y;
                        }
                    }
                }
            }

            self.translation = pos;
        } else {
            self.is_grounded = false;
            self.translation = pos;
        }

        self.needs_view_update = true;
    }

    pub fn fly_movement(&mut self, delta_time: Duration, keys: &HashSet<Key<SmolStr>>) {
        if self.control_mode != ControlMode::Fly {
            return;
        }

        let view_inverse = self.view().inverse();
        let absolute_forward = view_inverse.transform_vector3(-Vec3::Z);
        let forward = vec3(absolute_forward.x, 0.0, absolute_forward.z).normalize();
        let right = view_inverse.transform_vector3(Vec3::X);

        let mut velocity = Vec3::ZERO;

        if keys.contains(&FORWARD) {
            velocity += forward;
        } else if keys.contains(&BACKWARD) {
            velocity -= forward;
        }
        if keys.contains(&RIGHT) {
            velocity += right;
        } else if keys.contains(&LEFT) {
            velocity -= right;
        }
        if keys.contains(&UP) {
            velocity += Vec3::Y;
        } else if keys.contains(&CONTROL) {
            velocity -= Vec3::Y;
        }

        velocity = velocity.normalize_or_zero();

        self.translation += velocity * delta_time.as_secs_f32() * self.speed;

        self.needs_view_update = true;
    }

    pub fn rotate(&mut self, delta: (f64, f64)) {
        self.yaw -= (delta.0 * self.sensitivity) as f32;
        self.pitch -= (delta.1 * self.sensitivity) as f32;

        self.yaw = self.yaw.rem_euclid(TAU);

        self.pitch = self.pitch.clamp(Self::MIN_PITCH, Self::MAX_PITCH);

        self.needs_view_update = true;
    }

    fn orientation(&self) -> Quat {
        let yaw_q = Quat::from_rotation_y(self.yaw);
        let pitch_q = Quat::from_rotation_x(self.pitch);

        yaw_q * pitch_q
    }

    fn compute_view(&mut self) {
        let camera_pos = self.camera_position();
        let rot = self.orientation();
        let forward = rot * Vec3::new(0.0, 0.0, -1.0);
        let up = rot * Vec3::new(0.0, 1.0, 0.0);

        self.view =
            glam::camera::rh::view::look_at_mat4(camera_pos, camera_pos + forward, up);
    }

    pub fn handle_speed_change(&mut self, y_delta: f32) {
        if y_delta.is_sign_positive() {
            self.speed *= 1.5;
        } else {
            self.speed /= 1.5;
        }
    }

    /// Toggle between Fly and FPS control modes.
    pub fn toggle_control_mode(&mut self) {
        match self.control_mode {
            ControlMode::Fly => {
                self.control_mode = ControlMode::Fps;
                // Start airborne at current position with zero velocity.
                self.velocity = Vec3::ZERO;
                self.is_grounded = false;
            }
            ControlMode::Fps => {
                self.control_mode = ControlMode::Fly;
                // Zero velocity, keep position.
                self.velocity = Vec3::ZERO;
                self.is_grounded = false;
            }
        }
        self.needs_view_update = true;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Run N physics ticks and return the final state.
    fn run_ticks(
        controller: &mut PlayerController,
        tree: Option<&GpuTree64>,
        n: usize,
    ) {
        let empty_keys = HashSet::new();
        for _ in 0..n {
            controller.physics_tick(tree, &empty_keys);
        }
    }

    /// Simple VoxelModel backed by a HashMap.
    struct SimpleModel {
        occupied: HashMap<[usize; 3], u8>,
        dims: [u32; 3],
    }

    impl tree64::VoxelModel<u8> for &SimpleModel {
        fn dimensions(&self) -> [u32; 3] {
            self.dims
        }
        fn access(&self, coord: [usize; 3]) -> Option<u8> {
            self.occupied.get(&coord).copied()
        }
    }

    /// Build a GpuTree64 from occupied voxels with a given dimension.
    fn build_tree(voxels: &[([u32; 3], u8)], dim: u32) -> GpuTree64 {
        let mut occupied = HashMap::new();
        for &(coord, value) in voxels {
            occupied.insert(
                [coord[0] as usize, coord[1] as usize, coord[2] as usize],
                value,
            );
        }
        let model = SimpleModel {
            occupied,
            dims: [dim; 3],
        };
        GpuTree64::from_model(&&model)
    }

    /// Build a simple ground plane at y=0 (4×4 floor).
    fn ground_plane() -> GpuTree64 {
        let mut voxels = Vec::new();
        for x in 0..4u32 {
            for z in 0..4u32 {
                voxels.push(([x, 0, z], 1u8));
            }
        }
        build_tree(&voxels, 4)
    }

    #[test]
    fn gravity_accelerates_downward() {
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 10.0, 2.0);
        ctrl.velocity = Vec3::ZERO;
        ctrl.is_grounded = false;

        let dt = TICK_DURATION.as_secs_f32();
        let expected_vy = -GRAVITY * dt;

        ctrl.physics_tick(None, &HashSet::new());
        assert!((ctrl.velocity.y - expected_vy).abs() < 1e-5);
    }

    #[test]
    fn gravity_not_applied_when_grounded() {
        let tree = ground_plane();
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 1.0, 2.0); // above floor
        ctrl.velocity = Vec3::ZERO;
        ctrl.is_grounded = true;

        ctrl.physics_tick(Some(&tree), &HashSet::new());
        // Still grounded, velocity still zero.
        assert!(ctrl.is_grounded);
        assert_eq!(ctrl.velocity.y, 0.0);
    }

    #[test]
    fn falls_and_lands_on_solid_ground() {
        let tree = ground_plane();
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 5.0, 2.0);
        ctrl.velocity = Vec3::ZERO;
        ctrl.is_grounded = false;

        // Run enough ticks for the player to fall to the ground.
        // At 20 m/s², starting from 5m, takes ~0.7s to fall = ~42 ticks.
        run_ticks(&mut ctrl, Some(&tree), 200);

        assert!(ctrl.is_grounded);
        assert_eq!(ctrl.velocity.y, 0.0);
        // Player AABB bottom should be at or slightly above y=1.0 (top of floor voxels).
        assert!(ctrl.translation.y >= 1.0, "player y={}", ctrl.translation.y);
        assert!(ctrl.translation.y < 1.1, "player y={}", ctrl.translation.y);
    }

    #[test]
    fn becomes_airborne_when_no_ground_below() {
        // Build a tree with floor only on one side, so moving off it
        // makes the player airborne.
        let mut voxels = Vec::new();
        for x in 0..2u32 {
            for z in 0..4u32 {
                voxels.push(([x, 0, z], 1u8));
            }
        }
        let tree = build_tree(&voxels, 4);
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(1.0, 1.0, 2.0);
        ctrl.velocity = Vec3::new(2.0, 0.0, 0.0);
        ctrl.is_grounded = true;

        // Move right off the edge. The floor only covers x=[0,2).
        let mut keys = HashSet::new();
        keys.insert(RIGHT.clone());
        for _ in 0..30 {
            ctrl.physics_tick(Some(&tree), &keys);
        }

        // Should now be airborne since no ground under feet.
        assert!(!ctrl.is_grounded, "Expected airborne, player at {}", ctrl.translation);
    }

    #[test]
    fn stands_on_lattice_floor() {
        // Checkerboard floor at y=0.
        let mut voxels: Vec<([u32; 3], u8)> = Vec::new();
        for x in 0..4u32 {
            for z in 0..4u32 {
                if (x + z) % 2 == 0 {
                    voxels.push(([x, 0, z], 1u8));
                }
            }
        }
        let tree = build_tree(&voxels, 4);

        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 1.5, 2.0);
        ctrl.velocity = Vec3::ZERO;
        ctrl.is_grounded = false;

        run_ticks(&mut ctrl, Some(&tree), 100);
        assert!(ctrl.is_grounded);
    }

    #[test]
    fn aabb_does_not_intersect_at_rest() {
        let tree = ground_plane();
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 5.0, 2.0);
        ctrl.velocity = Vec3::ZERO;
        ctrl.is_grounded = false;

        run_ticks(&mut ctrl, Some(&tree), 200);

        // Verify body AABB is clear of the tree.
        let body_min = Vec3::new(
            ctrl.translation.x - PLAYER_HALF_WIDTH,
            ctrl.translation.y,
            ctrl.translation.z - PLAYER_HALF_WIDTH,
        );
        let body_max = Vec3::new(
            ctrl.translation.x + PLAYER_HALF_WIDTH,
            ctrl.translation.y + PLAYER_HEIGHT,
            ctrl.translation.z + PLAYER_HALF_WIDTH,
        );
        assert_eq!(
            query::aabb_collides(&tree, body_min, body_max),
            CollisionResult::Clear
        );
    }

    #[test]
    fn camera_eye_offset_in_fps_mode() {
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 1.0, 2.0);

        let cam_pos = ctrl.camera_position();
        assert!((cam_pos.y - (ctrl.translation.y + EYE_OFFSET)).abs() < 1e-5);
    }

    #[test]
    fn camera_no_eye_offset_in_fly_mode() {
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fly;
        ctrl.translation = Vec3::new(2.0, 1.0, 2.0);

        let cam_pos = ctrl.camera_position();
        assert_eq!(cam_pos, ctrl.translation);
    }

    // ---- WASD movement tests ----

    /// Build a wall at x=4 (one column of voxels).
    fn wall_at_x4() -> GpuTree64 {
        let mut voxels = Vec::new();
        for y in 0..4u32 {
            for z in 0..8u32 {
                voxels.push(([4, y, z], 1u8));
            }
        }
        build_tree(&voxels, 8)
    }

    #[test]
    fn wasd_applies_velocity_when_grounded() {
        let tree = ground_plane();
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 1.0, 2.0);
        ctrl.velocity = Vec3::ZERO;
        ctrl.is_grounded = true;

        let mut keys = HashSet::new();
        keys.insert(FORWARD.clone());

        ctrl.physics_tick(Some(&tree), &keys);

        assert!(ctrl.is_grounded);
        // Forward should apply 6 m/s in the camera's forward direction.
        assert!(ctrl.velocity.x.abs() > 0.01 || ctrl.velocity.z.abs() > 0.01);
    }

    #[test]
    fn releasing_keys_stops_horizontal() {
        let tree = ground_plane();
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 1.0, 2.0);
        ctrl.velocity = Vec3::new(3.0, 0.0, 0.0);
        ctrl.is_grounded = true;

        ctrl.physics_tick(Some(&tree), &HashSet::new());

        // Horizontal velocity should be zero after tick with no keys.
        assert_eq!(ctrl.velocity.x, 0.0);
        assert_eq!(ctrl.velocity.z, 0.0);
    }

    #[test]
    fn walking_into_wall_stops_at_surface() {
        let tree = wall_at_x4();
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        // Stand on ground at y=1, near wall at x=4.
        ctrl.translation = Vec3::new(3.5, 1.0, 4.0);
        ctrl.velocity = Vec3::ZERO;
        ctrl.is_grounded = true;

        let mut keys = HashSet::new();
        keys.insert(RIGHT.clone()); // Move in +X direction (right)

        // Run many ticks to push against wall.
        for _ in 0..60 {
            ctrl.physics_tick(Some(&tree), &keys);
        }

        // Player should not penetrate the wall at x=4.
        // Body right edge = translation.x + PLAYER_HALF_WIDTH = x + 0.3.
        // Wall voxel at x=4 occupies [4, 5). Body should stop before 4.
        assert!(
            ctrl.translation.x + PLAYER_HALF_WIDTH <= 4.0 + 0.01,
            "player right edge {} should be <= 4.0",
            ctrl.translation.x + PLAYER_HALF_WIDTH
        );
    }

    #[test]
    fn walks_within_tree_bounds() {
        let tree = ground_plane(); // 4×4 floor covering [0,4)×[0,1)×[0,4)
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        ctrl.translation = Vec3::new(2.0, 1.0, 2.0);
        ctrl.velocity = Vec3::new(10.0, 0.0, 0.0);
        ctrl.is_grounded = true;

        // Keys include RIGHT to keep moving right.
        let mut keys = HashSet::new();
        keys.insert(RIGHT.clone());

        // Run many ticks — player should not leave the tree bounds.
        for _ in 0..120 {
            ctrl.physics_tick(Some(&tree), &keys);
        }

        // Check body stays within world bounds [0, 4).
        assert!(ctrl.translation.x - PLAYER_HALF_WIDTH >= -0.01);
        assert!(ctrl.translation.x + PLAYER_HALF_WIDTH <= 4.0 + 0.01);
        assert!(ctrl.translation.z - PLAYER_HALF_WIDTH >= -0.01);
        assert!(ctrl.translation.z + PLAYER_HALF_WIDTH <= 4.0 + 0.01);
    }
}

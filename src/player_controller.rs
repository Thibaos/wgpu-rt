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

/// Control mode for the player controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlMode {
    Fly,
    Fps,
}

/// Physics constants for FPS mode.
pub const TICK_DURATION: Duration = Duration::from_nanos(16_666_667); // 60 Hz
const GRAVITY: f32 = 20.0;
const PLAYER_HALF_WIDTH: f32 = 0.3; // AABB half-width in XZ (total width = 0.6 m)
const PLAYER_HEIGHT: f32 = 1.8;
const EYE_OFFSET: f32 = 1.65;

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
        let translation = Vec3::new(32.0, 32.0, 80.0);

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
                Vec3::new(32.0, 16.0, 32.0),
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
    /// Handles gravity, ground detection via body AABB collision first
    /// (for solid ground push-up), then ground slab check (for lattice
    /// / partial support detection).
    pub fn physics_tick(&mut self, tree: Option<&GpuTree64>) {
        if self.control_mode != ControlMode::Fps {
            return;
        }

        // Apply gravity.
        if !self.is_grounded {
            self.velocity.y -= GRAVITY * TICK_DURATION.as_secs_f32();
        }

        // Integrate velocity into candidate position.
        let new_pos = self.translation + self.velocity * TICK_DURATION.as_secs_f32();

        let body_min = Vec3::new(
            new_pos.x - PLAYER_HALF_WIDTH,
            new_pos.y,
            new_pos.z - PLAYER_HALF_WIDTH,
        );
        let body_max = Vec3::new(
            new_pos.x + PLAYER_HALF_WIDTH,
            new_pos.y + PLAYER_HEIGHT,
            new_pos.z + PLAYER_HALF_WIDTH,
        );

        // 1. Body collision: push up if intersecting solid ground.
        let body_collision = tree.and_then(|t| {
            match query::aabb_collides(t, body_min, body_max) {
                CollisionResult::Blocked {
                    penetration_y, ..
                } => Some(penetration_y),
                CollisionResult::Clear => None,
            }
        });

        if let Some(push_up) = body_collision {
            // Body intersects voxels — snap up to clear them and ground.
            self.translation = new_pos + Vec3::new(0.0, push_up, 0.0);
            self.is_grounded = true;
            self.velocity.y = 0.0;
        } else {
            // 2. Ground slab below feet: detect partial support (lattice, beam edge).
            let slab_min = Vec3::new(body_min.x, body_min.y - 1.0, body_min.z);
            let slab_max = Vec3::new(body_max.x, body_min.y, body_max.z);

            let slab_hit = tree.is_some_and(|t| {
                matches!(
                    query::aabb_collides(t, slab_min, slab_max),
                    CollisionResult::Blocked { .. }
                )
            });

            if slab_hit {
                // Body is clear but slab detects voxels in the 1-voxel zone
                // below the feet. Snap the player down so the body just rests
                // on top of the surface: test the body shifted 1 voxel lower,
                // then push it up by the penetration.
                let low_y = new_pos.y - 1.0;
                let low_min = Vec3::new(body_min.x, low_y, body_min.z);
                let low_max = Vec3::new(body_max.x, low_y + PLAYER_HEIGHT, body_max.z);

                let snap_y = if let Some(t) = tree {
                    match query::aabb_collides(t, low_min, low_max) {
                        CollisionResult::Blocked { penetration_y, .. } => {
                            // Body shifted down intersects: snap to surface top.
                            low_y + penetration_y
                        }
                        CollisionResult::Clear => {
                            // Lattice / partial support: stay at current position.
                            new_pos.y
                        }
                    }
                } else {
                    new_pos.y
                };

                self.translation = Vec3::new(new_pos.x, snap_y, new_pos.z);
                self.is_grounded = true;
                self.velocity.y = 0.0;
            } else {
                // No support anywhere — airborne.
                self.is_grounded = false;
                self.translation = new_pos;
            }
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
        for _ in 0..n {
            controller.physics_tick(tree);
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

        ctrl.physics_tick(None);
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

        ctrl.physics_tick(Some(&tree));
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
        let tree = ground_plane();
        let mut ctrl = PlayerController::default();
        ctrl.control_mode = ControlMode::Fps;
        // Start grounded on the floor.
        ctrl.translation = Vec3::new(2.0, 1.0, 2.0);
        ctrl.velocity = Vec3::ZERO;
        ctrl.is_grounded = true;

        // Move off the edge: the 4×4 floor covers x=0..4, z=0..4.
        // Move to x=5 which is off the edge.
        ctrl.translation.x = 5.0;

        ctrl.physics_tick(Some(&tree));
        assert!(!ctrl.is_grounded);
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
}

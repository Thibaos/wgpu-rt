use core::f32;
use std::{
    collections::HashSet,
    f32::consts::{FRAC_PI_2, TAU},
    time::Duration,
};

use glam::{Mat4, Quat, Vec3, vec3};
use winit::keyboard::{Key, NamedKey, SmolStr};

const FORWARD: Key = Key::Character(SmolStr::new_static("z"));
const LEFT: Key = Key::Character(SmolStr::new_static("q"));
const BACKWARD: Key = Key::Character(SmolStr::new_static("s"));
const RIGHT: Key = Key::Character(SmolStr::new_static("d"));

const UP: Key = Key::Named(NamedKey::Space);
const CONTROL: Key = Key::Named(NamedKey::Control);

pub struct PlayerController {
    pub speed: f32,
    pub sensitivity: f64,
    pub translation: Vec3,

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
            yaw: 0.0,
            pitch: 0.0,
            view: glam::camera::rh::view::look_at_mat4(
                translation,
                Vec3::new(32.0, 0.0, 0.0),
                Vec3::Y,
            ),
            needs_view_update: true,
        }
    }
}

impl PlayerController {
    const MAX_PITCH: f32 = FRAC_PI_2 - 0.01;
    const MIN_PITCH: f32 = -Self::MAX_PITCH;

    pub const fn camera_position(&self) -> Vec3 {
        self.translation
    }

    pub fn view(&mut self) -> Mat4 {
        if self.needs_view_update {
            self.compute_view();
        }
        self.view
    }

    pub fn fly_movement(&mut self, delta_time: Duration, keys: &HashSet<Key<SmolStr>>) {
        let view_inverse = self.view().inverse();
        let absolute_forward = view_inverse.transform_vector3(-Vec3::Z);
        let forward = vec3(absolute_forward.x, 0.0, absolute_forward.z).normalize();
        let right = view_inverse.transform_vector3(Vec3::X);

        let mut velocity = Vec3::ZERO;

        if keys.contains(&FORWARD) {
            velocity = Vec3::new(
                velocity.x + forward.x,
                velocity.y + forward.y,
                velocity.z + forward.z,
            );
        } else if keys.contains(&BACKWARD) {
            velocity = Vec3::new(
                velocity.x - forward.x,
                velocity.y - forward.y,
                velocity.z - forward.z,
            );
        }
        if keys.contains(&RIGHT) {
            velocity = Vec3::new(
                velocity.x + right.x,
                velocity.y + right.y,
                velocity.z + right.z,
            );
        } else if keys.contains(&LEFT) {
            velocity = Vec3::new(
                velocity.x - right.x,
                velocity.y - right.y,
                velocity.z - right.z,
            );
        }
        if keys.contains(&UP) {
            velocity = Vec3::new(velocity.x + 0.0, velocity.y + 1.0, velocity.z + 0.0);
        } else if keys.contains(&CONTROL) {
            velocity = Vec3::new(velocity.x + 0.0, velocity.y - 1.0, velocity.z + 0.0);
        }

        velocity = velocity.normalize_or_zero();

        self.translation = Vec3::new(
            (velocity.x * delta_time.as_secs_f32()).mul_add(self.speed, self.translation.x),
            (velocity.y * delta_time.as_secs_f32()).mul_add(self.speed, self.translation.y),
            (velocity.z * delta_time.as_secs_f32()).mul_add(self.speed, self.translation.z),
        );

        self.needs_view_update = true;
    }

    pub fn rotate(&mut self, delta: (f64, f64)) {
        self.yaw -= crate::utils::f64_to_f32(delta.0 * self.sensitivity);
        self.pitch -= crate::utils::f64_to_f32(delta.1 * self.sensitivity);

        self.yaw = self.yaw.rem_euclid(TAU);

        self.pitch = self.pitch.clamp(Self::MIN_PITCH, Self::MAX_PITCH);

        self.needs_view_update = true;
    }

    fn orientation(&self) -> Quat {
        let yaw_q = Quat::from_rotation_y(self.yaw);
        let pitch_q = Quat::from_rotation_x(self.pitch);

        yaw_q.mul_quat(pitch_q)
    }

    fn compute_view(&mut self) {
        let camera_pos = self.camera_position();
        let rot = self.orientation();
        let forward = rot.mul_vec3(Vec3::new(0.0, 0.0, -1.0));
        let up = rot.mul_vec3(Vec3::new(0.0, 1.0, 0.0));

        self.view = glam::camera::rh::view::look_at_mat4(
            camera_pos,
            Vec3::new(
                camera_pos.x + forward.x,
                camera_pos.y + forward.y,
                camera_pos.z + forward.z,
            ),
            up,
        );
    }

    pub fn handle_speed_change(&mut self, y_delta: f32) {
        if y_delta.is_sign_positive() {
            self.speed *= 1.5;
        } else {
            self.speed /= 1.5;
        }
    }
}

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
    elev_min: 5.0_f32.to_radians(),
    elev_max: 55.0_f32.to_radians(),
    elev_period: 30.0,
};

/// Returns `(position, look_target)` of the orbit camera at `elapsed`
/// seconds. The elevation is a smooth `cos` sweep that never leaves
/// `[elev_min, elev_max]`; the azimuth advances monotonically. The camera is
/// always exactly `radius` from `target` and looks at it; up is +Y.
pub fn orbit_pose(elapsed: f32, target: Vec3, radius: f32, params: &OrbitParams) -> (Vec3, Vec3) {
    let azimuth = std::f32::consts::TAU * elapsed / params.az_period;
    let elev = ((params.elev_max - params.elev_min) * 0.5).mul_add(
        1.0 - (std::f32::consts::TAU * elapsed / params.elev_period).cos(),
        params.elev_min,
    );
    let (sin_e, cos_e) = elev.sin_cos();
    let (sin_a, cos_a) = azimuth.sin_cos();
    let pos = Vec3::new(
        (cos_e * cos_a).mul_add(radius, target.x),
        sin_e.mul_add(radius, target.y),
        (cos_e * sin_a).mul_add(radius, target.z),
    );
    (pos, target)
}

/// Returns an orbit radius (metres) that frames every chunk: the distance from
/// the chunk-centroid `target` to the farthest corner of any chunk AABB
/// `[origin, origin + side]^3`, times `margin`, floored at `chunk_side_world`.
/// `chunk_origins` are the world-space origins of the non-empty chunks.
pub fn orbit_radius_from_chunks(chunk_origins: &[Vec3], chunk_side_world: f32, margin: f32) -> f32 {
    let half = chunk_side_world * 0.5;
    // f32 component math (not subject to `arithmetic_side_effects`).
    let mut tx = 0.0f32;
    let mut ty = 0.0f32;
    let mut tz = 0.0f32;
    for o in chunk_origins {
        tx += o.x;
        ty += o.y;
        tz += o.z;
    }
    let inv_count = crate::utils::usize_to_f32(chunk_origins.len().max(1)).recip();
    let target = Vec3::new(
        tx.mul_add(inv_count, half),
        ty.mul_add(inv_count, half),
        tz.mul_add(inv_count, half),
    );
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

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    clippy::suboptimal_flops,
    clippy::uninlined_format_args,
    clippy::unwrap_used,
    clippy::expect_used
)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn orbit_pose_is_deterministic() {
        let params = DEFAULT_ORBIT_PARAMS;
        let (pos_a, target_a) = orbit_pose(12.5, vec3(10.0, -5.0, 3.0), 50.0, &params);
        let (pos_b, target_b) = orbit_pose(12.5, vec3(10.0, -5.0, 3.0), 50.0, &params);
        assert_eq!(pos_a, pos_b);
        assert_eq!(target_a, target_b);
    }

    #[test]
    fn orbit_pose_is_antipodal_after_half_revolution() {
        let params = OrbitParams {
            az_period: 60.0,
            elev_min: 30.0 * PI / 180.0,
            elev_max: 30.0 * PI / 180.0,
            elev_period: 30.0,
        };
        let target = vec3(10.0, -5.0, 3.0);
        let radius = 50.0;
        let (pos_a, _) = orbit_pose(7.5, target, radius, &params);
        let (pos_b, _) = orbit_pose(7.5 + 30.0, target, radius, &params);
        assert!(((pos_b.x - target.x) + (pos_a.x - target.x)).abs() < 1e-3);
        assert!(((pos_b.z - target.z) + (pos_a.z - target.z)).abs() < 1e-3);
        assert!((pos_b.y - pos_a.y).abs() < 1e-4);
    }

    #[test]
    fn orbit_elevation_stays_within_bounds() {
        let params = DEFAULT_ORBIT_PARAMS;
        let target = vec3(10.0, -5.0, 3.0);
        let radius = 50.0;
        for step in 0..200 {
            let elapsed = params.elev_period * 2.0 * step as f32 / 200.0;
            let (pos, _) = orbit_pose(elapsed, target, radius, &params);
            let elev = ((pos.y - target.y) / radius).asin();
            assert!(
                elev >= params.elev_min - 1e-4 && elev <= params.elev_max + 1e-4,
                "elevation {} out of bounds at elapsed {}",
                elev,
                elapsed
            );
        }
    }

    #[test]
    fn orbit_pose_distance_is_radius() {
        let params = DEFAULT_ORBIT_PARAMS;
        let target = vec3(10.0, -5.0, 3.0);
        let radius = 50.0;
        for step in 0..24 {
            let elapsed = params.az_period * 2.0 * step as f32 / 24.0;
            let (pos, _) = orbit_pose(elapsed, target, radius, &params);
            assert!(
                ((pos - target).length() - radius).abs() < 1e-4,
                "distance {} != radius {} at elapsed {}",
                (pos - target).length(),
                radius,
                elapsed
            );
        }
    }

    #[test]
    fn orbit_radius_frames_all_chunks() {
        let chunk_side_world = 32.0;
        let margin = 1.3;
        let origins = vec![
            vec3(0.0, 0.0, 0.0),
            vec3(64.0, 0.0, 64.0),
            vec3(-96.0, 32.0, -48.0),
        ];
        let radius = orbit_radius_from_chunks(&origins, chunk_side_world, margin);
        let half = chunk_side_world * 0.5;
        let target = origins.iter().fold(Vec3::ZERO, |acc, o| acc + *o) / origins.len() as f32
            + Vec3::splat(half);
        let mut max_corner_dist = 0.0f32;
        for o in &origins {
            for x in [o.x, o.x + chunk_side_world] {
                for y in [o.y, o.y + chunk_side_world] {
                    for z in [o.z, o.z + chunk_side_world] {
                        max_corner_dist = max_corner_dist.max(Vec3::new(x, y, z).distance(target));
                    }
                }
            }
        }
        assert!(radius >= max_corner_dist);
        assert!(radius >= chunk_side_world);
        let empty = orbit_radius_from_chunks(&[], chunk_side_world, margin);
        assert!((empty - chunk_side_world).abs() < 1e-6);
    }
}

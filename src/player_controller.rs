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
        let translation = Vec3::new(32.0, 32.0, 80.0);

        Self {
            speed: 32.0,
            sensitivity: 0.001,
            translation,
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
        let rot = self.orientation();
        let forward = rot * Vec3::new(0.0, 0.0, -1.0);
        let up = rot * Vec3::new(0.0, 1.0, 0.0);

        self.view =
            glam::camera::rh::view::look_at_mat4(self.translation, self.translation + forward, up);
    }

    pub fn handle_speed_change(&mut self, y_delta: f32) {
        if y_delta.is_sign_positive() {
            self.speed *= 1.5;
        } else {
            self.speed /= 1.5;
        }
    }
}

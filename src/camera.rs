use bytemuck::{Pod, Zeroable};
use winit::{
    event::{ElementState, KeyEvent, WindowEvent},
    keyboard::{KeyCode, PhysicalKey},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    view_inv: glam::Mat4,
    proj_inv: glam::Mat4,
}

pub struct Camera {
    eye: glam::Vec3,
    target: glam::Vec3,
    up: glam::Vec3,
    aspect: f32,
    fovy: f32,
    z_near: f32,
    z_far: f32,

    speed: f32,
    is_forward_pressed: bool,
    is_backward_pressed: bool,
    is_left_pressed: bool,
    is_right_pressed: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: glam::Vec3::ZERO,
            target: glam::Vec3::Z,
            up: glam::Vec3::Y,
            aspect: 16.0 / 9.0,
            fovy: 70.0,
            z_near: 0.01,
            z_far: 1000.0,

            speed: 0.1,
            is_forward_pressed: false,
            is_backward_pressed: false,
            is_left_pressed: false,
            is_right_pressed: false,
        }
    }
}

impl Camera {
    fn compute(&self) -> (glam::Mat4, glam::Mat4) {
        let view = glam::Mat4::look_at_rh(self.eye, self.target, self.up);
        let proj = glam::Mat4::perspective_rh(
            self.fovy.to_radians(),
            self.aspect,
            self.z_near,
            self.z_far,
        );

        (view.inverse(), proj.inverse())
    }

    pub fn uniform(&self) -> Uniforms {
        let computed = self.compute();
        Uniforms {
            view_inv: computed.0,
            proj_inv: computed.1,
        }
    }

    pub fn process_events(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state,
                        physical_key: PhysicalKey::Code(keycode),
                        ..
                    },
                ..
            } => {
                let is_pressed = *state == ElementState::Pressed;
                match keycode {
                    KeyCode::KeyW | KeyCode::ArrowUp => {
                        self.is_forward_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyA | KeyCode::ArrowLeft => {
                        self.is_left_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyS | KeyCode::ArrowDown => {
                        self.is_backward_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyD | KeyCode::ArrowRight => {
                        self.is_right_pressed = is_pressed;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    pub fn update(&mut self) {
        let forward = self.target - self.eye;
        let forward_norm = forward.normalize();
        let forward_mag = forward.length();

        // Prevents glitching when the camera gets too close to the
        // center of the scene.
        if self.is_forward_pressed && forward_mag > self.speed {
            self.eye += forward_norm * self.speed;
        }
        if self.is_backward_pressed {
            self.eye -= forward_norm * self.speed;
        }

        let right = forward_norm.cross(self.up);

        // Redo radius calc in case the forward/backward is pressed.
        let forward = self.target - self.eye;
        let forward_mag = forward.length();

        if self.is_right_pressed {
            // Rescale the distance between the target and the eye so
            // that it doesn't change. The eye, therefore, still
            // lies on the circle made by the target and eye.
            self.eye = self.target - (forward + right * self.speed).normalize() * forward_mag;
        }
        if self.is_left_pressed {
            self.eye = self.target - (forward - right * self.speed).normalize() * forward_mag;
        }
    }
}

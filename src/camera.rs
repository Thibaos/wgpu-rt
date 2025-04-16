use winit::{
    event::{ElementState, KeyEvent, WindowEvent},
    keyboard::{KeyCode, PhysicalKey},
};

#[rustfmt::skip]
pub const TO_WGPU_MATRIX: glam::Mat4 = glam::Mat4::from_cols(
    glam::Vec4::new(1.0, 0.0, 0.0, 0.0),
    glam::Vec4::new(0.0, 1.0, 0.0, 0.0),
    glam::Vec4::new(0.0, 0.0, 0.5, 0.0),
    glam::Vec4::new(0.0, 0.0, 0.5, 1.0),
);

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    view_inverse: [[f32; 4]; 4],
    proj_inverse: [[f32; 4]; 4],
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
    fn compute(&self) -> (glam::Mat4, glam::Mat4, glam::Mat4) {
        let view = glam::Mat4::look_at_rh(self.eye, self.target, self.up);
        let proj = glam::Mat4::perspective_rh(
            self.fovy.to_radians(),
            self.aspect,
            self.z_near,
            self.z_far,
        );

        (TO_WGPU_MATRIX * proj * view, view.inverse(), proj.inverse())
    }

    pub fn uniform(&self) -> CameraUniform {
        let computed = self.compute();
        CameraUniform {
            view_proj: computed.0.to_cols_array_2d(),
            view_inverse: computed.1.to_cols_array_2d(),
            proj_inverse: computed.2.to_cols_array_2d(),
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
                        dbg!("forward");
                        self.is_forward_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyA | KeyCode::ArrowLeft => {
                        dbg!("left");

                        self.is_left_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyS | KeyCode::ArrowDown => {
                        dbg!("backward");
                        self.is_backward_pressed = is_pressed;
                        true
                    }
                    KeyCode::KeyD | KeyCode::ArrowRight => {
                        dbg!("right");
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

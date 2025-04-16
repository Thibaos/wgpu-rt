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
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: glam::Vec3::Y,
            target: glam::Vec3::Z,
            up: glam::Vec3::Y,
            aspect: 16.0 / 9.0,
            fovy: 70.0,
            z_near: 0.01,
            z_far: 1000.0,
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
}

pub mod rayquery;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};

use crate::world::chunk::CHUNK_SIZE;

pub const INDEX_COUNT: usize = 36;
pub const VOXEL_SCALE: f32 = 0.125;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    _pos: [f32; 4],
    _tex_coord: [f32; 2],
}

/// One chunk-world AABB primitive in the BLAS input buffer (32-byte stride,
/// matching `GpuAabb` in assets/shaders/rayquery.wgsl). The AABB is the
/// chunk's full 32 m bounds; TLAS instance `i` owns the primitive at
/// `i * size_of::<GpuAabb>()`, so the shader recovers the hit chunk's bounds
/// as `gpu_aabbs[instance_index]`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuAabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub _pad: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct InstanceRaw {
    model: [[f32; 4]; 4],
    chunk_origin: [f32; 4],
}

pub struct Instance {
    pub position: Vec3,
}

impl Instance {
    pub fn to_raw(&self) -> InstanceRaw {
        let half_chunk_world =
            CHUNK_SIZE.mul_add(glam::Vec3::splat(VOXEL_SCALE * 0.5), glam::Vec3::ZERO);
        let center = glam::Vec3::new(
            self.position.x + half_chunk_world.x,
            self.position.y + half_chunk_world.y,
            self.position.z + half_chunk_world.z,
        );
        InstanceRaw {
            model: (Mat4::from_scale_rotation_translation(
                half_chunk_world,
                Quat::IDENTITY,
                center,
            ))
            .to_cols_array_2d(),
            chunk_origin: [self.position.x, self.position.y, self.position.z, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CameraUniforms {
    pub camera_pos: [f32; 4],
    pub view_inv: [[f32; 4]; 4],
    pub proj_inv: [[f32; 4]; 4],
    pub view_proj: [[f32; 4]; 4],
    pub viewport_and_heatmap: [f32; 4],
}

const fn vertex(pos: [i8; 3], tc: [i8; 2]) -> Vertex {
    // i8 -> f32 via `as`: `From<i8>` is not const-stable; inputs are the
    // literals below (-1..=1), which are exactly representable.
    #[allow(clippy::as_conversions)]
    let v = Vertex {
        _pos: [pos[0] as f32, pos[1] as f32, pos[2] as f32, 1.0],
        _tex_coord: [tc[0] as f32, tc[1] as f32],
    };
    v
}

pub const fn create_vertices() -> ([Vertex; 24], [u16; INDEX_COUNT]) {
    let vertex_data = [
        // top (0, 0, 1)
        vertex([-1, -1, 1], [0, 0]),
        vertex([1, -1, 1], [1, 0]),
        vertex([1, 1, 1], [1, 1]),
        vertex([-1, 1, 1], [0, 1]),
        // bottom (0, 0, -1)
        vertex([-1, 1, -1], [1, 0]),
        vertex([1, 1, -1], [0, 0]),
        vertex([1, -1, -1], [0, 1]),
        vertex([-1, -1, -1], [1, 1]),
        // right (1, 0, 0)
        vertex([1, -1, -1], [0, 0]),
        vertex([1, 1, -1], [1, 0]),
        vertex([1, 1, 1], [1, 1]),
        vertex([1, -1, 1], [0, 1]),
        // left (-1, 0, 0)
        vertex([-1, -1, 1], [1, 0]),
        vertex([-1, 1, 1], [0, 0]),
        vertex([-1, 1, -1], [0, 1]),
        vertex([-1, -1, -1], [1, 1]),
        // front (0, 1, 0)
        vertex([1, 1, -1], [1, 0]),
        vertex([-1, 1, -1], [0, 0]),
        vertex([-1, 1, 1], [0, 1]),
        vertex([1, 1, 1], [1, 1]),
        // back (0, -1, 0)
        vertex([1, -1, 1], [0, 0]),
        vertex([-1, -1, 1], [1, 0]),
        vertex([-1, -1, -1], [1, 1]),
        vertex([1, -1, -1], [0, 1]),
    ];

    let index_data: [u16; INDEX_COUNT] = [
        0, 1, 2, 2, 3, 0, // top
        4, 5, 6, 6, 7, 4, // bottom
        8, 9, 10, 10, 11, 8, // right
        12, 13, 14, 14, 15, 12, // left
        16, 17, 18, 18, 19, 16, // front
        20, 21, 22, 22, 23, 20, // back
    ];

    (vertex_data, index_data)
}

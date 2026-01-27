use bytemuck::{Pod, Zeroable};

pub mod chunks;
pub mod loader;
pub mod voxels;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    _pos: [f32; 4],
}

pub fn vertex(pos: [f32; 3]) -> Vertex {
    Vertex {
        _pos: [pos[0], pos[1], pos[2], 1.0],
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VertexColor {
    _pos: [f32; 4],
    _color: [f32; 4],
}

pub fn vertex_color(pos: [f32; 3], color: [f32; 4]) -> VertexColor {
    VertexColor {
        _pos: [pos[0], pos[1], pos[2], 1.0],
        _color: color,
    }
}

#[derive(Debug, Default)]
pub struct HostVoxel {
    scale: f32,
    material_index: u32,
}

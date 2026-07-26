use glam::{IVec3, Vec3};

pub const CHUNK_TEXTURE_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 64,
    height: 64,
    depth_or_array_layers: 64,
};

pub const CHUNK_SIZE: Vec3 = Vec3 {
    x: CHUNK_TEXTURE_SIZE.width as f32,
    y: CHUNK_TEXTURE_SIZE.height as f32,
    z: CHUNK_TEXTURE_SIZE.depth_or_array_layers as f32,
};

pub const CHUNKS_X: u32 = 8;
pub const CHUNKS_Y: u32 = 1;
pub const CHUNKS_Z: u32 = 8;

pub const TOTAL_CHUNKS: u32 = CHUNKS_X * CHUNKS_Y * CHUNKS_Z;

struct Chunk {
    position: IVec3,
    data: Vec<u8>,
}

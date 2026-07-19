use glam::IVec3;

pub const CHUNK_TEXTURE_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 64,
    height: 64,
    depth_or_array_layers: 64,
};

pub const TOTAL_CHUNKS: u32 = 512;

struct Chunk {
    position: IVec3,
    data: Vec<u8>,
}

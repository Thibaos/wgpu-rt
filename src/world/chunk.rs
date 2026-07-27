use std::collections::HashMap;

use glam::{IVec3, Vec3};
use wgpu::Extent3d;

use super::MIP_LEVELS;

pub const CHUNK_TEXTURE_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: u8::MAX as u32,
    height: u8::MAX as u32,
    depth_or_array_layers: u8::MAX as u32,
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

type ChunkVoxelData = HashMap<(u8, u8, u8), u8>;

#[derive(Debug)]
pub struct Chunk {
    position: IVec3,
    voxels: ChunkVoxelData,
}

impl Chunk {
    pub fn new(position: IVec3) -> Self {
        Self {
            position,
            voxels: HashMap::new(),
        }
    }

    pub fn grid_position(&self) -> IVec3 {
        self.position
    }

    pub fn insert(&mut self, local: (u8, u8, u8), material: u8) {
        self.voxels.insert(local, material);
    }

    pub fn is_empty(&self) -> bool {
        self.voxels.is_empty()
    }

    #[cfg(test)]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.flatten_voxels(CHUNK_TEXTURE_SIZE.width)
    }

    pub fn to_mip_bytes(&self) -> [Vec<u8>; MIP_LEVELS] {
        let size0 = CHUNK_TEXTURE_SIZE.width;
        let mip0 = self.flatten_voxels(size0);
        let mip1 = Self::downsample_max(&mip0, size0);
        let mip2 = Self::downsample_max(&mip1, size0 / 2);
        let mip3 = Self::downsample_max(&mip2, size0 / 4);
        let mip4 = Self::downsample_max(&mip3, size0 / 8);
        [mip0, mip1, mip2, mip3, mip4]
    }

    fn flatten_voxels(&self, size: u32) -> Vec<u8> {
        let total = (size as usize) * (size as usize) * (size as usize);
        let mut bytes = vec![0u8; total];

        for (&(x, y, z), &v) in &self.voxels {
            let idx = (z as usize * size as usize + y as usize) * size as usize + x as usize;
            bytes[idx] = v;
        }
        bytes
    }

    fn downsample_max(src: &[u8], src_size: u32) -> Vec<u8> {
        let dst_size = src_size / 2;
        let total = (dst_size as usize) * (dst_size as usize) * (dst_size as usize);
        let mut dst = vec![0u8; total];

        for z in 0..dst_size {
            for y in 0..dst_size {
                for x in 0..dst_size {
                    let mut found = 0u8;
                    'blk: for dz in 0..2u32 {
                        for dy in 0..2u32 {
                            for dx in 0..2u32 {
                                let sx = x * 2 + dx;
                                let sy = y * 2 + dy;
                                let sz = z * 2 + dz;
                                let si = (sz * src_size + sy) * src_size + sx;
                                let v = src[si as usize];
                                if v != 0 {
                                    found = v;
                                    break 'blk;
                                }
                            }
                        }
                    }
                    let di = (z * dst_size + y) * dst_size + x;
                    dst[di as usize] = found;
                }
            }
        }
        dst
    }

    pub fn create_texture(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        let desc = wgpu::TextureDescriptor {
            label: Some("chunk_texture"),
            size: CHUNK_TEXTURE_SIZE,
            mip_level_count: MIP_LEVELS as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };

        let tex = device.create_texture(&desc);

        let mip_bytes = self.to_mip_bytes();

        for (mip, data) in mip_bytes.iter().enumerate() {
            let dims = Extent3d {
                width: CHUNK_TEXTURE_SIZE.width / 2u32.pow(mip as u32),
                height: CHUNK_TEXTURE_SIZE.height / 2u32.pow(mip as u32),
                depth_or_array_layers: CHUNK_TEXTURE_SIZE.depth_or_array_layers
                    / 2u32.pow(mip as u32),
            };

            queue.write_texture(
                wgpu::TexelCopyTextureInfoBase {
                    texture: &tex,
                    mip_level: mip as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dims.width),
                    rows_per_image: Some(dims.height),
                },
                dims,
            );
        }

        tex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_origin_maps_to_byte_zero() {
        let mut chunk = Chunk::new(IVec3::ZERO);
        chunk.insert((0, 0, 0), 42);
        let bytes = chunk.to_bytes();
        assert_eq!(bytes[0], 42);
        // All other bytes should be zero
        assert_eq!(bytes.iter().filter(|&&b| b != 0).count(), 1);
    }

    #[test]
    fn local_max_maps_to_final_byte() {
        let size = CHUNK_TEXTURE_SIZE.width as usize;
        let mut chunk = Chunk::new(IVec3::ZERO);
        let lx = 254u8;
        let ly = 254u8;
        let lz = 254u8;
        chunk.insert((lx, ly, lz), 99);
        let bytes = chunk.to_bytes();
        let expected_idx = (lz as usize * size + ly as usize) * size + lx as usize;
        assert_eq!(expected_idx, size * size * size - 1);
        assert_eq!(bytes[expected_idx], 99);
    }

    #[test]
    fn empty_chunk_produces_full_sized_byte_buffer() {
        let chunk = Chunk::new(IVec3::new(0, 0, 0));
        let bytes = chunk.to_bytes();
        let expected_size = 255 * 255 * 255;
        assert_eq!(bytes.len(), expected_size);
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn material_value_survives_roundtrip() {
        let mut chunk = Chunk::new(IVec3::ZERO);
        chunk.insert((10, 20, 30), 200);
        let bytes = chunk.to_bytes();
        let size = CHUNK_TEXTURE_SIZE.width as usize;
        let idx = (30usize * size + 20usize) * size + 10usize;
        assert_eq!(bytes[idx], 200);
    }

    #[test]
    fn is_empty_detects_empty_chunk() {
        let chunk = Chunk::new(IVec3::ZERO);
        assert!(chunk.is_empty());
    }

    #[test]
    fn is_empty_detects_nonempty_chunk() {
        let mut chunk = Chunk::new(IVec3::ZERO);
        chunk.insert((0, 0, 0), 1);
        assert!(!chunk.is_empty());
    }
}

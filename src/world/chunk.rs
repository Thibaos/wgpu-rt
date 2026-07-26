use std::collections::HashMap;

use glam::{IVec3, Vec3};
use wgpu::util::DeviceExt;

use crate::world::MIP_LEVELS;

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

    fn to_world_position(position: IVec3) -> IVec3 {
        IVec3::new(
            position.x * CHUNK_TEXTURE_SIZE.width as i32,
            position.y * CHUNK_TEXTURE_SIZE.height as i32,
            position.z * CHUNK_TEXTURE_SIZE.depth_or_array_layers as i32,
        )
    }

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

        let offset = Self::to_world_position(self.position);

        for (&(x, y, z), &v) in &self.voxels {
            let xi = x as i32 + offset.x;
            let yi = y as i32 + offset.y;
            let zi = z as i32 + offset.z;
            if xi >= 0 && yi >= 0 && zi >= 0 {
                let xu = xi as u32;
                let yu = yi as u32;
                let zu = zi as u32;
                if xu < size && yu < size && zu < size {
                    let idx =
                        (zu as usize * size as usize + yu as usize) * size as usize + xu as usize;
                    bytes[idx] = v;
                }
            }
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
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };

        let data = self.to_bytes();

        device.create_texture_with_data(queue, &desc, wgpu::wgt::TextureDataOrder::default(), &data)
    }
}

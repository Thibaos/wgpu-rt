use std::collections::HashMap;
use std::path::Path;

pub mod loader;

use wgpu::util::DeviceExt;

use crate::app::VOXEL_TEXTURE_SIZE;
use crate::world::loader::SceneGraphLoader;

pub type VoxelWorldData = HashMap<(i16, i16, i16), u8>;

pub struct World {
    pub voxels: VoxelWorldData,
    pub palette: [[u8; 4]; 256],
    pub world_offset: [i32; 3],
}

impl World {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let vox_data =
            dot_vox::load(path.as_ref().to_str().unwrap()).expect("failed to parse .vox file");

        let palette_src: &[dot_vox::Color] = if vox_data.palette.is_empty() {
            &dot_vox::DEFAULT_PALETTE
        } else {
            &vox_data.palette
        };
        let mut palette_array = [[0u8; 4]; 256];
        for (i, color) in palette_src.iter().enumerate().take(256) {
            palette_array[i] = [color.r, color.g, color.b, color.a];
        }

        eprintln!(
            "Scene: {} models, {} scene nodes",
            vox_data.models.len(),
            vox_data.scenes.len(),
        );

        let world = SceneGraphLoader::load(vox_data, palette_array);

        Ok(world)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        Self::flatten_voxels(&self.voxels, VOXEL_TEXTURE_SIZE.width, self.world_offset)
    }

    pub fn to_mip_bytes(&self) -> [Vec<u8>; 5] {
        let size0 = VOXEL_TEXTURE_SIZE.width;
        let mip0 = Self::flatten_voxels(&self.voxels, size0, self.world_offset);
        let mip1 = Self::downsample_max(&mip0, size0);
        let mip2 = Self::downsample_max(&mip1, size0 / 2);
        let mip3 = Self::downsample_max(&mip2, size0 / 4);
        let mip4 = Self::downsample_max(&mip3, size0 / 8);
        [mip0, mip1, mip2, mip3, mip4]
    }

    /// Flatten sparse voxel data into a dense 3D byte array.
    /// Applies `offset` so that world-space positions (which may be
    /// negative) map to valid texture indices in `[0, size)`.
    fn flatten_voxels(voxels: &VoxelWorldData, size: u32, offset: [i32; 3]) -> Vec<u8> {
        let total = (size as usize) * (size as usize) * (size as usize);
        let mut bytes = vec![0u8; total];

        for (&(x, z, y), &v) in voxels {
            let xi = x as i32 + offset[0];
            let yi = y as i32 + offset[1];
            let zi = z as i32 + offset[2];
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
}

pub fn create_palette_buffer(device: &wgpu::Device, palette: &[[u8; 4]; 256]) -> wgpu::Buffer {
    let float_palette: [[f32; 4]; 256] = std::array::from_fn(|i| {
        let [r, g, b, a] = palette[i];
        [
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]
    });

    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("palette"),
        contents: bytemuck::cast_slice(&float_palette),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

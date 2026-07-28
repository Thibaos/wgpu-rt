use std::collections::HashMap;
use std::path::Path;

pub mod chunk;
pub mod loader;

use wgpu::util::DeviceExt;

use crate::world::chunk::{CHUNKS_X, CHUNKS_Y, CHUNKS_Z, Chunk, TOTAL_CHUNKS};
use crate::world::loader::SceneGraphLoader;

pub const MIP_LEVELS: usize = 5;

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

    pub fn into_chunks(self) -> Vec<Chunk> {
        let mut chunks: Vec<Chunk> = (0..TOTAL_CHUNKS)
            .map(|i| {
                let chunk_x = i % CHUNKS_X;
                let chunk_z = i / (CHUNKS_X * CHUNKS_Y);
                Chunk::new(glam::IVec3::new(chunk_x as i32, 0, chunk_z as i32))
            })
            .collect();

        let chunk_side: i32 = 255;

        for ((x, y, z), material) in self.voxels {
            let wx = x as i32 + self.world_offset[0];
            let wy = y as i32 + self.world_offset[1];
            let wz = z as i32 + self.world_offset[2];

            let chunk_x = wx.div_euclid(chunk_side);
            let chunk_y = wy.div_euclid(chunk_side);
            let chunk_z = wz.div_euclid(chunk_side);

            let local_x = wx.rem_euclid(chunk_side) as u8;
            let local_y = wy.rem_euclid(chunk_side) as u8;
            let local_z = wz.rem_euclid(chunk_side) as u8;

            if chunk_x < 0
                || chunk_y < 0
                || chunk_z < 0
                || chunk_x >= CHUNKS_X as i32
                || chunk_y >= CHUNKS_Y as i32
                || chunk_z >= CHUNKS_Z as i32
            {
                continue;
            }

            let index = (chunk_z as u32 * CHUNKS_Y + chunk_y as u32) * CHUNKS_X + chunk_x as u32;
            chunks[index as usize].insert((local_x, local_y, local_z), material);
        }

        chunks
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

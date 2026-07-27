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

    pub fn into_chunks(self) -> Result<Vec<Chunk>, String> {
        let mut chunks: Vec<Chunk> = (0..TOTAL_CHUNKS)
            .map(|i| {
                let chunk_x = i % CHUNKS_X;
                #[allow(clippy::modulo_one)]
                let chunk_y = (i / CHUNKS_X) % CHUNKS_Y;
                let chunk_z = i / (CHUNKS_X * CHUNKS_Y);
                Chunk::new(glam::IVec3::new(
                    chunk_x as i32,
                    chunk_y as i32,
                    chunk_z as i32,
                ))
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
                return Err(format!(
                    "voxel at world ({}, {}, {}) maps to chunk ({}, {}, {}), \
                     which is outside the fixed grid [0..{}, 0..{}, 0..{}]",
                    wx, wy, wz, chunk_x, chunk_y, chunk_z, CHUNKS_X, CHUNKS_Y, CHUNKS_Z
                ));
            }

            let index = (chunk_z as u32 * CHUNKS_Y + chunk_y as u32) * CHUNKS_X + chunk_x as u32;
            chunks[index as usize].insert((local_x, local_y, local_z), material);
        }

        Ok(chunks)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::chunk::CHUNK_TEXTURE_SIZE;

    fn make_world(voxels: VoxelWorldData, offset: [i32; 3]) -> World {
        World {
            voxels,
            palette: [[0u8; 4]; 256],
            world_offset: offset,
        }
    }

    #[test]
    fn global_255_maps_to_chunk_1_local_0() {
        let mut voxels = HashMap::new();
        voxels.insert((255, 0, 0), 42);
        let world = make_world(voxels, [0, 0, 0]);
        let chunks = world.into_chunks().unwrap();

        // chunk at (1, 0, 0) should have voxel at local (0, 0, 0)
        let chunk_idx = (0 * CHUNKS_Y + 0) * CHUNKS_X + 1;
        let chunk = &chunks[chunk_idx as usize];
        assert_eq!(chunk.grid_position().x, 1);
        let bytes = chunk.to_bytes();
        let size = CHUNK_TEXTURE_SIZE.width as usize;
        let byte_idx = (0usize * size + 0usize) * size + 0usize;
        assert_eq!(bytes[byte_idx], 42);
    }

    #[test]
    fn negative_one_uses_euclidean_division_then_rejected() {
        // -1.div_euclid(255) = -1, -1.rem_euclid(255) = 254
        let mut voxels = HashMap::new();
        voxels.insert((-1, 0, 0), 42);
        let world = make_world(voxels, [0, 0, 0]);
        let result = world.into_chunks();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("chunk (-1, 0, 0)"));
    }

    #[test]
    fn material_survives_partitioning() {
        let mut voxels = HashMap::new();
        voxels.insert((100, 50, 0), 200);
        let world = make_world(voxels, [0, 0, 0]);
        let chunks = world.into_chunks().unwrap();

        // (100, 50, 0) is in chunk (0, 0, 0), local (100, 50, 0)
        let chunk = &chunks[0];
        let bytes = chunk.to_bytes();
        let size = CHUNK_TEXTURE_SIZE.width as usize;
        let idx = (0usize * size + 50usize) * size + 100usize;
        assert_eq!(bytes[idx], 200);
    }

    #[test]
    fn world_offset_is_applied_exactly_once() {
        // voxel at global (10, 20, 30), offset = (100, 200, 300)
        // => world coord = (110, 220, 330)
        // => chunk (0, 0, 1) since 330/255 = 1, local (110, 220, 75)
        let mut voxels = HashMap::new();
        voxels.insert((10, 20, 30), 77);
        let world = make_world(voxels, [100, 200, 300]);
        let chunks = world.into_chunks().unwrap();

        let chunk_z = 1;
        let chunk_idx = (chunk_z * CHUNKS_Y + 0) * CHUNKS_X + 0;
        let chunk = &chunks[chunk_idx as usize];
        assert!(!chunk.is_empty());

        let bytes = chunk.to_bytes();
        let size = CHUNK_TEXTURE_SIZE.width as usize;
        // local coords: (110, 220, 75) because 330.rem_euclid(255) = 75
        let idx = (75usize * size + 220usize) * size + 110usize;
        assert_eq!(bytes[idx], 77);
    }

    #[test]
    fn into_chunks_returns_total_chunks_slots() {
        let world = make_world(HashMap::new(), [0, 0, 0]);
        let chunks = world.into_chunks().unwrap();
        assert_eq!(chunks.len(), TOTAL_CHUNKS as usize);
        assert!(chunks.iter().all(|c| c.is_empty()));
    }

    #[test]
    fn offset_not_applied_when_zero() {
        let mut voxels = HashMap::new();
        voxels.insert((0, 0, 0), 1);
        voxels.insert((127, 0, 0), 2);
        let world = make_world(voxels, [0, 0, 0]);
        let chunks = world.into_chunks().unwrap();

        let chunk0 = &chunks[0];
        assert!(!chunk0.is_empty());
        let bytes = chunk0.to_bytes();
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[127], 2);
    }
}

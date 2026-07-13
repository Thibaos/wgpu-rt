use std::collections::HashMap;
use std::path::Path;

pub mod loader;
pub mod renderer;

use crate::app::VOXEL_TEXTURE_SIZE;
use crate::world::loader::SceneGraphLoader;

pub type VoxelWorldData = HashMap<(i16, i16, i16), u8>;

pub struct World {
    pub voxels: VoxelWorldData,
    pub palette: [[u8; 4]; 256],
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
        let mut bytes = Vec::new();

        let iter = (0..VOXEL_TEXTURE_SIZE.width as i16).flat_map(move |x| {
            (0..VOXEL_TEXTURE_SIZE.height as i16).flat_map(move |y| {
                (0..VOXEL_TEXTURE_SIZE.depth_or_array_layers as i16).map(move |z| (x, y, z))
            })
        });

        for (i, pos) in iter.enumerate() {
            if i as u32 % (VOXEL_TEXTURE_SIZE.width * VOXEL_TEXTURE_SIZE.height) == 0 {
                log::info!(
                    "Texture data collection progress: {}%",
                    i as f32
                        / (VOXEL_TEXTURE_SIZE.width
                            * VOXEL_TEXTURE_SIZE.height
                            * VOXEL_TEXTURE_SIZE.depth_or_array_layers)
                            as f32
                        * 100.0
                );
            }

            bytes.push(*self.voxels.get(&pos).unwrap_or_else(|| &0u8))
        }
        bytes
    }
}

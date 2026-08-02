use std::collections::HashMap;
use std::path::Path;

pub mod chunk;
pub mod loader;

use wgpu::util::DeviceExt;

use crate::world::chunk::{
    CHUNKS_X, CHUNKS_X_INT, CHUNKS_Y, CHUNKS_Y_INT, CHUNKS_Z_INT, Chunk, TOTAL_CHUNKS_INT,
};
use crate::world::loader::SceneGraphLoader;

pub const MIP_LEVELS: usize = 9;

pub type VoxelWorldData = HashMap<(i16, i16, i16), u8>;

pub struct World {
    pub voxels: VoxelWorldData,
    pub palette: [[u8; 4]; 256],
    pub offset: [i32; 3],
}

impl World {
    pub fn load(path: impl AsRef<Path>) -> Self {
        let path_str = path
            .as_ref()
            .to_str()
            .unwrap_or_else(|| crate::utils::fatal("world path is not valid UTF-8"));
        let vox_data = dot_vox::load(path_str)
            .unwrap_or_else(|e| crate::utils::fatal(&format!("failed to parse {path_str}: {e}")));

        let palette_src: &[dot_vox::Color] = if vox_data.palette.is_empty() {
            &dot_vox::DEFAULT_PALETTE
        } else {
            &vox_data.palette
        };
        let mut palette_array = [[0u8; 4]; 256];
        for (i, color) in palette_src.iter().enumerate().take(256) {
            if let Some(slot) = palette_array.get_mut(i) {
                *slot = [color.r, color.g, color.b, color.a];
            }
        }

        eprintln!(
            "Scene: {} models, {} scene nodes",
            vox_data.models.len(),
            vox_data.scenes.len(),
        );

        SceneGraphLoader::load(&vox_data, &palette_array)
    }

    pub fn into_chunks(self) -> Vec<Chunk> {
        let mut chunks: Vec<Chunk> = (0..TOTAL_CHUNKS_INT)
            .map(|i| {
                let chunk_x = i.rem_euclid(CHUNKS_X_INT);
                let chunk_z = i.div_euclid(CHUNKS_X_INT * CHUNKS_Y_INT);
                Chunk::new(glam::IVec3::new(chunk_x, 0, chunk_z))
            })
            .collect();

        let chunk_side: i32 = 256;

        for ((x, y, z), material) in self.voxels {
            let wx = i32::from(x).saturating_add(self.offset[0]);
            let wy = i32::from(y).saturating_add(self.offset[1]);
            let wz = i32::from(z).saturating_add(self.offset[2]);

            let chunk_x = wx.div_euclid(chunk_side);
            let chunk_y = wy.div_euclid(chunk_side);
            let chunk_z = wz.div_euclid(chunk_side);

            let local_x = u8::try_from(wx.rem_euclid(chunk_side)).unwrap_or_default();
            let local_y = u8::try_from(wy.rem_euclid(chunk_side)).unwrap_or_default();
            let local_z = u8::try_from(wz.rem_euclid(chunk_side)).unwrap_or_default();

            if chunk_x < 0
                || chunk_y < 0
                || chunk_z < 0
                || chunk_x >= CHUNKS_X_INT
                || chunk_y >= CHUNKS_Y_INT
                || chunk_z >= CHUNKS_Z_INT
            {
                continue;
            }

            // chunk coords are non-negative here (checked above), so the
            // u32 conversion is lossless.
            let index = u32::try_from(chunk_z)
                .unwrap_or_default()
                .saturating_mul(CHUNKS_Y)
                .saturating_add(u32::try_from(chunk_y).unwrap_or_default())
                .saturating_mul(CHUNKS_X)
                .saturating_add(u32::try_from(chunk_x).unwrap_or_default());
            let index = usize::try_from(index).unwrap_or_default();
            let chunk = chunks.get_mut(index).unwrap_or_else(|| {
                crate::utils::fatal(&format!("chunk index {index} out of range"))
            });
            chunk.insert((local_x, local_y, local_z), material);
        }

        chunks
    }
}

pub fn create_palette_buffer(device: &wgpu::Device, palette: &[[u8; 4]; 256]) -> wgpu::Buffer {
    let float_palette: [[f32; 4]; 256] = std::array::from_fn(|i| {
        let [r, g, b, a] = palette.get(i).unwrap_or(&[0; 4]);
        [
            f32::from(*r) / 255.0,
            f32::from(*g) / 255.0,
            f32::from(*b) / 255.0,
            f32::from(*a) / 255.0,
        ]
    });

    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("palette"),
        contents: bytemuck::cast_slice(&float_palette),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

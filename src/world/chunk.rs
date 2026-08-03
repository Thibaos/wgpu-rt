use std::collections::HashMap;

use glam::{IVec3, Vec3};
use wgpu::Extent3d;

use super::MIP_LEVELS;

pub const CHUNK_TEXTURE_SIZE: wgpu::Extent3d = wgpu::Extent3d {
    width: 256,
    height: 256,
    depth_or_array_layers: 256,
};

pub const CHUNK_SIZE: Vec3 = Vec3 {
    // u32 texture dims (256) are exactly representable in f32.
    #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
    x: CHUNK_TEXTURE_SIZE.width as f32,
    #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
    y: CHUNK_TEXTURE_SIZE.height as f32,
    #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
    z: CHUNK_TEXTURE_SIZE.depth_or_array_layers as f32,
};

// Vertical extent matches x/z so the 2048^3 world fits tall scenes (bistro_sm
// is 2047 voxels tall). 512 chunk textures fit the binding array on this
// adapter (limit 1,048,576); adapters with lower limits fail loudly at
// `App::init`, never silently. Plan 015 retires the fixed grid entirely.
pub const CHUNKS_X: u32 = 8;
pub const CHUNKS_Y: u32 = 8;
pub const CHUNKS_Z: u32 = 8;

pub const CHUNKS_X_INT: i32 = 8;
pub const CHUNKS_Y_INT: i32 = 8;
pub const CHUNKS_Z_INT: i32 = 8;

pub const TOTAL_CHUNKS: u32 = CHUNKS_X * CHUNKS_Y * CHUNKS_Z;
pub const TOTAL_CHUNKS_INT: i32 = CHUNKS_X_INT * CHUNKS_Y_INT * CHUNKS_Z_INT;

type ChunkVoxelData = HashMap<(u8, u8, u8), u8>;

#[derive(Debug)]
pub struct Chunk {
    position: IVec3,
    voxels: ChunkVoxelData,
}

impl Chunk {
    // HashMap::new is not const-stable in this toolchain; the lint's suggestion
    // to make this `const` does not compile, so it is allowed explicitly.
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(position: IVec3) -> Self {
        Self {
            position,
            voxels: HashMap::new(),
        }
    }

    pub const fn grid_position(&self) -> IVec3 {
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

    pub fn to_mip_bytes(&self) -> Vec<Vec<u8>> {
        // MIP_LEVELS levels: 256 -> 128 -> ... -> 1. Each coarser cell stores the
        // first non-zero material found in its 2x2x2 children (occupancy, not a
        // meaningful aggregate) so the GPU can ask "is there anything below?".
        let mut levels = Vec::with_capacity(MIP_LEVELS);
        let mut current = self.flatten_voxels(CHUNK_TEXTURE_SIZE.width);
        levels.push(current.clone());
        let mut current_size = CHUNK_TEXTURE_SIZE.width;
        for _ in 1..MIP_LEVELS {
            current = Self::downsample_occupancy(&current, current_size);
            levels.push(current.clone());
            current_size /= 2;
        }
        levels
    }

    fn flatten_voxels(&self, size: u32) -> Vec<u8> {
        let size_u = usize::try_from(size).unwrap_or_default();
        let mut bytes = vec![0u8; size_u.pow(3)];

        for (&(x, y, z), &v) in &self.voxels {
            let mut idx = usize::from(z);
            idx = idx.saturating_mul(size_u);
            idx = idx.saturating_add(usize::from(y));
            idx = idx.saturating_mul(size_u);
            idx = idx.saturating_add(usize::from(x));
            let slot = bytes.get_mut(idx).unwrap_or_else(|| {
                crate::utils::fatal(&format!("flatten_voxels: index {idx} out of range"))
            });
            *slot = v;
        }
        bytes
    }

    fn downsample_occupancy(src: &[u8], src_size: u32) -> Vec<u8> {
        let dst_size = src_size / 2;
        let total = usize::try_from(dst_size).unwrap_or_default().pow(3);
        let mut dst = vec![0u8; total];

        for z in 0..dst_size {
            for y in 0..dst_size {
                for x in 0..dst_size {
                    let mut found = 0u8;
                    'blk: for dz in 0..2u32 {
                        for dy in 0..2u32 {
                            for dx in 0..2u32 {
                                let mut si = z.saturating_mul(2).saturating_add(dz);
                                si = si.saturating_mul(src_size);
                                si = si.saturating_add(y.saturating_mul(2).saturating_add(dy));
                                si = si.saturating_mul(src_size);
                                si = si.saturating_add(x.saturating_mul(2).saturating_add(dx));
                                let v = src
                                    .get(usize::try_from(si).unwrap_or_default())
                                    .copied()
                                    .unwrap_or_default();
                                if v != 0 {
                                    found = v;
                                    break 'blk;
                                }
                            }
                        }
                    }
                    let mut di = z.saturating_mul(dst_size);
                    di = di.saturating_add(y);
                    di = di.saturating_mul(dst_size);
                    di = di.saturating_add(x);
                    let slot = dst
                        .get_mut(usize::try_from(di).unwrap_or_default())
                        .unwrap_or_else(|| {
                            crate::utils::fatal(&format!("downsample: index {di} out of range"))
                        });
                    *slot = found;
                }
            }
        }
        dst
    }

    pub fn create_texture(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        let desc = wgpu::TextureDescriptor {
            label: Some("chunk_texture"),
            size: CHUNK_TEXTURE_SIZE,
            mip_level_count: u32::try_from(MIP_LEVELS).unwrap_or_default(),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };

        let tex = device.create_texture(&desc);

        let mip_bytes = self.to_mip_bytes();

        for (mip, data) in mip_bytes.iter().enumerate() {
            let level = u32::try_from(mip).unwrap_or_default();
            let divisor = 2u32.pow(level);
            let dims = Extent3d {
                width: CHUNK_TEXTURE_SIZE.width.div_euclid(divisor),
                height: CHUNK_TEXTURE_SIZE.height.div_euclid(divisor),
                depth_or_array_layers: CHUNK_TEXTURE_SIZE.depth_or_array_layers.div_euclid(divisor),
            };

            queue.write_texture(
                wgpu::TexelCopyTextureInfoBase {
                    texture: &tex,
                    mip_level: level,
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
#[allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used
)]
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
        let lx = 255u8;
        let ly = 255u8;
        let lz = 255u8;
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
        let expected_size = 256 * 256 * 256;
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

    #[test]
    fn to_mip_bytes_produces_all_levels_with_halving_sizes() {
        let mut chunk = Chunk::new(IVec3::ZERO);
        chunk.insert((0, 0, 0), 7);
        let mips = chunk.to_mip_bytes();
        assert_eq!(mips.len(), MIP_LEVELS);
        let mut expected = CHUNK_TEXTURE_SIZE.width;
        for level in &mips {
            let dim = expected as usize;
            assert_eq!(level.len(), dim * dim * dim);
            expected /= 2;
        }
        // Last level is 1^3.
        assert_eq!(expected, 0);
        let last = mips.last().unwrap();
        assert_eq!(last.len(), 1);
        // The single voxel lives in the bottom corner, so the 1x1x1 root is occupied.
        assert_eq!(last[0], 7);
    }

    #[test]
    fn downsample_occupancy_propagates_first_found_material() {
        // A single occupied voxel anywhere in a 2x2x2 block makes its parent non-zero,
        // carrying the first-found material.
        let src_size = 2u32;
        let src = vec![0u8, 5, 0, 0, 0, 0, 0, 9]; // (1,0,0)=5, (1,1,1)=9
        let dst = Chunk::downsample_occupancy(&src, src_size);
        assert_eq!(dst.len(), 1);
        assert_ne!(dst[0], 0);
        // First-found in iteration order dz,dy,dx: (1,0,0)=5 wins over (1,1,1)=9.
        assert_eq!(dst[0], 5);
    }

    #[test]
    fn downsample_occupancy_yields_zero_for_empty_block() {
        let src = vec![0u8; 8];
        let dst = Chunk::downsample_occupancy(&src, 2u32);
        assert_eq!(dst, vec![0u8]);
    }
}

use tree64::VoxelModel;

use crate::tree64_renderer::GpuTree64;

/// Precomputed 2D Perlin noise heightmap backing a sparse terrain model.
///
/// Dimensions are `[WORLD_SIZE, MAX_HEIGHT, WORLD_SIZE]` so `Tree64::new()`
/// only probes positions that *can* hold terrain (WORLD_SIZE × MAX_HEIGHT × WORLD_SIZE
/// ≈ 67M for the default 1024×64×1024). The tree itself spans the full 1024³ space;
/// the shader traverses empty air above `MAX_HEIGHT` naturally.
pub struct TerrainModel {
    /// Height for each (x, z): `heightmap[z * size + x]`
    heightmap: Vec<u8>,
    size: u32,
    max_height: u8,
}

impl TerrainModel {
    /// World size in voxels per horizontal axis.
    pub const WORLD_SIZE: u32 = 1024;
    /// Maximum terrain elevation in voxels.
    pub const MAX_HEIGHT: u8 = 64;
    /// Noise feature scale (larger = smoother terrain).
    const NOISE_SCALE: f64 = 256.0;

    /// Generate a terrain heightmap from Perlin noise.
    ///
    /// Uses 3-octave FBM: primary + 0.5×2× + 0.25×4×.
    /// `seed` controls the noise permutation.
    pub fn new(seed: u32) -> Self {
        use noise::{NoiseFn, Perlin};

        let perlin = Perlin::new(seed);
        let size = Self::WORLD_SIZE as usize;
        let mut heightmap = vec![0u8; size * size];

        for z in 0..size {
            for x in 0..size {
                let nx = x as f64 / Self::NOISE_SCALE;
                let nz = z as f64 / Self::NOISE_SCALE;

                // 3-octave FBM
                let h = perlin.get([nx, nz])
                    + 0.5 * perlin.get([nx * 2.0 + 100.0, nz * 2.0 + 100.0])
                    + 0.25 * perlin.get([nx * 4.0 + 200.0, nz * 4.0 + 200.0]);

                // Normalize from ~[-1.75, 1.75] to [0, 1]
                let normalized = (h + 1.75) / 3.5;
                let clamped = normalized.clamp(0.0, 1.0);
                let height = (clamped * Self::MAX_HEIGHT as f64) as u8;
                heightmap[z * size + x] = height;
            }
        }

        Self {
            heightmap,
            size: Self::WORLD_SIZE,
            max_height: Self::MAX_HEIGHT,
        }
    }
}

impl VoxelModel<u8> for TerrainModel {
    fn dimensions(&self) -> [u32; 3] {
        [self.size, self.max_height as u32, self.size]
    }

    fn access(&self, coord: [usize; 3]) -> Option<u8> {
        let (x, y, z) = (coord[0], coord[1], coord[2]);

        // Out of horizontal bounds → always empty
        if x >= self.size as usize || z >= self.size as usize {
            return None;
        }
        // Above max terrain height → always empty
        if y >= self.max_height as usize {
            return None;
        }

        let terrain_height = self.heightmap[z * self.size as usize + x] as usize;
        if y < terrain_height {
            // Material varies by height: lower = darker (stone), upper = lighter (grass/snow)
            Some(1)
        } else {
            None
        }
    }
}

/// Build a Tree64 from procedural Perlin-noise terrain.
pub fn build_tree64() -> GpuTree64 {
    log::info!(
        "Generating Perlin terrain: {}×{}×{} world...",
        TerrainModel::WORLD_SIZE,
        TerrainModel::MAX_HEIGHT,
        TerrainModel::WORLD_SIZE,
    );

    let model = TerrainModel::new(42);
    GpuTree64::from_model(&model)
}

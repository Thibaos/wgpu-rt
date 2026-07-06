pub mod chunk_manager;

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::formats::WorldFile;
use crate::tree64_renderer::GpuTree64;

/// Loaded world state: all chunks (some may be empty) and their metadata.
pub struct World {
    /// GpuTree64 for each chunk. Index: chunks[x + z * CHUNK_COUNT_X].
    /// None means the chunk was empty (not present in the .world file).
    pub chunks: Vec<Option<GpuTree64>>,
    pub chunk_count_x: u32,
    pub chunk_count_z: u32,
    pub chunk_voxel_x: u32,
    pub chunk_voxel_z: u32,
}

impl World {
    /// Load a .world file from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let file = File::open(path.as_ref())
            .map_err(|e| format!("failed to open world file {}: {e}", path.as_ref().display()))?;
        let mut reader = BufReader::new(file);
        let world_file =
            WorldFile::read(&mut reader).map_err(|e| format!("failed to read world file: {e}"))?;

        let total = world_file.header.total_chunks() as usize;
        let mut chunks: Vec<Option<GpuTree64>> = Vec::with_capacity(total);

        for chunk_opt in world_file.chunks {
            chunks.push(chunk_opt.map(|cd| cd.tree));
        }

        let loaded_count = chunks.iter().filter(|c| c.is_some()).count();

        log::info!(
            "Loaded world: {} chunks ({} non-empty), grid {}×{}",
            total,
            loaded_count,
            world_file.header.chunk_count_x,
            world_file.header.chunk_count_z,
        );

        Ok(Self {
            chunks,
            chunk_count_x: world_file.header.chunk_count_x,
            chunk_count_z: world_file.header.chunk_count_z,
            chunk_voxel_x: world_file.header.chunk_voxel_x,
            chunk_voxel_z: world_file.header.chunk_voxel_z,
        })
    }

    /// Get the GpuTree64 for a chunk, if present.
    pub fn get_chunk(&self, x: u32, z: u32) -> Option<&GpuTree64> {
        if x >= self.chunk_count_x || z >= self.chunk_count_z {
            return None;
        }
        let index = (x + z * self.chunk_count_x) as usize;
        self.chunks[index].as_ref()
    }
}

#![allow(dead_code)]

use std::collections::HashMap;

use crate::tree64_renderer::{GpuTree64, GpuTree64Buffers};

/// Identifies a chunk in the world grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkCoord {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// A loaded chunk with its GPU buffers.
pub struct LoadedChunk {
    pub coord: ChunkCoord,
    pub buffers: GpuTree64Buffers,
}

/// Manages the set of currently loaded chunks and their GPU resources.
pub struct ChunkManager {
    pub chunks: HashMap<ChunkCoord, LoadedChunk>,
}

impl Default for ChunkManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkManager {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }

    /// Load a chunk onto the GPU. Replaces any existing chunk at this coordinate.
    pub fn load_chunk(&mut self, coord: ChunkCoord, tree: GpuTree64, device: &wgpu::Device) {
        let buffers = tree.create_buffers(device);
        self.chunks.insert(coord, LoadedChunk { coord, buffers });
    }

    /// Remove a chunk and drop its GPU buffers.
    pub fn unload_chunk(&mut self, coord: &ChunkCoord) {
        self.chunks.remove(coord);
    }

    /// Returns an iterator over all loaded chunks.
    pub fn loaded_chunks(&self) -> impl Iterator<Item = &LoadedChunk> {
        self.chunks.values()
    }

    /// Clear all loaded chunks.
    pub fn clear(&mut self) {
        self.chunks.clear();
    }
}

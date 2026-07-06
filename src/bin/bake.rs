//! Bake tool: converts a MagicaVoxel .vox file into the .world binary format.
//!
//! Usage: cargo run --bin bake -- <input.vox> <output.world>
//!
//! The tool reads a .vox file, partitions the voxel grid into 16x16 chunks,
//! builds a Tree64 per chunk, and writes a .world file.

use std::env;
use std::fs::File;
use std::io::BufWriter;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input.vox> <output.world>", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    // Read .vox file
    let vox_data = dot_vox::load(input_path).expect("failed to parse .vox file");

    if vox_data.models.is_empty() {
        eprintln!("Error: .vox file contains no models");
        std::process::exit(1);
    }

    // Use the first model (scene with multiple models not yet supported).
    let model = &vox_data.models[0];
    let model_size = model.size;
    eprintln!(
        "Model: {}x{}x{} voxels, {} voxels total",
        model_size.x, model_size.y, model_size.z,
        model.voxels.len()
    );

    // Build world structure
    let mut world_file = wgpu_rt::formats::WorldFile::new();

    eprintln!(
        "Chunk grid: {}x{}x{}, chunk size: {}x{}x{} voxels",
        wgpu_rt::formats::CHUNK_COUNT_X,
        wgpu_rt::formats::CHUNK_COUNT_Y,
        wgpu_rt::formats::CHUNK_COUNT_Z,
        wgpu_rt::formats::CHUNK_VOXEL_X,
        wgpu_rt::formats::CHUNK_VOXEL_Y,
        wgpu_rt::formats::CHUNK_VOXEL_Z,
    );

    let mut chunks_written: u32 = 0;

    for cz in 0..wgpu_rt::formats::CHUNK_COUNT_Z {
        for cy in 0..wgpu_rt::formats::CHUNK_COUNT_Y {
            for cx in 0..wgpu_rt::formats::CHUNK_COUNT_X {
                let chunk_x_min = cx * wgpu_rt::formats::CHUNK_VOXEL_X;
                let chunk_y_min = cy * wgpu_rt::formats::CHUNK_VOXEL_Y;
                let chunk_z_min = cz * wgpu_rt::formats::CHUNK_VOXEL_Z;

                // Build a VoxelModel for this chunk's region
                let chunk_model = ChunkVoxelModel {
                    source: model,
                    offset_x: chunk_x_min,
                    offset_y: chunk_y_min,
                    offset_z: chunk_z_min,
                    chunk_size_x: wgpu_rt::formats::CHUNK_VOXEL_X,
                    chunk_size_y: wgpu_rt::formats::CHUNK_VOXEL_Y,
                    chunk_size_z: wgpu_rt::formats::CHUNK_VOXEL_Z,
                };

                // Build the Tree64
                let tree = tree64::Tree64::new(&chunk_model);

                // Skip completely empty chunks
                if tree.nodes.is_empty()
                    || (tree.root_state().index == 0 && tree.nodes[0].pop_mask == 0)
                {
                    continue;
                }

                // Convert to GpuTree64 and store
                let gpu_tree = wgpu_rt::tree64_renderer::GpuTree64::from_tree64(&tree);
                let index = wgpu_rt::formats::ChunkTable::chunk_index(
                    cx, cy, cz,
                    wgpu_rt::formats::CHUNK_COUNT_X,
                );

                world_file.set_chunk(index, wgpu_rt::formats::chunk::ChunkData::new(gpu_tree));
                chunks_written += 1;

                eprintln!(
                    "  Chunk ({}, {}, {}): {} nodes, {} bytes leaf data",
                    cx, cy, cz,
                    tree.nodes.len(),
                    tree.data.len(),
                );
            }
        }
    }

    eprintln!("Total non-empty chunks: {}", chunks_written);

    // Write world file
    let out_file = File::create(output_path).expect("failed to create output file");
    let writer = BufWriter::new(out_file);
    world_file.write(writer).expect("failed to write world file");

    eprintln!("Done: {} written", output_path);
}

/// A VoxelModel that wraps a dot_vox model and exposes a sub-region
/// as if it's a standalone model at origin (0,0,0).
struct ChunkVoxelModel<'a> {
    source: &'a dot_vox::Model,
    offset_x: u32,
    offset_y: u32,
    offset_z: u32,
    chunk_size_x: u32,
    chunk_size_y: u32,
    chunk_size_z: u32,
}

impl<'a> tree64::VoxelModel<u8> for &'a ChunkVoxelModel<'a> {
    fn dimensions(&self) -> [u32; 3] {
        [self.chunk_size_x, self.chunk_size_y, self.chunk_size_z]
    }

    fn access(&self, coord: [usize; 3]) -> Option<u8> {
        let (x, y, z) = (coord[0] as u32, coord[1] as u32, coord[2] as u32);

        if x >= self.chunk_size_x || y >= self.chunk_size_y || z >= self.chunk_size_z {
            return None;
        }

        let global_x = self.offset_x + x;
        let global_y = self.offset_y + y;
        let global_z = self.offset_z + z;

        if global_x >= self.source.size.x
            || global_y >= self.source.size.y
            || global_z >= self.source.size.z
        {
            return None;
        }

        // dot_vox stores voxels as a sparse list, each with its own (x, y, z, i).
        // Linear-scan to find the voxel at (global_x, global_y, global_z).
        self.source
            .voxels
            .iter()
            .find(|v| v.x == global_x as u8 && v.y == global_y as u8 && v.z == global_z as u8)
            .map(|v| v.i)
    }
}

//! Bake tool: converts a MagicaVoxel .vox file into the .world binary format.
//!
//! Usage: cargo run --bin bake -- <input.vox> <output.world>
//!
//! The tool reads a .vox file, partitions the voxel grid into 16x16 chunks,
//! builds a Tree64 per chunk, and writes a .world file.

use std::collections::HashMap;
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
        model_size.x,
        model_size.y,
        model_size.z,
        model.voxels.len()
    );

    // Build world structure
    let mut world_file = wgpu_rt::formats::WorldFile::new();

    // Extract palette from .vox file, falling back to default.
    {
        let palette_src: &[dot_vox::Color] = if vox_data.palette.is_empty() {
            &dot_vox::DEFAULT_PALETTE
        } else {
            &vox_data.palette
        };
        let mut palette_array = [[0u8; 4]; 256];
        for (i, color) in palette_src.iter().enumerate().take(256) {
            palette_array[i] = [color.r, color.g, color.b, color.a];
        }
        world_file.palette = palette_array;
    }

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

                // Build a VoxelModel for this chunk's region.
                // Clips to source model bounds internally.
                let chunk_model = ChunkVoxelModel::new(
                    model,
                    chunk_x_min,
                    chunk_y_min,
                    chunk_z_min,
                    wgpu_rt::formats::CHUNK_VOXEL_X,
                    wgpu_rt::formats::CHUNK_VOXEL_Y,
                    wgpu_rt::formats::CHUNK_VOXEL_Z,
                );

                // Skip chunks that have no overlap with the source model
                if chunk_model.chunk_size_x == 0
                    || chunk_model.chunk_size_y == 0
                    || chunk_model.chunk_size_z == 0
                {
                    continue;
                }

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
                    cx,
                    cy,
                    cz,
                    wgpu_rt::formats::CHUNK_COUNT_X,
                );

                world_file.set_chunk(index, wgpu_rt::formats::chunk::ChunkData::new(gpu_tree));
                chunks_written += 1;

                eprintln!(
                    "  Chunk ({}, {}, {}): {} nodes, {} bytes leaf data",
                    cx,
                    cy,
                    cz,
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
    world_file
        .write(writer)
        .expect("failed to write world file");

    eprintln!("Done: {} written", output_path);
}

/// A VoxelModel that wraps a dot_vox model and exposes a sub-region
/// as if it's a standalone model at origin (0,0,0).
///
/// Uses a HashMap for O(1) voxel lookup, built once from the source voxel list.
struct ChunkVoxelModel<'a> {
    source_size: &'a dot_vox::Size,
    voxel_map: HashMap<(u8, u8, u8), u8>,
    offset_x: u32,
    offset_y: u32,
    offset_z: u32,
    chunk_size_x: u32,
    chunk_size_y: u32,
    chunk_size_z: u32,
}

impl<'a> ChunkVoxelModel<'a> {
    fn new(
        source: &'a dot_vox::Model,
        offset_x: u32,
        offset_y: u32,
        offset_z: u32,
        chunk_size_x: u32,
        chunk_size_y: u32,
        chunk_size_z: u32,
    ) -> Self {
        let mut voxel_map: HashMap<(u8, u8, u8), u8> = HashMap::new();
        for v in &source.voxels {
            voxel_map.insert((v.x, v.y, v.z), v.i);
        }

        // Clip chunk dimensions to source model bounds so Tree64::new doesn't
        // traverse a massive empty volume.
        let src_end_x = offset_x + chunk_size_x;
        let src_end_y = offset_y + chunk_size_y;
        let src_end_z = offset_z + chunk_size_z;
        let clipped_w = (source.size.x.min(src_end_x)).saturating_sub(offset_x);
        let clipped_h = (source.size.y.min(src_end_y)).saturating_sub(offset_y);
        let clipped_d = (source.size.z.min(src_end_z)).saturating_sub(offset_z);

        Self {
            source_size: &source.size,
            voxel_map,
            offset_x,
            offset_y,
            offset_z,
            chunk_size_x: clipped_w,
            chunk_size_y: clipped_h,
            chunk_size_z: clipped_d,
        }
    }
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

        if global_x >= self.source_size.x
            || global_y >= self.source_size.y
            || global_z >= self.source_size.z
        {
            return None;
        }

        self.voxel_map
            .get(&(global_x as u8, global_y as u8, global_z as u8))
            .copied()
    }
}

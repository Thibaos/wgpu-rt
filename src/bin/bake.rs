use std::env;
use std::fs::File;
use std::io::BufWriter;

fn main() {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input.vox> <output.world>", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    let vox_data = dot_vox::load(input_path).expect("failed to parse .vox file");

    // Extract the 256-color RGBA8 palette.
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

    eprintln!(
        "Chunk grid: {}×{}×{}, chunk size: {}×{}×{} voxels",
        wgpu_rt::formats::CHUNK_COUNT_X,
        wgpu_rt::formats::CHUNK_COUNT_Y,
        wgpu_rt::formats::CHUNK_COUNT_Z,
        wgpu_rt::formats::CHUNK_VOXEL_X,
        wgpu_rt::formats::CHUNK_VOXEL_Y,
        wgpu_rt::formats::CHUNK_VOXEL_Z,
    );

    let world_file = wgpu_rt::world::loader::SceneGraphLoader::load(&vox_data, palette_array);

    eprintln!("World loading done.");

    let out_file = File::create(output_path).expect("failed to create output file");
    let writer = BufWriter::new(out_file);
    world_file
        .write(writer)
        .expect("failed to write world file");

    eprintln!("Done: {} written", output_path);
}

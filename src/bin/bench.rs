//! Headless GPU benchmark for the chunk render pass.
//!
//! Reuses the real App (world loading, chunk textures, pipeline, render loop)
//! but renders to an offscreen texture with no window. The orbit camera makes
//! the camera path deterministic across runs.
//!
//! Usage:
//!     cargo run --release --bin bench -- [frames]
//!
//! Modes (env vars, read by App::init):
//!   WGPU_RT_PROFILE=1   per-frame timestamp queries + blocking Wait readback
//!   WGPU_RT_STATS=1     compile the shader with atomic DDA-work counters.
//!                       The storage writes are a fragment-shader side effect,
//!                       which disables hardware early-Z — so this leg measures
//!                       the TOTAL traversal work (fragments, DDA cells, hits),
//!                       i.e. the cost WITHOUT early-Z culling. Run with
//!                       WGPU_RT_STATS=0 for the real (early-Z) GPU time.

#[path = "../app.rs"]
#[allow(dead_code)]
mod app;
#[path = "../player_controller.rs"]
#[allow(dead_code)]
mod player_controller;
#[path = "../render/mod.rs"]
#[allow(dead_code)]
mod render;
#[path = "../utils.rs"]
#[allow(dead_code)]
mod utils;
#[path = "../world/mod.rs"]
#[allow(dead_code)]
mod world;

use std::collections::HashSet;

use winit::keyboard::{Key, SmolStr};

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    // Read before overwriting so the shell can override the defaults.
    let profile = std::env::var("WGPU_RT_PROFILE").unwrap_or_else(|_| "1".to_string());
    let stats = std::env::var("WGPU_RT_STATS").unwrap_or_else(|_| "1".to_string());
    let frames: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(900);
    // Single-threaded startup: safe to mutate process env before App::init.
    unsafe {
        std::env::set_var("WGPU_RT_ORBIT", "1");
        std::env::set_var("WGPU_RT_PROFILE", &profile);
        std::env::set_var("WGPU_RT_STATS", &stats);
    }
    log::info!(
        "[bench] frames={frames} WGPU_RT_PROFILE={profile} WGPU_RT_STATS={stats} (stats=1 disables early-Z, measures full traversal work)"
    );

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no suitable adapter");
    let info = adapter.get_info();
    log::info!("[bench] adapter: {} ({:?})", info.name, info.backend);

    let mut needed_limits = app::App::required_limits().using_resolution(adapter.limits());
    needed_limits.max_binding_array_elements_per_shader_stage = world::chunk::TOTAL_CHUNKS
        .min(adapter.limits().max_binding_array_elements_per_shader_stage);

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: (app::App::optional_features() & adapter.features())
            | app::App::required_features(),
        required_limits: needed_limits,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
    }))
    .expect("device request failed");

    let width = 1920u32;
    let height = 1080u32;
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 1,
        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
        color_space: wgpu::SurfaceColorSpace::Auto,
        view_formats: vec![format],
    };

    let mut app = app::App::init(&config, &adapter, &device, &queue);
    app.resize(&config, &device, &queue);

    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_color"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());

    let keys: HashSet<Key<SmolStr>> = HashSet::new();

    let start = std::time::Instant::now();
    for _ in 0..frames {
        app.render(&color_view, &device, &queue, &keys);
    }
    let wall = start.elapsed().as_secs_f32();
    log::info!(
        "[bench] {} frames in {:.2}s wall ({:.1} fps wall, includes Wait-poll sync)",
        frames,
        wall,
        frames as f32 / wall
    );
}

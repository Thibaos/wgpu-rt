use std::borrow::Cow;
use std::collections::HashSet;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use wgpu::util::DeviceExt;

use crate::player_controller::PlayerController;
use crate::render::{
    CameraUniforms, INDEX_COUNT, Instance, InstanceRaw, VOXEL_SCALE, Vertex, create_vertices,
    rayquery::RayQueryResources,
};
use crate::utils::{fatal, i32_to_f32, u32_to_f32, u64_to_f32, u64_to_f64};
use crate::world::{World, chunk::CHUNK_SIZE, create_palette_buffer};
use winit::keyboard::{Key, SmolStr};

// Four booleans (heatmap, profile/stats/orbit enabled flags) trip the nursery
// `struct_excessive_bools` lint; grouping them is not worth the churn, so the
// lint is scoped to this struct.
#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub player_controller: PlayerController,

    // Timing
    last_frame_update: Instant,
    delta_time: Duration,

    // Surface size for projection
    surface_width: u32,
    surface_height: u32,

    // Rasterize AABBs pipeline
    rasterize_aabbs_pipeline: wgpu::RenderPipeline,
    camera_bind_group: wgpu::BindGroup,
    resource_bind_group: wgpu::BindGroup,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    camera_uniform_buf: wgpu::Buffer,
    _palette_buf: wgpu::Buffer,

    // Retain texture resources for binding-array lifetime
    _chunk_textures: Vec<wgpu::Texture>,
    _chunk_texture_views: Vec<wgpu::TextureView>,

    instances: Vec<Instance>,
    instance_buffer: wgpu::Buffer,

    depth_texture: wgpu::Texture,

    // Design A renderer (WGPU_RT_RAYQUERY=1): TLAS of chunk AABBs + compute
    // ray-query pass. `None` keeps the rasterized chunk-proxy DDA path, which
    // is the default and remains A/B-able in the bench.
    rayquery: Option<RayQueryResources>,

    // Display traversal cost instead of voxel colors.
    heatmap: bool,

    // GPU-side profiling. WGPU_RT_PROFILE=1 enables per-frame timestamp
    // queries for the chunk pass; WGPU_RT_STATS=1 additionally compiles the
    // shader with atomic DDA-work counters. Results are read back with a
    // blocking Wait poll, so profiling also caps the loop at GPU speed.
    stats_buf: Option<wgpu::Buffer>,
    stats_readback: Option<wgpu::Buffer>,
    timestamp_query: Option<wgpu::QuerySet>,
    timestamp_buf: Option<wgpu::Buffer>,
    timestamp_readback: Option<wgpu::Buffer>,
    timestamp_period: f32,
    profile_enabled: bool,
    profile_accum: ProfileAccum,
    profile_reported_at: Option<Instant>,

    // Debug orbit camera (plan 012)
    orbit_enabled: bool,
    orbit_elapsed: Duration,
    orbit_target: glam::Vec3, // app.rs does not import glam types; use the fully-qualified path
    orbit_radius: f32,
    last_orbit_log_secs: u64,

    // WGPU_RT_DUMP: write one raw frame dump (raster surface or ray-query
    // target) after this many rendered frames.
    frames_rendered: u64,
}

// `device.create_texture` is not const-callable; the lint misfires here.
#[allow(clippy::missing_const_for_fn)]
fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[wgpu::TextureFormat::Depth32Float],
    })
}

/// Per-second accumulation of GPU-side profiling counters (see `read_profile`).
#[derive(Default)]
struct ProfileAccum {
    frames: u32,
    gpu_ms: f64,
    fragments: u64,
    cells: u64,
    hits: u64,
}

/// `WGPU_RT_RAYQUERY=1` selects the Design A renderer (see
/// plans/019). The experimental ray-query feature is Vulkan-only, so
/// it is requested at device creation only when this gate is set.
fn rayquery_requested() -> bool {
    std::env::var("WGPU_RT_RAYQUERY").is_ok_and(|v| v == "1")
}

impl App {
    pub const SRGB: bool = true;

    // wgpu::Features bit-ops are not const-stable; the lint misfires here.
    #[allow(clippy::missing_const_for_fn)]
    pub fn required_features() -> wgpu::Features {
        let mut features = wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
        if rayquery_requested() {
            features |= wgpu::Features::EXPERIMENTAL_RAY_QUERY;
        }
        features
    }

    // wgpu::Features bit-ops are not const-stable; the lint misfires here.
    #[allow(clippy::missing_const_for_fn)]
    pub fn optional_features() -> wgpu::Features {
        // TIMESTAMP_QUERY_INSIDE_ENCODERS: the Design A ray-query pass
        // brackets its GPU time with `CommandEncoder::write_timestamp` (the
        // raster path uses RenderPassTimestampWrites, which needs
        // TIMESTAMP_QUERY_INSIDE_PASSES). Both are gated behind the adapter
        // check in the device request.
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
    }

    pub fn required_downlevel_capabilities() -> wgpu::DownlevelCapabilities {
        wgpu::DownlevelCapabilities {
            flags: wgpu::DownlevelFlags::COMPUTE_SHADERS,
            ..Default::default()
        }
    }

    pub fn required_limits() -> wgpu::Limits {
        let limits = wgpu::Limits::default();
        if rayquery_requested() {
            // Vulkan's guaranteed minimums for acceleration-structure limits;
            // requesting them only makes sense with EXPERIMENTAL_RAY_QUERY.
            limits.using_minimum_supported_acceleration_structure_values()
        } else {
            limits
        }
    }

    /// Device-side agreement to use experimental features (the ray-query API
    /// is acknowledged-UB experimental). Must be passed to the device request
    /// whenever `WGPU_RT_RAYQUERY=1` adds `EXPERIMENTAL_RAY_QUERY` to
    /// `required_features`.
    pub fn experimental_features() -> wgpu::ExperimentalFeatures {
        if rayquery_requested() {
            // SAFETY: the Design A renderer is behind an env gate; enabling
            // the experimental ray-query API is the user's explicit opt-in.
            unsafe { wgpu::ExperimentalFeatures::enabled() }
        } else {
            wgpu::ExperimentalFeatures::disabled()
        }
    }

    // The wgpu device/pipeline setup is one tightly-coupled block; splitting it
    // up would risk subtle behavior changes, so the line-count lint is scoped
    // to `init`.
    #[allow(clippy::too_many_lines)]
    pub fn init(
        config: &wgpu::SurfaceConfiguration,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Self {
        let width = config.width;
        let height = config.height;

        let profile_enabled = std::env::var("WGPU_RT_PROFILE").is_ok_and(|v| v == "1");
        let stats_enabled = std::env::var("WGPU_RT_STATS").is_ok_and(|v| v == "1");
        let heatmap_enabled = std::env::var("WGPU_RT_HEATMAP").is_ok_and(|v| v == "1");

        let player_controller = PlayerController::default();

        let world_path = std::env::var("WGPU_RT_WORLD")
            .unwrap_or_else(|_| "assets/models/monu1.vox".to_string());
        let world = World::load(&world_path);

        let voxel_count = world.voxels.len();
        log::info!("Loaded {voxel_count} voxels (material-0 already filtered by loader)");

        let palette = world.palette;
        let chunks = world.into_chunks();

        let non_empty_chunks: Vec<(usize, crate::world::chunk::Chunk)> = chunks
            .into_iter()
            .enumerate()
            .filter(|(_, c)| !c.is_empty())
            .collect();

        let texture_count = non_empty_chunks.len().max(1);
        log::info!(
            "Non-empty chunks: {}, texture binding count: {}",
            non_empty_chunks.len(),
            texture_count,
        );

        let max_binding = adapter.limits().max_binding_array_elements_per_shader_stage;
        log::info!("Adapter max_binding_array_elements_per_shader_stage: {max_binding}");
        if max_binding < u32::try_from(texture_count).unwrap_or(u32::MAX) {
            fatal(&format!(
                "STOP: required binding array count {texture_count} exceeds adapter limit {max_binding}",
            ));
        }

        let chunk_side_world = CHUNK_SIZE.x * VOXEL_SCALE;
        // let half_chunk_world = chunk_side_world * 0.5;

        let mut chunk_textures: Vec<wgpu::Texture> = Vec::with_capacity(texture_count);
        let mut chunk_texture_views: Vec<wgpu::TextureView> = Vec::with_capacity(texture_count);
        let mut instances: Vec<Instance> = Vec::with_capacity(texture_count);

        if non_empty_chunks.is_empty() {
            let dummy_chunk = crate::world::chunk::Chunk::new(glam::IVec3::ZERO);
            let tex = dummy_chunk.create_texture(device, queue);
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            chunk_textures.push(tex);
            chunk_texture_views.push(view);
        } else {
            for (_grid_index, chunk) in &non_empty_chunks {
                let tex = chunk.create_texture(device, queue);
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());

                let gp = chunk.grid_position();
                let position = glam::Vec3::new(
                    i32_to_f32(gp.x) * chunk_side_world,
                    i32_to_f32(gp.y) * chunk_side_world,
                    i32_to_f32(gp.z) * chunk_side_world,
                );

                instances.push(Instance { position });

                chunk_textures.push(tex);
                chunk_texture_views.push(view);
            }
        }

        let texture_count_final = chunk_texture_views.len();
        log::info!(
            "Created {} chunk textures, {} draw instances",
            texture_count_final,
            instances.len(),
        );

        let orbit_enabled = std::env::var("WGPU_RT_ORBIT").is_ok_and(|v| v == "1");
        let (orbit_target, orbit_radius) = if orbit_enabled {
            if instances.is_empty() {
                log::info!("Orbit camera: no chunks; falling back to target (0,0,0) radius 64.0");
                (glam::Vec3::ZERO, chunk_side_world * 2.0)
            } else {
                let origins: Vec<glam::Vec3> = instances.iter().map(|i| i.position).collect();
                // f32 component math: not subject to `arithmetic_side_effects`.
                let mut tx = 0.0f32;
                let mut ty = 0.0f32;
                let mut tz = 0.0f32;
                for o in &origins {
                    tx += o.x;
                    ty += o.y;
                    tz += o.z;
                }
                let inv_count = crate::utils::usize_to_f32(origins.len().max(1)).recip();
                let half_chunk = chunk_side_world * 0.5;
                let target = glam::Vec3::new(
                    tx.mul_add(inv_count, half_chunk),
                    ty.mul_add(inv_count, half_chunk),
                    tz.mul_add(inv_count, half_chunk),
                );
                let radius = crate::player_controller::orbit_radius_from_chunks(
                    &origins,
                    chunk_side_world,
                    1.3,
                );
                log::info!(
                    "Orbit camera: target=({:.1},{:.1},{:.1}) radius={:.1}m chunks={} (azimuth 60s, elevation 5..55 deg)",
                    target.x,
                    target.y,
                    target.z,
                    radius,
                    origins.len(),
                );
                (target, radius)
            }
        } else {
            (glam::Vec3::ZERO, chunk_side_world)
        };

        let palette_buf = create_palette_buffer(device, &palette);

        let camera_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera_uniform"),
            size: u64::try_from(std::mem::size_of::<CameraUniforms>()).unwrap_or_default(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let vertex_size = std::mem::size_of::<Vertex>();
        let instance_size = std::mem::size_of::<InstanceRaw>();
        let (vertex_data, index_data) = create_vertices();

        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&index_data),
            usage: wgpu::BufferUsages::INDEX,
        });

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instance_buffer"),
            contents: bytemuck::cast_slice(
                &instances
                    .iter()
                    .map(Instance::to_raw)
                    .collect::<Vec<InstanceRaw>>(),
            ),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let texture_view_refs: Vec<&wgpu::TextureView> = chunk_texture_views.iter().collect();
        // Always >= 1 (a dummy chunk is pushed when the world is empty).
        let bind_group_count =
            NonZeroU32::new(u32::try_from(texture_count_final).unwrap_or_default())
                .unwrap_or(NonZeroU32::MIN);

        // Per-frame DDA work counters. Only created/bound when WGPU_RT_STATS=1:
        // the atomic storage writes are a fragment-shader side effect that
        // would disable hardware early-Z, so the clean build must not carry
        // them (see the %%STATS_*%% shader markers).
        let stats_buf = stats_enabled.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("profile_stats_buf"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // COMPUTE added for the Design A ray-query pass, which
                    // shares this layout and bind group with the raster path.
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            u64::try_from(std::mem::size_of::<CameraUniforms>())
                                .unwrap_or_default(),
                        ),
                    },
                    count: None,
                }],
            });

        let mut resource_layout_entries: Vec<wgpu::BindGroupLayoutEntry> = vec![
            // palette
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(256 * 4 * 4),
                },
                count: None,
            },
            // chunk textures
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: Some(bind_group_count),
            },
        ];
        if stats_enabled {
            resource_layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            });
        }
        let resource_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("resource_bind_group_layout"),
                entries: &resource_layout_entries,
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniform_buf.as_entire_binding(),
            }],
            label: Some("camera_bind_group"),
        });

        let mut resource_entries: Vec<wgpu::BindGroupEntry> = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: palette_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureViewArray(&texture_view_refs),
            },
        ];
        if let Some(buf) = stats_buf.as_ref() {
            resource_entries.push(wgpu::BindGroupEntry {
                binding: 2,
                resource: buf.as_entire_binding(),
            });
        }
        let resource_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &resource_bind_group_layout,
            entries: &resource_entries,
            label: Some("resource_bind_group"),
        });

        // Design A renderer (WGPU_RT_RAYQUERY=1): one BLAS per chunk (static
        // world-bounds AABB, never rebuilt on edits) + one TLAS + a compute
        // ray-query pass. Gated so the default rasterized DDA path stays
        // untouched and A/B-able in the bench.
        let rayquery_resources = if rayquery_requested() {
            Some(RayQueryResources::new(
                device,
                queue,
                &crate::render::rayquery::RayQueryParams {
                    instances: &instances,
                    chunk_side_world,
                    palette_buf: &palette_buf,
                    texture_view_refs: &texture_view_refs,
                    bind_group_count,
                    stats_buf: stats_buf.as_ref(),
                    stats_enabled,
                    camera_bind_group_layout: &camera_bind_group_layout,
                    width,
                    height,
                    target_format: config
                        .view_formats
                        .first()
                        .copied()
                        .unwrap_or(config.format),
                },
            ))
        } else {
            None
        };

        let rasterize_aabbs_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rasterize_aabbs_pipeline_layout"),
                bind_group_layouts: &[
                    Some(&camera_bind_group_layout),
                    Some(&resource_bind_group_layout),
                ],
                ..Default::default()
            });

        // Compile the chunk shader. WGPU_RT_STATS=1 injects atomic DDA-work
        // counters via the %%STATS_*%% markers; the default build keeps the
        // fragment shader free of side effects so early-Z stays enabled.
        let shader_source = include_str!("../assets/shaders/chunk.wgsl")
            .replace("// %%STATS_DECLS%%", if stats_enabled {
                "struct Stats {\n    fragments: atomic<u32>,\n    processed_cells: atomic<u32>,\n    hits: atomic<u32>,\n};\n@group(1) @binding(2) var<storage, read_write> stats: Stats;\n"
            } else {
                ""
            })
            .replace(
                "// %%STATS_FRAGMENT%%",
                if stats_enabled {
                    "atomicAdd(&stats.fragments, 1u);"
                } else {
                    ""
                },
            )
            .replace(
                "// %%STATS_CELLS%%",
                if stats_enabled {
                    "atomicAdd(&stats.processed_cells, 1u);"
                } else {
                    ""
                },
            )
            .replace(
                "// %%STATS_HIT%%",
                if stats_enabled {
                    "atomicAdd(&stats.hits, 1u);"
                } else {
                    ""
                },
            );
        let rasterize_aabbs_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rasterize_aabbs_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_source)),
        });

        let vertex_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: u64::try_from(vertex_size).unwrap_or_default(),
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    // position
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 0,
                        shader_location: 0,
                    },
                    // uv
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 4 * 4,
                        shader_location: 1,
                    },
                ],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: u64::try_from(instance_size).unwrap_or_default(),
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &[
                    // mat4x4 transform
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 0,
                        shader_location: 2,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 4 * 4,
                        shader_location: 3,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 4 * 8,
                        shader_location: 4,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 4 * 12,
                        shader_location: 5,
                    },
                    // chunk origin
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 4 * 16,
                        shader_location: 6,
                    },
                ],
            }),
        ];

        let rasterize_aabbs_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("rasterize_aabbs_pipeline"),
                layout: Some(&rasterize_aabbs_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &rasterize_aabbs_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &vertex_buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &rasterize_aabbs_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(
                        config
                            .view_formats
                            .first()
                            .copied()
                            .unwrap_or(config.format)
                            .into(),
                    )],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Front),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let depth_texture = create_depth_texture(device, width, height);

        // Timestamp queries for the chunk pass GPU time (WGPU_RT_PROFILE=1).
        let (timestamp_query, timestamp_buf, timestamp_readback, timestamp_period) =
            if profile_enabled && device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
                let query = device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("profile_timestamp_query"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                });
                let ts_buf = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("profile_timestamp_buf"),
                    size: 16,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let ts_readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("profile_timestamp_readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                (
                    Some(query),
                    Some(ts_buf),
                    Some(ts_readback),
                    queue.get_timestamp_period(),
                )
            } else {
                if profile_enabled {
                    log::warn!(
                        "TIMESTAMP_QUERY not supported on this adapter; GPU timing disabled"
                    );
                }
                (None, None, None, 0.0)
            };
        let stats_readback = stats_enabled.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("profile_stats_readback"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });

        Self {
            player_controller,

            last_frame_update: Instant::now(),
            delta_time: Duration::default(),
            surface_width: width,
            surface_height: height,

            rasterize_aabbs_pipeline,
            camera_bind_group,
            resource_bind_group,
            vertex_buf,
            index_buf,
            camera_uniform_buf,
            _palette_buf: palette_buf,

            _chunk_textures: chunk_textures,
            _chunk_texture_views: chunk_texture_views,

            instances,
            instance_buffer,

            depth_texture,

            rayquery: rayquery_resources,

            stats_buf,
            stats_readback,
            timestamp_query,
            timestamp_buf,
            timestamp_readback,
            timestamp_period,
            profile_enabled,
            profile_accum: ProfileAccum::default(),
            profile_reported_at: None,

            heatmap: heatmap_enabled,

            orbit_enabled,
            orbit_target,
            orbit_radius,
            orbit_elapsed: Duration::ZERO,
            last_orbit_log_secs: 0,
            frames_rendered: 0,
        }
    }

    fn update_delta_time(&mut self) {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame_update);
        self.last_frame_update = now;
        self.delta_time = delta;
    }

    pub fn resize(
        &mut self,
        config: &wgpu::SurfaceConfiguration,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) {
        self.surface_width = config.width;
        self.surface_height = config.height;
        self.depth_texture = create_depth_texture(device, config.width, config.height);
        if let Some(res) = self.rayquery.as_mut() {
            res.recreate_target(device, config.width, config.height);
        }
    }

    // Same rationale as `init`: long but tightly coupled render setup.
    #[allow(clippy::too_many_lines)]
    pub fn render(
        &mut self,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        keys: &HashSet<Key<SmolStr>>,
    ) {
        self.update_delta_time();

        let (view_mat, camera_pos) = if self.orbit_enabled {
            self.orbit_elapsed = self.orbit_elapsed.saturating_add(self.delta_time);
            let orbit_params = crate::player_controller::DEFAULT_ORBIT_PARAMS;
            let (pos, target) = crate::player_controller::orbit_pose(
                self.orbit_elapsed.as_secs_f32(),
                self.orbit_target,
                self.orbit_radius,
                &orbit_params,
            );
            let view_mat = glam::camera::rh::view::look_at_mat4(pos, target, glam::Vec3::Y);
            let secs = self.orbit_elapsed.as_secs();
            if secs != self.last_orbit_log_secs {
                self.last_orbit_log_secs = secs;
                log::info!(
                    "Orbit: t={:.1}s az={:.1} deg elev={:.1} deg pos=({:.1},{:.1},{:.1})",
                    secs,
                    (std::f32::consts::TAU * u64_to_f32(secs) / orbit_params.az_period)
                        .to_degrees()
                        .rem_euclid(360.0),
                    ((pos.y - target.y) / self.orbit_radius).asin().to_degrees(),
                    pos.x,
                    pos.y,
                    pos.z,
                );
            }
            (view_mat, pos)
        } else {
            self.player_controller.fly_movement(self.delta_time, keys);
            let view_mat = self.player_controller.view();
            let camera_pos = self.player_controller.camera_position();
            (view_mat, camera_pos)
        };

        let aspect = u32_to_f32(self.surface_width) / u32_to_f32(self.surface_height);

        let proj_mat = glam::camera::rh::proj::directx::perspective(
            std::f32::consts::FRAC_PI_4,
            aspect,
            0.1,
            10000.0,
        );

        let view_proj = proj_mat.mul_mat4(&view_mat);
        let view_inv = view_mat.inverse();
        let proj_inv = proj_mat.inverse();

        let camera_uniforms = CameraUniforms {
            camera_pos: [camera_pos.x, camera_pos.y, camera_pos.z, 0.0],
            view_inv: view_inv.to_cols_array_2d(),
            proj_inv: proj_inv.to_cols_array_2d(),
            view_proj: view_proj.to_cols_array_2d(),
            viewport_and_heatmap: [
                u32_to_f32(self.surface_width),
                u32_to_f32(self.surface_height),
                if self.heatmap { 1.0 } else { 0.0 },
                0.0,
            ],
        };

        queue.write_buffer(
            &self.camera_uniform_buf,
            0,
            bytemuck::cast_slice(&[camera_uniforms]),
        );

        // Front-to-back instance ordering (raster path only; the ray-query
        // pass needs no draw order). With the frag_depth write removed, the
        // depth test runs before the fragment shader (hardware early-Z), but
        // a chunk's fragments are only rejected once a NEARER chunk has
        // already written depth for that pixel — so the culling benefit is
        // entirely draw-order dependent. Sort by chunk center projected on
        // the camera forward axis; chunk boxes tile the world exactly, so
        // this yields near-to-far order along every view ray.
        if self.rayquery.is_none() {
            let half_chunk_world = CHUNK_SIZE.x * VOXEL_SCALE * 0.5;
            let fwd = view_inv
                .mul_vec4(glam::Vec4::new(0.0, 0.0, -1.0, 0.0))
                .truncate();
            let mut order: Vec<&Instance> = self.instances.iter().collect();
            order.sort_by(|a, b| {
                let ka = glam::Vec3::new(
                    a.position.x + half_chunk_world - camera_pos.x,
                    a.position.y + half_chunk_world - camera_pos.y,
                    a.position.z + half_chunk_world - camera_pos.z,
                )
                .dot(fwd);
                let kb = glam::Vec3::new(
                    b.position.x + half_chunk_world - camera_pos.x,
                    b.position.y + half_chunk_world - camera_pos.y,
                    b.position.z + half_chunk_world - camera_pos.z,
                )
                .dot(fwd);
                ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
            });
            let sorted_raws: Vec<InstanceRaw> = order.iter().map(|i| i.to_raw()).collect();
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&sorted_raws));
        }

        if let Some(stats_buf) = self.stats_buf.as_ref() {
            queue.write_buffer(stats_buf, 0, bytemuck::cast_slice(&[0u32; 4]));
        }

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        if let Some(res) = self.rayquery.as_ref() {
            // Design A: compute ray-query pass against the chunk TLAS, then a
            // blit to the surface. GPU time is bracketed with write_timestamp
            // (the raster path times its render pass with
            // RenderPassTimestampWrites on the same query set).
            if let Some(query) = self.timestamp_query.as_ref() {
                encoder.write_timestamp(query, 0);
            }
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                cpass.push_debug_group("Design A ray query");
                cpass.set_pipeline(&res.pipeline);
                cpass.set_bind_group(0, Some(&self.camera_bind_group), &[]);
                cpass.set_bind_group(1, Some(&res.bind_group), &[]);
                cpass.set_bind_group(2, Some(&res.out_bind_group), &[]);
                cpass.dispatch_workgroups(
                    self.surface_width.div_ceil(8),
                    self.surface_height.div_ceil(8),
                    1,
                );
                cpass.pop_debug_group();
            }
            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rpass.push_debug_group("Blit ray-query target");
                rpass.set_pipeline(&res.blit_pipeline);
                rpass.set_bind_group(0, Some(&res.blit_bind_group), &[]);
                rpass.draw(0..4, 0..1);
                rpass.pop_debug_group();
            }
            if let Some(query) = self.timestamp_query.as_ref() {
                encoder.write_timestamp(query, 1);
            }
        } else {
            let depth_view = self
                .depth_texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timestamp_query.as_ref().map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rpass.push_debug_group("Prepare data for draw.");
            rpass.set_pipeline(&self.rasterize_aabbs_pipeline);
            rpass.set_bind_group(0, &self.camera_bind_group, &[]);
            rpass.set_bind_group(1, &self.resource_bind_group, &[]);
            rpass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            rpass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            rpass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            rpass.pop_debug_group();

            if !self.instances.is_empty() {
                rpass.insert_debug_marker("Draw!");
                rpass.draw_indexed(
                    0..u32::try_from(INDEX_COUNT).unwrap_or_default(),
                    0,
                    0..u32::try_from(self.instances.len()).unwrap_or_default(),
                );
            }
        }

        if let (Some(query), Some(ts_buf), Some(ts_readback)) = (
            self.timestamp_query.as_ref(),
            self.timestamp_buf.as_ref(),
            self.timestamp_readback.as_ref(),
        ) {
            encoder.resolve_query_set(query, 0..2, ts_buf, 0);
            encoder.copy_buffer_to_buffer(ts_buf, 0, ts_readback, 0, 16);
        }
        if let (Some(stats_buf), Some(stats_readback)) =
            (self.stats_buf.as_ref(), self.stats_readback.as_ref())
        {
            encoder.copy_buffer_to_buffer(stats_buf, 0, stats_readback, 0, 16);
        }

        queue.submit(Some(encoder.finish()));

        if self.profile_enabled {
            self.read_profile(device);
        }
        self.maybe_dump_frame(view, device, queue);
    }

    /// Maps the per-frame readback buffers (blocking Wait poll — this is the
    /// profiling loop being capped at GPU speed) and folds the results into
    /// `profile_accum`, reporting once per second.
    fn read_profile(&mut self, device: &wgpu::Device) {
        let Some(ts_readback) = self.timestamp_readback.as_ref() else {
            return;
        };
        ts_readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let stats_readback = self.stats_readback.as_ref();
        if let Some(sr) = stats_readback {
            sr.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        }
        device.poll(wgpu::PollType::wait_indefinitely()).ok();

        {
            let mapped = ts_readback
                .slice(..)
                .get_mapped_range()
                .unwrap_or_else(|_| fatal("timestamp readback buffer is not mapped"));
            let ts: &[u64] = bytemuck::cast_slice(&mapped);
            let start = ts.first().copied().unwrap_or_default();
            let end = ts.get(1).copied().unwrap_or_default();
            if end >= start {
                let mut gpu_ns = u64_to_f64(end.saturating_sub(start));
                gpu_ns *= f64::from(self.timestamp_period.max(1.0));
                gpu_ns /= 1e6;
                self.profile_accum.gpu_ms += gpu_ns;
            }
            drop(mapped);
            ts_readback.unmap();
        }

        if let Some(sr) = stats_readback {
            let mapped = sr
                .slice(..)
                .get_mapped_range()
                .unwrap_or_else(|_| fatal("stats readback buffer is not mapped"));
            let s: &[u32] = bytemuck::cast_slice(&mapped);
            let fragments = s.first().copied().unwrap_or_default();
            let cells = s.get(1).copied().unwrap_or_default();
            let hits = s.get(2).copied().unwrap_or_default();
            self.profile_accum.fragments = self
                .profile_accum
                .fragments
                .saturating_add(u64::from(fragments));
            self.profile_accum.cells = self.profile_accum.cells.saturating_add(u64::from(cells));
            self.profile_accum.hits = self.profile_accum.hits.saturating_add(u64::from(hits));
            drop(mapped);
            sr.unmap();
        }

        self.profile_accum.frames = self.profile_accum.frames.saturating_add(1);
        let now = Instant::now();
        if self
            .profile_reported_at
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
        {
            self.report_profile();
            self.profile_reported_at = Some(now);
        }
    }

    fn report_profile(&mut self) {
        let a = &self.profile_accum;
        if a.frames == 0 {
            return;
        }
        let pixels = f64::from(self.surface_width.saturating_mul(self.surface_height));
        let frames = f64::from(a.frames);
        let mut frags_per_frame = u64_to_f64(a.fragments);
        frags_per_frame /= frames;
        let mut cells_per_frame = u64_to_f64(a.cells);
        cells_per_frame /= frames;
        let mut gpu_ms = a.gpu_ms;
        gpu_ms /= frames;
        let mut hits_per_frame = u64_to_f64(a.hits);
        hits_per_frame /= frames;
        let mut cells_per_frag = u64_to_f64(a.cells);
        cells_per_frag /= u64_to_f64(a.fragments.max(1));
        let mut traffic = cells_per_frame;
        traffic *= 4.0;
        traffic /= 1e6;
        log::info!(
            "[profile] {} frames | gpu {:.2} ms | frags/frame {:>9.0} ({:.2}x px) | cells/frame {:>10.0} | cells/frag {:.1} | hits/frame {:>8.0} | {:>7.1} MB/frame texel traffic",
            a.frames,
            gpu_ms,
            frags_per_frame,
            frags_per_frame / pixels,
            cells_per_frame,
            cells_per_frag,
            hits_per_frame,
            traffic,
        );
        self.profile_accum = ProfileAccum::default();
    }

    pub fn toggle_heatmap(&mut self) {
        self.heatmap = !self.heatmap;
        log::info!(
            "Traversal heatmap {}",
            if self.heatmap { "enabled" } else { "disabled" }
        );
    }

    pub fn toggle_orbit_camera(&mut self) {
        self.orbit_enabled = !self.orbit_enabled;
        if self.orbit_enabled {
            self.orbit_elapsed = Duration::ZERO;
            self.last_orbit_log_secs = 0;
            log::info!(
                "Orbit camera enabled: target=({:.1},{:.1},{:.1}) radius={:.1}m",
                self.orbit_target.x,
                self.orbit_target.y,
                self.orbit_target.z,
                self.orbit_radius,
            );
        } else {
            log::info!("Orbit camera disabled");
        }
    }

    pub fn update_look_position(&mut self, delta: (f64, f64)) {
        if self.orbit_enabled {
            return;
        }
        self.player_controller.rotate(delta);
    }

    /// `WGPU_RT_DUMP=<dir>`: after a warm-up, write one raw frame dump from the
    /// active renderer (ray-query target or raster surface view). Debug path
    /// for visual A/B; the raster dump requires `COPY_SRC` on the target
    /// (bench's offscreen texture has it, the interactive swapchain does not).
    fn maybe_dump_frame(
        &mut self,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let Some(dump_dir) = std::env::var("WGPU_RT_DUMP").ok().filter(|s| !s.is_empty()) else {
            return;
        };
        self.frames_rendered = self.frames_rendered.saturating_add(1);
        if self.frames_rendered != 40 {
            return;
        }
        let width = self.surface_width;
        let height = self.surface_height;
        let bytes_per_row = width.saturating_mul(4);
        if !bytes_per_row.is_multiple_of(256) {
            log::warn!("WGPU_RT_DUMP skipped: width {width} needs bytes_per_row % 256 == 0");
            return;
        }
        let path = if self.rayquery.is_some() {
            std::path::Path::new(&dump_dir).join("dump_rayquery.bgra")
        } else {
            std::path::Path::new(&dump_dir).join("dump_raster.bgra")
        };
        log::info!("WGPU_RT_DUMP: writing {}", path.display());
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame_dump_buffer"),
            size: u64::from(bytes_per_row).saturating_mul(u64::from(height)),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: view.texture(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let mapped = slice.get_mapped_range().unwrap_or_else(|e| {
            crate::utils::fatal(&format!("frame dump buffer is not mapped: {e}"))
        });
        std::fs::write(&path, &mapped).unwrap_or_else(|e| {
            crate::utils::fatal(&format!("failed to write dump {}: {e}", path.display()))
        });
    }
}

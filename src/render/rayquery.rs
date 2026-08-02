//! Design A renderer: a fullscreen compute ray-query pass against a TLAS of
//! chunk AABBs (see plans/019). `WGPU_RT_RAYQUERY=1` swaps the
//! rasterized chunk-proxy pass for this compute pass + a blit to the surface.
//!
//! Dynamic-world properties preserved from the texture design: a chunk's BLAS
//! is its static world-bounds AABB, so it is **never** rebuilt on edits;
//! edits stay async `write_texture` texel copies; the TLAS (<=64 instances)
//! is rebuilt only when the chunk set changes. The chunk DDA (ported verbatim
//! into `assets/shaders/rayquery.wgsl`) is the procedural intersection test:
//! the hardware reports the chunk AABB as a candidate and the shader commits
//! the true voxel hit with `rayQueryGenerateIntersection`.

use std::num::NonZeroU32;

use wgpu::util::DeviceExt;

use crate::render::{GpuAabb, Instance};

/// Parameters for `RayQueryResources::new`, bundled to keep the constructor
/// argument count sane.
pub struct RayQueryParams<'a> {
    pub instances: &'a [Instance],
    pub chunk_side_world: f32,
    pub palette_buf: &'a wgpu::Buffer,
    pub texture_view_refs: &'a [&'a wgpu::TextureView],
    pub bind_group_count: NonZeroU32,
    pub stats_buf: Option<&'a wgpu::Buffer>,
    pub stats_enabled: bool,
    pub camera_bind_group_layout: &'a wgpu::BindGroupLayout,
    pub width: u32,
    pub height: u32,
    pub target_format: wgpu::TextureFormat,
}

/// Affine 3x4 (rows x columns, row-major) identity for TLAS instances: the
/// chunk AABBs are already in world space, so the instance transform is
/// identity.
const fn identity_transform() -> [f32; 12] {
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
}

fn create_rt_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rt_target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// GPU resources for the Design A renderer. Group layout:
/// - group(0): camera uniforms (shared layout with the raster path)
/// - group(1): palette, chunk textures, chunk AABBs, stats (optional), TLAS
/// - group(2): storage target written by the compute pass
pub struct RayQueryResources {
    /// One BLAS per non-empty chunk (the chunk's static world bounds).
    /// Never read on the CPU: kept alive so the TLAS instance references stay
    /// valid.
    /// The world BLAS: one AABB primitive per chunk, built once; a dirty
    /// chunk only requires rebuilding the BLAS with new AABB data (all
    /// primitives, microseconds for 8-64 chunks).
    _world_blas: wgpu::Blas,
    /// Kept alive so the bind group's `AccelerationStructure` binding stays
    /// valid.
    _tlas: wgpu::Tlas,
    /// BLAS input data; Vulkan requires the buffer to outlive every BLAS
    /// built from it, and the DDA re-reads it via the storage binding.
    _aabb_buf: wgpu::Buffer,
    rt_target: wgpu::Texture,
    rt_view: wgpu::TextureView,
    pub pipeline: wgpu::ComputePipeline,
    pub bind_group: wgpu::BindGroup,
    pub out_bind_group: wgpu::BindGroup,
    pub blit_pipeline: wgpu::RenderPipeline,
    pub blit_bind_group: wgpu::BindGroup,
    blit_sampler: wgpu::Sampler,
}

// Same rationale as `App::init`: the BLAS/TLAS/pipeline setup is one
// tightly-coupled block; splitting it up would risk subtle behavior changes.
#[allow(clippy::too_many_lines)]
impl RayQueryResources {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, p: &RayQueryParams<'_>) -> Self {
        // --- AABB buffer: one world-space chunk box per instance ---
        let aabb_data: Vec<GpuAabb> = p
            .instances
            .iter()
            .map(|i| GpuAabb {
                min: [i.position.x, i.position.y, i.position.z],
                max: [
                    i.position.x + p.chunk_side_world,
                    i.position.y + p.chunk_side_world,
                    i.position.z + p.chunk_side_world,
                ],
                _pad: [0.0, 0.0],
            })
            .collect();
        let aabb_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_aabb_buffer"),
            contents: bytemuck::cast_slice(&aabb_data),
            usage: wgpu::BufferUsages::BLAS_INPUT | wgpu::BufferUsages::STORAGE,
        });

        // --- One world BLAS holding one AABB primitive per chunk ---
        // (per-chunk BLASes with `primitive_offset` into a shared buffer were
        // the first attempt; multi-instance builds dropped ~60% of expected
        // hits, so the AS is built as a single primitive list and the chunk id
        // is recovered from the query's `primitive_index` instead. One BLAS
        // also keeps the edit path simple: a dirty chunk triggers one BLAS
        // rebuild of a handful of AABBs, microseconds.)
        let primitive_count = u32::try_from(p.instances.len().max(1)).unwrap_or_default();
        let aabb_size_desc = wgpu::BlasAABBGeometrySizeDescriptor {
            primitive_count,
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };
        let world_blas = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some("world_blas"),
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::AABBs {
                descriptors: vec![aabb_size_desc.clone()],
            },
        );

        // --- TLAS: a single identity instance over the world BLAS ---
        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("chunk_tlas"),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            max_instances: 1,
        });
        {
            let slot = tlas
                .get_mut_single(0)
                .unwrap_or_else(|| crate::utils::fatal("tlas instance slot 0 out of range"));
            *slot = Some(wgpu::TlasInstance::new(
                &world_blas,
                identity_transform(),
                0,
                0xFF,
            ));
        }
        let build_entries = [wgpu::BlasBuildEntry {
            blas: &world_blas,
            geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry {
                size: &aabb_size_desc,
                stride: u64::try_from(std::mem::size_of::<GpuAabb>()).unwrap_or_default(),
                aabb_buffer: &aabb_buf,
                // Primitive i is the AABB at byte i * 32; the shader recovers
                // it as gpu_aabbs[primitive_index].
                primitive_offset: 0,
            }]),
        }];
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.build_acceleration_structures(build_entries.iter(), std::iter::once(&tlas));
        queue.submit(Some(encoder.finish()));

        // --- Compute ray-query pipeline ---
        let mut resource_layout_entries: Vec<wgpu::BindGroupLayoutEntry> = vec![
            // palette
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
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
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: Some(p.bind_group_count),
            },
            // chunk AABBs (BLAS input, re-read by the DDA); 32 bytes covers
            // one GpuAabb element.
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(32),
                },
                count: None,
            },
        ];
        if p.stats_enabled {
            resource_layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            });
        }
        resource_layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::AccelerationStructure {
                vertex_return: false,
            },
            count: None,
        });
        let rayquery_resource_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rayquery_resource_bind_group_layout"),
                entries: &resource_layout_entries,
            });

        let rayquery_out_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rayquery_out_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                }],
            });

        let rayquery_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rayquery_pipeline_layout"),
                bind_group_layouts: &[
                    Some(p.camera_bind_group_layout),
                    Some(&rayquery_resource_layout),
                    Some(&rayquery_out_layout),
                ],
                ..Default::default()
            });

        // Compile the ray-query shader; the %%STATS_*%% markers mirror the
        // chunk.wgsl stats instrumentation (WGPU_RT_STATS=1).
        let shader_source = include_str!("../../assets/shaders/rayquery.wgsl")
            .replace("// %%STATS_DECLS%%", if p.stats_enabled {
                "struct Stats {\n    fragments: atomic<u32>,\n    processed_cells: atomic<u32>,\n    hits: atomic<u32>,\n};\n@group(1) @binding(3) var<storage, read_write> stats: Stats;\n"
            } else {
                ""
            })
            .replace(
                "// %%STATS_PIXEL%%",
                if p.stats_enabled {
                    "atomicAdd(&stats.fragments, 1u);"
                } else {
                    ""
                },
            )
            .replace(
                "// %%STATS_CELLS%%",
                if p.stats_enabled {
                    "atomicAdd(&stats.processed_cells, 1u);"
                } else {
                    ""
                },
            )
            .replace(
                "// %%STATS_HIT%%",
                if p.stats_enabled {
                    "atomicAdd(&stats.hits, 1u);"
                } else {
                    ""
                },
            );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rayquery_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(shader_source)),
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("rayquery_pipeline"),
            layout: Some(&rayquery_pipeline_layout),
            module: &shader,
            entry_point: Some("rq_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // --- Bind groups ---
        let mut resource_entries: Vec<wgpu::BindGroupEntry> = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: p.palette_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureViewArray(p.texture_view_refs),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: aabb_buf.as_entire_binding(),
            },
        ];
        if let Some(stats_buf) = p.stats_buf {
            resource_entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: stats_buf.as_entire_binding(),
            });
        }
        resource_entries.push(wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::AccelerationStructure(&tlas),
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &rayquery_resource_layout,
            entries: &resource_entries,
            label: Some("rayquery_bind_group"),
        });

        let (rt_target, rt_view) = create_rt_target(device, p.width, p.height);

        let out_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &rayquery_out_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            }],
            label: Some("rayquery_out_bind_group"),
        });

        // --- Blit pipeline (storage target -> surface) ---
        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blit_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit_pipeline_layout"),
            bind_group_layouts: &[Some(&blit_bind_group_layout)],
            ..Default::default()
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit_pipeline"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("blit_vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("blit_fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(p.target_format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&rt_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&blit_sampler),
                },
            ],
            label: Some("blit_bind_group"),
        });

        Self {
            _world_blas: world_blas,
            _tlas: tlas,
            _aabb_buf: aabb_buf,
            rt_target,
            rt_view,
            pipeline,
            bind_group,
            out_bind_group,
            blit_pipeline,
            blit_bind_group,
            blit_sampler,
        }
    }

    /// Recreates the storage target and the bind groups that reference it
    /// (resize). The ray-query bind group (group 1: palette / textures /
    /// AABBs / stats / TLAS) does not reference the target and is untouched.
    pub fn recreate_target(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (rt_target, rt_view) = create_rt_target(device, width, height);
        self.rt_target = rt_target;
        self.rt_view = rt_view;

        let out_layout = self.pipeline.get_bind_group_layout(2);
        let out_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &out_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&self.rt_view),
            }],
            label: Some("rayquery_out_bind_group"),
        });
        self.out_bind_group = out_bind_group;

        let blit_layout = self.blit_pipeline.get_bind_group_layout(0);
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.rt_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
            ],
            label: Some("blit_bind_group"),
        });
        self.blit_bind_group = blit_bind_group;
    }
}

use std::borrow::Cow;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use glam::UVec3;
use wgpu::util::DeviceExt;

use winit::event::WindowEvent;
use winit::keyboard::{Key, SmolStr};

use crate::player_controller::PlayerController;
use crate::render::{INDEX_COUNT, Instance, InstanceRaw, Vertex, create_vertices};
use crate::world::chunk::{CHUNK_TEXTURE_SIZE, CHUNKS_X, CHUNKS_Y, CHUNKS_Z, TOTAL_CHUNKS};

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
    rasterize_aabbs_bind_group: wgpu::BindGroup,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    uniform_buf: wgpu::Buffer,

    instances: Vec<Instance>,
    instance_buffer: wgpu::Buffer,

    // Display traversal cost instead of voxel colors.
    heatmap: bool,
}

impl App {
    pub const SRGB: bool = true;

    pub fn required_features() -> wgpu::Features {
        wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SHADER_INT64
    }

    pub fn optional_features() -> wgpu::Features {
        wgpu::Features::empty()
    }

    pub fn required_downlevel_capabilities() -> wgpu::DownlevelCapabilities {
        wgpu::DownlevelCapabilities {
            flags: wgpu::DownlevelFlags::COMPUTE_SHADERS,
            ..Default::default()
        }
    }

    pub fn required_limits() -> wgpu::Limits {
        wgpu::Limits::default()
    }

    pub fn init(
        config: &wgpu::SurfaceConfiguration,
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
    ) -> Self {
        let width = config.width;
        let height = config.height;

        let aspect = width as f32 / height as f32;
        let mut player_controller = PlayerController::default();

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

        let rasterize_aabbs_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rasterize_aabbs_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(64),
                    },
                    count: None,
                }],
            });

        let proj = glam::camera::rh::proj::directx::perspective(
            std::f32::consts::FRAC_PI_4,
            aspect,
            0.1,
            10000.0,
        );
        let view = player_controller.view();

        let mx_total = proj * view;
        let mx_ref: &[f32; 16] = mx_total.as_ref();
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(mx_ref),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let rasterize_aabbs_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &rasterize_aabbs_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
            label: Some("rasterize_aabbs_bind_group"),
        });

        let rasterize_aabbs_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rasterize_aabbs_pipeline_layout"),
                bind_group_layouts: &[Some(&rasterize_aabbs_bind_group_layout)],
                immediate_size: 0,
            });

        let rasterize_aabbs_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rasterize_aabbs_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/aabb_texture.wgsl"
            ))),
        });

        let vertex_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: vertex_size as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 4 * 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 4 * 4,
                        shader_location: 1,
                    },
                ],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: instance_size as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 4 * 0,
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
                    compilation_options: Default::default(),
                    buffers: &vertex_buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &rasterize_aabbs_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(config.view_formats[0].into())],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let instances = (0..TOTAL_CHUNKS)
            .map(|i| Instance {
                position: UVec3::new(
                    i % CHUNKS_X * CHUNK_TEXTURE_SIZE.width,
                    i / CHUNKS_X % CHUNKS_Y * CHUNK_TEXTURE_SIZE.height,
                    i / (CHUNKS_X * CHUNKS_Y) % CHUNKS_Z * CHUNK_TEXTURE_SIZE.depth_or_array_layers,
                )
                .as_vec3(),
            })
            .collect::<Vec<Instance>>();

        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instance_buffer"),
            contents: bytemuck::cast_slice(
                &instances
                    .iter()
                    .map(Instance::to_raw)
                    .collect::<Vec<InstanceRaw>>(),
            ),
            usage: wgpu::BufferUsages::VERTEX,
        });

        App {
            player_controller,

            last_frame_update: Instant::now(),
            delta_time: Duration::default(),
            surface_width: width,
            surface_height: height,

            rasterize_aabbs_pipeline,
            rasterize_aabbs_bind_group,
            vertex_buf,
            index_buf,
            uniform_buf,

            instances,
            instance_buffer,

            heatmap: false,
        }
    }

    fn update_delta_time(&mut self) {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame_update);
        self.last_frame_update = now;
        self.delta_time = delta;
    }

    pub fn update(&mut self, _event: WindowEvent) {}

    pub fn resize(
        &mut self,
        config: &wgpu::SurfaceConfiguration,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) {
        self.surface_width = config.width;
        self.surface_height = config.height;
    }

    pub fn render(
        &mut self,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        keys: &HashSet<Key<SmolStr>>,
    ) {
        self.update_delta_time();

        self.player_controller.fly_movement(self.delta_time, keys);

        let aspect = self.surface_width as f32 / self.surface_height as f32;

        let proj_mat = glam::camera::rh::proj::directx::perspective(
            std::f32::consts::FRAC_PI_4,
            aspect,
            0.1,
            10000.0,
        );
        let view_mat = self.player_controller.view();

        let mx_total = proj_mat * view_mat;
        let mx_ref: &[f32; 16] = mx_total.as_ref();

        queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(mx_ref));

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

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
            rpass.push_debug_group("Prepare data for draw.");
            rpass.set_pipeline(&self.rasterize_aabbs_pipeline);
            rpass.set_bind_group(0, &self.rasterize_aabbs_bind_group, &[]);
            rpass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            rpass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            rpass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            rpass.pop_debug_group();
            rpass.insert_debug_marker("Draw!");
            rpass.draw_indexed(0..INDEX_COUNT as u32, 0, 0..self.instances.len() as u32);
        }

        queue.submit(Some(encoder.finish()));
    }

    pub fn toggle_heatmap(&mut self) {
        self.heatmap = !self.heatmap;
        log::info!(
            "Traversal heatmap {}",
            if self.heatmap { "enabled" } else { "disabled" }
        );
    }

    pub fn update_look_position(&mut self, delta: (f64, f64)) {
        self.player_controller.rotate(delta);
    }
}

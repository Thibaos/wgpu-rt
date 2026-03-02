use std::borrow::Cow;
use std::collections::HashSet;
use std::f32::consts::{self};
use std::time::{Duration, Instant};
use std::u32;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use wgpu::{
    Extent3d, Sampler, StoreOp, TextureDescriptor, TextureFormat, TextureUsages, TextureView,
};
use winit::event::WindowEvent;
use winit::keyboard::{Key, SmolStr};

use crate::player_controller::PlayerController;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_inverse: Mat4,
    proj_inverse: Mat4,
    palette: [Vec4; 256],
}

pub struct App {
    rt_view: TextureView,
    rt_sampler: Sampler,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: usize,
    camera_position_buffer: wgpu::Buffer,
    camera_view_proj_buffer: wgpu::Buffer,
    camera_view_buffer: wgpu::Buffer,
    camera_proj_buffer: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    // storage_bind_group: wgpu::BindGroup,
    camera_bind_group: wgpu::BindGroup,
    blit_view_bind_group: wgpu::BindGroup,
    blit_camera_bind_group: wgpu::BindGroup,
    pub player_controller: PlayerController,
    // palette: [Vec4; 256],
    last_frame_update: Instant,
    delta_time: Duration,
}

#[derive(Clone, Copy, Zeroable, Pod)]
#[repr(C)]
struct AABB {
    // world-space min corner of the AABB
    aabb_min: Vec3,
    // the scale of the node represented by the AABB
    // octants internally are half this scale value
    // max corner is `min + vec3<f32>(scale)`
    scale: f32,
    // 2x2x2 occupancy bitmap of the child nodes of an octree (only 8 of 32 bits used)
    // NOTE 4x4x4 fits perfectly in vec2<u32> and is likely better performance overall
    octants: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    _pos: [f32; 4],
    _tex_coord: [f32; 2],
}

fn vertex(pos: [i8; 3], tc: [i8; 2]) -> Vertex {
    Vertex {
        _pos: [pos[0] as f32, pos[1] as f32, pos[2] as f32, 1.0],
        _tex_coord: [tc[0] as f32, tc[1] as f32],
    }
}

fn create_vertices() -> (Vec<Vertex>, Vec<u16>) {
    let vertex_data = [
        // top (0, 0, 1)
        vertex([-1, -1, 1], [0, 0]),
        vertex([1, -1, 1], [1, 0]),
        vertex([1, 1, 1], [1, 1]),
        vertex([-1, 1, 1], [0, 1]),
        // bottom (0, 0, -1)
        vertex([-1, 1, -1], [1, 0]),
        vertex([1, 1, -1], [0, 0]),
        vertex([1, -1, -1], [0, 1]),
        vertex([-1, -1, -1], [1, 1]),
        // right (1, 0, 0)
        vertex([1, -1, -1], [0, 0]),
        vertex([1, 1, -1], [1, 0]),
        vertex([1, 1, 1], [1, 1]),
        vertex([1, -1, 1], [0, 1]),
        // left (-1, 0, 0)
        vertex([-1, -1, 1], [1, 0]),
        vertex([-1, 1, 1], [0, 0]),
        vertex([-1, 1, -1], [0, 1]),
        vertex([-1, -1, -1], [1, 1]),
        // front (0, 1, 0)
        vertex([1, 1, -1], [1, 0]),
        vertex([-1, 1, -1], [0, 0]),
        vertex([-1, 1, 1], [0, 1]),
        vertex([1, 1, 1], [1, 1]),
        // back (0, -1, 0)
        vertex([1, -1, 1], [0, 0]),
        vertex([-1, -1, 1], [1, 0]),
        vertex([-1, -1, -1], [1, 1]),
        vertex([1, -1, -1], [0, 1]),
    ];

    let index_data: &[u16] = &[
        0, 1, 2, 2, 3, 0, // top
        4, 5, 6, 6, 7, 4, // bottom
        8, 9, 10, 10, 11, 8, // right
        12, 13, 14, 14, 15, 12, // left
        16, 17, 18, 18, 19, 16, // front
        20, 21, 22, 22, 23, 20, // back
    ];

    (vertex_data.to_vec(), index_data.to_vec())
}

impl App {
    pub const SRGB: bool = true;

    pub fn required_features() -> wgpu::Features {
        wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::VERTEX_WRITABLE_STORAGE
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
        wgpu::Limits::default().using_minimum_supported_acceleration_structure_values()
    }

    pub fn init(
        config: &wgpu::SurfaceConfiguration,
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
    ) -> Self {
        // let voxel_data = voxels::open_file("assets/models/nuke.vox");
        // let world = world_from_model(&voxel_data);
        // let palette = get_palette(&voxel_data);

        let format = TextureFormat::R32Uint;

        let render_target = device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::STORAGE_BINDING,
            view_formats: &[format],
        });

        let rt_view = render_target.create_view(&wgpu::TextureViewDescriptor {
            label: Some("rt_view"),
            format: Some(format),
            dimension: Some(wgpu::TextureViewDimension::D2),
            usage: Some(TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: None,
        });

        let rt_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rt_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let vertex_size = size_of::<Vertex>();
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

        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: vertex_size as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 4 * 4,
                    shader_location: 1,
                },
            ],
        }];

        let camera_position = Vec3::new(0.0, 0.0, 2.5);

        let view_inv = Mat4::look_at_rh(camera_position, Vec3::ZERO, Vec3::Y).inverse();
        let proj_inv = Mat4::perspective_rh(
            consts::FRAC_PI_4,
            config.width as f32 / config.height as f32,
            0.001,
            1000.0,
        )
        .inverse();

        let contents = &[
            AABB {
                aabb_min: Vec3::ZERO,
                scale: 128.0,
                octants: u32::MAX,
            },
            AABB {
                aabb_min: Vec3::splat(128.0),
                scale: 128.0,
                octants: 2u32.pow(8),
            },
        ];

        let storage_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(contents),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let camera_position_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[camera_position]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_view_proj_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[(view_inv)]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_view_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[(view_inv)]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_proj_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[(proj_inv)]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/vertex_rt.wgsl"
            ))),
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/blit2.wgsl"
            ))),
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(64),
                    },
                    count: None,
                }],
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_bind_group_layout,
            entries: &[
                // wgpu::BindGroupEntry {
                //     binding: 0,
                //     resource: camera_position_buffer.as_entire_binding(),
                // },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: camera_view_proj_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&camera_bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &vertex_buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(config.format.into())],
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

        // let storage_bind_group_layout = pipeline.get_bind_group_layout(0);
        // let storage_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        //     label: Some("storage"),
        //     layout: &storage_bind_group_layout,
        //     entries: &[wgpu::BindGroupEntry {
        //         binding: 0,
        //         resource: storage_buffer.as_entire_binding(),
        //     }],
        // });

        let blit_view_bind_group_layout = blit_pipeline.get_bind_group_layout(0);
        let blit_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view"),
            layout: &blit_view_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&rt_view),
                },
                // wgpu::BindGroupEntry {
                //     binding: 1,
                //     resource: wgpu::BindingResource::Sampler(&rt_sampler),
                // },
            ],
        });

        let blit_camera_bind_group_layout = blit_pipeline.get_bind_group_layout(1);
        let blit_camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &blit_camera_bind_group_layout,
            entries: &[
                // wgpu::BindGroupEntry {
                //     binding: 0,
                //     resource: camera_position_buffer.as_entire_binding(),
                // },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: camera_view_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: camera_proj_buffer.as_entire_binding(),
                },
            ],
        });

        let player_controller = PlayerController::default();

        App {
            rt_view,
            rt_sampler,
            vertex_buf,
            index_buf,
            index_count: index_data.len(),
            camera_position_buffer,
            camera_view_proj_buffer,
            camera_view_buffer,
            camera_proj_buffer,
            pipeline,
            blit_pipeline,
            camera_bind_group,
            // storage_bind_group,
            blit_view_bind_group,
            blit_camera_bind_group,
            player_controller,
            last_frame_update: Instant::now(),
            delta_time: Duration::default(),
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
        _config: &wgpu::SurfaceConfiguration,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) {
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

        queue.write_buffer(
            &self.camera_position_buffer,
            0,
            bytemuck::cast_slice(&[self.player_controller.translation]),
        );

        let view_mat = self.player_controller.view();
        let proj_mat = Mat4::perspective_rh(
            consts::FRAC_PI_4,
            view.texture().width() as f32 / view.texture().height() as f32,
            0.001,
            1000.0,
        );

        queue.write_buffer(
            &self.camera_view_proj_buffer,
            0,
            bytemuck::cast_slice(&[view_mat * proj_mat]),
        );

        queue.write_buffer(
            &self.camera_view_buffer,
            0,
            bytemuck::cast_slice(&[view_mat.inverse()]),
        );

        queue.write_buffer(
            &self.camera_proj_buffer,
            0,
            bytemuck::cast_slice(&[proj_mat.inverse()]),
        );

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.rt_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            rpass.push_debug_group("Prepare data for draw");

            rpass.set_pipeline(&self.pipeline);
            // rpass.set_bind_group(0, Some(&self.storage_bind_group), &[]);
            rpass.set_bind_group(0, Some(&self.camera_bind_group), &[]);

            rpass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            rpass.set_vertex_buffer(0, self.vertex_buf.slice(..));

            rpass.pop_debug_group();

            rpass.insert_debug_marker("Draw");
            rpass.draw_indexed(0..self.index_count as u32, 0, 0..1);
        }

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::GREEN),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            rpass.set_pipeline(&self.blit_pipeline);
            rpass.set_bind_group(0, Some(&self.blit_view_bind_group), &[]);
            rpass.set_bind_group(1, Some(&self.blit_camera_bind_group), &[]);
            rpass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }

    pub fn update_look_position(&mut self, delta: (f64, f64)) {
        self.player_controller.rotate(delta);
    }
}

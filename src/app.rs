use std::borrow::Cow;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use std::u32;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::{BufferInitDescriptor, DeviceExt};

use wgpu::{
    ColorTargetState, ColorWrites, Extent3d, StoreOp, Texture, TextureDescriptor, TextureFormat,
    TextureUsages, TextureView, TextureViewDescriptor,
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
    render_target: Texture,
    view: TextureView,
    camera_position_buffer: wgpu::Buffer,
    camera_view_proj_buffer: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    storage_bind_group: wgpu::BindGroup,
    camera_bind_group: wgpu::BindGroup,
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
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[format],
        });

        let view = render_target.create_view(&TextureViewDescriptor::default());

        let camera_position = Vec3::new(0.0, 0.0, 2.5);

        let view_mat = Mat4::look_at_rh(camera_position, Vec3::ZERO, Vec3::Y);
        let proj_mat = Mat4::perspective_rh(
            59.0_f32.to_radians(),
            config.width as f32 / config.height as f32,
            0.001,
            1000.0,
        );

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
            contents: bytemuck::cast_slice(&[view_mat * proj_mat]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/vertex_rt.wgsl"
            ))),
        });

        let color_target_state = ColorTargetState {
            format,
            blend: None,
            write_mask: ColorWrites::default(),
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(color_target_state)],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let storage_bind_group_layout = pipeline.get_bind_group_layout(0);

        let camera_bind_group_layout = pipeline.get_bind_group_layout(1);

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &camera_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_position_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: camera_view_proj_buffer.as_entire_binding(),
                },
            ],
        });

        let storage_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &storage_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: storage_buffer.as_entire_binding(),
            }],
        });

        let player_controller = PlayerController::default();

        App {
            render_target,
            view,
            camera_position_buffer,
            camera_view_proj_buffer,
            pipeline,
            camera_bind_group,
            storage_bind_group,
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
            59.0_f32.to_radians(),
            view.texture().width() as f32 / view.texture().height() as f32,
            0.001,
            1000.0,
        );

        queue.write_buffer(
            &self.camera_view_proj_buffer,
            0,
            bytemuck::cast_slice(&[view_mat * proj_mat]),
        );

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
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

            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, Some(&self.storage_bind_group), &[]);
            rpass.set_bind_group(1, Some(&self.camera_bind_group), &[]);
            rpass.draw(0..12, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }

    pub fn update_look_position(&mut self, delta: (f64, f64)) {
        self.player_controller.rotate(delta);
    }
}

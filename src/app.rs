use std::borrow::Cow;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use wgpu::StoreOp;
use winit::event::WindowEvent;
use winit::keyboard::{Key, SmolStr};

use crate::player_controller::PlayerController;
use crate::world::voxels;
use crate::world::voxels::get_palette;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_inverse: Mat4,
    proj_inverse: Mat4,
    palette: [Vec4; 256],
}

pub struct App {
    compute_target: wgpu::Texture,
    #[expect(dead_code)]
    compute_view: wgpu::TextureView,
    #[expect(dead_code)]
    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    compute_pipeline: wgpu::ComputePipeline,
    compute_bind_group: wgpu::BindGroup,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group: wgpu::BindGroup,
    pub player_controller: PlayerController,
    palette: [Vec4; 256],
    // world: Chunks,
    last_frame_update: Instant,
    delta_time: Duration,
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
        let compute_target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rt_target"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
        });

        let compute_view = compute_target.create_view(&wgpu::TextureViewDescriptor {
            label: None,
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            dimension: Some(wgpu::TextureViewDimension::D2),
            usage: None,
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rt_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let voxel_data = voxels::open_file("assets/models/nuke.vox");
        // let world = world_from_model(&voxel_data);
        let palette = get_palette(&voxel_data);

        let uniforms = {
            let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 2.5), Vec3::ZERO, Vec3::Y);
            let proj = Mat4::perspective_rh(
                59.0_f32.to_radians(),
                config.width as f32 / config.height as f32,
                0.001,
                1000.0,
            );

            Uniforms {
                view_inverse: view.inverse(),
                proj_inverse: proj.inverse(),
                palette,
            }
        };

        let brickmap = [0];

        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let storage_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Storage Buffer"),
            contents: &brickmap,
            usage: wgpu::BufferUsages::STORAGE,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compute_tree64"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/compute_tree64.wgsl"
            ))),
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/blit.wgsl"
            ))),
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("compute"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let compute_bind_group_layout = compute_pipeline.get_bind_group_layout(0);

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&compute_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: storage_buf.as_entire_binding(),
                },
            ],
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

        let blit_bind_group_layout = blit_pipeline.get_bind_group_layout(0);

        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&compute_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let player_controller = PlayerController::default();

        // for x in 0..chunks::WORLD_WIDTH {
        //     for y in 0..chunks::WORLD_HEIGHT {
        //         for z in 0..chunks::WORLD_DEPTH {
        //             let i = (x * chunks::WORLD_WIDTH + y * chunks::WORLD_HEIGHT + z) as usize;
        //             let transform: [f32; 12] = [
        //                 chunks::CHUNK_WIDTH as f32,
        //                 0.0,
        //                 0.0,
        //                 (chunks::WORLD_WIDTH * x) as f32,
        //                 0.0,
        //                 chunks::CHUNK_WIDTH as f32,
        //                 0.0,
        //                 (chunks::WORLD_HEIGHT * y) as f32,
        //                 0.0,
        //                 0.0,
        //                 chunks::CHUNK_WIDTH as f32,
        //                 (chunks::WORLD_DEPTH * z) as f32,
        //             ];
        //         }
        //     }
        // }

        App {
            compute_target,
            compute_view,
            sampler,
            uniform_buf,
            compute_pipeline,
            compute_bind_group,
            blit_pipeline,
            blit_bind_group,
            palette,
            player_controller,
            // world,
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

        let uniforms = {
            let view_mat = self.player_controller.view();
            let proj_mat = Mat4::perspective_rh(
                59.0_f32.to_radians(),
                view.texture().width() as f32 / view.texture().height() as f32,
                0.001,
                1000.0,
            );

            Uniforms {
                view_inverse: view_mat.inverse(),
                proj_inverse: proj_mat.inverse(),
                palette: self.palette,
            }
        };

        queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(&[uniforms]));

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.compute_pipeline);
            cpass.set_bind_group(0, Some(&self.compute_bind_group), &[]);
            cpass.dispatch_workgroups(
                self.compute_target.width() / 8,
                self.compute_target.height() / 8,
                1,
            );
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
            rpass.set_bind_group(0, Some(&self.blit_bind_group), &[]);
            rpass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }

    pub fn update_look_position(&mut self, delta: (f64, f64)) {
        self.player_controller.rotate(delta);
    }
}

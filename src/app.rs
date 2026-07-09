use std::borrow::Cow;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use wgpu::{Extent3d, StoreOp, TextureDescriptor, TextureFormat, TextureUsages};
use winit::event::WindowEvent;
use winit::keyboard::{Key, SmolStr};

use crate::player_controller::PlayerController;
use crate::tree64_renderer::GpuTree64Buffers;
use crate::world::World;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniforms {
    pos: [f32; 4],
    view_inv: [[f32; 4]; 4],
    proj_inv: [[f32; 4]; 4],
}

pub struct App {
    // Compute pipeline
    compute_pipeline: wgpu::ComputePipeline,

    // Single tree bind group
    tree_bind_group: Option<wgpu::BindGroup>,

    // Tree GPU buffers (owned here, not by a chunk manager)
    tree_buffers: Option<GpuTree64Buffers>,

    // Camera
    camera_buffer: wgpu::Buffer,
    pub player_controller: PlayerController,

    // Blit pass
    blit_pipeline: wgpu::RenderPipeline,
    blit_view_bind_group: wgpu::BindGroup,

    // Timing
    last_frame_update: Instant,
    delta_time: Duration,

    // Surface size for projection
    surface_width: u32,
    surface_height: u32,

    // RT output texture (recreated on resize)
    rt_texture: wgpu::Texture,

    // Layouts (reused for bind-group recreation on resize)
    tree_bind_group_layout: wgpu::BindGroupLayout,
    blit_bind_group_layout: wgpu::BindGroupLayout,

    // Palette buffer (world-level color lookup)
    palette_buffer: wgpu::Buffer,
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
        let format = TextureFormat::Rgba8Unorm;

        let rt_texture = device.create_texture(&TextureDescriptor {
            label: Some("rt_output"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[format],
        });

        let rt_view = rt_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("rt_view"),
            format: Some(format),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });

        // Load world from a hardcoded path for now.
        let world_path = std::env::current_dir()
            .unwrap_or_default()
            .join("assets")
            .join("castle.world");

        let world = if world_path.exists() {
            World::load(&world_path).expect("failed to load world file")
        } else {
            log::warn!(
                "World file not found at {:?}, using empty world. \
                 Run `cargo run --bin bake` first.",
                world_path
            );
            World {
                tree: None,
                palette: [[0u8; 4]; 256],
            }
        };

        let palette_buffer = crate::tree64_renderer::create_palette_buffer(device, &world.palette);

        let aspect = width as f32 / height as f32;
        let mut player_controller = PlayerController::default();
        let camera_uniforms = CameraUniforms {
            pos: [
                player_controller.translation.x,
                player_controller.translation.y,
                player_controller.translation.z,
                1.0,
            ],
            view_inv: player_controller.view().inverse().to_cols_array_2d(),
            proj_inv: glam::camera::rh::proj::vulkan::perspective(
                std::f32::consts::FRAC_PI_4,
                aspect,
                0.1,
                10000.0,
            )
            .inverse()
            .to_cols_array_2d(),
        };

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_uniforms"),
            contents: bytemuck::bytes_of(&camera_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tree64_raycast"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/tree64_compiled.wgsl"
            ))),
        });

        let tree_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("tree64_bind_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(32),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                (256 * 4 * std::mem::size_of::<f32>()) as u64,
                            ),
                        },
                        count: None,
                    },
                ],
            });

        let (tree_buffers, tree_bind_group) = if let Some(ref tree) = world.tree {
            let buffers = tree.create_buffers(device);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tree_bind_group"),
                layout: &tree_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&rt_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buffers.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: camera_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: buffers.nodes.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: buffers.leaf_data.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: palette_buffer.as_entire_binding(),
                    },
                ],
            });
            (Some(buffers), Some(bind_group))
        } else {
            (None, None)
        };

        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tree64_pipeline_layout"),
                bind_group_layouts: &[Some(&tree_bind_group_layout)],
                immediate_size: 0,
            });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("tree64_compute_pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "../assets/shaders/blit.wgsl"
            ))),
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit_pipeline"),
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

        let blit_view_bind_group_layout = blit_pipeline.get_bind_group_layout(0);
        let blit_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit_view"),
            layout: &blit_view_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            }],
        });

        App {
            compute_pipeline,
            tree_bind_group,
            tree_buffers,
            camera_buffer,
            player_controller,
            blit_pipeline,
            blit_view_bind_group,
            last_frame_update: Instant::now(),
            delta_time: Duration::default(),
            surface_width: width,
            surface_height: height,
            rt_texture,
            tree_bind_group_layout,
            blit_bind_group_layout: blit_view_bind_group_layout,
            palette_buffer,
        }
    }

    fn update_delta_time(&mut self) {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame_update);
        self.last_frame_update = now;
        self.delta_time = delta;
    }

    pub fn update(&mut self, _event: WindowEvent) {}

    fn recreate_render_target(&mut self, device: &wgpu::Device) {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let width = self.surface_width;
        let height = self.surface_height;

        self.rt_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rt_output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[format],
        });

        let rt_view = self.rt_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("rt_view"),
            format: Some(format),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });

        // Rebuild the single tree bind group with the new render target
        if let Some(ref buffers) = self.tree_buffers {
            self.tree_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tree_bind_group"),
                layout: &self.tree_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&rt_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buffers.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.camera_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: buffers.nodes.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: buffers.leaf_data.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: self.palette_buffer.as_entire_binding(),
                    },
                ],
            }));
        }

        self.blit_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit_view"),
            layout: &self.blit_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&rt_view),
            }],
        });
    }

    pub fn resize(
        &mut self,
        config: &wgpu::SurfaceConfiguration,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) {
        self.surface_width = config.width;
        self.surface_height = config.height;
        self.recreate_render_target(device);
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
        let camera_uniforms = CameraUniforms {
            pos: [
                self.player_controller.translation.x,
                self.player_controller.translation.y,
                self.player_controller.translation.z,
                1.0,
            ],
            view_inv: self.player_controller.view().inverse().to_cols_array_2d(),
            proj_inv: glam::camera::rh::proj::vulkan::perspective(
                70f32.to_radians(),
                aspect,
                0.1,
                10000.0,
            )
            .inverse()
            .to_cols_array_2d(),
        };

        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniforms));

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        {
            let workgroup_x = self.surface_width.div_ceil(8);
            let workgroup_y = self.surface_height.div_ceil(8);

            if let Some(ref bind_group) = self.tree_bind_group {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("tree64_compute_pass"),
                    timestamp_writes: None,
                });
                cpass.set_pipeline(&self.compute_pipeline);
                cpass.set_bind_group(0, bind_group, &[]);
                cpass.dispatch_workgroups(workgroup_x, workgroup_y, 1);
            }
        }

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
            rpass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }

    pub fn update_look_position(&mut self, delta: (f64, f64)) {
        self.player_controller.rotate(delta);
    }
}

use winit::dpi::PhysicalSize;

use crate::texture;

pub struct ComputePassLoader {
    pub texture_loader: texture::Texture,
    compute_pipeline: wgpu::ComputePipeline,

    texture_bind_group: wgpu::BindGroup,
    camera_bind_group: wgpu::BindGroup,
}

impl ComputePassLoader {
    pub fn new(
        size: PhysicalSize<u32>,
        device: &wgpu::Device,
        camera_buffer: &wgpu::Buffer,
    ) -> Self {
        let texture_loader =
            texture::Texture::new(size, device, Some("RayTracingCompute::output")).unwrap();

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("RayTracingCompute::texture_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: texture_loader.texture.format(),
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                }],
            });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("RayTracingCompute::camera_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RayTracingCompute::pipeline_layout"),
            bind_group_layouts: &[&texture_bind_group_layout, &camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let module = device.create_shader_module(wgpu::include_wgsl!("../assets/compute.wgsl"));

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RayTracingCompute::pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("compute_ray_tracing"),
            compilation_options: Default::default(),
            cache: None,
        });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RayTracingCompute::texture_bind_group"),
            layout: &texture_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_loader.view),
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RayTracingCompute::camera_bind_group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        Self {
            texture_loader,
            compute_pipeline,
            texture_bind_group,
            camera_bind_group,
        }
    }

    pub fn render(&self, size: PhysicalSize<u32>, device: &wgpu::Device, queue: &wgpu::Queue) {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RayTracingCompute::compute_pass"),
            ..Default::default()
        });

        pass.set_pipeline(&self.compute_pipeline);
        pass.set_bind_group(0, &self.texture_bind_group, &[]);
        pass.set_bind_group(1, &self.camera_bind_group, &[]);
        pass.dispatch_workgroups(size.width / 16, size.height / 16, 1);

        drop(pass);

        queue.submit([encoder.finish()]);
    }
}

use wgpu::CommandEncoder;
use winit::dpi::PhysicalSize;

use crate::texture;

pub struct ComputePassLoader {
    pub texture_loader: texture::Texture,
    compute_pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
}

impl ComputePassLoader {
    pub fn new(
        size: PhysicalSize<u32>,
        device: &wgpu::Device,
        camera_buffer: &wgpu::Buffer,
        tlas: &wgpu::Tlas,
    ) -> Self {
        let texture_loader =
            texture::Texture::new(size, device, Some("RayTracingCompute::output")).unwrap();

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RayTracingCompute::texture_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: texture_loader.texture.format(),
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
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure {
                        vertex_return: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RayTracingCompute::pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RayTracingCompute::texture_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_loader.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
            ],
        });

        Self {
            texture_loader,
            compute_pipeline,
            bind_group,
        }
    }

    pub fn render(&self, size: PhysicalSize<u32>, encoder: &mut CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RayTracingCompute::compute_pass"),
            ..Default::default()
        });

        pass.set_pipeline(&self.compute_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch_workgroups(size.width / 16, size.height / 16, 1);
    }
}

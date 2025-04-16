use winit::dpi::PhysicalSize;

use crate::texture;

pub struct ComputePassLoader {
    pub texture_loader: texture::Texture,
    bind_group_layout: wgpu::BindGroupLayout,
    compute_pipeline: wgpu::ComputePipeline,
}

impl ComputePassLoader {
    pub fn new(size: PhysicalSize<u32>, device: &wgpu::Device) -> Self {
        let texture_loader =
            texture::Texture::new(size, device, Some("RayTracingCompute::output")).unwrap();

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RayTracingCompute::layout"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let module = device.create_shader_module(wgpu::include_wgsl!("../assets/compute.wgsl"));

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RayTracingCompute"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("compute_ray_tracing"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            texture_loader,
            bind_group_layout,
            compute_pipeline,
        }
    }

    pub fn render(&self, size: PhysicalSize<u32>, device: &wgpu::Device, queue: &wgpu::Queue) {
        let label = Some("RayTracinCompute::pass");

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label,
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&self.texture_loader.view),
            }],
        });

        let mut encoder = device.create_command_encoder(&Default::default());
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label,
            ..Default::default()
        });

        pass.set_pipeline(&self.compute_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(size.width / 16, size.height / 16, 1);

        drop(pass);

        queue.submit([encoder.finish()]);
    }
}

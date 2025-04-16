pub struct AccelerationStructureBuilder {
    blas: wgpu::Blas,
    tlas: wgpu::TlasPackage,
}

impl AccelerationStructureBuilder {
    pub fn new(device: &wgpu::Device) -> Self {
        let descriptor = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: 3 * 2 * 6,
            index_count: None,
            index_format: None,
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };

        let blas = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some("Cube geometry"),
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![descriptor],
            },
        );

        let tlas_inner = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("Cube instances"),
            max_instances: 2u32.pow(24),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::PreferUpdate,
        });

        let tlas = wgpu::TlasPackage::new(tlas_inner);

        Self { blas, tlas }
    }
}

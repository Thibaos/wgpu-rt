use wgpu::{
    AccelerationStructureFlags, AccelerationStructureUpdateMode, Blas, BlasGeometrySizeDescriptors,
    BlasTriangleGeometrySizeDescriptor, CreateBlasDescriptor, CreateTlasDescriptor, Device,
    TlasPackage, VertexFormat, wgt::AccelerationStructureGeometryFlags,
};

pub struct AccelerationStructureBuilder {
    blas: Blas,
    tlas: TlasPackage,
}

impl AccelerationStructureBuilder {
    pub fn new(device: &Device) -> Self {
        let descriptor = BlasTriangleGeometrySizeDescriptor {
            vertex_format: VertexFormat::Float32x3,
            vertex_count: 3 * 2 * 6,
            index_count: None,
            index_format: None,
            flags: AccelerationStructureGeometryFlags::OPAQUE,
        };

        let blas = device.create_blas(
            &CreateBlasDescriptor {
                label: Some("Cube geometry"),
                flags: AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: AccelerationStructureUpdateMode::Build,
            },
            BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![descriptor],
            },
        );

        let tlas_inner = device.create_tlas(&CreateTlasDescriptor {
            label: Some("Cube instances"),
            max_instances: 2u32.pow(24),
            flags: AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: AccelerationStructureUpdateMode::PreferUpdate,
        });

        let tlas = TlasPackage::new(tlas_inner);

        Self { blas, tlas }
    }
}

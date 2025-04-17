const CUBE_VERTICES_LEN: usize = 3 * 2 * 6;

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex3D {
    position: [f32; 3],
}

pub fn triangles_from_box(position: glam::Vec3) -> [Vertex3D; CUBE_VERTICES_LEN] {
    let glam::Vec3 { x, y, z } = position;

    [
        // left face
        Vertex3D {
            position: [x - 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y - 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y + 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y + 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y + 0.5, z + 0.5],
        },
        // right face
        Vertex3D {
            position: [x + 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y - 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z + 0.5],
        },
        // bottom face
        Vertex3D {
            position: [x - 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y - 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y - 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y - 0.5, z + 0.5],
        },
        // top face
        Vertex3D {
            position: [x - 0.5, y + 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y + 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y + 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z + 0.5],
        },
        // back face
        Vertex3D {
            position: [x - 0.5, y - 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y - 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y - 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y + 0.5, z + 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z + 0.5],
        },
        // front face
        Vertex3D {
            position: [x - 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y - 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x - 0.5, y + 0.5, z - 0.5],
        },
        Vertex3D {
            position: [x + 0.5, y + 0.5, z - 0.5],
        },
    ]
}

pub struct AccelerationStructureBuilder {
    blas: wgpu::Blas,
    tlas: wgpu::TlasPackage,
}

impl AccelerationStructureBuilder {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let descriptor = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: CUBE_VERTICES_LEN as u32,
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

        let vertices = triangles_from_box(glam::Vec3::ZERO);

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("BLAS vertex buffer"),
            size: vertices.len() as u64 * 3,
            usage: wgpu::BufferUsages::BLAS_INPUT,
            mapped_at_creation: false,
        });

        let blas_size = &wgpu::wgt::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: CUBE_VERTICES_LEN as u32,
            index_format: None,
            index_count: None,
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };

        let geometries = wgpu::BlasTriangleGeometry {
            size: blas_size,
            vertex_buffer: &vertex_buffer,
            first_vertex: 0,
            vertex_stride: 3,
            index_buffer: None,
            first_index: None,
            transform_buffer: None,
            transform_buffer_offset: None,
        };

        let build_blas = wgpu::BlasBuildEntry {
            blas: &blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![geometries]),
        };

        let tlas_inner = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("Cube instances"),
            max_instances: 2u32.pow(24),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::PreferUpdate,
        });

        let mut tlas_instances = vec![];

        for x in 0..16 {
            for z in 0..16 {
                let translation = glam::vec3(x as f32, 0.0, z as f32);
                let transform = glam::Mat4::from_translation(translation);

                let col_0 = transform.col(0).to_array();
                let col_1 = transform.col(1).to_array();
                let col_2 = transform.col(2).to_array();

                let transform_array: [f32; 12] = unsafe {
                    let mut result = std::mem::MaybeUninit::uninit();
                    let dest = result.as_mut_ptr() as *mut f32;
                    std::ptr::copy_nonoverlapping(col_0.as_ptr(), dest, 4);
                    std::ptr::copy_nonoverlapping(col_1.as_ptr(), dest.add(4), 4);
                    std::ptr::copy_nonoverlapping(col_2.as_ptr(), dest.add(8), 4);
                    result.assume_init()
                };

                let instance = wgpu::TlasInstance::new(&blas, transform_array, 0, 0xff);

                tlas_instances.push(Some(instance));
            }
        }

        let tlas = wgpu::TlasPackage::new_with_instances(tlas_inner, tlas_instances);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        encoder.build_acceleration_structures([&build_blas], [&tlas]);

        queue.submit(std::iter::once(encoder.finish()));

        Self { blas, tlas }
    }
}

use std::mem;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const SIDE_COUNT: u32 = 32;
pub const AS_COUNT: u32 = SIDE_COUNT * SIDE_COUNT * SIDE_COUNT;

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

#[inline]
pub fn affine_to_rows(mat: &glam::Affine3A) -> [f32; 12] {
    let row_0 = mat.matrix3.row(0);
    let row_1 = mat.matrix3.row(1);
    let row_2 = mat.matrix3.row(2);
    let translation = mat.translation;
    [
        row_0.x,
        row_0.y,
        row_0.z,
        translation.x,
        row_1.x,
        row_1.y,
        row_1.z,
        translation.y,
        row_2.x,
        row_2.y,
        row_2.z,
        translation.z,
    ]
}

pub struct AccelerationStructureLoader {
    blas: wgpu::Blas,
    pub tlas: wgpu::Tlas,
}

impl AccelerationStructureLoader {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let (vertex_data, index_data) = create_vertices();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::BLAS_INPUT,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&index_data),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::BLAS_INPUT,
        });

        let descriptor = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: vertex_data.len() as u32,
            index_count: Some(index_data.len() as u32),
            index_format: Some(wgpu::IndexFormat::Uint16),
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };

        let blas = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: None,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![descriptor.clone()],
            },
        );

        let geometries = wgpu::BlasTriangleGeometry {
            size: &descriptor,
            vertex_buffer: &vertex_buffer,
            first_vertex: 0,
            vertex_stride: mem::size_of::<Vertex>() as u64,
            index_buffer: Some(&index_buffer),
            first_index: Some(0),
            transform_buffer: None,
            transform_buffer_offset: None,
        };

        let build_blas = wgpu::BlasBuildEntry {
            blas: &blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![geometries]),
        };

        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("Cube instances"),
            max_instances: SIDE_COUNT * SIDE_COUNT * SIDE_COUNT,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });

        for x in 0..SIDE_COUNT {
            for y in 0..SIDE_COUNT {
                for z in 0..SIDE_COUNT {
                    let translation = glam::vec3(x as f32, y as f32, z as f32 + 30.0);
                    let affine = glam::Affine3A::from_scale_rotation_translation(
                        glam::Vec3::splat(0.5),
                        glam::Quat::IDENTITY,
                        translation,
                    );

                    let instance = wgpu::TlasInstance::new(&blas, affine_to_rows(&affine), 0, 0xff);

                    tlas[(x + y * SIDE_COUNT + z * SIDE_COUNT * SIDE_COUNT) as usize] =
                        Some(instance);
                }
            }
        }

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        encoder.build_acceleration_structures([&build_blas], [&tlas]);

        queue.submit(std::iter::once(encoder.finish()));

        Self { blas, tlas }
    }
}

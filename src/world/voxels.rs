use crate::world::{Vertex, vertex};

pub fn open_file(path: &str) -> dot_vox::DotVoxData {
    let vox_data = dot_vox::load(path).unwrap();

    #[cfg(debug_assertions)]
    assert!(vox_data.palette.len() == 256);

    vox_data
}

pub fn triangles_from_box(position: glam::Vec3) -> (Vec<Vertex>, Vec<u16>) {
    let glam::Vec3 { x, y, z } = position;

    let vertices = [
        // left face
        vertex([x - 0.5, y - 0.5, z - 0.5]),
        vertex([x - 0.5, y - 0.5, z + 0.5]),
        vertex([x - 0.5, y + 0.5, z + 0.5]),
        vertex([x - 0.5, y - 0.5, z - 0.5]),
        vertex([x - 0.5, y + 0.5, z - 0.5]),
        vertex([x - 0.5, y + 0.5, z + 0.5]),
        // right face
        vertex([x + 0.5, y - 0.5, z - 0.5]),
        vertex([x + 0.5, y - 0.5, z + 0.5]),
        vertex([x + 0.5, y + 0.5, z + 0.5]),
        vertex([x + 0.5, y - 0.5, z - 0.5]),
        vertex([x + 0.5, y + 0.5, z - 0.5]),
        vertex([x + 0.5, y + 0.5, z + 0.5]),
        // bottom face
        vertex([x - 0.5, y - 0.5, z - 0.5]),
        vertex([x + 0.5, y - 0.5, z - 0.5]),
        vertex([x + 0.5, y - 0.5, z + 0.5]),
        vertex([x - 0.5, y - 0.5, z - 0.5]),
        vertex([x - 0.5, y - 0.5, z + 0.5]),
        vertex([x + 0.5, y - 0.5, z + 0.5]),
        // top face
        vertex([x - 0.5, y + 0.5, z - 0.5]),
        vertex([x + 0.5, y + 0.5, z - 0.5]),
        vertex([x + 0.5, y + 0.5, z + 0.5]),
        vertex([x - 0.5, y + 0.5, z - 0.5]),
        vertex([x - 0.5, y + 0.5, z + 0.5]),
        vertex([x + 0.5, y + 0.5, z + 0.5]),
        // back face
        vertex([x - 0.5, y - 0.5, z + 0.5]),
        vertex([x + 0.5, y - 0.5, z + 0.5]),
        vertex([x + 0.5, y + 0.5, z + 0.5]),
        vertex([x - 0.5, y - 0.5, z + 0.5]),
        vertex([x - 0.5, y + 0.5, z + 0.5]),
        vertex([x + 0.5, y + 0.5, z + 0.5]),
        // front face
        vertex([x - 0.5, y - 0.5, z - 0.5]),
        vertex([x + 0.5, y - 0.5, z - 0.5]),
        vertex([x + 0.5, y + 0.5, z - 0.5]),
        vertex([x - 0.5, y - 0.5, z - 0.5]),
        vertex([x - 0.5, y + 0.5, z - 0.5]),
        vertex([x + 0.5, y + 0.5, z - 0.5]),
    ];

    let indices: &[u16] = &[
        0, 1, 2, 2, 3, 0, // top
        4, 5, 6, 6, 7, 4, // bottom
        8, 9, 10, 10, 11, 8, // right
        12, 13, 14, 14, 15, 12, // left
        16, 17, 18, 18, 19, 16, // front
        20, 21, 22, 22, 23, 20, // back
    ];

    (vertices.to_vec(), indices.to_vec())
}

pub fn get_palette(data: &dot_vox::DotVoxData) -> [glam::Vec4; 256] {
    let mut array = [glam::Vec4::ZERO; 256];
    for (i, value) in array.iter_mut().enumerate() {
        let color = data.palette.get(i).unwrap();
        *value = glam::Vec4::new(
            f32::from(color.r) / 255.0,
            f32::from(color.g) / 255.0,
            f32::from(color.b) / 255.0,
            f32::from(color.a) / 255.0,
        )
    }

    array
}

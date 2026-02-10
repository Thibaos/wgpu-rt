#![allow(unused)]
use dot_vox::DotVoxData;
use glam::{IVec3, UVec3, Vec4, Vec4Swizzles, ivec3};
use rand::Rng;

use crate::{
    app::App,
    world::{HostVoxel, chunks::Chunks, loader::SceneGraphTraverser},
};

pub fn random_world_gen() -> Chunks {
    let mut chunks = Chunks::empty();

    let mut rng = rand::rng();

    for _ in 0..App::MAX_INSTANCE_COUNT {
        let position = ivec3(
            rng.random_range(0..64),
            rng.random_range(0..64),
            rng.random_range(0..64),
        );

        chunks.insert_voxel(
            position,
            HostVoxel {
                scale: 1.0,
                material_index: 1,
            },
        );
    }

    chunks
}

pub fn world_from_model(voxel_data: &DotVoxData) -> Chunks {
    let mut chunks = Chunks::empty();

    let mut loader = SceneGraphTraverser {
        chunks: &mut chunks,
        scene: voxel_data,
        models: vec![],
    };

    loader.traverse();

    println!("traverse done");

    let model_count = loader.models.len();

    for (i, (translation, rotation, size, voxels)) in loader.models.iter().enumerate() {
        println!("model {}/{}", i, model_count);
        let transform = SceneGraphTraverser::to_transform(*translation, *rotation, *size);

        for voxel in voxels {
            let local_position =
                UVec3::new(voxel.x as u32, voxel.z as u32, size.y - voxel.y as u32 - 1).as_ivec3();

            let position = (transform
                * Vec4::new(
                    local_position.x as f32,
                    local_position.y as f32,
                    local_position.z as f32,
                    1.0,
                ))
            .xyz()
            .as_ivec3();

            let p = IVec3::new(position.x, -position.y, position.z);

            Chunks::insert_voxel(
                &mut chunks,
                p,
                HostVoxel {
                    scale: 1.0,
                    material_index: voxel.i.into(),
                },
            );
        }
    }

    dbg!(chunks.vox_count());

    chunks
}

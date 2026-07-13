use std::collections::{HashMap, HashSet};

use dot_vox::{DotVoxData, Rotation, SceneNode, Voxel};
use glam::{IVec3, Mat4, UVec3, Vec3A};
use rayon::prelude::*;

use crate::world::{VoxelWorldData, World};

struct ModelInstance<'a> {
    transform: Mat4,
    voxels: &'a [Voxel],
}

pub struct SceneGraphLoader;

impl SceneGraphLoader {
    pub fn load(vox_data: DotVoxData, palette: [[u8; 4]; 256]) -> World {
        let instances = Self::collect_instances(&vox_data);
        let voxels = Self::collect_all_voxels(&instances);

        World { voxels, palette }
    }

    fn collect_instances(vox_data: &DotVoxData) -> Vec<ModelInstance<'_>> {
        let mut instances = Vec::new();

        if vox_data.scenes.is_empty() {
            for model in &vox_data.models {
                if model.voxels.is_empty() {
                    continue;
                }
                let transform = Self::to_transform(
                    IVec3::ZERO,
                    Rotation::IDENTITY,
                    UVec3::new(model.size.x, model.size.y, model.size.z),
                );
                instances.push(ModelInstance {
                    transform,
                    voxels: &model.voxels,
                });
            }
        } else {
            log::info!(
                "Traversing scene graph: {} scene nodes, {} models …",
                vox_data.scenes.len(),
                vox_data.models.len(),
            );
            let mut node_count: u32 = 0;
            let mut visited: HashSet<u32> = HashSet::with_capacity(vox_data.scenes.len());
            Self::traverse_recursive(
                vox_data,
                0,
                0,
                IVec3::ZERO,
                Rotation::IDENTITY,
                &mut instances,
                &mut node_count,
                &mut visited,
            );
            log::info!(
                "Visited {} scene nodes, collected {} model instances",
                node_count,
                instances.len()
            );
        }

        instances
    }

    #[allow(clippy::too_many_arguments)]
    fn traverse_recursive<'a>(
        vox_data: &'a DotVoxData,
        node_index: u32,
        depth: u32,
        translation: IVec3,
        rotation: Rotation,
        instances: &mut Vec<ModelInstance<'a>>,
        node_count: &mut u32,
        visited: &mut HashSet<u32>,
    ) {
        *node_count += 1;

        if !visited.insert(node_index) {
            log::warn!(
                "Scene graph cycle detected at node {} (depth {}), skipping re-visit",
                node_index,
                depth,
            );
            return;
        }

        let node = &vox_data.scenes[node_index as usize];
        match node {
            SceneNode::Transform {
                frames,
                child,
                layer_id,
                ..
            } => {
                if frames.len() != 1 {
                    unimplemented!("Multiple frames in transform node");
                }
                let frame = &frames[0];
                let this_translation = frame
                    .position()
                    .map(|p| IVec3::new(p.x, p.y, p.z))
                    .unwrap_or(IVec3::ZERO);
                let this_rotation = frame.orientation().unwrap_or(Rotation::IDENTITY);

                log::debug!(
                    "{:indent$}Transform #{} — t={:?}, layer={}, child={}",
                    "",
                    node_index,
                    this_translation,
                    layer_id,
                    child,
                    indent = depth as usize * 2,
                );

                let new_translation = {
                    let cols = rotation.to_cols_array_2d();
                    let t = this_translation;
                    translation
                        + IVec3::new(
                            cols[0][0] as i32 * t.x
                                + cols[1][0] as i32 * t.y
                                + cols[2][0] as i32 * t.z,
                            cols[0][1] as i32 * t.x
                                + cols[1][1] as i32 * t.y
                                + cols[2][1] as i32 * t.z,
                            cols[0][2] as i32 * t.x
                                + cols[1][2] as i32 * t.y
                                + cols[2][2] as i32 * t.z,
                        )
                };
                let new_rotation = rotation * this_rotation;

                Self::traverse_recursive(
                    vox_data,
                    *child,
                    depth + 1,
                    new_translation,
                    new_rotation,
                    instances,
                    node_count,
                    visited,
                );
            }
            SceneNode::Group { children, .. } => {
                log::debug!(
                    "{:indent$}Group #{} — {} children",
                    "",
                    node_index,
                    children.len(),
                    indent = depth as usize * 2,
                );
                for child in children {
                    Self::traverse_recursive(
                        vox_data,
                        *child,
                        depth + 1,
                        translation,
                        rotation,
                        instances,
                        node_count,
                        visited,
                    );
                }
            }
            SceneNode::Shape { models, .. } => {
                if models.len() != 1 {
                    unimplemented!("Multiple shape models in Shape node");
                }
                let shape_model = &models[0];
                let model = &vox_data.models[shape_model.model_id as usize];

                if model.voxels.is_empty() {
                    return;
                }

                let size = UVec3::new(model.size.x, model.size.y, model.size.z);
                let transform = Self::to_transform(translation, rotation, size);

                instances.push(ModelInstance {
                    transform,
                    voxels: &model.voxels,
                });
            }
        }
    }

    fn to_transform(translation: IVec3, rotation: Rotation, size: UVec3) -> Mat4 {
        let translation = translation.as_vec3a();

        let (quat, scale) = rotation.to_quat_scale();
        let quat = glam::Quat::from_array(quat);
        let scale = Vec3A::from_array(scale);

        let mut offset = Vec3A::new(
            if size.x.is_multiple_of(2) { 0.0 } else { 0.5 },
            if size.y.is_multiple_of(2) { 0.0 } else { 0.5 },
            if size.z.is_multiple_of(2) { 0.0 } else { 0.5 },
        );
        offset = quat.mul_vec3a(offset);

        let center = quat
            * Vec3A::new(
                size.x as f32 / 2.0,
                size.y as f32 / 2.0,
                size.z as f32 / 2.0,
            );

        Mat4::from_scale_rotation_translation(
            scale.into(),
            quat,
            (translation - center * scale + offset).into(),
        )
    }

    fn collect_all_voxels(instances: &[ModelInstance<'_>]) -> VoxelWorldData {
        let total_voxels: usize = instances.iter().map(|i| i.voxels.len()).sum();
        log::info!(
            "Collecting {} voxels across {} instances…",
            total_voxels,
            instances.len(),
        );

        let result = instances
            .par_iter()
            .fold(HashMap::new, |mut acc, instance| {
                let transform = &instance.transform;
                for voxel in instance.voxels {
                    let local_engine = Vec3A::new(voxel.x as f32, voxel.y as f32, voxel.z as f32);
                    let world_f = transform.transform_point3a(local_engine);
                    let world_x = world_f.x.round() as i16;
                    let world_y = world_f.y.round() as i16;
                    let world_z = world_f.z.round() as i16;
                    acc.insert((world_x, world_y, world_z), voxel.i);
                }
                acc
            })
            .reduce(HashMap::new, |mut a, b| {
                a.extend(b);
                a
            });

        let unique = result.len();
        log::info!(
            "Collected {} unique voxels ({} raw, {} overlap-collapsed)",
            unique,
            total_voxels,
            total_voxels - unique,
        );

        result
    }
}

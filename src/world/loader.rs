use std::collections::{HashMap, HashSet};

use dot_vox::{DotVoxData, Rotation, SceneNode, Voxel};
use glam::{IVec3, Mat4, UVec3, Vec3A};
use rayon::prelude::*;

use crate::utils::{f32_to_i16, f32_to_i32, i32_to_f32, u32_to_f32};
use crate::world::{VoxelWorldData, World, chunk::CHUNK_TEXTURE_SIZE};

struct ModelInstance<'a> {
    transform: Mat4,
    voxels: &'a [Voxel],
}

pub struct SceneGraphLoader;

impl SceneGraphLoader {
    pub fn load(vox_data: &DotVoxData, palette: &[[u8; 4]; 256]) -> World {
        let instances = Self::collect_instances(vox_data);
        let voxels = Self::collect_all_voxels(&instances);

        let (world_offset, voxels) = Self::center_world(voxels);

        World {
            voxels,
            palette: *palette,
            offset: world_offset,
        }
    }

    fn center_world(voxels: VoxelWorldData) -> ([i32; 3], VoxelWorldData) {
        let (w_x, w_y, w_z) = (
            i32::try_from(CHUNK_TEXTURE_SIZE.width.div_euclid(2)).unwrap_or_default(),
            i32::try_from(CHUNK_TEXTURE_SIZE.height.div_euclid(2)).unwrap_or_default(),
            i32::try_from(CHUNK_TEXTURE_SIZE.depth_or_array_layers.div_euclid(2))
                .unwrap_or_default(),
        );

        if voxels.is_empty() {
            return ([w_x, w_y, w_z], voxels);
        }

        let mut min = (i16::MAX, i16::MAX, i16::MAX);
        let mut max = (i16::MIN, i16::MIN, i16::MIN);
        for &(x, y, z) in voxels.keys() {
            min.0 = min.0.min(x);
            min.1 = min.1.min(y);
            min.2 = min.2.min(z);
            max.0 = max.0.max(x);
            max.1 = max.1.max(y);
            max.2 = max.2.max(z);
        }

        let mut cx = f32::from(min.0);
        cx += f32::from(max.0);
        cx *= 0.5;
        let mut cy = f32::from(min.1);
        cy += f32::from(max.1);
        cy *= 0.5;
        let mut cz = f32::from(min.2);
        cz += f32::from(max.2);
        cz *= 0.5;

        let mut ox = i32_to_f32(w_x);
        ox -= cx;
        let ox = f32_to_i32(ox.round());
        let mut oy = i32_to_f32(w_y);
        oy -= cy;
        let oy = f32_to_i32(oy.round());
        let mut oz = i32_to_f32(w_z);
        oz -= cz;
        let oz = f32_to_i32(oz.round());

        log::info!(
            "Scene bounds: ({}, {}, {}) → ({}, {}, {}), offset: ({}, {}, {})",
            min.0,
            min.1,
            min.2,
            max.0,
            max.1,
            max.2,
            ox,
            oy,
            oz,
        );

        ([ox, oy, oz], voxels)
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

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
        *node_count = node_count.saturating_add(1);

        if !visited.insert(node_index) {
            log::warn!(
                "Scene graph cycle detected at node {node_index} (depth {depth}), skipping re-visit",
            );
            return;
        }

        let node = vox_data
            .scenes
            .get(usize::try_from(node_index).unwrap_or_default())
            .unwrap_or_else(|| {
                crate::utils::fatal(&format!("Scene node {node_index} out of range"))
            });
        match node {
            SceneNode::Transform {
                frames,
                child,
                layer_id,
                ..
            } => {
                let Some(frame) = frames.first() else {
                    log::warn!("Transform node #{node_index} has no frames, skipping");
                    return;
                };
                if frames.len() > 1 {
                    log::warn!(
                        "Transform node #{node_index} has {} frames; using the first only",
                        frames.len()
                    );
                }
                let this_translation = frame
                    .position()
                    .map_or(IVec3::ZERO, |p| IVec3::new(p.x, p.y, p.z));
                let this_rotation = frame.orientation().unwrap_or(Rotation::IDENTITY);

                log::debug!(
                    "{:indent$}Transform #{} — t={:?}, layer={}, child={}",
                    "",
                    node_index,
                    this_translation,
                    layer_id,
                    child,
                    indent = usize::try_from(depth).unwrap_or_default().saturating_mul(2),
                );

                // Rotate the child translation by this node's rotation, exactly as
                // the original column-major matrix math did.
                let [[m00, m01, m02], [m10, m11, m12], [m20, m21, m22]] =
                    rotation.to_cols_array_2d();
                let t = this_translation;
                let rotated = IVec3::new(
                    f32_to_i32(m00)
                        .saturating_mul(t.x)
                        .saturating_add(f32_to_i32(m10).saturating_mul(t.y))
                        .saturating_add(f32_to_i32(m20).saturating_mul(t.z)),
                    f32_to_i32(m01)
                        .saturating_mul(t.x)
                        .saturating_add(f32_to_i32(m11).saturating_mul(t.y))
                        .saturating_add(f32_to_i32(m21).saturating_mul(t.z)),
                    f32_to_i32(m02)
                        .saturating_mul(t.x)
                        .saturating_add(f32_to_i32(m12).saturating_mul(t.y))
                        .saturating_add(f32_to_i32(m22).saturating_mul(t.z)),
                );
                let new_translation = IVec3::new(
                    translation.x.saturating_add(rotated.x),
                    translation.y.saturating_add(rotated.y),
                    translation.z.saturating_add(rotated.z),
                );
                let new_rotation = std::ops::Mul::mul(rotation, this_rotation);

                Self::traverse_recursive(
                    vox_data,
                    *child,
                    depth.saturating_add(1),
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
                    indent = usize::try_from(depth).unwrap_or_default().saturating_mul(2),
                );
                for child in children {
                    Self::traverse_recursive(
                        vox_data,
                        *child,
                        depth.saturating_add(1),
                        translation,
                        rotation,
                        instances,
                        node_count,
                        visited,
                    );
                }
            }
            SceneNode::Shape { models, .. } => {
                if models.len() > 1 {
                    log::warn!(
                        "Shape node #{node_index} has {} models; instancing each",
                        models.len()
                    );
                }
                for shape_model in models {
                    let Some(model) = vox_data
                        .models
                        .get(usize::try_from(shape_model.model_id).unwrap_or_default())
                    else {
                        log::warn!("Shape node #{node_index} references missing model, skipping");
                        continue;
                    };

                    if model.voxels.is_empty() {
                        continue;
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

        let center = quat.mul_vec3a(Vec3A::new(
            u32_to_f32(size.x) / 2.0,
            u32_to_f32(size.y) / 2.0,
            u32_to_f32(size.z) / 2.0,
        ));

        let pos = Vec3A::new(
            center.x.mul_add(-scale.x, translation.x) + offset.x,
            center.y.mul_add(-scale.y, translation.y) + offset.y,
            center.z.mul_add(-scale.z, translation.z) + offset.z,
        );

        Mat4::from_scale_rotation_translation(scale.into(), quat, pos.into())
    }

    fn collect_all_voxels(instances: &[ModelInstance<'_>]) -> VoxelWorldData {
        let total_voxels: usize = instances.iter().map(|i| i.voxels.len()).sum();
        let material_zero_count: usize = instances
            .iter()
            .map(|i| i.voxels.iter().filter(|v| v.i == 0).count())
            .sum();
        log::info!(
            "Collecting {} voxels across {} instances ({} material-0 to filter)…",
            total_voxels,
            instances.len(),
            material_zero_count,
        );

        let result = instances
            .par_iter()
            .fold(HashMap::new, |mut acc, instance| {
                let transform = &instance.transform;
                for voxel in instance.voxels {
                    // Material 0 is the empty sentinel in .vox format and in our shader.
                    // Skip it rather than storing empty air voxels.
                    if voxel.i == 0 {
                        continue;
                    }
                    let local_engine =
                        Vec3A::new(f32::from(voxel.x), f32::from(voxel.y), f32::from(voxel.z));
                    let world_f = transform.transform_point3a(local_engine);
                    let world_x = f32_to_i16(world_f.x.round());
                    let world_y = f32_to_i16(world_f.y.round());
                    let world_z = f32_to_i16(world_f.z.round());
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
            total_voxels.saturating_sub(unique),
        );

        result
    }
}

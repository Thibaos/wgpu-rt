use std::collections::{HashMap, HashSet};
use std::time::Instant;

use dot_vox::{DotVoxData, Rotation, SceneNode, Voxel};
use glam::{IVec3, Mat4, UVec3, Vec3A};
use rayon::prelude::*;

use super::builder::build_gpu_tree;
use crate::formats::WorldFile;

// ---- scene graph traversal ----

struct ModelInstance<'a> {
    transform: Mat4,
    voxels: &'a [Voxel],
}

pub struct SceneGraphLoader;

impl SceneGraphLoader {
    pub fn load(vox_data: DotVoxData, palette: [[u8; 4]; 256]) -> WorldFile {
        let t_total = Instant::now();

        let instances = Self::collect_instances(&vox_data);
        let all_voxels = Self::collect_all_voxels(&instances);
        // Release the scene graph borrows before building the tree.
        drop(instances);
        drop(vox_data);
        let world = Self::build_world_file(all_voxels, palette);

        log::info!("Total bake time: {:.2}s", t_total.elapsed().as_secs_f32());
        world
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

    fn collect_all_voxels(instances: &[ModelInstance<'_>]) -> HashMap<(i32, i32, i32), u8> {
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
                    let world_x = world_f.x.round() as i32;
                    let world_y = world_f.y.round() as i32;
                    let world_z = world_f.z.round() as i32;
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

    // ---- tight-bounding-box tree construction ----

    /// Round `n` up to the next power-of-four (4, 16, 64, 256, 1024, 4096, …).
    fn round_up_pow4(n: u32) -> u32 {
        let mut s = n.max(4).next_power_of_two();
        if s.ilog2() % 2 == 1 {
            s *= 2;
        }
        s
    }

    fn build_world_file(
        voxels: HashMap<(i32, i32, i32), u8>,
        palette: [[u8; 4]; 256],
    ) -> WorldFile {
        let mut world_file = WorldFile::new();
        world_file.palette = palette;

        if voxels.is_empty() {
            log::warn!("No voxels in world — output will be empty.");
            return world_file;
        }

        // --- 1. compute tight AABB (signed world-space) ---
        let t_aabb = Instant::now();
        let mut bb_min = IVec3::splat(i32::MAX);
        let mut bb_max = IVec3::splat(i32::MIN);
        for &(x, y, z) in voxels.keys() {
            bb_min.x = bb_min.x.min(x);
            bb_min.y = bb_min.y.min(y);
            bb_min.z = bb_min.z.min(z);
            bb_max.x = bb_max.x.max(x);
            bb_max.y = bb_max.y.max(y);
            bb_max.z = bb_max.z.max(z);
        }
        let aabb_min = bb_min;
        let aabb_size = (bb_max - bb_min + IVec3::ONE).as_uvec3();

        // --- 2. round up to a power-of-four cube ---
        let max_dim = aabb_size.x.max(aabb_size.y).max(aabb_size.z);
        let tree_dim = Self::round_up_pow4(max_dim);

        log::info!(
            "Scene AABB: min=({},{},{}) size=({},{},{}) → tree {tree_dim}³ ({:.2}s)",
            bb_min.x,
            bb_min.y,
            bb_min.z,
            aabb_size.x,
            aabb_size.y,
            aabb_size.z,
            t_aabb.elapsed().as_secs_f32(),
        );

        // --- 3. convert to local coordinates and build GPU tree directly ---
        let t_tree = Instant::now();
        let local_voxels = voxels.into_iter().map(|((x, y, z), value)| {
            let local = [
                u32::try_from(x - aabb_min.x).expect("x coordinate outside AABB"),
                u32::try_from(y - aabb_min.y).expect("y coordinate outside AABB"),
                u32::try_from(z - aabb_min.z).expect("z coordinate outside AABB"),
            ];
            (local, value)
        });

        let gpu_tree = build_gpu_tree(local_voxels, tree_dim, aabb_min.to_array())
            .expect("failed to build occupancy-driven GPU tree");

        log::info!(
            "GPU tree built: {} nodes, {} bytes leaf data, tree_scale={} ({:.2}s)",
            gpu_tree.nodes.len(),
            gpu_tree.leaf_data.len(),
            gpu_tree.tree_scale,
            t_tree.elapsed().as_secs_f32(),
        );

        log::info!(
            "GPU tree: root_offset=({},{},{})",
            gpu_tree.root_offset[0],
            gpu_tree.root_offset[1],
            gpu_tree.root_offset[2],
        );

        world_file.tree = Some(gpu_tree);
        world_file
    }
}

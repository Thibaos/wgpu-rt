use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use dot_vox::{DotVoxData, Rotation, SceneNode, Voxel};
use glam::{IVec3, Mat4, UVec3, Vec3A, Vec3Swizzles};
use rayon::prelude::*;

use crate::formats::WorldFile;
use crate::formats::chunk::ChunkData;
use crate::tree64_renderer::GpuTree64;

const WORLD_X: u32 = crate::formats::CHUNK_COUNT_X * crate::formats::CHUNK_VOXEL_X;
const WORLD_Y: u32 = crate::formats::CHUNK_COUNT_Y * crate::formats::CHUNK_VOXEL_Y;
const WORLD_Z: u32 = crate::formats::CHUNK_COUNT_Z * crate::formats::CHUNK_VOXEL_Z;

const CHUNK_DIM_X: u32 = crate::formats::CHUNK_VOXEL_X;
const CHUNK_DIM_Z: u32 = crate::formats::CHUNK_VOXEL_Z;
const CHUNKS_X: u32 = crate::formats::CHUNK_COUNT_X;
const CHUNKS_Z: u32 = crate::formats::CHUNK_COUNT_Z;

struct ModelInstance<'a> {
    transform: Mat4,
    voxels: &'a [Voxel],
}

struct ChunkVoxels {
    /// Voxel data already shifted so the AABB min is at (0,0,0).
    voxels: HashMap<(u32, u32, u32), u8>,
    /// AABB size (max - min), at least 1 in each axis.
    dims: [u32; 3],
    /// The original minimum coordinate of the AABB within the chunk.
    /// Will be used as `root_offset` in the GPU tree.
    aabb_min: [i32; 3],
}

impl ChunkVoxels {
    /// Build a `ChunkVoxels` from a raw (unchanged) voxel map.
    /// Computes the AABB, shifts all coordinates to start at (0,0,0),
    /// and stores the AABB size and the original minimum.
    fn from_raw(raw: HashMap<(u32, u32, u32), u8>) -> Self {
        if raw.is_empty() {
            return Self {
                voxels: HashMap::new(),
                dims: [1, 1, 1],
                aabb_min: [0, 0, 0],
            };
        }

        let mut min = [u32::MAX; 3];
        let mut max = [0u32; 3];
        for &(x, y, z) in raw.keys() {
            min[0] = min[0].min(x);
            min[1] = min[1].min(y);
            min[2] = min[2].min(z);
            max[0] = max[0].max(x);
            max[1] = max[1].max(y);
            max[2] = max[2].max(z);
        }

        // Cap each dimension at 256 to keep tree64 scale ≤ 4⁴.
        // Voxels beyond this are clipped (same policy as X/Z bounds).
        const MAX_DIM: u32 = 256;
        let dims = [
            (max[0] - min[0] + 1).min(MAX_DIM),
            (max[1] - min[1] + 1).min(MAX_DIM),
            (max[2] - min[2] + 1).min(MAX_DIM),
        ];

        let total_before = raw.len();
        let voxels: HashMap<(u32, u32, u32), u8> = raw
            .into_iter()
            .filter_map(|((x, y, z), v)| {
                let sx = x - min[0];
                let sy = y - min[1];
                let sz = z - min[2];
                if sx >= dims[0] || sy >= dims[1] || sz >= dims[2] {
                    None
                } else {
                    Some(((sx, sy, sz), v))
                }
            })
            .collect();

        let clipped = total_before - voxels.len();
        if clipped > 0 {
            log::warn!(
                "Clipped {} voxels from AABB {}×{}×{} (capped at 256³)",
                clipped,
                max[0] - min[0] + 1,
                max[1] - min[1] + 1,
                max[2] - min[2] + 1,
            );
        }

        Self {
            voxels,
            dims,
            aabb_min: [min[0] as i32, min[1] as i32, min[2] as i32],
        }
    }
}

impl tree64::VoxelModel<u8> for &ChunkVoxels {
    fn dimensions(&self) -> [u32; 3] {
        self.dims
    }

    fn access(&self, coord: [usize; 3]) -> Option<u8> {
        self.voxels
            .get(&(coord[0] as u32, coord[1] as u32, coord[2] as u32))
            .copied()
    }
}

pub struct SceneGraphLoader;

impl SceneGraphLoader {
    pub fn load(vox_data: &DotVoxData, palette: [[u8; 4]; 256]) -> WorldFile {
        let t_total = Instant::now();

        let t0 = Instant::now();
        let instances = Self::collect_instances(vox_data);
        log::info!(
            "Phase A — traversal: {} instances in {:.2}s",
            instances.len(),
            t0.elapsed().as_secs_f32(),
        );

        let t0 = Instant::now();
        let chunk_buckets = Self::bucket_voxels(&instances);
        log::info!(
            "Phase B — bucketing: {} chunks in {:.2}s",
            chunk_buckets.len(),
            t0.elapsed().as_secs_f32(),
        );

        let t0 = Instant::now();
        let world = Self::build_world_file(chunk_buckets, palette);
        log::info!("Phase C — tree build: {:.2}s", t0.elapsed().as_secs_f32(),);

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
                "  Visited {} scene nodes, collected {} model instances",
                node_count,
                instances.len()
            );
        }

        instances
    }

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

        // Cycle guard — a well-formed .vox should never hit this.
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

                log::debug!(
                    "{:indent$}Shape #{} — model_id={}, size={}×{}×{}, {} voxels",
                    "",
                    node_index,
                    shape_model.model_id,
                    model.size.x,
                    model.size.y,
                    model.size.z,
                    model.voxels.len(),
                    indent = depth as usize * 2,
                );

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
        let mut translation = translation.as_vec3a().xzy();
        translation.z *= -1.0;

        let (quat, scale) = rotation.to_quat_scale();
        let quat = glam::Quat::from_array(quat);
        let quat = glam::Quat::from_xyzw(quat.x, quat.z, -quat.y, quat.w);
        let scale = Vec3A::from_array(scale).xzy();

        let mut offset = Vec3A::new(
            if size.x.is_multiple_of(2) { 0.0 } else { 0.5 },
            if size.z.is_multiple_of(2) { 0.0 } else { 0.5 },
            if size.y.is_multiple_of(2) { 0.0 } else { -0.5 },
        );
        offset = quat.mul_vec3a(offset);

        let center = quat * (size.xzy().as_vec3a() / 2.0);

        Mat4::from_scale_rotation_translation(
            scale.into(),
            quat,
            (translation - center * scale + offset).into(),
        )
    }

    fn bucket_voxels(instances: &[ModelInstance<'_>]) -> HashMap<usize, ChunkVoxels> {
        let total_voxels: usize = instances.iter().map(|i| i.voxels.len()).sum();
        log::info!(
            "Bucketing {} voxels across {} instances…",
            total_voxels,
            instances.len(),
        );

        let out_of_bounds = AtomicU64::new(0);
        let world_bounds = UVec3::new(WORLD_X, WORLD_Y, WORLD_Z);

        let merged: HashMap<usize, HashMap<(u32, u32, u32), u8>> = instances
            .par_iter()
            .fold(
                HashMap::new,
                |mut acc: HashMap<usize, HashMap<(u32, u32, u32), u8>>, instance| {
                    let transform = &instance.transform;
                    log::debug!("  {} voxels", instance.voxels.len());
                    for voxel in instance.voxels {
                        let local_engine =
                            Vec3A::new(voxel.x as f32, voxel.z as f32, voxel.y as f32);
                        let world_f = transform.transform_point3a(local_engine);

                        let world_x = world_f.x.round() as i32;
                        let world_y = world_f.y.round() as i32;
                        let world_z = world_f.z.round() as i32;

                        if world_x < 0
                            || world_y < 0
                            || world_z < 0
                            || world_x as u32 >= world_bounds.x
                            || world_y as u32 >= world_bounds.y
                            || world_z as u32 >= world_bounds.z
                        {
                            out_of_bounds.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }

                        let wx = world_x as u32;
                        let wy = world_y as u32;
                        let wz = world_z as u32;

                        let cx = wx / CHUNK_DIM_X;
                        let cz = wz / CHUNK_DIM_Z;
                        let chunk_index = (cx + cz * CHUNKS_X) as usize;

                        let local_x = wx % CHUNK_DIM_X;
                        let local_y = wy;
                        let local_z = wz % CHUNK_DIM_Z;

                        acc.entry(chunk_index)
                            .or_default()
                            .insert((local_x, local_y, local_z), voxel.i);
                    }
                    acc
                },
            )
            .reduce(HashMap::new, |mut a, b| {
                for (chunk_idx, voxels) in b {
                    a.entry(chunk_idx).or_default().extend(voxels);
                }
                a
            });

        let out_of_bounds_count = out_of_bounds.load(Ordering::Relaxed);
        if out_of_bounds_count > 0 {
            log::warn!(
                "{} voxels fell outside world bounds ({}×{}×{}) and were dropped",
                out_of_bounds_count,
                WORLD_X,
                WORLD_Y,
                WORLD_Z,
            );
        }

        merged
            .into_iter()
            .map(|(index, voxel_map)| (index, ChunkVoxels::from_raw(voxel_map)))
            .collect()
    }

    fn build_world_file(
        chunk_buckets: HashMap<usize, ChunkVoxels>,
        palette: [[u8; 4]; 256],
    ) -> WorldFile {
        let mut world_file = WorldFile::new();
        world_file.palette = palette;

        let total_chunks = (CHUNKS_X * CHUNKS_Z) as usize;

        log::info!(
            "Building trees for {} non-empty chunks (parallel) …",
            chunk_buckets.len(),
        );

        let chunk_entries: Vec<(usize, ChunkData)> = chunk_buckets
            .into_par_iter()
            .filter_map(|(chunk_index, chunk_voxels)| {
                if chunk_index >= total_chunks {
                    log::warn!("Chunk index {} out of range, skipping", chunk_index);
                    return None;
                }

                let aabb = chunk_voxels.dims;
                let aabb_min = chunk_voxels.aabb_min;

                log::info!(
                    "  Chunk [{}]: building tree (AABB {}×{}×{} @ ({},{},{}))…",
                    chunk_index,
                    aabb[0],
                    aabb[1],
                    aabb[2],
                    aabb_min[0],
                    aabb_min[1],
                    aabb_min[2],
                );

                let tree = tree64::Tree64::new(&chunk_voxels);

                if tree.nodes.is_empty()
                    || (tree.root_state().index == 0 && tree.nodes[0].pop_mask == 0)
                {
                    return None;
                }

                log::info!(
                    "  Chunk [{}]: done — {} nodes, {} bytes leaf",
                    chunk_index,
                    tree.nodes.len(),
                    tree.data.len(),
                );

                let mut gpu_tree = GpuTree64::from_tree64(&tree);
                gpu_tree.root_offset = aabb_min;
                Some((chunk_index, ChunkData::new(gpu_tree)))
            })
            .collect();

        let chunks_written = chunk_entries.len() as u32;
        for (index, data) in chunk_entries {
            world_file.set_chunk(index, data);
        }

        log::info!(
            "Total non-empty chunks: {} / {}",
            chunks_written,
            total_chunks,
        );

        world_file
    }
}

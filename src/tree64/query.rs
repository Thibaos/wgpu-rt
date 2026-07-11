use glam::Vec3;

use super::renderer::GpuTree64;

/// Result of an AABB collision query against a Tree64 world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CollisionResult {
    /// No occupied voxels overlap the query AABB.
    Clear,
    /// The AABB overlaps at least one occupied voxel. Each field is the
    /// distance the AABB must move along that axis direction to reach the
    /// nearest non-overlapping position.
    ///
    /// `penetration_x`  → slide along +X (right)
    /// `penetration_neg_x` → slide along −X (left)
    /// `penetration_y`  → slide along +Y (up)
    /// `penetration_neg_y` → slide along −Y (down)
    /// `penetration_z`  → slide along +Z (forward)
    /// `penetration_neg_z` → slide along −Z (backward)
    Blocked {
        penetration_x: f32,
        penetration_neg_x: f32,
        penetration_y: f32,
        penetration_neg_y: f32,
        penetration_z: f32,
        penetration_neg_z: f32,
    },
}

/// Perform hierarchical AABB-vs-Tree64 collision query.
///
/// `aabb_min` / `aabb_max` are in world-space (meters). Voxels outside the
/// tree bounds (determined by `tree.root_offset` and `tree.tree_scale`) are
/// treated as empty.
pub fn aabb_collides(
    tree: &GpuTree64,
    aabb_min: Vec3,
    aabb_max: Vec3,
) -> CollisionResult {
    let world_size = (1u64 << tree.tree_scale) as f32;
    let offset = Vec3::new(
        tree.root_offset[0] as f32,
        tree.root_offset[1] as f32,
        tree.root_offset[2] as f32,
    );

    // Convert to local tree coordinates.
    let local_min = aabb_min - offset;
    let local_max = aabb_max - offset;

    // Clamp query to tree bounds — outside is empty.
    let clamped_min = local_min.max(Vec3::ZERO);
    let clamped_max = local_max.min(Vec3::splat(world_size));

    if clamped_min.x >= clamped_max.x
        || clamped_min.y >= clamped_max.y
        || clamped_min.z >= clamped_max.z
    {
        return CollisionResult::Clear;
    }

    let num_levels = tree.tree_scale / 2;

    let mut ctx = TraversalCtx {
        tree,
        query_min: clamped_min,
        query_max: clamped_max,
        penetrations: None,
    };

    ctx.traverse(
        tree.root_node_index,
        num_levels,
        Vec3::ZERO,
        4u32.pow(num_levels) as f32,
    );

    match ctx.penetrations {
        None => CollisionResult::Clear,
        Some(p) => CollisionResult::Blocked {
            penetration_x: p[0],
            penetration_neg_x: p[1],
            penetration_y: p[2],
            penetration_neg_y: p[3],
            penetration_z: p[4],
            penetration_neg_z: p[5],
        },
    }
}

/// Mutable traversal state carried through the recursive AABB query.
struct TraversalCtx<'a> {
    tree: &'a GpuTree64,
    query_min: Vec3,
    query_max: Vec3,
    /// Accumulated per-axis maximum penetration: [px, nx, py, ny, pz, nz].
    penetrations: Option<[f32; 6]>,
}

impl TraversalCtx<'_> {
    /// Recursive hierarchical traversal.
    /// Recursive hierarchical traversal.
    fn traverse(
        &mut self,
        node_idx: u32,
        depth: u32,
        origin: Vec3,
        size: f32,
    ) {
        if depth == 0 {
            return;
        }

        let node = &self.tree.nodes[node_idx as usize];
        let is_leaf = (node.packed_data & 1) == 1;
        let pop_mask = (node.pop_mask_lo as u64) | ((node.pop_mask_hi as u64) << 32);

        if pop_mask == 0 {
            return;
        }

        let child_size = size / 4.0;

        if is_leaf {
            // Leaf: depth should be 1. Each set bit is an individual voxel.
            for child_idx in 0..64u32 {
                if pop_mask & (1u64 << child_idx) == 0 {
                    continue;
                }

                let cx = child_idx & 3;
                let cy = (child_idx >> 2) & 3;
                let cz = (child_idx >> 4) & 3;

                let voxel_min =
                    origin + Vec3::new(cx as f32, cy as f32, cz as f32) * child_size;
                let voxel_max = voxel_min + Vec3::splat(child_size);

                // AABB overlap test
                if voxel_min.x >= self.query_max.x
                    || voxel_max.x <= self.query_min.x
                    || voxel_min.y >= self.query_max.y
                    || voxel_max.y <= self.query_min.y
                    || voxel_min.z >= self.query_max.z
                    || voxel_max.z <= self.query_min.z
                {
                    continue;
                }

                // Accumulate per-axis penetration distances.
                let px = voxel_max.x - self.query_min.x; // push along +X (right)
                let nx = self.query_max.x - voxel_min.x; // push along -X (left)
                let py = voxel_max.y - self.query_min.y; // push along +Y (up)
                let ny = self.query_max.y - voxel_min.y; // push along -Y (down)
                let pz = voxel_max.z - self.query_min.z; // push along +Z (forward)
                let nz = self.query_max.z - voxel_min.z; // push along -Z (backward)

                match &mut self.penetrations {
                    None => {
                        self.penetrations = Some([px, nx, py, ny, pz, nz]);
                    }
                    Some(p) => {
                        p[0] = p[0].max(px);
                        p[1] = p[1].max(nx);
                        p[2] = p[2].max(py);
                        p[3] = p[3].max(ny);
                        p[4] = p[4].max(pz);
                        p[5] = p[5].max(nz);
                    }
                }
            }
        } else {
            // Internal node: test each child's AABB, recurse into overlapping ones.
            for child_idx in 0..64u32 {
                if pop_mask & (1u64 << child_idx) == 0 {
                    continue;
                }

                let cx = child_idx & 3;
                let cy = (child_idx >> 2) & 3;
                let cz = (child_idx >> 4) & 3;

                let child_origin =
                    origin + Vec3::new(cx as f32, cy as f32, cz as f32) * child_size;
                let child_max = child_origin + Vec3::splat(child_size);

                // AABB overlap test
                if child_origin.x >= self.query_max.x
                    || child_max.x <= self.query_min.x
                    || child_origin.y >= self.query_max.y
                    || child_max.y <= self.query_min.y
                    || child_origin.z >= self.query_max.z
                    || child_max.z <= self.query_min.z
                {
                    continue;
                }

                // Compute rank: number of set bits below child_idx in pop_mask.
                let rank = popcnt_below(pop_mask, child_idx);
                let child_node_idx = (node.packed_data >> 1) + rank as u32;

                self.traverse(child_node_idx, depth - 1, child_origin, child_size);
            }
        }
    }
}

/// Count the number of set bits in `mask` below the given `index`.
fn popcnt_below(mask: u64, index: u32) -> usize {
    let below = if index == 0 { 0 } else { (1u64 << index) - 1 };
    (mask & below).count_ones() as usize
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree64::builder::build_gpu_tree;

    /// Helper: build a tree, query a world-space AABB, extract the result.
    fn collide(
        voxels: &[([u32; 3], u8)],
        tree_dim: u32,
        offset: [i32; 3],
        aabb_min: Vec3,
        aabb_max: Vec3,
    ) -> CollisionResult {
        let tree = build_gpu_tree(voxels.iter().copied(), tree_dim, offset).unwrap();
        aabb_collides(&tree, aabb_min, aabb_max)
    }

    // ------------------------------------------------------------------
    // Empty space
    // ------------------------------------------------------------------

    #[test]
    fn clear_empty_tree() {
        // No voxels → build fails. Use a tree with one voxel but query far away.
        let voxels = [([0u32, 0, 0], 1u8)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(10.0, 10.0, 10.0),
            Vec3::new(11.0, 11.0, 11.0),
        );
        assert_eq!(result, CollisionResult::Clear);
    }

    #[test]
    fn clear_adjacent_no_overlap() {
        let voxels = [([0u32, 0, 0], 1u8)];
        // AABB at [1, 0, 0] just barely not overlapping [0,0,0]
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 1.0),
        );
        assert_eq!(result, CollisionResult::Clear);
    }

    // ------------------------------------------------------------------
    // Single voxel overlap
    // ------------------------------------------------------------------

    #[test]
    fn blocked_single_voxel_full() {
        let voxels = [([0u32, 0, 0], 1u8)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        );
        assert_eq!(
            result,
            CollisionResult::Blocked {
                penetration_x: 1.0,
                penetration_neg_x: 1.0,
                penetration_y: 1.0,
                penetration_neg_y: 1.0,
                penetration_z: 1.0,
                penetration_neg_z: 1.0,
            }
        );
    }

    #[test]
    fn blocked_partial_overlap_x() {
        let voxels = [([0u32, 0, 0], 1u8)];
        // AABB from [-0.5, 0, 0] to [0.5, 1, 1] — overlaps half the voxel
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(-0.5, 0.0, 0.0),
            Vec3::new(0.5, 1.0, 1.0),
        );
        assert_eq!(
            result,
            CollisionResult::Blocked {
                penetration_x: 1.0,     // push right: voxel_max.x - aabb_min.x = 1 - (-0.5) = 1.5
                penetration_neg_x: 0.5, // push left: aabb_max.x - voxel_min.x = 0.5 - 0 = 0.5
                penetration_y: 1.0,
                penetration_neg_y: 1.0,
                penetration_z: 1.0,
                penetration_neg_z: 1.0,
            }
        );
    }

    #[test]
    fn blocked_partial_overlap_y() {
        let voxels = [([0u32, 0, 0], 1u8)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(0.0, -0.25, 0.0),
            Vec3::new(1.0, 0.75, 1.0),
        );
        assert_eq!(
            result,
            CollisionResult::Blocked {
                penetration_x: 1.0,
                penetration_neg_x: 1.0,
                penetration_y: 1.0,      // push up: 1 - (-0.25) = 1.25
                penetration_neg_y: 0.75, // push down: 0.75 - 0 = 0.75
                penetration_z: 1.0,
                penetration_neg_z: 1.0,
            }
        );
    }

    #[test]
    fn blocked_multiple_voxels() {
        // Two voxels: (0,0,0) and (1,0,0). AABB covers both.
        let voxels = [([0u32, 0, 0], 1u8), ([1, 0, 0], 2)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 1.0),
        );
        // Overlaps both voxels. Penetration is the max across all.
        assert_eq!(
            result,
            CollisionResult::Blocked {
                penetration_x: 2.0,     // push right past voxel (1,0,0): 2 - 0 = 2
                penetration_neg_x: 2.0, // push left past voxel (0,0,0): 2 - 0 = 2
                penetration_y: 1.0,
                penetration_neg_y: 1.0,
                penetration_z: 1.0,
                penetration_neg_z: 1.0,
            }
        );
    }

    // ------------------------------------------------------------------
    // Per-axis penetration correctness
    // ------------------------------------------------------------------

    #[test]
    fn penetration_x_direction() {
        // Voxel at (1,0,0). AABB at [1.5, 0, 0] → [2.5, 1, 1].
        // Overlap region: x = [1.5, 2.0) — AABB's left half overlaps voxel's right half.
        let voxels = [([1u32, 0, 0], 1u8)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(1.5, 0.0, 0.0),
            Vec3::new(2.5, 1.0, 1.0),
        );
        assert_eq!(
            result,
            CollisionResult::Blocked {
                penetration_x: 0.5,     // push right: 2 - 1.5 = 0.5
                penetration_neg_x: 1.5, // push left: 2.5 - 1 = 1.5
                penetration_y: 1.0,
                penetration_neg_y: 1.0,
                penetration_z: 1.0,
                penetration_neg_z: 1.0,
            }
        );
    }

    #[test]
    fn penetration_y_direction() {
        // Voxel at (0,2,0) occupies local [0,1]×[2,3]×[0,1].
        // Query AABB from below: [0, 0, 0] → [1, 2.5, 1].
        // pen_y (up) = voxel_max.y - query_min.y = 3 - 0 = 3.0
        // pen_neg_y (down) = query_max.y - voxel_min.y = 2.5 - 2 = 0.5
        let voxels = [([0u32, 2, 0], 1u8)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 2.5, 1.0),
        );
        assert_eq!(
            result,
            CollisionResult::Blocked {
                penetration_x: 1.0,
                penetration_neg_x: 1.0,
                penetration_y: 3.0,
                penetration_neg_y: 0.5,
                penetration_z: 1.0,
                penetration_neg_z: 1.0,
            }
        );
    }

    #[test]
    fn penetration_z_direction() {
        // Voxel at (0,0,3) occupies local [0,1]×[0,1]×[3,4].
        // Query AABB from z=3.2: [0, 0, 3.2] → [1, 1, 4.0].
        // pen_z (push +Z) = voxel_max.z - query_min.z = 4 - 3.2 = 0.8
        // pen_neg_z (push -Z) = query_max.z - voxel_min.z = 4 - 3 = 1.0
        let voxels = [([0u32, 0, 3], 1u8)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(0.0, 0.0, 3.2),
            Vec3::new(1.0, 1.0, 4.0),
        );
        assert!(
            matches!(result, CollisionResult::Blocked {
                penetration_z, penetration_neg_z, ..
            } if (penetration_z - 0.8).abs() < 1e-5 && (penetration_neg_z - 1.0).abs() < 1e-5),
            "expected penetration_z≈0.8, penetration_neg_z≈1.0, got {result:?}"
        );
    }

    // ------------------------------------------------------------------
    // Hierarchical pruning
    // ------------------------------------------------------------------

    #[test]
    fn prunes_empty_branches() {
        // 16³ tree with voxels only in the (0,0,0) corner
        let voxels = [([0u32, 0, 0], 1u8)];
        let tree = build_gpu_tree(voxels, 16, [0, 0, 0]).unwrap();

        // Query far from the voxel — the tree should only visit the root
        // and notice the far octant has no populated bits. We can't easily
        // count visits, but we can verify correctness: Clear for far AABB.
        let result = aabb_collides(
            &tree,
            Vec3::new(12.0, 12.0, 12.0),
            Vec3::new(13.0, 13.0, 13.0),
        );
        assert_eq!(result, CollisionResult::Clear);

        // Meanwhile, query near the voxel should give Blocked.
        let result = aabb_collides(
            &tree,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        );
        assert!(matches!(result, CollisionResult::Blocked { .. }));
    }

    // ------------------------------------------------------------------
    // Ground-detection slab shape
    // ------------------------------------------------------------------

    #[test]
    fn ground_slab_stand_on_full_floor() {
        // Full floor at y=0.
        let mut voxels = Vec::new();
        for x in 0..4u32 {
            for z in 0..4u32 {
                voxels.push(([x, 0, z], 1u8));
            }
        }
        // Player footprint 0.6×0.6 at y=0 (AABB bottom at y=0, slab y=0..1)
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(1.7, 0.0, 1.7), // feet (AABB min, slab bottom)
            Vec3::new(2.3, 1.0, 2.3), // slab top
        );
        assert!(matches!(result, CollisionResult::Blocked { .. }));
    }

    #[test]
    fn ground_slab_on_lattice() {
        // Lattice floor: checkerboard pattern at y=0
        let mut voxels = Vec::new();
        for x in 0..4u32 {
            for z in 0..4u32 {
                if (x + z) % 2 == 0 {
                    voxels.push(([x, 0, z], 1u8));
                }
            }
        }
        // The slab covers the full 4×4 footprint area. At least one voxel
        // is present, so we expect Blocked.
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 1.0, 4.0),
        );
        assert!(matches!(result, CollisionResult::Blocked { .. }));
    }

    #[test]
    fn ground_slab_over_empty_space() {
        // Single voxel at (0,0,0). Slab query at y=1 (one voxel above ground).
        let voxels = [([0u32, 0, 0], 1u8)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 2.0, 1.0),
        );
        assert_eq!(result, CollisionResult::Clear);
    }

    #[test]
    fn ground_slab_beam_edge() {
        // A thin beam: single voxels at (0,0,0) and (3,0,0).
        // Slab from (0,0,0)-(4,1,4) should detect the beam.
        let voxels = [([0u32, 0, 0], 1u8), ([3, 0, 0], 2)];
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 1.0, 4.0),
        );
        assert!(matches!(result, CollisionResult::Blocked { .. }));
    }

    // ------------------------------------------------------------------
    // Tree boundaries
    // ------------------------------------------------------------------

    #[test]
    fn query_beyond_positive_bounds_is_clear() {
        let voxels = [([0u32, 0, 0], 1u8)];
        // 4³ tree with root_offset (0,0,0). World covers [0,4) in local coords.
        // Query at local [5, 0, 0] is entirely out of bounds → Clear.
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(6.0, 1.0, 1.0),
        );
        assert_eq!(result, CollisionResult::Clear);
    }

    #[test]
    fn query_beyond_negative_bounds_with_offset() {
        // Tree with root_offset = [10, 0, 0]. World covers [10, 14) in world-space.
        let voxels = [([0u32, 0, 0], 1u8)];
        let result = collide(
            &voxels,
            4,
            [10, 0, 0],
            Vec3::new(0.0, 0.0, 0.0),   // world-space, left of tree
            Vec3::new(9.0, 1.0, 1.0),
        );
        assert_eq!(result, CollisionResult::Clear);
    }

    #[test]
    fn query_partially_beyond_bounds() {
        // AABB straddles the tree boundary at x=4 (local). Only the portion
        // inside [0,4) contributes.
        let voxels = [([3u32, 0, 0], 1u8)];
        // AABB from x=3.5 to x=5.0 in local coords.
        // Overlaps the voxel at x=3. The clamped portion is [3.5, 4.0].
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(3.5, 0.0, 0.0),
            Vec3::new(5.0, 1.0, 1.0),
        );
        // The voxel at [3,0,0] occupies local [3,4)×[0,1)×[0,1).
        // Overlap: x=[3.5,4.0], y=[0,1], z=[0,1].
        assert_eq!(
            result,
            CollisionResult::Blocked {
                penetration_x: 0.5,     // push right: 4 - 3.5 = 0.5
                penetration_neg_x: 1.0, // push left: 4.0 - 3 = 1.0 (but wait...)
                // Actually query_max was clamped to 4.0, so:
                // penetration_neg_x = clamped_max.x - voxel_min.x = 4.0 - 3.0 = 1.0
                penetration_y: 1.0,
                penetration_neg_y: 1.0,
                penetration_z: 1.0,
                penetration_neg_z: 1.0,
            }
        );
    }

    // ------------------------------------------------------------------
    // Larger tree dimensions
    // ------------------------------------------------------------------

    #[test]
    fn large_tree_64x64x64() {
        let voxels = [([32u32, 32, 32], 42u8)];
        let result = collide(
            &voxels,
            64,
            [0, 0, 0],
            Vec3::new(32.0, 32.0, 32.0),
            Vec3::new(33.0, 33.0, 33.0),
        );
        assert!(matches!(result, CollisionResult::Blocked { .. }));
    }

    #[test]
    fn large_tree_with_offset() {
        let voxels = [([0u32, 0, 0], 1u8), ([63, 63, 63], 2)];
        let result = collide(
            &voxels,
            64,
            [-32, -32, -32],
            Vec3::new(-32.0, -32.0, -32.0),
            Vec3::new(-31.0, -31.0, -31.0),
        );
        assert!(matches!(result, CollisionResult::Blocked { .. }));
    }

    // ------------------------------------------------------------------
    // Complex multi-voxel collision
    // ------------------------------------------------------------------

    #[test]
    fn wall_collision_front() {
        // Wall at z=4 (one layer of voxels at z=4, spanning x-y).
        // Use 16³ tree (dim must be power of 4).
        let mut voxels = Vec::new();
        for x in 0..8u32 {
            for y in 0..8u32 {
                voxels.push(([x, y, 4], 1u8));
            }
        }
        // Player AABB approaching the wall from z<4.
        // Voxels at z=4 occupy local [x,x+1)×[y,y+1)×[4,5).
        // Query AABB: [2, 0, 3.5] → [3, 1.8, 4.2]
        // Overlaps voxels at x=2, y=0 and y=1, z=4.
        // Y: voxel y=0 gives pen_y=1.0, pen_neg_y=1.8; y=1 gives pen_y=2.0, pen_neg_y=0.8
        // Max: pen_y=2.0, pen_neg_y=1.8
        let result = collide(
            &voxels,
            16,
            [0, 0, 0],
            Vec3::new(2.0, 0.0, 3.5),
            Vec3::new(3.0, 1.8, 4.2),
        );
        assert!(
            matches!(result, CollisionResult::Blocked {
                penetration_x, penetration_neg_x, penetration_y,
                penetration_neg_y, penetration_z, penetration_neg_z
            } if
                (penetration_x - 1.0).abs() < 1e-5 &&
                (penetration_neg_x - 1.0).abs() < 1e-5 &&
                (penetration_y - 2.0).abs() < 1e-5 &&
                (penetration_neg_y - 1.8).abs() < 1e-5 &&
                (penetration_z - 1.5).abs() < 1e-5 &&
                (penetration_neg_z - 0.2).abs() < 1e-5
            ),
            "expected specific penetrations, got {result:?}"
        );
    }

    #[test]
    fn ceiling_collision() {
        let mut voxels = Vec::new();
        for x in 0..4u32 {
            for z in 0..4u32 {
                voxels.push(([x, 3, z], 1u8));
            }
        }
        // Player head hitting ceiling at y=3.
        let result = collide(
            &voxels,
            4,
            [0, 0, 0],
            Vec3::new(1.0, 1.2, 1.0),
            Vec3::new(2.0, 3.2, 2.0), // max.y = 3.2 → overlaps y=3 layer
        );
        assert!(matches!(result, CollisionResult::Blocked { .. }));
        if let CollisionResult::Blocked { penetration_neg_y, .. } = result {
            // pen_neg_y (push down): query_max.y - voxel_min.y = 3.2 - 3 = 0.2
            assert!((penetration_neg_y - 0.2).abs() < 1e-5);
        }
    }

    // ------------------------------------------------------------------
    // Root offset
    // ------------------------------------------------------------------

    #[test]
    fn root_offset_consistency() {
        // Same voxel at local (0,0,0) but different offsets.
        // Querying at world-space (5,5,5) with offset [5,5,5] is local (0,0,0).
        let voxels = [([0u32, 0, 0], 1u8)];
        let result = collide(
            &voxels,
            4,
            [5, 5, 5],
            Vec3::new(5.0, 5.0, 5.0),
            Vec3::new(6.0, 6.0, 6.0),
        );
        assert!(matches!(result, CollisionResult::Blocked { .. }));

        // Query at world-space (0,0,0) with offset [5,5,5] → local (-5,-5,-5).
        // Clamped to [0,0,0] → [0,0,0] means empty intersection → Clear.
        let result = collide(
            &voxels,
            4,
            [5, 5, 5],
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        );
        assert_eq!(result, CollisionResult::Clear);
    }
}

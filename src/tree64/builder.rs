use std::fmt;

use super::renderer::{GpuNode, GpuTree64};

/// Maximum pointer value that can be safely packed into `GpuNode`:
/// `(ptr << 1) | is_leaf` must not overflow u32, so ptr < 2^31.
const MAX_PTR: u32 = (1u32 << 31) - 1;

#[derive(Debug)]
pub(crate) enum TreeBuildError {
    EmptyInput,
    InvalidDimension { dim: u32, reason: &'static str },
    OutOfBoundsCoord { coord: [u32; 3], dim: u32 },
    DuplicateCoord { coord: [u32; 3] },
    NodeCountOverflow { count: usize },
    LeafDataOverflow { count: usize },
    NodePtrOverflow,
    LeafPtrOverflow,
}

impl fmt::Display for TreeBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => write!(f, "no voxels provided"),
            Self::InvalidDimension { dim, reason } => {
                write!(f, "invalid tree dimension {dim}: {reason}")
            }
            Self::OutOfBoundsCoord { coord, dim } => {
                write!(
                    f,
                    "coordinate [{}, {}, {}] out of bounds for tree dimension {dim}",
                    coord[0], coord[1], coord[2]
                )
            }
            Self::DuplicateCoord { coord } => {
                write!(
                    f,
                    "duplicate voxel at [{}, {}, {}]",
                    coord[0], coord[1], coord[2]
                )
            }
            Self::NodeCountOverflow { count } => {
                write!(f, "node count {count} exceeds u32 serializer limit")
            }
            Self::LeafDataOverflow { count } => {
                write!(f, "leaf data size {count} exceeds u32 serializer limit")
            }
            Self::NodePtrOverflow => {
                write!(f, "node index exceeds packed pointer limit")
            }
            Self::LeafPtrOverflow => {
                write!(f, "leaf data offset exceeds packed pointer limit")
            }
        }
    }
}

impl std::error::Error for TreeBuildError {}

/// Build a `GpuTree64` from occupied local-coordinate voxels.
///
/// Voxels are given as `(local_coordinate, material_value)`. Coordinates must satisfy
/// `0 <= coord < tree_dim` on all axes. `tree_dim` must be a power of four (4, 16, 64,
/// 256, ...). `root_offset` maps local coordinate zero to a signed world-space origin.
pub(crate) fn build_gpu_tree<I>(
    voxels: I,
    tree_dim: u32,
    root_offset: [i32; 3],
) -> Result<GpuTree64, TreeBuildError>
where
    I: IntoIterator<Item = ([u32; 3], u8)>,
{
    let num_levels = validate_dim(tree_dim)?;
    let records = make_records(voxels, tree_dim, num_levels)?;
    assemble_tree(&records, num_levels, tree_dim, root_offset)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate_dim(tree_dim: u32) -> Result<u32, TreeBuildError> {
    if tree_dim < 4 {
        return Err(TreeBuildError::InvalidDimension {
            dim: tree_dim,
            reason: "must be at least 4",
        });
    }
    if !tree_dim.is_power_of_two() {
        return Err(TreeBuildError::InvalidDimension {
            dim: tree_dim,
            reason: "must be a power of two",
        });
    }
    let log2 = tree_dim.ilog2();
    if !log2.is_multiple_of(2) {
        return Err(TreeBuildError::InvalidDimension {
            dim: tree_dim,
            reason: "must be a power of four (even log2)",
        });
    }
    let num_levels = log2 / 2;
    // Path key uses num_levels * 6 bits; must fit in u128.
    if num_levels as usize * 6 > 128 {
        return Err(TreeBuildError::InvalidDimension {
            dim: tree_dim,
            reason: "too many levels for path key encoding",
        });
    }
    Ok(num_levels)
}

// ---------------------------------------------------------------------------
// Record creation and sorting
// ---------------------------------------------------------------------------

struct VoxelRecord {
    /// Root-to-leaf path key: 6 bits per level, MSB = root octant.
    path_key: u128,
    value: u8,
    coords: [u32; 3],
}

fn compute_path_key(coords: [u32; 3], num_levels: u32) -> u128 {
    let mut key: u128 = 0;
    for level in (0..num_levels).rev() {
        let shift = 2 * level;
        let cx = ((coords[0] >> shift) & 3) as u128;
        let cy = ((coords[1] >> shift) & 3) as u128;
        let cz = ((coords[2] >> shift) & 3) as u128;
        let child_index = cx | (cy << 2) | (cz << 4);
        key = (key << 6) | child_index;
    }
    key
}

fn make_records<I>(
    voxels: I,
    tree_dim: u32,
    num_levels: u32,
) -> Result<Vec<VoxelRecord>, TreeBuildError>
where
    I: IntoIterator<Item = ([u32; 3], u8)>,
{
    let mut records: Vec<VoxelRecord> = voxels
        .into_iter()
        .map(|(coords, value)| {
            if coords[0] >= tree_dim || coords[1] >= tree_dim || coords[2] >= tree_dim {
                return Err(TreeBuildError::OutOfBoundsCoord {
                    coord: coords,
                    dim: tree_dim,
                });
            }
            let path_key = compute_path_key(coords, num_levels);
            Ok(VoxelRecord {
                path_key,
                value,
                coords,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    if records.is_empty() {
        return Err(TreeBuildError::EmptyInput);
    }

    records.sort_by(|a, b| {
        a.path_key
            .cmp(&b.path_key)
            .then_with(|| a.coords[0].cmp(&b.coords[0]))
            .then_with(|| a.coords[1].cmp(&b.coords[1]))
            .then_with(|| a.coords[2].cmp(&b.coords[2]))
    });

    // Detect duplicates: identical coordinates after sorting.
    for w in records.windows(2) {
        if w[0].coords == w[1].coords {
            return Err(TreeBuildError::DuplicateCoord { coord: w[0].coords });
        }
    }

    Ok(records)
}

// ---------------------------------------------------------------------------
// Recursive assembly
// ---------------------------------------------------------------------------

#[cfg(test)]
fn popcnt_below(mask: u64, index: u32) -> usize {
    let below = if index == 0 { 0 } else { (1u64 << index) - 1 };
    (mask & below).count_ones() as usize
}

fn assemble_tree(
    records: &[VoxelRecord],
    num_levels: u32,
    _tree_dim: u32,
    root_offset: [i32; 3],
) -> Result<GpuTree64, TreeBuildError> {
    let mut nodes: Vec<GpuNode> = Vec::new();
    let mut leaf_data: Vec<u8> = Vec::new();

    // Build from the root level down.
    let root = build_recursive(records, num_levels - 1, &mut nodes, &mut leaf_data)?;

    let root_node_index = nodes.len() as u32;
    nodes.push(root);

    // Per-serializer limits (both are written as u32).
    if nodes.len() > u32::MAX as usize {
        return Err(TreeBuildError::NodeCountOverflow { count: nodes.len() });
    }
    if leaf_data.len() > u32::MAX as usize {
        return Err(TreeBuildError::LeafDataOverflow {
            count: leaf_data.len(),
        });
    }

    let tree_scale = num_levels * 2;

    Ok(GpuTree64 {
        nodes,
        leaf_data,
        root_node_index,
        tree_scale,
        root_offset,
    })
}

fn build_recursive(
    records: &[VoxelRecord],
    depth: u32,
    nodes: &mut Vec<GpuNode>,
    leaf_data: &mut Vec<u8>,
) -> Result<GpuNode, TreeBuildError> {
    let shift = 6 * depth;
    let mut pop_mask: u64 = 0;

    if depth == 0 {
        // Leaf: each record is one occupied voxel in a 4³ cell.
        let byte_offset = leaf_data.len();

        for record in records {
            let child_index = ((record.path_key >> shift) & 0x3F) as u32;
            pop_mask |= 1u64 << child_index;
            leaf_data.push(record.value);
        }

        let ptr = u32::try_from(byte_offset).map_err(|_| TreeBuildError::LeafDataOverflow {
            count: leaf_data.len(),
        })?;
        if ptr > MAX_PTR {
            return Err(TreeBuildError::LeafPtrOverflow);
        }

        if leaf_data.len() > u32::MAX as usize {
            return Err(TreeBuildError::LeafDataOverflow {
                count: leaf_data.len(),
            });
        }

        Ok(GpuNode::new(true, ptr, pop_mask))
    } else {
        // Internal level: first build all children recursively, collecting their
        // descriptors. Then push all descriptors contiguously so the parent's
        // childPtr references a compact, rank-indexable range.
        let mut child_descriptors: Vec<(u32, GpuNode)> = Vec::new();
        let mut i = 0;

        while i < records.len() {
            let child_index = ((records[i].path_key >> shift) & 0x3F) as u32;
            pop_mask |= 1u64 << child_index;

            let group_start = i;
            while i < records.len() && ((records[i].path_key >> shift) & 0x3F) as u32 == child_index
            {
                i += 1;
            }

            let child_node =
                build_recursive(&records[group_start..i], depth - 1, nodes, leaf_data)?;
            child_descriptors.push((child_index, child_node));
        }

        // child_descriptors are already in ascending child_index order because
        // records are sorted by path_key. Push contiguously.
        let child_ptr = nodes.len();
        for (_, desc) in &child_descriptors {
            nodes.push(*desc);
        }

        let ptr = u32::try_from(child_ptr)
            .map_err(|_| TreeBuildError::NodeCountOverflow { count: nodes.len() })?;
        if ptr > MAX_PTR {
            return Err(TreeBuildError::NodePtrOverflow);
        }

        if nodes.len() > u32::MAX as usize {
            return Err(TreeBuildError::NodeCountOverflow { count: nodes.len() });
        }

        Ok(GpuNode::new(false, ptr, pop_mask))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Shader-equivalent lookup: traverse the tree exactly as the WGSL shader does.
    fn lookup(gpu_tree: &GpuTree64, local_coord: [u32; 3]) -> Option<u8> {
        let num_levels = gpu_tree.tree_scale / 2;
        let mut node_idx = gpu_tree.root_node_index;

        for level in (0..num_levels).rev() {
            let shift = 2 * level;
            let cx = (local_coord[0] >> shift) & 3;
            let cy = (local_coord[1] >> shift) & 3;
            let cz = (local_coord[2] >> shift) & 3;
            let child_index = cx + cy * 4 + cz * 16;

            let node = &gpu_tree.nodes[node_idx as usize];
            let is_leaf = node.packed_data & 1 == 1;
            let pop_mask = (node.pop_mask_lo as u64) | ((node.pop_mask_hi as u64) << 32);

            if pop_mask & (1u64 << child_index) == 0 {
                return None;
            }

            let rank = popcnt_below(pop_mask, child_index);

            if is_leaf {
                let byte_offset = (node.packed_data >> 1) as usize + rank;
                return Some(gpu_tree.leaf_data[byte_offset]);
            } else {
                node_idx = (node.packed_data >> 1) + rank as u32;
            }
        }

        None
    }

    // -----------------------------------------------------------------------
    // Basic validation
    // -----------------------------------------------------------------------

    #[test]
    fn empty_input() {
        let voxels: [([u32; 3], u8); 0] = [];
        let result = build_gpu_tree(voxels, 4, [0, 0, 0]);
        assert!(matches!(result, Err(TreeBuildError::EmptyInput)));
    }

    #[test]
    fn invalid_dimension_not_pow4() {
        let voxels = [([0u32, 0, 0], 1u8)];
        let result = build_gpu_tree(voxels, 8, [0, 0, 0]);
        assert!(matches!(
            result,
            Err(TreeBuildError::InvalidDimension { .. })
        ));
    }

    #[test]
    fn invalid_dimension_too_small() {
        let voxels = [([0u32, 0, 0], 1u8)];
        let result = build_gpu_tree(voxels, 1, [0, 0, 0]);
        assert!(matches!(
            result,
            Err(TreeBuildError::InvalidDimension { .. })
        ));
    }

    #[test]
    fn invalid_dimension_odd_log2() {
        let voxels = [([0u32, 0, 0], 1u8)];
        // 32 = 2^5, odd log2, not a power of four.
        let result = build_gpu_tree(voxels, 32, [0, 0, 0]);
        assert!(matches!(
            result,
            Err(TreeBuildError::InvalidDimension { .. })
        ));
    }

    #[test]
    fn out_of_bounds_coord() {
        let voxels = [([4u32, 0, 0], 1u8)];
        let result = build_gpu_tree(voxels, 4, [0, 0, 0]);
        assert!(matches!(
            result,
            Err(TreeBuildError::OutOfBoundsCoord { .. })
        ));
    }

    #[test]
    fn duplicate_coord() {
        let voxels = [([0u32, 0, 0], 1u8), ([0, 0, 0], 2)];
        let result = build_gpu_tree(voxels, 4, [0, 0, 0]);
        assert!(matches!(result, Err(TreeBuildError::DuplicateCoord { .. })));
    }

    // -----------------------------------------------------------------------
    // Single voxel
    // -----------------------------------------------------------------------

    #[test]
    fn single_voxel_corner() {
        let voxels = [([0u32, 0, 0], 42u8)];
        let tree = build_gpu_tree(voxels, 4, [0, 0, 0]).unwrap();
        assert_eq!(tree.tree_scale, 2);
        assert_eq!(tree.root_offset, [0, 0, 0]);
        assert_eq!(lookup(&tree, [0, 0, 0]), Some(42));
        assert_eq!(lookup(&tree, [1, 0, 0]), None);
    }

    #[test]
    fn single_voxel_far_corner() {
        let voxels = [([3u32, 3, 3], 99u8)];
        let tree = build_gpu_tree(voxels, 4, [0, 0, 0]).unwrap();
        assert_eq!(lookup(&tree, [3, 3, 3]), Some(99));
        assert_eq!(lookup(&tree, [0, 0, 0]), None);
    }

    #[test]
    fn single_voxel_at_max_dim_minus_one() {
        let dm1 = 15u32;
        let voxels = [([dm1, dm1, dm1], 7u8)];
        let tree = build_gpu_tree(voxels, 16, [0, 0, 0]).unwrap();
        assert_eq!(tree.tree_scale, 4);
        assert_eq!(lookup(&tree, [dm1, dm1, dm1]), Some(7));
        assert_eq!(lookup(&tree, [0, 0, 0]), None);
    }

    // -----------------------------------------------------------------------
    // Zero-valued material
    // -----------------------------------------------------------------------

    #[test]
    fn zero_material_is_occupied() {
        let voxels = [([0u32, 0, 0], 0u8)];
        let tree = build_gpu_tree(voxels, 4, [0, 0, 0]).unwrap();
        // Material 0 is a valid occupied voxel; lookup returns Some(0), not None.
        assert_eq!(lookup(&tree, [0, 0, 0]), Some(0));
        assert_eq!(lookup(&tree, [1, 0, 0]), None);
    }

    // -----------------------------------------------------------------------
    // Cell ordering across 4³ boundaries
    // -----------------------------------------------------------------------

    #[test]
    fn voxels_across_octant_boundary() {
        // [3,3,3] is child_index 63 in the root octant.
        // [4,4,4] is child_index 0 in a different root octant (since tree_dim=16
        // has num_levels=2; level 1 shift=2: 4>>2=1, child_index 1+4+16=21).
        let voxels = [([3u32, 3, 3], 1u8), ([4, 4, 4], 2u8)];
        let tree = build_gpu_tree(voxels, 16, [0, 0, 0]).unwrap();
        assert_eq!(lookup(&tree, [3, 3, 3]), Some(1));
        assert_eq!(lookup(&tree, [4, 4, 4]), Some(2));
        // Adjacent but empty coords should be None.
        assert_eq!(lookup(&tree, [3, 3, 4]), None);
        assert_eq!(lookup(&tree, [4, 4, 3]), None);
    }

    // -----------------------------------------------------------------------
    // Non-cubic occupied region in larger cubic root
    // -----------------------------------------------------------------------

    #[test]
    fn non_cubic_region_in_large_root() {
        // Occupied region is only [0..1, 0..1, 0..0] but wrapped in a 16³ tree.
        let voxels = [
            ([0u32, 0, 0], 10),
            ([1, 0, 0], 11),
            ([0, 1, 0], 12),
            ([1, 1, 0], 13),
        ];
        let tree = build_gpu_tree(voxels, 16, [0, 0, 0]).unwrap();
        assert_eq!(tree.tree_scale, 4);

        // Occupied coordinates are found.
        assert_eq!(lookup(&tree, [0, 0, 0]), Some(10));
        assert_eq!(lookup(&tree, [1, 0, 0]), Some(11));
        assert_eq!(lookup(&tree, [0, 1, 0]), Some(12));
        assert_eq!(lookup(&tree, [1, 1, 0]), Some(13));

        // Padded coordinates return None.
        assert_eq!(lookup(&tree, [0, 0, 1]), None);
        assert_eq!(lookup(&tree, [2, 2, 0]), None);
        assert_eq!(lookup(&tree, [15, 15, 15]), None);
    }

    // -----------------------------------------------------------------------
    // Signed root offset
    // -----------------------------------------------------------------------

    #[test]
    fn root_offset_preserved() {
        let voxels = [([0u32, 0, 0], 1u8)];
        let offset = [-5, 10, -3];
        let tree = build_gpu_tree(voxels, 4, offset).unwrap();
        assert_eq!(tree.root_offset, offset);
    }

    // -----------------------------------------------------------------------
    // Synthetic 16384³ sparse test
    // -----------------------------------------------------------------------

    #[test]
    fn synthetic_16384_two_corners() {
        let dim = 16384u32;
        let dm1 = dim - 1;
        let voxels = [([0u32, 0, 0], 100u8), ([dm1, dm1, dm1], 200u8)];
        let tree = build_gpu_tree(voxels, dim, [0, 0, 0]).unwrap();

        // Tree metadata.
        assert_eq!(tree.tree_scale, 14);
        assert_eq!(tree.root_node_index, tree.nodes.len() as u32 - 1);

        // Lookup returns correct values.
        assert_eq!(lookup(&tree, [0, 0, 0]), Some(100));
        assert_eq!(lookup(&tree, [dm1, dm1, dm1]), Some(200));

        // Empty spots return None.
        assert_eq!(lookup(&tree, [1, 0, 0]), None);
        assert_eq!(lookup(&tree, [dm1 - 1, dm1, dm1]), None);

        // Footprint is tiny — only the occupied paths exist.
        assert!(
            tree.nodes.len() < 100,
            "expected <100 nodes, got {}",
            tree.nodes.len()
        );
        assert!(
            tree.leaf_data.len() < 10,
            "expected <10 bytes leaf data, got {}",
            tree.leaf_data.len()
        );
    }

    // -----------------------------------------------------------------------
    // Small-dimension reference comparison against tree64::Tree64::new
    // -----------------------------------------------------------------------

    /// A minimal VoxelModel backed by a set of occupied (coordinate, value) pairs.
    /// Only used in tests to compare builder output against the existing Tree64
    /// constructor.
    struct TestModel {
        occupied: std::collections::HashMap<[usize; 3], u8>,
        dims: [u32; 3],
    }

    impl tree64::VoxelModel<u8> for &TestModel {
        fn dimensions(&self) -> [u32; 3] {
            self.dims
        }

        fn access(&self, coord: [usize; 3]) -> Option<u8> {
            self.occupied.get(&coord).copied()
        }
    }

    fn build_reference_gpu(
        voxels: &[([u32; 3], u8)],
        tree_dim: u32,
        root_offset: [i32; 3],
    ) -> GpuTree64 {
        let mut occupied = std::collections::HashMap::new();
        for &(coords, value) in voxels {
            occupied.insert(
                [coords[0] as usize, coords[1] as usize, coords[2] as usize],
                value,
            );
        }
        let model = TestModel {
            occupied,
            dims: [tree_dim; 3],
        };
        let tree = tree64::Tree64::new(&model);
        let mut gpu = GpuTree64::from_tree64(&tree);
        gpu.root_offset = root_offset;
        gpu
    }

    #[test]
    fn reference_comparison_dim4() {
        let voxels: Vec<([u32; 3], u8)> = (0..4u32)
            .flat_map(|x| {
                (0..4).flat_map(move |y| (0..4).map(move |z| ([x, y, z], (x + y + z) as u8)))
            })
            .collect();
        let builder_tree = build_gpu_tree(voxels.clone(), 4, [0, 0, 0]).unwrap();
        let ref_tree = build_reference_gpu(&voxels, 4, [0, 0, 0]);

        for x in 0..4u32 {
            for y in 0..4u32 {
                for z in 0..4u32 {
                    assert_eq!(
                        lookup(&builder_tree, [x, y, z]),
                        lookup(&ref_tree, [x, y, z]),
                        "mismatch at [{x}, {y}, {z}]"
                    );
                }
            }
        }
    }

    #[test]
    fn reference_comparison_dim16_sparse() {
        let voxels = [
            ([0u32, 0, 0], 10),
            ([15, 0, 0], 20),
            ([0, 15, 0], 30),
            ([0, 0, 15], 40),
            ([7, 7, 7], 50),
            ([15, 15, 15], 60),
        ];
        let builder_tree = build_gpu_tree(voxels, 16, [-3, 5, 0]).unwrap();
        let ref_tree = build_reference_gpu(&voxels, 16, [-3, 5, 0]);

        // Check occupied coords.
        for &(coord, _) in &voxels {
            assert_eq!(
                lookup(&builder_tree, coord),
                lookup(&ref_tree, coord),
                "mismatch at occupied [{}, {}, {}]",
                coord[0],
                coord[1],
                coord[2]
            );
        }

        // Spot-check some empty coords.
        for coord in [[0, 1, 0], [15, 1, 0], [8, 8, 8]] {
            assert_eq!(
                lookup(&builder_tree, coord),
                lookup(&ref_tree, coord),
                "mismatch at empty [{}, {}, {}]",
                coord[0],
                coord[1],
                coord[2]
            );
        }
    }

    #[test]
    fn reference_comparison_dim64_sparse() {
        let voxels = [
            ([0u32, 0, 0], 1),
            ([63, 0, 0], 2),
            ([0, 63, 0], 3),
            ([0, 0, 63], 4),
            ([63, 63, 63], 5),
            ([32, 32, 32], 6),
        ];
        let builder_tree = build_gpu_tree(voxels, 64, [0, 0, 0]).unwrap();
        let ref_tree = build_reference_gpu(&voxels, 64, [0, 0, 0]);

        for &(coord, _) in &voxels {
            assert_eq!(
                lookup(&builder_tree, coord),
                lookup(&ref_tree, coord),
                "mismatch at occupied [{}, {}, {}]",
                coord[0],
                coord[1],
                coord[2]
            );
        }

        // Spot-check empty coords.
        for coord in [[1, 0, 0], [63, 1, 0], [40, 40, 40]] {
            assert_eq!(
                lookup(&builder_tree, coord),
                lookup(&ref_tree, coord),
                "mismatch at empty [{}, {}, {}]",
                coord[0],
                coord[1],
                coord[2]
            );
        }
    }
}

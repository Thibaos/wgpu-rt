use bytemuck::{Pod, Zeroable};
use tree64::{Tree64, VoxelModel};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuNode {
    /// Packed: bit 0 = is_leaf, bits 1..31 = child_ptr or data_ptr (in u32 units for data)
    pub packed_data: u32,
    /// 64-bit occupancy mask
    pub pop_mask_lo: u32,
    pub pop_mask_hi: u32,
}

impl GpuNode {
    pub fn new(is_leaf: bool, ptr: u32, pop_mask: u64) -> Self {
        Self {
            packed_data: (ptr << 1) | (is_leaf as u32),
            pop_mask_lo: pop_mask as u32,
            pop_mask_hi: (pop_mask >> 32) as u32,
        }
    }
}

pub struct GpuTree64 {
    pub nodes: Vec<GpuNode>,
    pub leaf_data: Vec<u8>,
    pub root_node_index: u32,
    pub tree_scale: u32, // 2^tree_scale = world size
    pub root_offset: [i32; 3],
}

impl GpuTree64 {
    pub fn from_model(model: &impl VoxelModel<u8>) -> Self {
        let tree = Tree64::new(model);
        let root_state = tree.root_state();

        let nodes: Vec<GpuNode> = tree
            .nodes
            .iter()
            .map(|n| {
                let tree64_node = *n; // tree64::Node is Pod
                let is_leaf = (tree64_node.is_leaf_and_ptr & 1) == 1;
                let ptr = tree64_node.is_leaf_and_ptr >> 1;
                GpuNode::new(is_leaf, ptr, tree64_node.pop_mask)
            })
            .collect();

        let leaf_data = tree.data.to_vec();

        // TreeScale: log2 of the world size the tree covers.
        // The root state's num_levels means the tree has 4^num_levels voxels per axis.
        let world_size = 4u32.pow(root_state.num_levels as u32);
        let tree_scale = world_size.ilog2();

        Self {
            nodes,
            leaf_data,
            root_node_index: root_state.index,
            tree_scale,
            root_offset: root_state.offset.to_array(),
        }
    }

    pub fn create_buffers(&self, device: &wgpu::Device) -> GpuTree64Buffers {
        let nodes_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tree64_nodes"),
            contents: bytemuck::cast_slice(&self.nodes),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let data_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tree64_leaf_data"),
            contents: &self.leaf_data,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tree64_params"),
            contents: bytemuck::bytes_of(&Tree64Params {
                root_node_index: self.root_node_index,
                tree_scale: self.tree_scale,
                root_offset: self.root_offset,
                _pad0: [0; 2],
                _pad1: 0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        GpuTree64Buffers {
            nodes: nodes_buf,
            leaf_data: data_buf,
            params: params_buf,
        }
    }
}

pub struct GpuTree64Buffers {
    pub nodes: wgpu::Buffer,
    pub leaf_data: wgpu::Buffer,
    pub params: wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Tree64Params {
    pub root_node_index: u32,  // offset 0
    pub tree_scale: u32,       // offset 4
    pub _pad0: [u32; 2],       // offset 8  — std140 padding to align vec3 to 16
    pub root_offset: [i32; 3], // offset 16
    pub _pad1: u32,            // offset 28 — end padding to reach 32 bytes
}

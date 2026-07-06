use bytemuck::{Pod, Zeroable};
use std::io;
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

/// Create a storage buffer holding the 256-color palette as vec4<f32>.
/// Each RGBA8 entry becomes [r/255, g/255, b/255, a/255].
pub fn create_palette_buffer(device: &wgpu::Device, palette: &[[u8; 4]; 256]) -> wgpu::Buffer {
    let float_palette: [[f32; 4]; 256] = std::array::from_fn(|i| {
        let [r, g, b, a] = palette[i];
        [
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        ]
    });
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("palette"),
        contents: bytemuck::cast_slice(&float_palette),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

#[allow(dead_code)]
impl GpuTree64 {
    /// Build GpuTree64 from an already-constructed Tree64<u8>.
    /// Useful when deserializing a pre-built tree (no VoxelModel needed).
    pub fn from_tree64(tree: &tree64::Tree64<u8>) -> Self {
        let root_state = tree.root_state();

        let nodes: Vec<GpuNode> = tree
            .nodes
            .iter()
            .map(|n| {
                let tree64_node = *n;
                let is_leaf = (tree64_node.is_leaf_and_ptr & 1) == 1;
                let ptr = tree64_node.is_leaf_and_ptr >> 1;
                GpuNode::new(is_leaf, ptr, tree64_node.pop_mask)
            })
            .collect();

        let leaf_data = tree.data.to_vec();

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

    /// Create a shallow clone that shares no ownership — copies the data.
    pub fn clone_ref(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            leaf_data: self.leaf_data.clone(),
            root_node_index: self.root_node_index,
            tree_scale: self.tree_scale,
            root_offset: self.root_offset,
        }
    }

    /// Serialize to a writer in the GPU-ready format.
    pub fn serialize<W: io::Write>(&self, mut writer: W) -> io::Result<()> {
        let params = Tree64Params {
            root_node_index: self.root_node_index,
            tree_scale: self.tree_scale,
            _pad0: [0; 2],
            root_offset: self.root_offset,
            _pad1: 0,
        };
        writer.write_all(bytemuck::bytes_of(&params))?;

        let node_count = self.nodes.len() as u32;
        let node_bytes = node_count * std::mem::size_of::<GpuNode>() as u32;
        writer.write_all(&node_count.to_le_bytes())?;
        writer.write_all(&node_bytes.to_le_bytes())?;
        writer.write_all(bytemuck::cast_slice(&self.nodes))?;

        let leaf_count = self.leaf_data.len() as u32;
        writer.write_all(&leaf_count.to_le_bytes())?;
        writer.write_all(&leaf_count.to_le_bytes())?;
        writer.write_all(&self.leaf_data)?;

        Ok(())
    }

    /// Deserialize from a reader. Reads the format written by `serialize`.
    pub fn deserialize<R: io::Read>(mut reader: R) -> io::Result<Self> {
        let mut params_bytes = [0u8; std::mem::size_of::<Tree64Params>()];
        reader.read_exact(&mut params_bytes)?;
        let params: Tree64Params = *bytemuck::from_bytes(&params_bytes);

        let mut node_count_bytes = [0u8; 4];
        reader.read_exact(&mut node_count_bytes)?;
        let node_count = u32::from_le_bytes(node_count_bytes);

        let mut _node_bytes_sanity = [0u8; 4];
        reader.read_exact(&mut _node_bytes_sanity)?;

        let mut nodes: Vec<GpuNode> = vec![
            GpuNode {
                packed_data: 0,
                pop_mask_lo: 0,
                pop_mask_hi: 0,
            };
            node_count as usize
        ];
        reader.read_exact(bytemuck::cast_slice_mut(&mut nodes))?;

        let mut leaf_count_bytes = [0u8; 4];
        reader.read_exact(&mut leaf_count_bytes)?;
        let leaf_count = u32::from_le_bytes(leaf_count_bytes);

        let mut _leaf_bytes_sanity = [0u8; 4];
        reader.read_exact(&mut _leaf_bytes_sanity)?;

        let mut leaf_data = vec![0u8; leaf_count as usize];
        reader.read_exact(&mut leaf_data)?;

        Ok(Self {
            nodes,
            leaf_data,
            root_node_index: params.root_node_index,
            tree_scale: params.tree_scale,
            root_offset: params.root_offset,
        })
    }
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

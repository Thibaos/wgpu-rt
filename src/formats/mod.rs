//! Binary world format (.world) — header, palette, and a single GPU-ready tree blob.

use std::io::{self};

use crate::tree64_renderer::GpuTree64;

pub const WORLD_MAGIC: [u8; 4] = *b"WRLD";
pub const WORLD_VERSION: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WorldHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub tree_present: u32,
    pub reserved: [u8; 52],
}

impl WorldHeader {
    pub fn new(tree_present: bool) -> Self {
        Self {
            magic: WORLD_MAGIC,
            version: WORLD_VERSION,
            tree_present: if tree_present { 1 } else { 0 },
            reserved: [0; 52],
        }
    }

    pub fn read(mut reader: impl io::Read) -> io::Result<Self> {
        let mut bytes = [0u8; 64];
        reader.read_exact(&mut bytes)?;
        let header: Self = unsafe { std::ptr::read(bytes.as_ptr() as *const Self) };
        if header.magic != WORLD_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid world file magic",
            ));
        }
        if header.version > WORLD_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported world version: {} (max {}) — re-bake the world file",
                    header.version, WORLD_VERSION
                ),
            ));
        }
        if header.version < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "world file is pre-v3 (chunk format); re-bake with `cargo run --bin bake`",
            ));
        }
        Ok(header)
    }

    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        let bytes: &[u8; 64] = unsafe { std::mem::transmute(self) };
        writer.write_all(bytes)
    }
}

/// Complete world file: header + palette + single tree blob.
pub struct WorldFile {
    pub header: WorldHeader,
    /// 256-color RGBA8 palette (from .vox file or zeros).
    pub palette: [[u8; 4]; 256],
    /// The single tree covering the full world, or None for an empty world.
    pub tree: Option<GpuTree64>,
}

impl Default for WorldFile {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldFile {
    pub fn new() -> Self {
        Self {
            header: WorldHeader::new(false),
            palette: [[0u8; 4]; 256],
            tree: None,
        }
    }

    /// Write the complete world file.
    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        let header = WorldHeader::new(self.tree.is_some());
        header.write(&mut writer)?;

        // Palette: 256 colors × 4 bytes = 1024 bytes
        let palette_bytes: [u8; 1024] = bytemuck::cast(self.palette);
        writer.write_all(&palette_bytes)?;

        // Single tree blob
        if let Some(ref tree) = self.tree {
            tree.serialize(&mut writer)?;
        }

        Ok(())
    }

    /// Read a complete world file.
    pub fn read(mut reader: impl io::Read) -> io::Result<Self> {
        let header = WorldHeader::read(&mut reader)?;

        // Read palette: 1024 bytes of RGBA8
        let mut palette_bytes = [0u8; 1024];
        reader.read_exact(&mut palette_bytes)?;
        let palette: [[u8; 4]; 256] = bytemuck::cast(palette_bytes);

        let tree = if header.tree_present != 0 {
            Some(GpuTree64::deserialize(&mut reader)?)
        } else {
            None
        };

        Ok(Self {
            header,
            palette,
            tree,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree64_renderer::{GpuNode, GpuTree64};
    use std::io::Cursor;

    fn make_dummy_gpu_tree() -> GpuTree64 {
        GpuTree64 {
            nodes: vec![
                GpuNode::new(false, 1, 0b0001_0001_0001_0001u64),
                GpuNode::new(true, 0, 0b1111_0000_0000_0000u64),
            ],
            leaf_data: vec![1, 2, 3, 4],
            root_node_index: 0,
            tree_scale: 8,
            root_offset: [0, 0, 0],
        }
    }

    #[test]
    fn world_file_roundtrip() {
        let mut world = WorldFile::new();
        world.palette[1] = [255, 0, 0, 255];
        world.tree = Some(make_dummy_gpu_tree());

        let mut buf = Cursor::new(Vec::new());
        world.write(&mut buf).unwrap();

        buf.set_position(0);
        let loaded = WorldFile::read(&mut buf).unwrap();

        assert_eq!(loaded.header.magic, WORLD_MAGIC);
        assert_eq!(loaded.header.version, WORLD_VERSION);
        assert_eq!(loaded.header.tree_present, 1);
        assert_eq!(loaded.palette[1], [255, 0, 0, 255]);

        let tree = loaded.tree.unwrap();
        assert_eq!(tree.nodes.len(), 2);
        assert_eq!(tree.leaf_data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn world_file_empty_roundtrip() {
        let world = WorldFile::new();

        let mut buf = Cursor::new(Vec::new());
        world.write(&mut buf).unwrap();

        buf.set_position(0);
        let loaded = WorldFile::read(&mut buf).unwrap();

        assert_eq!(loaded.header.tree_present, 0);
        assert!(loaded.tree.is_none());
    }
}

#![allow(dead_code)]

use std::io;

use crate::tree64_renderer::GpuTree64;

/// A chunk's serialized data blob.
/// Wraps GpuTree64 serialize/deserialize for use within the world format.
pub struct ChunkData {
    pub tree: GpuTree64,
}

impl ChunkData {
    pub fn new(tree: GpuTree64) -> Self {
        Self { tree }
    }

    /// Read a chunk blob from a reader.
    /// This reads exactly the bytes written by `write`.
    pub fn read(mut reader: impl io::Read) -> io::Result<Self> {
        let tree = GpuTree64::deserialize(&mut reader)?;
        Ok(Self { tree })
    }

    /// Write the chunk blob to a writer.
    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        self.tree.serialize(&mut writer)
    }
}

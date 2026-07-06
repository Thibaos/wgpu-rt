//! Binary world format (.world) — header, chunk table of contents,
//! and per-chunk GPU-ready blobs.

#![allow(dead_code)]

pub mod chunk;

use std::io;

/// Magic bytes identifying a .world file.
pub const WORLD_MAGIC: [u8; 4] = *b"WRLD";

/// Current format version.
pub const WORLD_VERSION: u32 = 1;

/// World dimensions (immutable for this format version).
pub const CHUNK_COUNT_X: u32 = 16;
pub const CHUNK_COUNT_Y: u32 = 1;
pub const CHUNK_COUNT_Z: u32 = 16;
pub const CHUNK_VOXEL_X: u32 = 256;
pub const CHUNK_VOXEL_Y: u32 = 2048;
pub const CHUNK_VOXEL_Z: u32 = 256;
pub const TOTAL_CHUNKS: u32 = CHUNK_COUNT_X * CHUNK_COUNT_Y * CHUNK_COUNT_Z;

/// Header of a .world file (64 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WorldHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub chunk_count_x: u32,
    pub chunk_count_y: u32,
    pub chunk_count_z: u32,
    pub chunk_voxel_x: u32,
    pub chunk_voxel_y: u32,
    pub chunk_voxel_z: u32,
    pub reserved: [u8; 32],
}

impl WorldHeader {
    pub fn new() -> Self {
        Self {
            magic: WORLD_MAGIC,
            version: WORLD_VERSION,
            chunk_count_x: CHUNK_COUNT_X,
            chunk_count_y: CHUNK_COUNT_Y,
            chunk_count_z: CHUNK_COUNT_Z,
            chunk_voxel_x: CHUNK_VOXEL_X,
            chunk_voxel_y: CHUNK_VOXEL_Y,
            chunk_voxel_z: CHUNK_VOXEL_Z,
            reserved: [0; 32],
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
        if header.version != WORLD_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported world version: {}", header.version),
            ));
        }
        Ok(header)
    }

    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        let bytes: &[u8; 64] = unsafe { std::mem::transmute(self) };
        writer.write_all(bytes)
    }

    pub fn total_chunks(&self) -> u32 {
        self.chunk_count_x * self.chunk_count_y * self.chunk_count_z
    }
}

impl Default for WorldHeader {
    fn default() -> Self {
        Self::new()
    }
}

/// Single entry in the chunk table of contents (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ChunkTocEntry {
    pub byte_offset: u64,
    pub size: u64,
}

impl ChunkTocEntry {
    pub fn read(mut reader: impl io::Read) -> io::Result<Self> {
        let mut bytes = [0u8; 16];
        reader.read_exact(&mut bytes)?;
        Ok(unsafe { std::ptr::read(bytes.as_ptr() as *const Self) })
    }

    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        let bytes: &[u8; 16] = unsafe { std::mem::transmute(self) };
        writer.write_all(bytes)
    }
}

/// The full chunk table of contents (256 entries × 16 bytes = 4096 bytes).
pub struct ChunkTable {
    pub entries: Vec<ChunkTocEntry>,
}

impl ChunkTable {
    pub fn new(chunk_count: u32) -> Self {
        Self {
            entries: vec![ChunkTocEntry::default(); chunk_count as usize],
        }
    }

    pub fn read(mut reader: impl io::Read, chunk_count: u32) -> io::Result<Self> {
        let mut entries = Vec::with_capacity(chunk_count as usize);
        for _ in 0..chunk_count {
            entries.push(ChunkTocEntry::read(&mut reader)?);
        }
        Ok(Self { entries })
    }

    pub fn write(&self, mut writer: impl io::Write) -> io::Result<()> {
        for entry in &self.entries {
            entry.write(&mut writer)?;
        }
        Ok(())
    }

    /// Convert a 3D chunk coordinate to a flat index.
    /// Layout: index = x + z * chunk_count_x  (y is always 0 since chunk_count_y = 1).
    pub fn chunk_index(x: u32, _y: u32, z: u32, chunk_count_x: u32) -> usize {
        (x + z * chunk_count_x) as usize
    }
}

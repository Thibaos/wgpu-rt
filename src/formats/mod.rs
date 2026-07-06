//! Binary world format (.world) — header, chunk table of contents,
//! and per-chunk GPU-ready blobs.

#![allow(dead_code)]

pub mod chunk;

use std::io::{self};

use crate::formats::chunk::ChunkData;

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

/// Complete world file: header + chunk table + chunk data blobs.
pub struct WorldFile {
    pub header: WorldHeader,
    pub table: ChunkTable,
    /// Chunk data indexed by the same flat index as the TOC.
    pub chunks: Vec<Option<ChunkData>>,
}

impl Default for WorldFile {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldFile {
    pub fn new() -> Self {
        let header = WorldHeader::new();
        let total = header.total_chunks() as usize;
        Self {
            header,
            table: ChunkTable::new(total as u32),
            chunks: (0..total).map(|_| None).collect(),
        }
    }

    /// Set chunk data for the given flat index.
    pub fn set_chunk(&mut self, index: usize, data: ChunkData) {
        self.chunks[index] = Some(data);
    }

    /// Write the complete world file.
    pub fn write(&self, mut writer: impl io::Write + io::Seek) -> io::Result<()> {
        // Write header (64 bytes)
        self.header.write(&mut writer)?;

        // Reserve space for the TOC (write placeholder zeros, seek back later)
        let toc_size = self.table.entries.len() * 16;
        let toc_start = writer.stream_position()?;
        let zeros = vec![0u8; toc_size];
        writer.write_all(&zeros)?;

        // Write chunk data, building TOC entries as we go
        let mut toc_entries = vec![ChunkTocEntry::default(); self.table.entries.len()];

        for (i, chunk_opt) in self.chunks.iter().enumerate() {
            if let Some(chunk) = chunk_opt {
                let offset = writer.stream_position()?;
                chunk.write(&mut writer)?;
                let end = writer.stream_position()?;
                toc_entries[i] = ChunkTocEntry {
                    byte_offset: offset,
                    size: end - offset,
                };
            }
        }

        // Seek back and write the TOC
        writer.seek(io::SeekFrom::Start(toc_start))?;
        for entry in &toc_entries {
            entry.write(&mut writer)?;
        }

        Ok(())
    }

    /// Read a complete world file.
    pub fn read(mut reader: impl io::Read + io::Seek) -> io::Result<Self> {
        let header = WorldHeader::read(&mut reader)?;
        let total = header.total_chunks() as usize;
        let table = ChunkTable::read(&mut reader, total as u32)?;

        let mut chunks: Vec<Option<ChunkData>> = Vec::with_capacity(total);

        for entry in &table.entries {
            if entry.byte_offset == 0 {
                chunks.push(None);
            } else {
                reader.seek(io::SeekFrom::Start(entry.byte_offset))?;
                let data = ChunkData::read(&mut reader)?;
                chunks.push(Some(data));
            }
        }

        Ok(Self {
            header,
            table,
            chunks,
        })
    }
}

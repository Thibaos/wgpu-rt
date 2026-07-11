use std::fs::File;
use std::io::BufReader;
use std::path::Path;

mod builder;
pub mod loader;
pub mod renderer;

use crate::formats::WorldFile;
use crate::tree64::renderer::GpuTree64;

/// Loaded world state: a single tree covering the full world volume.
pub struct World {
    pub tree: Option<GpuTree64>,
    /// 256-color RGBA8 palette (from .vox file or zeros).
    pub palette: [[u8; 4]; 256],
}

impl World {
    /// Load a .world file from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let file = File::open(path.as_ref())
            .map_err(|e| format!("failed to open world file {}: {e}", path.as_ref().display()))?;
        let mut reader = BufReader::new(file);
        let world_file =
            WorldFile::read(&mut reader).map_err(|e| format!("failed to read world file: {e}"))?;

        let tree = world_file.tree;

        let found = tree.is_some();
        log::info!(
            "Loaded world: tree {}",
            if found { "found" } else { "empty" }
        );

        Ok(Self {
            tree,
            palette: world_file.palette,
        })
    }
}

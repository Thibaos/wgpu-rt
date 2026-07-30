//! Offline WGSL validation for the chunk shader.
//!
//! The shader is compiled by wgpu only at GPU runtime, which makes syntax/type
//! errors invisible to `cargo test`. This test parses `chunk.wgsl` with naga
//! (the same front end wgpu uses) so a broken shader fails CI instead of
//! panicking at first frame.

use std::path::PathBuf;

fn shader_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("shaders")
        .join("chunk.wgsl")
}

#[test]
fn chunk_wgsl_parses_and_validates() {
    let source = std::fs::read_to_string(shader_path()).expect("chunk.wgsl must exist");
    // `parse` runs the WGSL front end: syntax, typing, and module validation.
    // A GPU/device is not required.
    naga::front::wgsl::parse_str(&source).expect("chunk.wgsl failed WGSL validation");
}

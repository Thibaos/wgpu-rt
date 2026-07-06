use std::path::{Path, PathBuf};
use std::process::Command;

fn find_slangc() -> Option<String> {
    if let Ok(path) = std::env::var("SLANGC") {
        return Some(path);
    }

    for candidate in &["slangc", "slangc.exe"] {
        if Command::new(candidate)
            .arg("-h")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return Some(candidate.to_string());
        }
    }

    None
}

fn compile_shader(slangc: &str, input: &Path, output: &Path) -> bool {
    let status = Command::new(slangc)
        .arg(input)
        .args(["-target", "wgsl"])
        .arg("-o")
        .arg(output)
        .status();

    match status {
        Ok(s) if s.success() => true,
        Ok(s) => {
            println!(
                "cargo:warning=slangc failed with exit code {:?} for {}",
                s.code(),
                input.display()
            );
            false
        }
        Err(e) => {
            println!(
                "cargo:warning=Failed to run slangc for {}: {e}",
                input.display()
            );
            false
        }
    }
}

/// Slang's WGSL backend emits `read_write` for `RWTexture2D`, but wgpu
/// only supports `WriteOnly` for `Rgba8Unorm` storage textures.
/// Patch `read_write` → `write` on storage texture declarations.
fn fix_read_write_to_write(wgsl: &str) -> String {
    wgsl.replace("read_write>", "write>")
}

fn is_stale(input: &Path, output: &Path) -> bool {
    let out_path = match output.canonicalize() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let in_path = match input.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };

    let out_meta = match std::fs::metadata(&out_path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let in_meta = match std::fs::metadata(&in_path) {
        Ok(m) => m,
        Err(_) => {
            println!(
                "cargo:warning=Source shader not found: {}",
                in_path.display()
            );
            return false;
        }
    };
    in_meta.modified().unwrap() > out_meta.modified().unwrap()
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let shader_dir = manifest_dir.join("assets").join("shaders");

    let shaders: &[(&str, &str)] = &[("tree64_compute.slang", "tree64_compiled.wgsl")];

    let Some(slangc) = find_slangc() else {
        println!("cargo:warning=slangc not found — using pre-compiled .wgsl files as-is");
        for (_input, output) in shaders {
            println!(
                "cargo:rerun-if-changed={}",
                shader_dir.join(output).display()
            );
        }
        return;
    };

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SLANGC");

    for &(input_name, output_name) in shaders {
        let input = shader_dir.join(input_name);
        let output = shader_dir.join(output_name);

        println!("cargo:rerun-if-changed={}", input.display());

        if !is_stale(&input, &output) {
            continue;
        }

        println!("cargo:warning=Compiling {} → {}", input_name, output_name);

        if !compile_shader(&slangc, &input, &output) {
            continue;
        }

        match std::fs::read_to_string(&output) {
            Ok(contents) => {
                let fixed = fix_read_write_to_write(&contents);
                if fixed != contents {
                    println!(
                        "cargo:warning=Post-processing {} (read_write → write)",
                        output_name
                    );
                    if let Err(e) = std::fs::write(&output, fixed) {
                        println!("cargo:warning=Failed to write post-processed shader: {e}");
                    }
                }
            }
            Err(e) => {
                println!("cargo:warning=Failed to read compiled shader for post-processing: {e}");
            }
        }
    }
}

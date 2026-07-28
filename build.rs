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

fn compile_entry_point(slangc: &str, input: &Path, entry: &str, output: &Path) -> bool {
    let status = Command::new(slangc)
        .arg(input)
        .arg("-entry")
        .arg(entry)
        .args(["-target", "spirv"])
        .arg("-o")
        .arg(output)
        .status();

    match status {
        Ok(s) if s.success() => true,
        Ok(s) => {
            println!(
                "cargo:warning=slangc failed for {}:{} with exit code {:?}",
                input.display(),
                entry,
                s.code()
            );
            false
        }
        Err(e) => {
            println!(
                "cargo:warning=Failed to run slangc for {}:{}: {e}",
                input.display(),
                entry
            );
            false
        }
    }
}

fn is_stale(input: &Path, outputs: &[&Path]) -> bool {
    let in_meta = match std::fs::metadata(input) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let in_time = in_meta.modified().unwrap();

    outputs.iter().any(|output| {
        std::fs::metadata(output)
            .map(|m| m.modified().unwrap() < in_time)
            .unwrap_or(true) // missing → stale
    })
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let shader_dir = manifest_dir.join("assets").join("shaders");

    let input = shader_dir.join("chunk.slang");
    let vert_output = shader_dir.join("chunk.vert.spv");
    let frag_output = shader_dir.join("chunk.frag.spv");

    println!("cargo:rerun-if-changed={}", input.display());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SLANGC");

    let Some(slangc) = find_slangc() else {
        println!("cargo:warning=slangc not found — using pre-compiled .spv files as-is");
        println!("cargo:rerun-if-changed={}", vert_output.display());
        println!("cargo:rerun-if-changed={}", frag_output.display());
        return;
    };

    if !is_stale(&input, &[&vert_output, &frag_output]) {
        return;
    }

    println!("cargo:warning=Compiling chunk.slang → chunk.vert.spv + chunk.frag.spv");

    compile_entry_point(&slangc, &input, "vs_main", &vert_output);
    compile_entry_point(&slangc, &input, "fs_main", &frag_output);
}

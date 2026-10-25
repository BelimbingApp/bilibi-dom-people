//! Discover proof files without a shared `mod` list. Generated Rust stays in OUT_DIR.
use std::{env, fmt::Write, fs, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let source = manifest.join("src");
    // Watching the directory catches new and removed files, as well as edits.
    println!("cargo:rerun-if-changed=src");
    let mut files = fs::read_dir(&source)
        .expect("walkthrough source directory")
        .map(|entry| entry.expect("source entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .filter(|path| {
            !matches!(
                path.file_stem().and_then(|s| s.to_str()),
                Some("lib" | "main" | "registry")
            )
        })
        .collect::<Vec<_>>();
    files.sort();
    let mut generated = String::new();
    for file in files {
        let name = file
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("UTF-8 proof filename");
        assert!(
            name.starts_with(|c: char| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "proof filename must be a snake_case Rust module: {name}"
        );
        writeln!(generated, "#[path = {file:?}] mod {name};").expect("write module declaration");
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    fs::write(output.join("proofs.rs"), generated).expect("generated proof declarations");
}

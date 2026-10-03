use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::Path;

/// Files in a release directory that are not release assets.
const NOT_ASSETS: [&str; 3] = ["checksums.txt", "RELEASE_NOTES.md", "README.md"];

/// Write `checksums.txt` for every asset in a release directory.
///
/// Assets are discovered rather than hardcoded so a renamed or added binary can
/// never be silently left out of the checksum file.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <release-dir>", args[0]);
        eprintln!("Example: {} releases/20261003_1", args[0]);
        std::process::exit(1);
    }

    let dir = &args[1];
    if !Path::new(dir).is_dir() {
        eprintln!("Error: {} is not a directory", dir);
        std::process::exit(1);
    }

    let mut assets: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if NOT_ASSETS.contains(&name.as_str()) || name.starts_with('.') {
            continue;
        }
        assets.push((name, fs::read(&path)?));
    }

    if assets.is_empty() {
        eprintln!("Error: no release assets found in {}", dir);
        std::process::exit(1);
    }

    assets.sort_by(|a, b| a.0.cmp(&b.0));

    let mut lines: Vec<String> = Vec::new();
    for (name, data) in &assets {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hex::encode(hasher.finalize());
        println!("{hash}  {name}  ({} bytes)", data.len());
        lines.push(format!("{hash}  {name}"));
    }

    let checksums_path = format!("{}/checksums.txt", dir);
    fs::write(&checksums_path, lines.join("\n") + "\n")?;
    println!("\nWrote {} ({} assets)", checksums_path, assets.len());

    Ok(())
}

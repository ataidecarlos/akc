use sha2::{Sha256, Digest};
use std::env;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <release-dir>", args[0]);
        eprintln!("Example: {} releases/20260919_1", args[0]);
        std::process::exit(1);
    }

    let dir = &args[1];
    if !Path::new(dir).is_dir() {
        eprintln!("Error: {} is not a directory", dir);
        std::process::exit(1);
    }

    let binaries = ["akc.exe", "akc-x86_64", "akc-arm64"];
    let mut lines: Vec<String> = Vec::new();

    for name in &binaries {
        let path = format!("{}/{}", dir, name);
        if !Path::new(&path).exists() {
            continue;
        }
        let data = fs::read(&path)?;
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let hash = hex::encode(hasher.finalize());
        lines.push(format!("{}  {}", hash, name));
        println!("{}  {}  ({})", hash, name, data.len());
    }

    if lines.is_empty() {
        eprintln!("No binaries found in {}", dir);
        std::process::exit(1);
    }

    let checksums_path = format!("{}/checksums.txt", dir);
    fs::write(&checksums_path, lines.join("\n") + "\n")?;
    println!("\nWrote {}", checksums_path);

    Ok(())
}

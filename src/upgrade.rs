use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use semver::Version;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

const GITHUB_OWNER: &str = "ataidecarlos";
const GITHUB_REPO: &str = "akc";

struct ReleaseInfo {
    version: Version,
    binary_name: String,
    download_url: String,
    checksum_url: String,
}

pub fn run(force: bool, yes: bool, version: Option<&str>) -> Result<()> {
    let current_str = env!("CARGO_PKG_VERSION");
    let current = Version::parse(current_str)?;
    println!("Current version: {}", current);

    // Resolve the asset name up front so an unsupported platform fails immediately
    // instead of after a GitHub round-trip.
    let binary_name = get_platform_binary_name()?;

    let release = match version {
        Some(v) => {
            let v = v.strip_prefix('v').unwrap_or(v);
            let target = Version::parse(v)?;
            println!("Target version: {}", target);
            get_release_by_tag(&format!("v{}", target), &binary_name)?
        }
        None => {
            let release = get_latest_release(&binary_name)?;
            println!("Latest version: {}", release.version);
            release
        }
    };

    if !force && version.is_none() && current >= release.version {
        println!("Already up to date.");
        return Ok(());
    }

    if !yes {
        let action = if current < release.version {
            "Upgrade"
        } else {
            "Downgrade"
        };
        print!(
            "\n{} from {} to {}? [y/N] ",
            action, current, release.version
        );
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!("Downloading {}...", release.binary_name);
    let binary_data = download_file(&release.download_url)?;

    println!("Verifying checksum...");
    let checksum_data = download_file(&release.checksum_url)?;
    verify_checksum(&binary_data, &checksum_data, &release.binary_name)?;

    println!("Installing...");
    replace_executable(&binary_data)?;

    println!("Successfully upgraded to {}", release.version);
    Ok(())
}

fn make_client() -> Result<Client> {
    Client::builder()
        .user_agent(format!("akc/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")
}

fn get_latest_release(binary_name: &str) -> Result<ReleaseInfo> {
    let client = make_client()?;
    let url = format!(
        "https://api.github.com/repos/{}/{}",
        GITHUB_OWNER, GITHUB_REPO
    );
    let response: serde_json::Value = client
        .get(format!("{}/releases/latest", url))
        .send()
        .context("failed to connect to GitHub")?
        .error_for_status()
        .context("no releases found")?
        .json()
        .context("failed to parse GitHub response")?;

    parse_release(&response, binary_name)
}

fn get_release_by_tag(tag: &str, binary_name: &str) -> Result<ReleaseInfo> {
    let client = make_client()?;
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/tags/{}",
        GITHUB_OWNER, GITHUB_REPO, tag
    );
    let response: serde_json::Value = client
        .get(&url)
        .send()
        .context("failed to connect to GitHub")?
        .error_for_status()
        .context(format!("release {} not found", tag))?
        .json()
        .context("failed to parse GitHub response")?;

    parse_release(&response, binary_name)
}

fn parse_release(response: &serde_json::Value, binary_name: &str) -> Result<ReleaseInfo> {
    let tag_name = response["tag_name"]
        .as_str()
        .context("missing tag_name in release")?;

    let version_str = tag_name.strip_prefix('v').unwrap_or(tag_name);
    let version = Version::parse(version_str)
        .with_context(|| format!("invalid version in tag: {}", tag_name))?;

    let download_url = format!(
        "https://github.com/{}/{}/releases/download/{}/{}",
        GITHUB_OWNER, GITHUB_REPO, tag_name, binary_name
    );
    let checksum_url = format!(
        "https://github.com/{}/{}/releases/download/{}/checksums.txt",
        GITHUB_OWNER, GITHUB_REPO, tag_name
    );

    Ok(ReleaseInfo {
        version,
        binary_name: binary_name.to_string(),
        download_url,
        checksum_url,
    })
}

/// Release asset name for the running platform.
///
/// Asset names are architecture-explicit. A single generic `akc.exe` cannot
/// serve both Windows architectures: the file would silently be the wrong one
/// for half of them.
fn get_platform_binary_name() -> Result<String> {
    let asset = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "akc-windows-x86_64.exe",
        ("windows", "aarch64") => "akc-windows-aarch64.exe",
        ("linux", "x86_64") => "akc-linux-x86_64",
        ("linux", "aarch64") => "akc-linux-aarch64",
        (os, arch) => {
            bail!("akc does not publish a binary for this platform (OS: {os}, arch: {arch})")
        }
    };
    Ok(asset.to_string())
}

fn download_file(url: &str) -> Result<Vec<u8>> {
    let client = make_client()?;
    let response = client.get(url).send()?.error_for_status()?;
    Ok(response.bytes()?.to_vec())
}

fn verify_checksum(binary_data: &[u8], checksum_data: &[u8], binary_name: &str) -> Result<()> {
    let checksums_str = String::from_utf8_lossy(checksum_data);
    let mut expected_hash = None;

    for line in checksums_str.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2 && parts[1] == binary_name {
            expected_hash = Some(parts[0]);
            break;
        }
    }

    let expected_hash =
        expected_hash.with_context(|| format!("{} not found in checksums.txt", binary_name))?;

    let mut hasher = Sha256::new();
    hasher.update(binary_data);
    let actual_hash = hex::encode(hasher.finalize());

    // Constant-time comparison is not meaningful here: both values are public
    // hashes of a public download, not secrets.
    if actual_hash != expected_hash {
        bail!(
            "checksum mismatch!\n  expected: {}\n  actual:   {}",
            expected_hash,
            actual_hash
        );
    }

    Ok(())
}

/// A path next to `exe`: `akc.exe` with `.new.exe` gives `akc.new.exe`.
fn sibling(exe: &Path, suffix: &str) -> Result<PathBuf> {
    let mut name = exe
        .file_stem()
        .context("cannot determine the executable name")?
        .to_os_string();
    name.push(suffix);
    Ok(exe.with_file_name(name))
}

fn replace_executable(binary_data: &[u8]) -> Result<()> {
    let exe_path = std::env::current_exe()?;

    #[cfg(windows)]
    {
        // A running image cannot be overwritten or deleted, so the current
        // binary is renamed aside first. If installing the replacement then
        // fails, the old binary is put back -- the user must never be left
        // without a working akc.
        let new_path = sibling(&exe_path, ".new.exe")?;
        let old_path = sibling(&exe_path, ".old.exe")?;
        std::fs::write(&new_path, binary_data).context("failed to write the new binary")?;

        if let Err(err) = std::fs::rename(&exe_path, &old_path) {
            let _ = std::fs::remove_file(&new_path);
            return Err(err).context("failed to move the running binary aside");
        }

        if let Err(err) = std::fs::rename(&new_path, &exe_path) {
            let rollback = std::fs::rename(&old_path, &exe_path);
            let _ = std::fs::remove_file(&new_path);
            return match rollback {
                Ok(()) => Err(err).context("failed to install the new binary"),
                Err(rb) => bail!(
                    "failed to install the new binary: {err}\n  \
                     the previous binary is at {} (restore it by renaming it back): {rb}",
                    old_path.display()
                ),
            };
        }

        // Expected to fail: this process still holds the old image open. Say so
        // instead of swallowing the error and leaving a mystery file behind.
        if let Err(err) = std::fs::remove_file(&old_path) {
            println!(
                "note: could not remove {} ({err}); delete it at any time",
                old_path.display()
            );
        }
        Ok(())
    }

    #[cfg(unix)]
    {
        let new_path = sibling(&exe_path, ".new")?;
        std::fs::write(&new_path, binary_data).context("failed to write the new binary")?;
        let perms = std::fs::metadata(&exe_path)?.permissions();
        std::fs::set_permissions(&new_path, perms)
            .context("failed to copy permissions to the new binary")?;
        std::fs::rename(&new_path, &exe_path).with_context(|| {
            format!("failed to install the new binary at {}", exe_path.display())
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_is_architecture_explicit() {
        let name = get_platform_binary_name().unwrap();
        assert!(name.contains(std::env::consts::OS), "{name}");
        // A generic name cannot distinguish Windows architectures, so the
        // architecture must appear in the asset name.
        if std::env::consts::OS == "windows" {
            assert!(name.ends_with(".exe"), "{name}");
            assert!(
                name.contains("x86_64") || name.contains("aarch64"),
                "windows asset name must be architecture-explicit: {name}"
            );
        }
    }

    #[test]
    fn checksums_verify_against_matching_name() {
        let data = b"binary contents";
        let hash = hex::encode(Sha256::digest(data));
        let checksums = format!("{hash}  akc-windows-aarch64.exe\n");
        assert!(verify_checksum(data, checksums.as_bytes(), "akc-windows-aarch64.exe").is_ok());
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let checksums = format!("{}  akc.exe\n", "0".repeat(64));
        let err = verify_checksum(b"data", checksums.as_bytes(), "akc.exe")
            .unwrap_err()
            .to_string();
        assert!(err.contains("checksum mismatch"), "{err}");
    }

    #[test]
    fn missing_asset_in_checksums_is_rejected() {
        // A checksums file listing some other asset must not be accepted, even
        // if one of its hashes happens to match.
        let hash = hex::encode(Sha256::digest(b"data"));
        let checksums = format!("{hash}  akc-linux-x86_64\n");
        let err = verify_checksum(b"data", checksums.as_bytes(), "akc.exe")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found in checksums.txt"), "{err}");
    }

    #[test]
    fn sibling_paths_stay_next_to_the_executable() {
        let exe = Path::new("/usr/local/bin/akc");
        assert_eq!(
            sibling(exe, ".new").unwrap(),
            PathBuf::from("/usr/local/bin/akc.new")
        );
        let exe = Path::new(r"C:\tools\akc.exe");
        assert_eq!(
            sibling(exe, ".new.exe").unwrap(),
            PathBuf::from(r"C:\tools\akc.new.exe")
        );
    }
}

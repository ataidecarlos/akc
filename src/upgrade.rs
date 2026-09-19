use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use semver::Version;
use sha2::{Sha256, Digest};
use std::io::Write;

const GITHUB_OWNER: &str = "ataidecarlos";
const GITHUB_REPO: &str = "akc";

struct ReleaseInfo {
    tag_name: String,
    version: Version,
    binary_name: String,
    download_url: String,
    checksum_url: String,
}

pub fn run(force: bool, yes: bool, version: Option<&str>) -> Result<()> {
    let current_str = env!("CARGO_PKG_VERSION");
    let current = Version::parse(current_str)?;
    println!("Current version: {}", current);

    let release = match version {
        Some(v) => {
            let v = v.strip_prefix('v').unwrap_or(v);
            let target = Version::parse(v)?;
            println!("Target version: {}", target);
            get_release_by_tag(&format!("v{}", target))?
        }
        None => {
            let release = get_latest_release()?;
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
        print!("\n{} from {} to {}? [y/N] ", action, current, release.version);
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

fn get_latest_release() -> Result<ReleaseInfo> {
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

    parse_release(&response)
}

fn get_release_by_tag(tag: &str) -> Result<ReleaseInfo> {
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

    parse_release(&response)
}

fn parse_release(response: &serde_json::Value) -> Result<ReleaseInfo> {
    let tag_name = response["tag_name"]
        .as_str()
        .context("missing tag_name in release")?;

    let version_str = tag_name.strip_prefix('v').unwrap_or(tag_name);
    let version = Version::parse(version_str)
        .with_context(|| format!("invalid version in tag: {}", tag_name))?;

    let binary_name = get_platform_binary_name()?;
    let download_url = format!(
        "https://github.com/{}/{}/releases/download/{}/{}",
        GITHUB_OWNER, GITHUB_REPO, tag_name, binary_name
    );
    let checksum_url = format!(
        "https://github.com/{}/{}/releases/download/{}/checksums.txt",
        GITHUB_OWNER, GITHUB_REPO, tag_name
    );

    Ok(ReleaseInfo {
        tag_name: tag_name.to_string(),
        version,
        binary_name,
        download_url,
        checksum_url,
    })
}

fn get_platform_binary_name() -> Result<String> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Ok("akc.exe".to_string());
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok("akc-x86_64".to_string());
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Ok("akc-arm64".to_string());
    }

    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    )))]
    {
        bail!("unsupported platform (OS: {}, arch: {})",
            std::env::consts::OS, std::env::consts::ARCH);
    }
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

    let expected_hash = expected_hash.context(format!(
        "{} not found in checksums.txt",
        binary_name
    ))?;

    let mut hasher = Sha256::new();
    hasher.update(binary_data);
    let actual_hash = hex::encode(hasher.finalize());

    if actual_hash != expected_hash {
        bail!(
            "checksum mismatch!\n  expected: {}\n  actual:   {}",
            expected_hash,
            actual_hash
        );
    }

    Ok(())
}

fn replace_executable(binary_data: &[u8]) -> Result<()> {
    let exe_path = std::env::current_exe()?;
    let temp_path = exe_path.with_extension("akc_new");

    std::fs::write(&temp_path, binary_data)
        .context("failed to write new binary")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&exe_path)?.permissions();
        std::fs::set_permissions(&temp_path, perms)?;
    }

    #[cfg(windows)]
    {
        let old_path = exe_path.with_extension("akc_old");
        std::fs::rename(&exe_path, &old_path)
            .context("failed to rename current binary")?;
        std::fs::rename(&temp_path, &exe_path)
            .context("failed to install new binary")?;
        let _ = std::fs::remove_file(&old_path);
    }

    #[cfg(unix)]
    {
        std::fs::rename(&temp_path, &exe_path)
            .context("failed to install new binary")?;
    }

    Ok(())
}

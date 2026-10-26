//! Where the keychain and the master key live.
//!
//! Resolved from `~/.config/akc/config`, so `akc NAME` works from any process
//! with no arguments and no prompt -- which is the point of the tool. The file
//! is deliberately `key = value` rather than real TOML: two settings do not
//! justify pulling in a parser.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use zeroize::Zeroizing;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub keychain: Option<PathBuf>,
    pub key_file: Option<PathBuf>,
    pub sync_key_file: Option<PathBuf>,
}

impl Config {
    /// Load the config from its default location.
    ///
    /// A missing file is not an error: it just means nothing is configured, and
    /// the paths have to come from flags.
    pub fn load() -> Result<Self> {
        match config_path() {
            Some(path) => Self::load_from(&path),
            None => Ok(Config::default()),
        }
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Config::default());
            }
            Err(err) => {
                return Err(err).with_context(|| format!("cannot read {}", path.display()));
            }
        };

        let mut config = Config::default();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = || format!("{}:{}", path.display(), index + 1);
            let (key, value) = line
                .split_once('=')
                .with_context(|| format!("{}: expected `key = value`", at()))?;
            // Quotes are optional, so both `keychain = "~/v.akc"` and
            // `keychain = ~/v.akc` work.
            let value = expand(value.trim().trim_matches('"'));
            match key.trim() {
                "keychain" => config.keychain = Some(value),
                "key_file" => config.key_file = Some(value),
                "sync_key_file" => config.sync_key_file = Some(value),
                other => bail!("{}: unknown setting `{other}`", at()),
            }
        }
        Ok(config)
    }
}

/// `~/.config/akc`, or the platform equivalent.
///
/// `AKC_CONFIG_DIR` overrides everything, which is what keeps the tests off the
/// developer's real config.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("AKC_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(dir).join("akc"));
    }
    if cfg!(windows)
        && let Some(dir) = std::env::var_os("APPDATA")
    {
        return Some(PathBuf::from(dir).join("akc"));
    }
    home_dir().map(|home| home.join(".config").join("akc"))
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("config"))
}

/// The key file used when nothing else names one.
pub fn default_key_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("key"))
}

/// The sync credential used when nothing else names one.
pub fn default_sync_key_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("sync-key"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Expand a leading `~`, so a config file can be written without absolute paths.
fn expand(value: &str) -> PathBuf {
    let rest = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"));
    match (rest, home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(value),
    }
}

/// Read the master key from a file.
///
/// Only the trailing newline is stripped. Spaces are legitimate password
/// material and must survive, so a key written by `printf` behaves identically
/// to one written by `echo`.
pub fn read_key_file(path: &Path) -> Result<Zeroizing<String>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read key file {}", path.display()))?;
    let key = raw.strip_suffix('\n').unwrap_or(&raw);
    let key = key.strip_suffix('\r').unwrap_or(key);
    ensure!(!key.is_empty(), "key file {} is empty", path.display());
    Ok(Zeroizing::new(key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per call. A timestamp alone is not enough: these tests are fast
    /// enough to run inside the same clock tick, and two of them sharing a
    /// directory means one test reads the other's config file.
    fn unique_dir(prefix: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "akc-{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn write_config(contents: &str) -> PathBuf {
        let dir = unique_dir("config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn reads_both_settings() {
        let path = write_config("keychain = /vault/secrets.akc\nkey_file = /keys/master\n");
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.keychain, Some(PathBuf::from("/vault/secrets.akc")));
        assert_eq!(config.key_file, Some(PathBuf::from("/keys/master")));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn ignores_comments_blanks_and_quotes() {
        let path = write_config(
            "# a comment\n\n  keychain = \"/v.akc\"  \n\n# another\nkey_file = /k\n",
        );
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.keychain, Some(PathBuf::from("/v.akc")));
        assert_eq!(config.key_file, Some(PathBuf::from("/k")));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn reads_the_sync_key_file() {
        let Some(home) = home_dir() else { return };
        let path = write_config("sync_key_file = ~/.config/akc/sync-key\n");
        let config = Config::load_from(&path).unwrap();
        assert_eq!(
            config.sync_key_file,
            Some(home.join(".config/akc/sync-key"))
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let config = Config::load_from(Path::new("/nonexistent/akc/config")).unwrap();
        assert_eq!(config, Config::default());
    }

    /// A typo'd setting must fail loudly rather than be silently ignored, or
    /// the user is left wondering why their path had no effect.
    #[test]
    fn unknown_settings_are_rejected_with_a_location() {
        let path = write_config("keychain = /v\nkeychian = /typo\n");
        let err = Config::load_from(&path).unwrap_err().to_string();
        assert!(err.contains("unknown setting"), "{err}");
        assert!(err.contains("keychian"), "{err}");
        assert!(err.contains(":2"), "should name the line: {err}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_line_without_an_equals_is_rejected() {
        let path = write_config("keychain /v\n");
        let err = Config::load_from(&path).unwrap_err().to_string();
        assert!(err.contains("key = value"), "{err}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn expands_a_leading_tilde() {
        let Some(home) = home_dir() else {
            return;
        };
        let path = write_config("keychain = ~/v.akc\n");
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.keychain, Some(home.join("v.akc")));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    // --- key files ---

    #[test]
    fn key_file_strips_only_the_trailing_newline() {
        let dir = unique_dir("key");
        std::fs::create_dir_all(&dir).unwrap();

        for (name, contents, expected) in [
            ("echo", "secret\n", "secret"),
            ("printf", "secret", "secret"),
            ("crlf", "secret\r\n", "secret"),
            ("spaces", "  secret  \n", "  secret  "),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, contents).unwrap();
            assert_eq!(read_key_file(&path).unwrap().as_str(), expected, "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_key_file_is_rejected() {
        let dir = unique_dir("key-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("key");
        std::fs::write(&path, "\n").unwrap();
        assert!(read_key_file(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

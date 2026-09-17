use std::collections::BTreeMap;
use std::io::{Read, Write};

use anyhow::{Context, ensure};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::crypto;

const FORMAT_VERSION: u32 = 1;
const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Store {
    version: u32,
    secrets: BTreeMap<String, String>,
}

impl Store {
    fn empty() -> Self {
        Store {
            version: FORMAT_VERSION,
            secrets: BTreeMap::new(),
        }
    }

    fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        let store: Store = serde_json::from_slice(bytes)?;
        if store.version != FORMAT_VERSION {
            anyhow::bail!("unsupported keychain format version: {}", store.version);
        }
        Ok(store)
    }

    fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }
}

pub struct Keychain {
    store: Store,
}

impl Keychain {
    pub fn new() -> Self {
        Keychain {
            store: Store::empty(),
        }
    }

    pub fn load(path: &Path, password: &str) -> anyhow::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .with_context(|| format!("cannot open {}", path.display()))?
            .take(MAX_FILE_SIZE + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_SIZE,
            "keychain exceeds 16 MiB limit"
        );
        let plaintext = Zeroizing::new(crypto::decrypt_payload(&bytes, password)?);
        let store = Store::from_bytes(&plaintext)?;
        Ok(Keychain { store })
    }

    pub fn save(&self, path: &Path, password: &str, create: bool) -> anyhow::Result<()> {
        let plaintext = Zeroizing::new(self.store.to_bytes()?);
        ensure!(
            plaintext.len() as u64 + 60 <= MAX_FILE_SIZE,
            "keychain exceeds 16 MiB limit"
        );
        let encrypted = crypto::encrypt_payload(&plaintext, password)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&encrypted)?;
        temp.as_file().sync_all()?;
        if create {
            temp.persist_noclobber(path).with_context(|| {
                format!("cannot create {}; file may already exist", path.display())
            })?;
        } else {
            temp.persist(path).context("cannot replace keychain")?;
        }
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: String) {
        if let Some(mut old) = self.store.secrets.insert(key.to_string(), value) {
            old.zeroize();
        }
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.store.secrets.get(key)
    }

    pub fn delete(&mut self, key: &str) -> bool {
        if let Some((mut name, mut value)) = self.store.secrets.remove_entry(key) {
            name.zeroize();
            value.zeroize();
            true
        } else {
            false
        }
    }

    pub fn keys(&self) -> Vec<&str> {
        self.store.secrets.keys().map(String::as_str).collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.store.secrets.len()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        for (mut key, mut value) in std::mem::take(&mut self.secrets) {
            key.zeroize();
            value.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!(
            "akc-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn save_load_roundtrip() {
        let path = temp_path();
        let mut kc = Keychain::new();
        kc.set("api_key", "value1".to_string());
        kc.set("db", "value2".to_string());
        kc.save(&path, "pw", false).unwrap();

        let loaded = Keychain::load(&path, "pw").unwrap();
        assert_eq!(loaded.get("api_key").map(String::as_str), Some("value1"));
        assert_eq!(loaded.get("db").map(String::as_str), Some("value2"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn save_replaces_existing_file() {
        let path = temp_path();
        let mut kc = Keychain::new();
        kc.set("a", "1".to_string());
        kc.save(&path, "pw", false).unwrap();
        kc.set("b", "2".to_string());
        kc.save(&path, "pw", false).unwrap();

        let loaded = Keychain::load(&path, "pw").unwrap();
        assert_eq!(loaded.len(), 2);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn delete_missing_returns_false() {
        let mut kc = Keychain::new();
        assert!(!kc.delete("nope"));
    }

    #[test]
    fn keys_are_sorted() {
        let mut kc = Keychain::new();
        kc.set("zeta", "v".to_string());
        kc.set("alpha", "v".to_string());
        assert_eq!(kc.keys(), vec!["alpha", "zeta"]);
    }
}

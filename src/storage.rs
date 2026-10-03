use std::collections::BTreeMap;
use std::io::{Read, Write};

use anyhow::{Context, ensure};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::crypto;
use crate::crypto::KdfParams;

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
    /// Argon2 costs this vault was opened with. Preserved on save so that
    /// neither raising nor lowering the default silently rewrites a vault's
    /// strength. Legacy v1 files report the defaults and are migrated to the
    /// current container on their next write.
    params: KdfParams,
}

impl Keychain {
    pub fn new() -> Self {
        Keychain {
            store: Store::empty(),
            params: KdfParams::DEFAULT,
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
            "keychain exceeds {} MiB limit",
            MAX_FILE_SIZE / 1024 / 1024
        );
        let params = crypto::read_params(&bytes)?;
        let plaintext = Zeroizing::new(crypto::decrypt_payload(&bytes, password)?);
        let store = Store::from_bytes(&plaintext)?;
        Ok(Keychain { store, params })
    }

    /// Write the keychain to `path`.
    ///
    /// `create` makes the write exclusive: the target must not already exist.
    /// `akc init` uses it so a keychain can never be clobbered by a race
    /// between the existence check and the write.
    pub fn save(&self, path: &Path, password: &str, create: bool) -> anyhow::Result<()> {
        let plaintext = Zeroizing::new(self.store.to_bytes()?);
        ensure!(
            plaintext.len() as u64 + crypto::MAX_OVERHEAD as u64 <= MAX_FILE_SIZE,
            "keychain exceeds {} MiB limit",
            MAX_FILE_SIZE / 1024 / 1024
        );
        let encrypted = crypto::encrypt_payload_with_params(&plaintext, password, self.params)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&encrypted)?;
        temp.as_file().sync_all()?;
        if create {
            temp.persist_noclobber(path).with_context(|| {
                format!(
                    "cannot create {}; the file may already exist",
                    path.display()
                )
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
    use crate::crypto::{FORMAT_VERSION, HEADER_LEN, MAGIC, TAG_LEN};

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

    fn read_raw(path: &Path) -> Vec<u8> {
        std::fs::read(path).unwrap()
    }

    /// Write a container in the akc 1.x layout, using the shared v1 fixture.
    fn write_legacy_v1(path: &Path, secrets: &[(&str, &str)], password: &str) {
        let mut store = Store::empty();
        for (k, v) in secrets {
            store.secrets.insert((*k).to_string(), (*v).to_string());
        }
        let json = Zeroizing::new(store.to_bytes().unwrap());
        std::fs::write(path, crypto::encrypt_legacy_v1(&json, password)).unwrap();
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

    // --- container format ---

    #[test]
    fn new_keychain_is_written_with_current_container() {
        let path = temp_path();
        Keychain::new().save(&path, "pw", false).unwrap();
        let raw = read_raw(&path);
        assert_eq!(&raw[..8], MAGIC);
        assert_eq!(
            u32::from_le_bytes(raw[8..12].try_into().unwrap()),
            FORMAT_VERSION
        );
        // Header + GCM tag + a minimal JSON store.
        assert!(raw.len() >= HEADER_LEN + TAG_LEN);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn legacy_v1_keychain_loads() {
        let path = temp_path();
        write_legacy_v1(&path, &[("old_key", "old_value")], "pw");
        assert!(crypto::is_legacy(&read_raw(&path)));

        let kc = Keychain::load(&path, "pw").unwrap();
        assert_eq!(kc.get("old_key").map(String::as_str), Some("old_value"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn legacy_v1_keychain_is_migrated_on_save() {
        let path = temp_path();
        write_legacy_v1(&path, &[("old_key", "old_value")], "pw");

        let mut kc = Keychain::load(&path, "pw").unwrap();
        kc.set("new_key", "new_value".to_string());
        kc.save(&path, "pw", false).unwrap();

        let raw = read_raw(&path);
        assert!(!crypto::is_legacy(&raw), "expected the v2 container");
        assert_eq!(&raw[..8], MAGIC);

        // Data survived the migration and the file is still readable.
        let reloaded = Keychain::load(&path, "pw").unwrap();
        assert_eq!(
            reloaded.get("old_key").map(String::as_str),
            Some("old_value")
        );
        assert_eq!(
            reloaded.get("new_key").map(String::as_str),
            Some("new_value")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn legacy_keychain_with_wrong_password_still_fails() {
        let path = temp_path();
        write_legacy_v1(&path, &[("k", "v")], "pw");
        assert!(Keychain::load(&path, "wrong").is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn saving_preserves_non_default_kdf_params() {
        let path = temp_path();
        let cheap = KdfParams {
            m_cost: 8 * 1024,
            t_cost: 1,
            p_cost: 1,
        };
        let json = Zeroizing::new(Store::empty().to_bytes().unwrap());
        let raw = crypto::encrypt_payload_with_params(&json, "pw", cheap).unwrap();
        std::fs::write(&path, &raw).unwrap();

        let kc = Keychain::load(&path, "pw").unwrap();
        kc.save(&path, "pw", false).unwrap();

        // A save must not silently upgrade or downgrade the vault's strength.
        assert_eq!(crypto::read_params(&read_raw(&path)).unwrap(), cheap);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn oversize_store_is_rejected_before_writing() {
        let path = temp_path();
        let mut kc = Keychain::new();
        kc.set("big", "x".repeat(17 * 1024 * 1024));
        let err = kc.save(&path, "pw", false).unwrap_err().to_string();
        assert!(err.contains("exceeds"), "{err}");
        assert!(!path.exists(), "no file should have been created");
    }
}

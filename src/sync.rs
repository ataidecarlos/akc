//! Encrypted cloud sync.
//!
//! The local vault keeps its current v2 container: a sync decrypts it in
//! memory, merges per-secret changes, and saves it again. So there is no new
//! file format, no migration, and every existing command behaves as before.
//!
//! The sync identity is derived from the master key, so there is no second
//! secret to carry between machines: the password (or key file) that opens the
//! vault also opens its online backup. Nothing is stored on the machine besides
//! the server URL.
//!
//! The server only ever holds AES-256-GCM blobs keyed by a PRF of the secret
//! name, so it learns neither names nor values. The entry key, the item ids,
//! and the bearer token are all subkeys of the master key.
//!
//! Merge rules, in order:
//! - Remote wins the name; a differing local value is kept as
//!   `name (conflict <date>)`. Nothing is ever dropped, and both devices pick
//!   the same winner, so they converge.
//! - A secret present on the server but deleted locally is not resurrected; the
//!   deletion is pushed as a tombstone.
//! - Tombstones are kept forever. They are a few dozen bytes, and a device that
//!   was offline for a year still converges.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Context, Result, bail, ensure};
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::config;
use crate::crypto;
use crate::storage::{Keychain, Lock};

const NONCE_LEN: usize = 12;
const ENTRY_TAG_LEN: usize = 16;
const TIMEOUT: Duration = Duration::from_secs(15);
/// Must match `MAX_PUTS` in the Worker: one request carries at most this many.
const MAX_PUTS_PER_REQUEST: usize = 500;

/// A sync identity: the server endpoint plus 32 bytes derived from the master
/// key. Two machines with the same master key compute the same identity, which
/// is what makes the online backup reachable with the password alone.
pub struct SyncKey {
    pub endpoint: String,
    secret: Zeroizing<[u8; 32]>,
}

impl SyncKey {
    pub fn from_master(password: &str, endpoint: &str) -> Result<Self> {
        Ok(Self {
            endpoint: validate_endpoint(endpoint)?,
            secret: Zeroizing::new(crypto::derive_sync_key(password)?),
        })
    }

    /// Cheap-cost construction for tests, where the real derivation cost would
    /// dominate the runtime.
    #[cfg(test)]
    pub fn for_test(password: &str, endpoint: &str) -> Self {
        let params = crypto::KdfParams {
            m_cost: 8 * 1024,
            t_cost: 1,
            p_cost: 1,
        };
        SyncKey {
            endpoint: validate_endpoint(endpoint).unwrap(),
            secret: Zeroizing::new(
                crypto::derive_sync_key_with(password, params).expect("cheap derivation"),
            ),
        }
    }

    /// What goes on the wire as the bearer token. It is a one-way subkey, so a
    /// server that logs it still cannot derive the entry key from it.
    fn auth_token(&self) -> String {
        hex::encode(self.subkey(b"akc-sync-auth"))
    }

    /// The server-facing vault name: a hash of the bearer token, so the token
    /// itself is never stored server-side.
    pub fn vault_id(&self) -> String {
        hex::encode(&Sha256::digest(self.auth_token().as_bytes())[..16])
    }

    /// Domain-separated 32-byte subkeys, so the id function and the entry
    /// cipher never share key material.
    fn subkey(&self, label: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        Hkdf::<Sha256>::from_prk(&*self.secret)
            .expect("32-byte key is a valid PRK")
            .expand(label, &mut out)
            .expect("32 output bytes fit");
        out
    }

    /// A PRF over arbitrary input, used for item ids and value hashes.
    fn prf(&self, label: &[u8], info: &[u8]) -> [u8; 32] {
        let mut key = self.subkey(label);
        let mut out = [0u8; 32];
        Hkdf::<Sha256>::from_prk(&key)
            .expect("32-byte key is a valid PRK")
            .expand(info, &mut out)
            .expect("32 output bytes fit");
        key.zeroize();
        out
    }

    fn id_for(&self, name: &str) -> String {
        hex::encode(&self.prf(b"akc-sync-item-id", name.as_bytes())[..16])
    }

    /// Hash of the value, safe to store in the plaintext state file: it is
    /// keyed, so a low-entropy secret cannot be brute-forced from it.
    fn hash_for(&self, name: &str, value: &str) -> String {
        let mut info = Vec::with_capacity(name.len() + value.len() + 8);
        info.extend_from_slice(&(name.len() as u64).to_be_bytes());
        info.extend_from_slice(name.as_bytes());
        info.extend_from_slice(value.as_bytes());
        hex::encode(&self.prf(b"akc-sync-item-hash", &info)[..16])
    }

    fn entry_key(&self) -> [u8; 32] {
        self.subkey(b"akc-sync-entry")
    }

    fn seal(&self, name: &str, value: &str) -> Result<String> {
        let plaintext = Zeroizing::new(serde_json::to_vec(&WireItem {
            name: name.to_string(),
            value: value.to_string(),
        })?);
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let mut key = self.entry_key();
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        key.zeroize();
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
            .map_err(|_| anyhow::anyhow!("encryption failed"))?;
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(hex::encode(blob))
    }

    fn open(&self, blob: &str) -> Result<(String, String)> {
        let raw = hex::decode(blob).context("sync entry is not hex")?;
        ensure!(
            raw.len() >= NONCE_LEN + ENTRY_TAG_LEN,
            "sync entry is truncated"
        );
        let (nonce, ciphertext) = raw.split_at(NONCE_LEN);
        let mut key = self.entry_key();
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        key.zeroize();
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(Nonce::from_slice(nonce), ciphertext)
                .map_err(|_| {
                    anyhow::anyhow!("sync entry cannot be decrypted with this sync key")
                })?,
        );
        let item: WireItem =
            serde_json::from_slice(&plaintext).context("sync entry has an unknown format")?;
        Ok((item.name, item.value))
    }
}

fn validate_endpoint(endpoint: &str) -> Result<String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    ensure!(!endpoint.is_empty(), "sync endpoint is empty");
    ensure!(
        endpoint.starts_with("https://")
            || endpoint.starts_with("http://127.0.0.1")
            || endpoint.starts_with("http://localhost"),
        "sync endpoint must use https:// (http://localhost is allowed for development)"
    );
    Ok(endpoint.to_string())
}

#[derive(Serialize, Deserialize)]
struct WireItem {
    name: String,
    value: String,
}

/// One entry as sent to the server. `blob` is absent for a tombstone: the
/// deleting side knows the name, but no other device needs it to delete its own
/// copy -- it matches the id against its local names.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Put {
    pub id: String,
    pub updated_at: i64,
    pub deleted: bool,
    pub hash: String,
    pub blob: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Change {
    pub id: String,
    pub seq: u64,
    pub updated_at: i64,
    pub deleted: bool,
    pub blob: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Request {
    since: u64,
    puts: Vec<Put>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Response {
    pub seq: u64,
    pub changes: Vec<Change>,
}

/// Storage the merge logic needs. Two implementations: HTTP for real use, and
/// memory for tests, which keeps the merge testable without a server.
pub trait Backend {
    fn sync(&self, vault: &str, since: u64, puts: Vec<Put>) -> Result<Response>;

    /// Drop everything the server holds for this vault. Used after `--rotate`:
    /// the identity follows the master key, so the retired password must not
    /// keep a readable online copy.
    fn forget(&self, vault: &str) -> Result<()>;
}

pub struct HttpBackend {
    endpoint: String,
    token: String,
    client: reqwest::blocking::Client,
}

impl HttpBackend {
    pub fn new(key: &SyncKey) -> Result<Self> {
        Ok(HttpBackend {
            endpoint: key.endpoint.clone(),
            token: key.auth_token(),
            client: reqwest::blocking::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .context("cannot build the HTTP client")?,
        })
    }
}

impl Backend for HttpBackend {
    fn sync(&self, _vault: &str, since: u64, puts: Vec<Put>) -> Result<Response> {
        let response = self
            .client
            .post(format!("{}/v1/sync", self.endpoint))
            .bearer_auth(&self.token)
            .json(&Request { since, puts })
            .send()
            .with_context(|| format!("cannot reach the sync server at {}", self.endpoint))?;
        let status = response.status();
        if status == reqwest::StatusCode::GONE {
            bail!(
                "the synced vault was retired by a key rotation on another machine: \
                 run `akc --rotate --new-password <new>` here to move this machine \
                 to the new key, then `akc --sync`"
            );
        }
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            bail!("sync server returned {status}: {}", body.trim());
        }
        response
            .json()
            .context("sync server returned an unreadable response")
    }

    fn forget(&self, _vault: &str) -> Result<()> {
        let response = self
            .client
            .delete(format!("{}/v1/sync", self.endpoint))
            .bearer_auth(&self.token)
            .send()
            .with_context(|| format!("cannot reach the sync server at {}", self.endpoint))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            bail!("sync server returned {status}: {}", body.trim());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Default)]
struct State {
    /// Which sync identity this cursor belongs to. A cursor is a sequence
    /// number in one server-side vault, so reusing it after the identity
    /// changes (a rotation, or an older build's token) would skip changes.
    #[serde(default)]
    vault: Option<String>,
    /// Highest server sequence already seen.
    cursor: u64,
    /// Item id -> value hash last seen both locally and on the server. Only
    /// hashes are stored, so the state file never leaks a secret name.
    #[serde(default)]
    synced: BTreeMap<String, String>,
}

impl State {
    fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("cannot parse sync state {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(err) => Err(err).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut temp =
            tempfile::NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
        temp.write_all(&serde_json::to_vec(self)?)?;
        temp.as_file().sync_all()?;
        // Windows: an indexer or virus scanner can hold a just-written file for
        // a moment, which makes the replace fail with access-denied. Sync writes
        // this file twice per run, so a short retry is cheaper than a flake.
        let mut attempts = 0;
        loop {
            match temp.persist(path) {
                Ok(_) => return Ok(()),
                Err(err) if attempts < 5 => {
                    attempts += 1;
                    temp = err.file;
                    std::thread::sleep(Duration::from_millis(20 * attempts));
                }
                Err(err) => return Err(err.error).context("cannot write the sync state"),
            }
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Summary {
    pub pulled: usize,
    pub pushed: usize,
    pub conflicts: usize,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.pulled == 0 && self.pushed == 0 {
            return write!(f, "already up to date");
        }
        write!(
            f,
            "synced: {} pulled, {} pushed, {} conflict(s)",
            self.pulled, self.pushed, self.conflicts
        )
    }
}

/// Pull, merge, push, merge again.
///
/// The vault is saved after the pull so a failed upload never discards work
/// that already arrived from the server. Merging the push response again is
/// what catches a write that landed between the two requests; our own writes
/// are no-ops because the value already matches.
pub fn run(
    file: &Path,
    password: &str,
    key: &SyncKey,
    backend: &dyn Backend,
    state_path: &Path,
) -> Result<Summary> {
    let _lock = Lock::acquire(file)?;
    // A machine with no vault yet is how a second device joins: the master key
    // is all it has, so the pull creates the file.
    let created = !file.exists();
    let mut keychain = if created {
        Keychain::new()
    } else {
        Keychain::load(file, password)?
    };
    let vault = key.vault_id();
    let mut state = State::load(state_path)?;
    if state.vault.as_deref() != Some(vault.as_str()) {
        state = State::default();
        state.vault = Some(vault.clone());
    }
    let mut summary = Summary::default();

    let pulled = backend.sync(&vault, state.cursor, Vec::new())?;
    // An empty download on a machine with no vault means the password (or the
    // URL) does not name an existing vault. Creating a new empty one here would
    // turn a typo into a silent, empty "backup".
    if created && pulled.changes.is_empty() {
        bail!(
            "no vault found at {} for this master key: check the password and \
             sync_url, or run `akc --init` to start a new one",
            key.endpoint
        );
    }
    let mut changed = apply_changes(&mut keychain, key, &mut state, &pulled, &mut summary)?;
    state.cursor = state.cursor.max(pulled.seq);
    if changed || created {
        keychain.save(file, password, false)?;
    }
    state.save(state_path)?;

    let mut pending = collect_puts(&keychain, key, &state)?;
    summary.pushed = pending.len();
    // The server rejects a request above its cap, so push a large vault in
    // batches. Each batch is applied before the next one is built.
    while !pending.is_empty() {
        let rest = pending.split_off(pending.len().min(MAX_PUTS_PER_REQUEST));
        let batch = pending;
        pending = rest;

        let pushed = backend.sync(&vault, state.cursor, batch)?;
        changed = apply_changes(&mut keychain, key, &mut state, &pushed, &mut summary)?;
        state.cursor = state.cursor.max(pushed.seq);
        state.save(state_path)?;
        if changed {
            keychain.save(file, password, false)?;
        }
    }
    Ok(summary)
}

fn apply_changes(
    keychain: &mut Keychain,
    key: &SyncKey,
    state: &mut State,
    response: &Response,
    summary: &mut Summary,
) -> Result<bool> {
    let mut changed = false;
    for change in &response.changes {
        changed |= apply_change(keychain, key, state, change, summary)?;
    }
    Ok(changed)
}

fn apply_change(
    keychain: &mut Keychain,
    key: &SyncKey,
    state: &mut State,
    change: &Change,
    summary: &mut Summary,
) -> Result<bool> {
    if change.deleted {
        let Some(name) = local_name_for(keychain, key, &change.id) else {
            // Nothing to delete here: either we never had it, or we already
            // applied this tombstone.
            state.synced.remove(&change.id);
            return Ok(false);
        };
        let value = keychain.get(&name).cloned().unwrap_or_default();
        if !is_synced(key, state, &name, &value) {
            // The local edit is newer than the deletion as far as we can tell.
            // Keep it; the push resurrects the item.
            return Ok(false);
        }
        keychain.delete(&name);
        state.synced.remove(&change.id);
        summary.pulled += 1;
        return Ok(true);
    }

    let blob = change
        .blob
        .as_deref()
        .context("sync server sent an entry with no data")?;
    let (name, value) = key.open(blob)?;
    // The server could otherwise move one secret into another slot.
    ensure!(
        change.id == key.id_for(&name),
        "sync entry does not match its id"
    );
    let remote_hash = key.hash_for(&name, &value);

    match keychain.get(&name).cloned() {
        None if state.synced.contains_key(&change.id) => {
            // Deleted locally and not yet pushed: the tombstone wins.
            Ok(false)
        }
        None => {
            keychain.set(&name, value);
            state.synced.insert(change.id.clone(), remote_hash);
            summary.pulled += 1;
            Ok(true)
        }
        Some(local) if local == value => {
            state.synced.insert(change.id.clone(), remote_hash);
            Ok(false)
        }
        Some(local) if is_synced(key, state, &name, &local) => {
            keychain.set(&name, value);
            state.synced.insert(change.id.clone(), remote_hash);
            summary.pulled += 1;
            Ok(true)
        }
        Some(local) => {
            // Both sides changed. The server's copy keeps the name so every
            // device picks the same winner; the local copy is preserved under
            // its own name and pushed on this same sync.
            let copy = conflict_name(keychain, &name);
            keychain.set(&copy, local);
            keychain.set(&name, value);
            state.synced.insert(change.id.clone(), remote_hash);
            summary.conflicts += 1;
            Ok(true)
        }
    }
}

/// Everything that is new or changed locally, plus a tombstone for every id
/// that was synced before and is gone now.
fn collect_puts(keychain: &Keychain, key: &SyncKey, state: &State) -> Result<Vec<Put>> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut puts = Vec::new();
    let mut present = BTreeSet::new();

    for name in keychain.keys() {
        let id = key.id_for(name);
        present.insert(id.clone());
        let value = keychain.get(name).expect("name came from keys()");
        let hash = key.hash_for(name, value);
        if state.synced.get(&id) != Some(&hash) {
            puts.push(Put {
                id,
                updated_at: now,
                deleted: false,
                hash,
                blob: Some(key.seal(name, value)?),
            });
        }
    }

    for id in state.synced.keys() {
        if !present.contains(id) {
            puts.push(Put {
                id: id.clone(),
                updated_at: now,
                deleted: true,
                // Anything but the live hash, which the server may already have
                // stored for a previous tombstone.
                hash: "deleted".to_string(),
                blob: None,
            });
        }
    }
    Ok(puts)
}

fn is_synced(key: &SyncKey, state: &State, name: &str, value: &str) -> bool {
    state.synced.get(&key.id_for(name)) == Some(&key.hash_for(name, value))
}

fn local_name_for(keychain: &Keychain, key: &SyncKey, id: &str) -> Option<String> {
    keychain
        .keys()
        .into_iter()
        .find(|name| key.id_for(name) == id)
        .map(str::to_string)
}

/// `name (conflict 2026-10-24 15:03)`, with a counter if that is taken.
fn conflict_name(keychain: &Keychain, name: &str) -> String {
    let base = format!(
        "{name} (conflict {})",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    );
    if keychain.get(&base).is_none() {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base} {suffix}");
        if keychain.get(&candidate).is_none() {
            return candidate;
        }
    }
    unreachable!("an unused conflict name always exists")
}

/// `~/.config/akc/sync-state.json`: one sync key per machine, so one cursor.
pub fn default_state_path() -> Result<PathBuf> {
    config::config_dir()
        .map(|dir| dir.join("sync-state.json"))
        .context("cannot determine the config directory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Vault {
        seq: u64,
        rows: HashMap<String, Row>,
    }

    struct Row {
        seq: u64,
        updated_at: i64,
        deleted: bool,
        hash: String,
        blob: Option<String>,
    }

    #[derive(Default)]
    struct Memory {
        vaults: Mutex<HashMap<String, Vault>>,
        /// Mirrors the Worker's `vaults` table: identities retired by a rotation.
        superseded: Mutex<std::collections::HashSet<String>>,
    }

    impl Memory {
        fn rows(&self, vault: &str) -> Vec<(String, Row)> {
            let vaults = self.vaults.lock().unwrap();
            vaults
                .get(vault)
                .map(|v| {
                    v.rows
                        .iter()
                        .map(|(id, r)| {
                            (
                                id.clone(),
                                Row {
                                    seq: r.seq,
                                    updated_at: r.updated_at,
                                    deleted: r.deleted,
                                    hash: r.hash.clone(),
                                    blob: r.blob.clone(),
                                },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    impl Backend for Memory {
        fn sync(&self, vault: &str, since: u64, puts: Vec<Put>) -> Result<Response> {
            anyhow::ensure!(
                puts.len() <= MAX_PUTS_PER_REQUEST,
                "too many entries in one request"
            );
            anyhow::ensure!(
                !self.superseded.lock().unwrap().contains(vault),
                "the synced vault was retired by a key rotation on another machine"
            );
            let mut vaults = self.vaults.lock().unwrap();
            let entry = vaults.entry(vault.to_string()).or_default();
            for put in puts {
                if entry
                    .rows
                    .get(&put.id)
                    .map(|row| row.hash == put.hash)
                    .unwrap_or(false)
                {
                    continue;
                }
                entry.seq += 1;
                entry.rows.insert(
                    put.id,
                    Row {
                        seq: entry.seq,
                        updated_at: put.updated_at,
                        deleted: put.deleted,
                        hash: put.hash,
                        blob: put.blob,
                    },
                );
            }
            let mut changes: Vec<Change> = entry
                .rows
                .iter()
                .filter(|(_, row)| row.seq > since)
                .map(|(id, row)| Change {
                    id: id.clone(),
                    seq: row.seq,
                    updated_at: row.updated_at,
                    deleted: row.deleted,
                    blob: row.blob.clone(),
                })
                .collect();
            changes.sort_by_key(|change| change.seq);
            Ok(Response {
                seq: entry.seq,
                changes,
            })
        }

        fn forget(&self, vault: &str) -> Result<()> {
            self.vaults.lock().unwrap().remove(vault);
            self.superseded
                .lock()
                .unwrap()
                .insert(vault.to_string());
            Ok(())
        }
    }

    struct Device {
        vault: PathBuf,
        state: PathBuf,
        key: SyncKey,
    }

    fn root(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "akc-sync-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Every device that shares a master key shares the sync identity; that is
    /// the whole point of deriving it from the password.
    fn device(dir: &Path, name: &str, password: &str) -> Device {
        let base = dir.join(name);
        std::fs::create_dir_all(&base).unwrap();
        Device {
            vault: base.join("v.akc"),
            state: base.join("state.json"),
            key: SyncKey::for_test(password, "https://sync.test"),
        }
    }

    fn init(device: &Device, password: &str) {
        let params = crate::crypto::KdfParams {
            m_cost: 8 * 1024,
            t_cost: 1,
            p_cost: 1,
        };
        Keychain::new_with_params(params)
            .save(&device.vault, password, false)
            .unwrap();
    }

    fn set(device: &Device, password: &str, name: &str, value: &str) {
        let mut keychain = Keychain::load(&device.vault, password).unwrap();
        keychain.set(name, value.to_string());
        keychain.save(&device.vault, password, false).unwrap();
    }

    fn delete(device: &Device, password: &str, name: &str) {
        let mut keychain = Keychain::load(&device.vault, password).unwrap();
        assert!(keychain.delete(name));
        keychain.save(&device.vault, password, false).unwrap();
    }

    fn sync(device: &Device, password: &str, backend: &dyn Backend) -> Summary {
        run(&device.vault, password, &device.key, backend, &device.state).unwrap()
    }

    fn get(device: &Device, password: &str, name: &str) -> Option<String> {
        Keychain::load(&device.vault, password)
            .unwrap()
            .get(name)
            .cloned()
    }

    fn keys(device: &Device, password: &str) -> Vec<String> {
        Keychain::load(&device.vault, password)
            .unwrap()
            .keys()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    // --- identity ---

    #[test]
    fn the_same_password_names_the_same_vault() {
        let a = SyncKey::for_test("pass", "https://x.test");
        let b = SyncKey::for_test("pass", "https://x.test/");
        assert_eq!(a.vault_id(), b.vault_id());
    }

    #[test]
    fn different_passwords_are_different_vaults() {
        let a = SyncKey::for_test("one", "https://x.test");
        let b = SyncKey::for_test("two", "https://x.test");
        assert_ne!(a.vault_id(), b.vault_id());
    }

    #[test]
    fn plain_http_is_rejected_except_on_localhost() {
        assert!(SyncKey::from_master("pass", "http://example.test").is_err());
        assert!(SyncKey::from_master("pass", "http://localhost:8787").is_ok());
        assert!(SyncKey::from_master("pass", "https://example.test").is_ok());
    }

    #[test]
    fn the_wire_token_is_not_the_encryption_key() {
        let key = SyncKey::for_test("pass", "https://x.test");
        let auth = key.auth_token();
        assert_ne!(auth, hex::encode(&*key.secret));
        // The server hashes the bearer it receives; the client agrees.
        assert_eq!(
            key.vault_id(),
            hex::encode(&Sha256::digest(auth.as_bytes())[..16])
        );
    }

    // --- sync ---

    /// The whole point of the derived identity: a machine with no vault file,
    /// no token, and no state pulls everything with the master password alone.
    #[test]
    fn a_fresh_device_joins_with_the_password_alone() {
        let dir = root("move");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        let b = device(&dir, "b", "pw");
        init(&a, "pw");
        set(&a, "pw", "api_key", "sk-123");

        let summary = sync(&a, "pw", &backend);
        assert_eq!(summary.pushed, 1);

        // B has no vault file at all.
        assert!(!b.vault.exists());
        sync(&b, "pw", &backend);
        assert_eq!(get(&b, "pw", "api_key").as_deref(), Some("sk-123"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A typo in the password names a different vault, and an empty one. It must
    /// not be created as a "successful" empty backup.
    #[test]
    fn a_fresh_device_with_the_wrong_password_creates_nothing() {
        let dir = root("wrong-pw");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        let b = device(&dir, "b", "typo");
        init(&a, "pw");
        set(&a, "pw", "k", "v");
        sync(&a, "pw", &backend);

        let err = run(&b.vault, "typo", &b.key, &backend, &b.state)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no vault found"), "{err}");
        assert!(!b.vault.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_second_sync_is_a_no_op() {
        let dir = root("idempotent");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        init(&a, "pw");
        set(&a, "pw", "k", "v");

        sync(&a, "pw", &backend);
        let summary = sync(&a, "pw", &backend);
        assert_eq!(summary, Summary::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_delete_propagates_and_stays_deleted() {
        let dir = root("delete");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        let b = device(&dir, "b", "pw");
        init(&a, "pw");
        init(&b, "pw");
        set(&a, "pw", "gone", "value");

        sync(&a, "pw", &backend);
        sync(&b, "pw", &backend);
        assert!(get(&b, "pw", "gone").is_some());

        delete(&a, "pw", "gone");
        sync(&a, "pw", &backend);
        sync(&b, "pw", &backend);
        assert_eq!(get(&b, "pw", "gone"), None);

        // A full round trip later, the tombstone must not resurrect it.
        sync(&a, "pw", &backend);
        sync(&b, "pw", &backend);
        assert_eq!(get(&b, "pw", "gone"), None);
        assert!(!keys(&a, "pw").contains(&"gone".to_string()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn concurrent_edits_keep_both_values_and_converge() {
        let dir = root("conflict");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        let b = device(&dir, "b", "pw");
        init(&a, "pw");
        init(&b, "pw");
        set(&a, "pw", "k", "one");
        sync(&a, "pw", &backend);
        sync(&b, "pw", &backend);

        // Both offline edits, pushed in one order.
        set(&a, "pw", "k", "from-a");
        sync(&a, "pw", &backend);
        set(&b, "pw", "k", "from-b");
        let summary = sync(&b, "pw", &backend);

        assert_eq!(summary.conflicts, 1);
        assert_eq!(
            get(&b, "pw", "k").as_deref(),
            Some("from-a"),
            "server copy wins the name"
        );
        let b_keys = keys(&b, "pw");
        assert!(
            b_keys
                .iter()
                .any(|name| name != "k" && get(&b, "pw", name).as_deref() == Some("from-b")),
            "the losing value must survive: {b_keys:?}"
        );

        // A gets the preserved value; repeated syncs add no more conflicts.
        sync(&a, "pw", &backend);
        sync(&b, "pw", &backend);
        let before = keys(&b, "pw").len();
        sync(&a, "pw", &backend);
        sync(&b, "pw", &backend);
        assert_eq!(
            keys(&b, "pw").len(),
            before,
            "conflict copies must not accumulate"
        );
        assert!(
            keys(&a, "pw")
                .iter()
                .any(|name| get(&a, "pw", name).as_deref() == Some("from-b"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_local_delete_beats_a_remote_edit() {
        let dir = root("delete-vs-edit");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        let b = device(&dir, "b", "pw");
        init(&a, "pw");
        init(&b, "pw");
        set(&a, "pw", "k", "one");
        sync(&a, "pw", &backend);
        sync(&b, "pw", &backend);

        delete(&b, "pw", "k");
        set(&a, "pw", "k", "two");
        sync(&a, "pw", &backend);

        sync(&b, "pw", &backend);
        assert_eq!(get(&b, "pw", "k"), None, "the deletion must not be undone");

        sync(&a, "pw", &backend);
        assert_eq!(get(&a, "pw", "k"), None, "the deletion reaches the editor");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_different_password_cannot_read_the_vault() {
        let dir = root("isolation");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        let stranger = device(&dir, "s", "other");
        init(&a, "pw");
        init(&stranger, "other");
        set(&a, "pw", "k", "secret");

        sync(&a, "pw", &backend);
        sync(&stranger, "other", &backend);
        assert!(keys(&stranger, "other").is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_server_never_sees_names_or_values() {
        let dir = root("opaque");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        init(&a, "pw");
        set(&a, "pw", "db_password", "hunter2");
        sync(&a, "pw", &backend);

        let rows = backend.rows(&a.key.vault_id());
        assert_eq!(rows.len(), 1);
        let (id, row) = &rows[0];
        let blob = row.blob.as_deref().unwrap();
        assert!(!blob.contains("hunter2"));
        assert!(!blob.contains(&hex::encode("hunter2")));
        assert!(!id.contains("db_password"));
        assert!(!blob.contains("db_password"));
        // The id is a PRF of the name, not the name itself.
        assert_eq!(id, &a.key.id_for("db_password"));

        // The state file stores only keyed hashes.
        let state = std::fs::read_to_string(&a.state).unwrap();
        assert!(!state.contains("db_password"));
        assert!(!state.contains("hunter2"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_large_vault_is_pushed_in_batches() {
        let dir = root("batches");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        let b = device(&dir, "b", "pw");
        init(&a, "pw");
        init(&b, "pw");

        let size = MAX_PUTS_PER_REQUEST + 120;
        let mut keychain = Keychain::load(&a.vault, "pw").unwrap();
        for i in 0..size {
            keychain.set(&format!("k{i}"), format!("v{i}"));
        }
        keychain.save(&a.vault, "pw", false).unwrap();

        // The in-memory backend enforces the same cap as the Worker.
        assert_eq!(sync(&a, "pw", &backend).pushed, size);

        sync(&b, "pw", &backend);
        let last = format!("k{}", size - 1);
        let expected = format!("v{}", size - 1);
        assert_eq!(get(&b, "pw", &last).as_deref(), Some(expected.as_str()));
        assert_eq!(keys(&b, "pw").len(), size);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_state_file_survives_a_round_trip() {
        let dir = root("state");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        init(&a, "pw");
        set(&a, "pw", "k", "v");
        sync(&a, "pw", &backend);

        let text = std::fs::read_to_string(&a.state).unwrap();
        let state: State = serde_json::from_str(&text).unwrap();
        assert!(state.cursor > 0);
        assert_eq!(state.synced.len(), 1);
        assert_eq!(state.vault.as_deref(), Some(a.key.vault_id().as_str()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// After a rotation the identity changes, so the old cursor (a sequence
    /// number in a different server-side vault) must not be reused: it could
    /// skip changes that arrived under the new identity.
    #[test]
    fn a_cursor_from_another_identity_is_discarded() {
        let dir = root("identity");
        let backend = Memory::default();
        let a = device(&dir, "a", "new");
        let sender = device(&dir, "s", "new");
        init(&a, "new");
        init(&sender, "new");
        set(&sender, "new", "k", "v");
        sync(&sender, "new", &backend);

        // What a pre-rotation state file looks like: a cursor far ahead of this
        // vault's sequence, and hashes under a key that no longer exists.
        let stale = State {
            vault: Some("previous-identity".to_string()),
            cursor: 999,
            synced: std::collections::BTreeMap::from([("old-id".to_string(), "h".to_string())]),
        };
        stale.save(&a.state).unwrap();

        let summary = sync(&a, "new", &backend);
        assert_eq!(summary.pulled, 1, "the stale cursor skipped a change");
        assert_eq!(get(&a, "new", "k").as_deref(), Some("v"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forget_drops_the_remote_vault() {
        let dir = root("forget");
        let backend = Memory::default();
        let a = device(&dir, "a", "pw");
        init(&a, "pw");
        set(&a, "pw", "k", "v");
        sync(&a, "pw", &backend);
        assert!(!backend.rows(&a.key.vault_id()).is_empty());

        backend.forget(&a.key.vault_id()).unwrap();
        assert!(backend.rows(&a.key.vault_id()).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A machine still on the retired key must not repopulate the vault another
    /// machine just rotated away from, and must not touch its local vault.
    #[test]
    fn a_retired_identity_cannot_sync() {
        let dir = root("retired");
        let backend = Memory::default();
        let a = device(&dir, "a", "old");
        init(&a, "old");
        set(&a, "old", "k", "v");
        sync(&a, "old", &backend);

        // What `--rotate` on another machine does to this identity.
        backend.forget(&a.key.vault_id()).unwrap();
        let before = std::fs::read(&a.vault).unwrap();

        let err = run(&a.vault, "old", &a.key, &backend, &a.state)
            .unwrap_err()
            .to_string();
        assert!(err.contains("retired"), "{err}");
        assert_eq!(std::fs::read(&a.vault).unwrap(), before, "local vault changed");
        assert!(backend.rows(&a.key.vault_id()).is_empty(), "stale copy recreated");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::Argon2;
use rand::RngCore;
use zeroize::Zeroize;

use anyhow::{Context, ensure};

pub const SALT_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;
pub const KEY_LEN: usize = 32;

/// Cleartext marker identifying an akc container.
///
/// Long enough that a legacy (v1) file -- whose first bytes are a random salt --
/// colliding with it is negligible.
pub(crate) const MAGIC: &[u8; 8] = b"AKCSTORE";

/// Container version written by this build.
///
/// v1 had no header at all: `salt(32) + nonce(12) + ciphertext`. v2 prefixes a
/// cleartext header carrying the KDF parameters, so a vault written with
/// non-default Argon2 costs can still be opened, and costs can be raised later.
pub const FORMAT_VERSION: u32 = 2;

const MAGIC_LEN: usize = 8;
const VERSION_LEN: usize = 4;
const COST_LEN: usize = 4;

/// Header layout: magic(8) version(4) nonce(12) m(4) t(4) p(4) salt(32).
pub const HEADER_LEN: usize =
    MAGIC_LEN + VERSION_LEN + NONCE_LEN + COST_LEN + COST_LEN + COST_LEN + SALT_LEN;

const OFF_VERSION: usize = MAGIC_LEN;
const OFF_NONCE: usize = OFF_VERSION + VERSION_LEN;
const OFF_M_COST: usize = OFF_NONCE + NONCE_LEN;
const OFF_T_COST: usize = OFF_M_COST + COST_LEN;
const OFF_P_COST: usize = OFF_T_COST + COST_LEN;
const OFF_SALT: usize = OFF_P_COST + COST_LEN;

/// Length of a v1 header: salt + nonce.
const LEGACY_HEADER_LEN: usize = SALT_LEN + NONCE_LEN;

/// AES-GCM authentication tag appended to every ciphertext.
pub(crate) const TAG_LEN: usize = 16;

/// Worst-case bytes a container adds on top of its plaintext.
pub const MAX_OVERHEAD: usize = HEADER_LEN + TAG_LEN;

pub const PARAM_M_COST: u32 = 64 * 1024;
pub const PARAM_T_COST: u32 = 3;
pub const PARAM_P_COST: u32 = 4;

// Bounds enforced on KDF parameters read back from a file header. Without these
// a hostile file could request a multi-gigabyte allocation and exhaust memory.
const MIN_M_COST: u32 = 8 * 1024;
const MAX_M_COST: u32 = 1024 * 1024;
const MIN_T_COST: u32 = 1;
const MAX_T_COST: u32 = 16;
const MIN_P_COST: u32 = 1;
const MAX_P_COST: u32 = 16;

/// Argon2id cost parameters for a vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParams {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl KdfParams {
    pub const DEFAULT: Self = KdfParams {
        m_cost: PARAM_M_COST,
        t_cost: PARAM_T_COST,
        p_cost: PARAM_P_COST,
    };

    /// Reject parameters that are out of range or would make key derivation
    /// unreasonably slow or memory hungry.
    fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            (MIN_M_COST..=MAX_M_COST).contains(&self.m_cost),
            "keychain requests an unsupported memory cost ({} KiB); supported range is {MIN_M_COST}..={MAX_M_COST}",
            self.m_cost
        );
        ensure!(
            (MIN_T_COST..=MAX_T_COST).contains(&self.t_cost),
            "keychain requests an unsupported time cost ({}); supported range is {MIN_T_COST}..={MAX_T_COST}",
            self.t_cost
        );
        ensure!(
            (MIN_P_COST..=MAX_P_COST).contains(&self.p_cost),
            "keychain requests an unsupported parallelism ({}); supported range is {MIN_P_COST}..={MAX_P_COST}",
            self.p_cost
        );
        Ok(())
    }
}

fn derive_key(password: &str, salt: &[u8], params: &KdfParams) -> anyhow::Result<[u8; KEY_LEN]> {
    params.validate()?;
    let argon2_params =
        argon2::Params::new(params.m_cost, params.t_cost, params.p_cost, Some(KEY_LEN))
            .context("invalid Argon2 parameters")?;
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2_params,
    );
    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .context("Argon2id key derivation failed")?;
    Ok(key)
}

fn make_cipher(key: &mut [u8; KEY_LEN]) -> Aes256Gcm {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    key.zeroize();
    cipher
}

fn auth_failure() -> anyhow::Error {
    anyhow::anyhow!("wrong password or corrupted file")
}

pub fn encrypt_payload_with_params(
    payload: &[u8],
    password: &str,
    params: KdfParams,
) -> anyhow::Result<Vec<u8>> {
    params.validate()?;

    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let mut key = derive_key(password, &salt, &params)?;
    let ciphertext = {
        let cipher = make_cipher(&mut key);
        cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), payload)
            .map_err(|_| anyhow::anyhow!("encryption failed"))?
    };

    let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&params.m_cost.to_le_bytes());
    out.extend_from_slice(&params.t_cost.to_le_bytes());
    out.extend_from_slice(&params.p_cost.to_le_bytes());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// True when `data` uses the headerless v1 layout.
///
/// Call this before [`decrypt_payload`] to decide whether a vault needs
/// migrating to the current container on its next write.
pub fn is_legacy(data: &[u8]) -> bool {
    data.len() < MAGIC_LEN || &data[..MAGIC_LEN] != MAGIC
}

/// KDF parameters recorded in a container header, or the defaults for a v1 file.
pub fn read_params(data: &[u8]) -> anyhow::Result<KdfParams> {
    if is_legacy(data) {
        return Ok(KdfParams::DEFAULT);
    }
    let params = KdfParams {
        m_cost: read_u32(data, OFF_M_COST),
        t_cost: read_u32(data, OFF_T_COST),
        p_cost: read_u32(data, OFF_P_COST),
    };
    params.validate()?;
    Ok(params)
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        data[offset..offset + COST_LEN]
            .try_into()
            .expect("4-byte slice"),
    )
}

/// Decrypt a container of either version.
pub fn decrypt_payload(data: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
    if is_legacy(data) {
        decrypt_v1(data, password)
    } else {
        decrypt_v2(data, password)
    }
}

fn decrypt_v1(data: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
    ensure!(
        data.len() >= LEGACY_HEADER_LEN + TAG_LEN,
        "file is truncated or not a keychain file"
    );
    let (salt, rest) = data.split_at(SALT_LEN);
    let (nonce_bytes, ciphertext) = rest.split_at(NONCE_LEN);
    let mut key = derive_key(password, salt, &KdfParams::DEFAULT)?;
    let cipher = make_cipher(&mut key);
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| auth_failure())
}

fn decrypt_v2(data: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
    ensure!(
        data.len() >= HEADER_LEN + TAG_LEN,
        "file is truncated or not a keychain file"
    );
    let version = read_u32(data, OFF_VERSION);
    ensure!(
        version == FORMAT_VERSION,
        "unsupported keychain format version: {version} (this build reads version {FORMAT_VERSION})"
    );

    let params = read_params(data)?;
    let mut key = derive_key(password, &data[OFF_SALT..OFF_SALT + SALT_LEN], &params)?;
    let cipher = make_cipher(&mut key);
    cipher
        .decrypt(
            Nonce::from_slice(&data[OFF_NONCE..OFF_NONCE + NONCE_LEN]),
            &data[HEADER_LEN..],
        )
        .map_err(|_| auth_failure())
}

/// Build a container in the legacy v1 layout (`salt(32) + nonce(12) + ct`), as
/// written by akc 1.x.
///
/// Test-only: it exists so the backward-compatibility path can be exercised
/// without checking a real 1.x vault into the repository. Keeping the layout in
/// one place stops the fixture from drifting away from [`decrypt_v1`].
#[cfg(test)]
pub(crate) fn encrypt_legacy_v1(payload: &[u8], password: &str) -> Vec<u8> {
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let mut key = derive_key(password, &salt, &KdfParams::DEFAULT).unwrap();
    let ciphertext = {
        let cipher = make_cipher(&mut key);
        cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), payload)
            .unwrap()
    };
    let mut out = Vec::new();
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use encrypt_legacy_v1 as encrypt_v1;

    /// Encrypt with the default cost parameters.
    fn encrypt_payload(payload: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
        encrypt_payload_with_params(payload, password, KdfParams::DEFAULT)
    }

    #[test]
    fn roundtrip() {
        let data = b"hello world";
        let encrypted = encrypt_payload(data, "pass").unwrap();
        let decrypted = decrypt_payload(&encrypted, "pass").unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn wrong_password_fails() {
        let encrypted = encrypt_payload(b"secret", "right").unwrap();
        assert!(decrypt_payload(&encrypted, "wrong").is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let mut encrypted = encrypt_payload(b"secret", "pass").unwrap();
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;
        assert!(decrypt_payload(&encrypted, "pass").is_err());
    }

    #[test]
    fn tampered_salt_fails() {
        let mut encrypted = encrypt_payload(b"secret", "pass").unwrap();
        encrypted[OFF_SALT] ^= 0xFF;
        assert!(decrypt_payload(&encrypted, "pass").is_err());
    }

    #[test]
    fn truncated_file_fails() {
        assert!(decrypt_payload(&[0u8; 10], "pass").is_err());
    }

    #[test]
    fn different_salts_produce_different_ciphertexts() {
        let a = encrypt_payload(b"same", "pass").unwrap();
        let b = encrypt_payload(b"same", "pass").unwrap();
        assert_ne!(a, b);
    }

    // --- container header ---

    #[test]
    fn header_records_magic_version_and_params() {
        let encrypted = encrypt_payload(b"x", "pass").unwrap();
        assert_eq!(&encrypted[..MAGIC_LEN], MAGIC);
        assert_eq!(read_u32(&encrypted, OFF_VERSION), FORMAT_VERSION);
        assert!(!is_legacy(&encrypted));
        assert_eq!(read_params(&encrypted).unwrap(), KdfParams::DEFAULT);
        assert_eq!(encrypted.len(), HEADER_LEN + 1 + TAG_LEN);
    }

    #[test]
    fn non_default_params_roundtrip() {
        // Cheaper costs keep the test fast; the point is that the header, not a
        // hardcoded constant, drives key derivation.
        let params = KdfParams {
            m_cost: MIN_M_COST,
            t_cost: 1,
            p_cost: 1,
        };
        let encrypted = encrypt_payload_with_params(b"payload", "pass", params).unwrap();
        assert_eq!(read_params(&encrypted).unwrap(), params);
        assert_eq!(decrypt_payload(&encrypted, "pass").unwrap(), b"payload");
    }

    #[test]
    fn out_of_range_params_are_rejected_before_allocating() {
        let mut encrypted = encrypt_payload(b"x", "pass").unwrap();
        encrypted[OFF_M_COST..OFF_M_COST + COST_LEN].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = read_params(&encrypted).unwrap_err().to_string();
        assert!(err.contains("memory cost"), "{err}");
        // Must fail on the bounds check, not by attempting the allocation.
        assert!(decrypt_payload(&encrypted, "pass").is_err());
    }

    #[test]
    fn absurd_parallelism_is_rejected() {
        let mut encrypted = encrypt_payload(b"x", "pass").unwrap();
        encrypted[OFF_P_COST..OFF_P_COST + COST_LEN].copy_from_slice(&9999u32.to_le_bytes());
        assert!(read_params(&encrypted).is_err());
    }

    #[test]
    fn unknown_future_version_is_rejected() {
        let mut encrypted = encrypt_payload(b"x", "pass").unwrap();
        encrypted[OFF_VERSION..OFF_VERSION + VERSION_LEN].copy_from_slice(&99u32.to_le_bytes());
        let err = decrypt_payload(&encrypted, "pass").unwrap_err().to_string();
        assert!(err.contains("unsupported keychain format version"), "{err}");
    }

    #[test]
    fn header_truncation_is_rejected() {
        let encrypted = encrypt_payload(b"hello", "pass").unwrap();
        // Chop into the header: magic still present, so it must not fall through
        // to the legacy path and report a misleading "wrong password".
        let err = decrypt_payload(&encrypted[..HEADER_LEN], "pass")
            .unwrap_err()
            .to_string();
        assert!(err.contains("truncated"), "{err}");
    }

    // --- backward compatibility ---

    #[test]
    fn legacy_v1_files_still_decrypt() {
        let encrypted = encrypt_v1(b"legacy payload", "pass");
        assert!(is_legacy(&encrypted));
        assert_eq!(
            decrypt_payload(&encrypted, "pass").unwrap(),
            b"legacy payload"
        );
    }

    #[test]
    fn legacy_v1_wrong_password_still_fails() {
        let encrypted = encrypt_v1(b"legacy payload", "pass");
        assert!(decrypt_payload(&encrypted, "wrong").is_err());
    }

    #[test]
    fn legacy_v1_tamper_still_fails() {
        let mut encrypted = encrypt_v1(b"legacy payload", "pass");
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;
        assert!(decrypt_payload(&encrypted, "pass").is_err());
    }

    #[test]
    fn legacy_v1_reports_default_params() {
        assert_eq!(
            read_params(&encrypt_v1(b"x", "pass")).unwrap(),
            KdfParams::DEFAULT
        );
    }

    #[test]
    fn short_file_is_rejected_not_misread_as_legacy() {
        assert!(decrypt_payload(&[0u8; 4], "pass").is_err());
    }
}

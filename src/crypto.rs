use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::Argon2;
use rand::RngCore;
use zeroize::Zeroize;

pub const SALT_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;
pub const KEY_LEN: usize = 32;

pub const PARAM_M_COST: u32 = 64 * 1024;
pub const PARAM_T_COST: u32 = 3;
pub const PARAM_P_COST: u32 = 4;

fn derive_key(password: &str, salt: &[u8]) -> [u8; KEY_LEN] {
    let params = argon2::Params::new(PARAM_M_COST, PARAM_T_COST, PARAM_P_COST, Some(KEY_LEN))
        .expect("static Argon2 parameters are valid");
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .expect("Argon2id derivation into a 32-byte buffer cannot fail");
    key
}

fn make_cipher(key: &mut [u8; KEY_LEN]) -> Aes256Gcm {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    key.zeroize();
    cipher
}

pub fn encrypt_payload(payload: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);

    let mut key = derive_key(password, &salt);
    let cipher = make_cipher(&mut key);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, payload)
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;

    let mut out = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decrypt_payload(data: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
    if data.len() < SALT_LEN + NONCE_LEN {
        anyhow::bail!("file is truncated or not a keychain file");
    }
    let (salt, rest) = data.split_at(SALT_LEN);
    let (nonce_bytes, ciphertext) = rest.split_at(NONCE_LEN);

    let mut key = derive_key(password, salt);
    let cipher = make_cipher(&mut key);

    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("wrong password or corrupted file"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        encrypted[0] ^= 0xFF;
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
}

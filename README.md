# akc

A minimal, portable encrypted keychain. Single binary, single data file.

- **Encrypted file:** `salt(32) + nonce(12) + AES-256-GCM ciphertext` of a JSON store. Both key names and values are encrypted.
- **KDF:** Argon2id (64 MiB, t=3, p=4).
- **Portable:** the `.akc` file works on any OS and is safe to keep in OneDrive/Dropbox (atomic temp-file writes, never partially synced states).
- **Tiny:** ~400 KB static binary, no runtime.

## Usage

```powershell
akc init secrets.akc                  # prompts for password (twice)
akc set secrets.akc api_key sk-123    # add or update
akc get secrets.akc api_key           # print value
akc list secrets.akc                  # list key names only
akc delete secrets.akc api_key
```

Add `--password <pw>` to any command for non-interactive use (otherwise you are prompted with hidden input).

## Build

```powershell
cargo build --release
# target/release/akc(.exe)
```

## Security notes

- Wrong password fails via GCM authentication; truncated or tampered files are rejected.
- Writes go to a temp file in the same directory, then atomically replace the target (`init` refuses to overwrite an existing file).
- Secret material is zeroized from memory when dropped.
- An empty keychain file is ~107 bytes.

# akc

A minimal encrypted secret store. CLI only: no server or network listener. Single executable, single portable data file.

- **Encrypted file:** `salt(32) + nonce(12) + AES-256-GCM ciphertext` of a JSON store. Both key names and values are encrypted.
- **KDF:** Argon2id (64 MiB, t=3, p=4).
- **Portable:** the data file works on any OS and is safe to keep in OneDrive/Dropbox (atomic temp-file writes, never partially synced states). Use any filename; `.akc` is only a suggested convention.
- **Tiny:** ~400 KB static binary, no runtime.

## Installation

### Download Pre-built Binary

Download the latest release from [releases/latest/](releases/latest/):
- `akc.exe` — Windows x64

Or browse all releases in [releases/](releases/).

### Build from Source

```powershell
cargo build --release
# Binary: target/release/akc.exe
```

## Usage

```powershell
akc init secrets.akc                  # prompts for password (twice)
akc set secrets.akc api_key sk-123    # add or update
akc get secrets.akc api_key           # print value
akc list secrets.akc                  # list key names only
akc delete secrets.akc api_key
```

Add `--password <pw>` to any command for non-interactive use (otherwise you are prompted with hidden input).

## Security

- Wrong password fails via GCM authentication; truncated or tampered files are rejected.
- Writes go to a temp file in the same directory, then atomically replace the target (`init` refuses to overwrite an existing file).
- Secret material is zeroized from memory when dropped.
- An empty data file is ~107 bytes.

## Development

- **Source code:** `src/`
- **Testing:** See [test/README.md](test/README.md)
- **Releases:** `releases/` (archive of all versions)
- **Changelog:** [CHANGELOG.md](CHANGELOG.md)

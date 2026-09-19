# akc — 20260919_1

**Release Date:** 2026-09-19

## What's New

Initial release of akc — a minimal encrypted secret store.

## Features

- **5 CLI commands:** `init`, `set`, `get`, `list`, `delete`
- **Strong encryption:** AES-256-GCM with Argon2id key derivation
- **Portable:** Single encrypted file, safe for cloud sync (OneDrive/Dropbox)
- **Tiny:** ~400 KB binary, no runtime dependencies
- **Secure:** Wrong password detection, tamper protection, memory zeroization

## Security

- Encrypted file format: `salt(32) + nonce(12) + AES-256-GCM ciphertext`
- KDF: Argon2id (64 MiB, t=3, p=4)
- Atomic writes prevent corruption
- `init` refuses to overwrite existing files
- Secret material zeroized from memory

## Platforms

**Windows:**
- `akc.exe` — Windows x64

**Linux:**
- `akc-x86_64` — Linux x86_64 (static, musl)
- `akc-arm64` — Linux ARM64 (static, musl)

## Usage

```powershell
akc init secrets.akc                  # prompts for password (twice)
akc set secrets.akc api_key sk-123    # add or update
akc get secrets.akc api_key           # print value
akc list secrets.akc                  # list key names only
akc delete secrets.akc api_key
```

Add `--password <pw>` for non-interactive use.

**Linux setup:**
```bash
chmod +x akc-x86_64  # or akc-arm64
./akc-x86_64 --help
```

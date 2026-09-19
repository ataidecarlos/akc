# akc — 20260919_2

**Release Date:** 2026-09-19
**Version:** 1.1.0

## What's New

- `upgrade` command: check for and install updates from GitHub releases
- `--password` option now visible in `akc --help`

## Features

- **6 CLI commands:** `init`, `set`, `get`, `list`, `delete`, `upgrade`
- **Self-upgrade:** Download and install updates with checksum verification
- **Strong encryption:** AES-256-GCM with Argon2id key derivation
- **Portable:** Single encrypted file, safe for cloud sync
- **Tiny:** ~400-750 KB binary, no runtime dependencies
- **Secure:** Wrong password detection, tamper protection, memory zeroization

## Platforms

**Windows:**
- `akc.exe` — Windows x64

**Linux:**
- `akc-x86_64` — Linux x86_64 (static, musl)
- `akc-arm64` — Linux ARM64 (static, musl)

## Checksums

See `checksums.txt` for SHA256 hashes of all binaries.

## Upgrade

```powershell
akc upgrade                           # check for and install updates
akc upgrade --yes                     # skip confirmation prompt
akc upgrade --force                   # force even if already on latest
akc upgrade --version 1.0.0           # upgrade to a specific version
```

## Usage

```powershell
akc init secrets.akc                  # prompts for password (twice)
akc set secrets.akc api_key sk-123    # add or update
akc get secrets.akc api_key           # print value
akc list secrets.akc                  # list key names only
akc delete secrets.akc api_key
```

Add `--password <pw>` for non-interactive use.

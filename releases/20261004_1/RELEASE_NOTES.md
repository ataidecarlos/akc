# akc — 20261004_1

**Release Date:** 2026-10-04
**Version:** 3.0.0

## What's New

This release fixes a set of bugs found reviewing v2.0.0, and splits password
handling into two distinct operations.

- **`change-password`** — re-key a keychain while keeping every secret. The
  current password is verified first, so a wrong one changes nothing.
- **`init` no longer overwrites.** It refuses an existing keychain, so a typo'd
  password can't lock your secrets away behind one you don't have. Use
  `init --force` when you really mean to discard them.
- **Passwords are confirmed twice.** Both `init` and `change-password` ask you to
  type the password twice. A password passed via `--password`/`AKC_PASSWORD` is
  used as-is, since it can't contain a typo.
- **Values are normalized.** Surrounding whitespace is always removed, and a
  value that is empty afterwards is rejected instead of stored.
- **KDF parameters live in the file header**, so a keychain keeps working if the
  defaults change. Existing 1.x/2.x keychains load normally and migrate on their
  next write.
- **`upgrade` works on Windows ARM64**, and release assets are now named for their
  architecture so you can no longer download a binary your machine can't run.

## Upgrade notes (breaking)

- `init` refuses to overwrite an existing keychain → use `change-password`, or
  `init --force` to discard.
- Values are trimmed, and blank values are rejected.
- No interactive `init` → `change-password` in a session, `init --force` from the
  CLI.
- Release asset names changed → re-download rather than `akc upgrade` from 1.x.

Existing keychains need no migration step: they are read as-is and rewritten in the
current format the first time you change something.

## Features

- **7 CLI commands:** `init`, `change-password`, `set`, `get`, `list`, `delete`, `upgrade`
- **Strong encryption:** AES-256-GCM with Argon2id (64 MiB, t=3, p=4)
- **Portable:** single encrypted file, safe for cloud sync
- **Small:** ~1.8–2.5 MB static binary, no runtime dependencies
- **Secure:** GCM authentication on wrong password, tamper detection, range-checked
  header parameters, memory zeroization

## Platforms

Assets are named for the platform they target. Pick the one matching your machine.

**Windows:**
- `akc-windows-x86_64.exe` — Windows x64
- `akc-windows-aarch64.exe` — Windows ARM64

**Linux:**
- `akc-linux-x86_64` — Linux x86_64 (static, musl)
- `akc-linux-aarch64` — Linux ARM64 (static, musl)

## Checksums

See `checksums.txt` for SHA256 hashes of all binaries. `akc upgrade` verifies the
downloaded binary against it before installing. This protects against a corrupted
or truncated download; it is not a signature, since the checksums are fetched from
the same release as the binary.

## Usage

```powershell
akc secrets.akc init                  # create; asks for the password twice
akc secrets.akc set api_key sk-123    # add or update
akc secrets.akc get api_key           # print value
akc secrets.akc list                  # list key names only
akc secrets.akc delete api_key
akc secrets.akc change-password       # re-key, keeping every secret
```

Add `--password <pw>` or set `AKC_PASSWORD` for non-interactive use.

## Upgrade

```powershell
akc upgrade                           # check for and install updates
akc upgrade --yes                     # skip the confirmation prompt
akc upgrade --force                   # reinstall even if already current
akc upgrade --version 3.0.0           # a specific version
```
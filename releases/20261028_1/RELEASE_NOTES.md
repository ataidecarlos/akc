# akc — 20261028_1

**Build Date:** 2026-10-28
**Version:** 4.0.0 + sync (unreleased)
**Source:** commit `bdff360` ("Derive sync identity from the master key")

## Partial archive

This folder holds **two assets**:

- `akc-windows-aarch64.exe` — **Windows ARM64** (2,040,320 bytes)
- `akc-linux-aarch64` — **Linux ARM64**, static musl (2,429,392 bytes)

The other three assets (`akc-linux-x86_64`, `akc-windows-x86_64.exe`,
`akc-macos-aarch64`) are not built yet. They will be added by builds on the
other machines. `releases/latest` intentionally still points at
[`../20261015_1/`](../20261015_1/), because `akc --update` downloads the asset
for the current platform and a missing one is a 404, not a no-op. Do not point
`releases/latest` here until all five assets exist.

## What's new

Sync no longer needs a second secret.

- **One password opens the vault and its online backup.** The sync identity
  (bearer token, entry key, item ids) is derived from the master password with
  Argon2id, so a new machine needs only the password and `sync_url`.
  `akc --sync` creates the local vault from the download.
- **`--sync-init`, `--sync-join`, `AKC_SYNC_KEY`, and the `sync-key` file are
  removed.** The config gains `sync_url`, overridable with `--sync-url` or
  `AKC_SYNC_URL`.
- **A typo cannot make an empty backup.** If a machine has no vault and the
  server has nothing under that password, `--sync` fails instead of creating an
  empty vault.
- **Rotation re-publishes.** `--rotate` uploads under the new key, then deletes
  the copy the retired key opened. A machine still on the old key gets
  `410 Gone` and is told to rotate, rather than recreating a stale copy.
- **Requires the updated Worker.** Rerun `worker/schema.sql` (it adds the
  `vaults` marker table) and `wrangler deploy` before rotating on a machine
  that syncs. A Worker without the change still accepts writes from a retired
  identity.

## Verification

```bash
sha256sum -c checksums.txt        # Linux/macOS
Get-FileHash akc-windows-aarch64.exe -Algorithm SHA256   # Windows
```

Expected hashes:

```text
07ece62db9bd92794c766e42c9749cd5dad7f65decde270fb58c6b54fc34a3cd  akc-linux-aarch64
fbbb390a0cd1c2ad9e024e266961e8a2e4f7c44b66571869d04811002a39020d  akc-windows-aarch64.exe
```

The Windows asset was built on Windows with `cargo build --release --target
aarch64-pc-windows-msvc` (PE machine `0xAA64`). The Linux asset was built
natively inside an aarch64 WSL Ubuntu 24.04 machine with
`aarch64-unknown-linux-musl`; it is statically linked and stripped, so it runs
on any Linux aarch64 distribution. No zig cross-compiler was used.

`akc-linux-x86_64`, `akc-windows-x86_64.exe`, and `akc-macos-aarch64` remain
from `releases/20261015_1/` and do not include these sync changes.

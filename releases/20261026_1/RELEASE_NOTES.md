# akc — 20261026_1

**Build Date:** 2026-10-26
**Version:** 4.0.0 + sync (unreleased)
**Source:** commit `fbf84b2` ("Add optional encrypted sync through a Cloudflare Worker")

## Partial archive

This folder holds **two assets**:

- `akc-windows-aarch64.exe` — **Windows ARM64** (2,041,856 bytes)
- `akc-linux-aarch64` — **Linux ARM64**, static musl (2,361,592 bytes)

The other three assets (`akc-linux-x86_64`, `akc-windows-x86_64.exe`,
`akc-macos-aarch64`) are not built yet. They will be added by builds on the
other machines. `releases/latest` intentionally still points at
[`../20261015_1/`](../20261015_1/), because `akc --update` downloads the asset
for the current platform and a missing one is a 404, not a no-op. Do not point
`releases/latest` here until all five assets exist.

## What's new

- **`--sync`, `--sync-init`, `--sync-join`.** Optional encrypted sync through a
  Cloudflare Worker and D1 database. The server sees a SHA-256 of the sync
  token, item ids (a keyed hash of each name), sizes, and timestamps — never
  names or values. A lost token means a lost cloud copy; the local vault is
  unaffected.
- **`worker/`.** The Worker source and D1 schema, deployable with `wrangler`.

## Verification

```bash
sha256sum -c checksums.txt        # Linux/macOS
Get-FileHash akc-windows-aarch64.exe -Algorithm SHA256   # Windows
```

Expected hashes:

```text
bb236e8838398da384bb305bacbb905d5c7fde2e7df0ee3c03713d19887c9c7d  akc-windows-aarch64.exe
05a8127dad3c3a3224124a7b46c9f06842473b4fa10999cfd581fc3658b9fb70  akc-linux-aarch64
```

The Linux binary was built from the same commit in WSL (Ubuntu, aarch64) with
`scripts/build-release.sh` and zig 0.17.0 as the musl cross-compiler. It is
statically linked, so it runs on any Linux aarch64 distribution.

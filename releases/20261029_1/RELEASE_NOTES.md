# akc — 4.1.0 (20261029_1)

**Release Date:** 2026-10-29
**Version:** 4.1.0

This is the only published release. Older releases and build archives have been
removed from the repository and from GitHub.

## Assets

Only ARM64 is published:

- `akc-windows-aarch64.exe` — Windows ARM64
- `akc-linux-aarch64` — Linux ARM64, static musl

There is no x86_64 or macOS asset in this release. `akc --update` on those
platforms downloads an asset by name and will fail with a 404; install those
platforms from a build of this commit instead.

## What's new

Sync no longer needs a second secret.

- **One password opens the vault and its online backup.** The sync identity
  (bearer token, entry key, item ids) is derived from the master password with
  Argon2id, so a new machine needs only the password and `sync_url`.
  `akc --sync` downloads the vault and creates the local file.
- **`--sync`, `--sync-url`.** Optional encrypted sync through a Cloudflare
  Worker and D1 database that only stores ciphertext. Configure the server once:

  ```text
  # ~/.config/akc/config
  sync_url = https://akc-sync.your-subdomain.workers.dev
  ```

- **`--sync-init`, `--sync-join`, `AKC_SYNC_KEY`, and the `sync-key` file are
  gone.** There is nothing to copy between machines but the password.
- **A typo cannot make an empty backup.** If a machine has no vault and the
  server has nothing under that password, `--sync` fails instead of creating an
  empty vault.
- **Rotation re-publishes.** `--rotate` uploads under the new key, then deletes
  the copy the retired key opened. A machine still on the old key gets
  `410 Gone` and is told to rotate, instead of recreating a stale copy.
- **Requires the updated Worker.** Rerun `worker/schema.sql` (it adds the
  `vaults` marker table) and `wrangler deploy` before rotating on a machine that
  syncs.

The vault format is unchanged. Existing vaults need no migration; `--sync` is
additive, so every earlier command behaves as before.

## Verification

```bash
sha256sum -c checksums.txt        # Linux
Get-FileHash akc-windows-aarch64.exe -Algorithm SHA256   # Windows
```

Expected hashes:

```text
9f8df531fde46cf0b6176f0e2aa32eb569b70d3c289f828501ea13fdb6b54221  akc-linux-aarch64
43d65a5d65c7c937f9fcfad302fa829c559ffa690dd58f23e66e8460b49e6338  akc-windows-aarch64.exe
```

Build method: Windows ARM64 with
`cargo build --release --target aarch64-pc-windows-msvc`; Linux ARM64 built
natively inside an aarch64 WSL Ubuntu 24.04 machine with
`aarch64-unknown-linux-musl` and `CC_aarch64_unknown_linux_musl=gcc`. The Linux
asset is statically linked and stripped.

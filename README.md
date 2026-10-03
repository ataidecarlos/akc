# akc

A minimal encrypted secret store. CLI only: no server or network listener. Single executable, single portable data file.

- **Encrypted file:** a cleartext header (`magic(8) + version(4) + nonce(12) + KDF params(12) + salt(32)`) followed by the AES-256-GCM ciphertext of a JSON store. Both key names and values are encrypted.
- **KDF:** Argon2id (64 MiB, t=3, p=4). The parameters are recorded in the header, so a vault keeps working if the defaults change.
- **Portable:** the data file works on any OS and is safe to keep in OneDrive/Dropbox (atomic temp-file writes, never partially synced states). Use any filename; `.akc` is only a suggested convention.
- **Tiny:** single static binary, no runtime.

## Compatibility

Keychains written by akc 1.x and 2.x (headerless `salt + nonce + ciphertext`) are still readable. They are migrated to the current container automatically the first time they are written, with no action required.

## Installation

### Download and Install

Download the latest release from [releases/latest/](releases/latest/), then place the binary in a directory on your `PATH`.

Release assets are named for the platform they target:

| Asset | Platform |
|---|---|
| `akc-windows-x86_64.exe` | Windows x64 |
| `akc-windows-aarch64.exe` | Windows ARM64 |
| `akc-linux-x86_64` | Linux x86_64 (static, musl) |
| `akc-linux-aarch64` | Linux ARM64 (static, musl) |

**Windows:**
```powershell
# Pick the asset matching your machine, then:
Copy-Item akc-windows-aarch64.exe "$env:USERPROFILE\.local\bin\akc.exe"
akc --version
```

**Linux:**
```bash
# Pick the asset matching your machine, then:
chmod +x akc-linux-aarch64
sudo mv akc-linux-aarch64 /usr/local/bin/akc
akc --version
```

The installed binary can be named `akc` regardless of the asset name. Or browse all releases in [releases/](releases/).

### Build from Source

```powershell
cargo build --release
# Binary: target/release/akc.exe
```

### Build Release Binaries

Release assets are architecture-explicit, so all four are built from one script per platform.

```powershell
# Windows (both architectures) -> target\release-dist
.\scripts\build-release.ps1
```

```bash
# Linux (both architectures, static musl) -> target/release-dist
# Run inside WSL. Uses zig as the musl cross-compiler when available:
ZIG=/path/to/zig ./scripts/build-release.sh
```

Then produce the checksum file:

```bash
cargo run --example generate_checksums -- target/release-dist
```

Required Rust targets:

```bash
rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
```

The Linux build prefers [zig](https://ziglang.org) for cross-compiling because it needs no root and ships every musl target. `musl-tools` works too if the host can build the target directly.

## Usage

```powershell
akc secrets.akc init                  # create; prompts for password (twice)
akc secrets.akc set api_key sk-123    # add or update
akc secrets.akc get api_key           # print value
akc secrets.akc list                  # list key names only
akc secrets.akc delete api_key
akc secrets.akc change-password       # re-key, keeping every secret
akc upgrade                           # check for updates
```

Add `--password <pw>` or set `AKC_PASSWORD` for non-interactive use (otherwise you are prompted with hidden input).

### Passwords

Creating a keychain and changing its password both **ask you to type the password twice**. The confirmation is what catches a typo before it locks your secrets away.

```powershell
akc secrets.akc init
# Password:
# Confirm password:
```

The double entry applies to a *typed* password. One supplied with `--password` or `AKC_PASSWORD` is used as-is, since it cannot contain a typo and asking twice would break scripted use.

`change-password` verifies the current password before touching anything, so a wrong one changes nothing:

```powershell
akc secrets.akc change-password
# Password:            <- current
# Password:            <- new
# Confirm password:    <- new, again

# Non-interactive form:
akc secrets.akc change-password --new-password <new> --password <current>
# or: AKC_NEW_PASSWORD=<new> AKC_PASSWORD=<current> akc secrets.akc change-password
```

`init` refuses to overwrite an existing keychain, so a mistyped password can never silently replace a vault you already had:

```text
error: secrets.akc already exists: use 'akc secrets.akc change-password' to keep its
       secrets, or 'akc secrets.akc init --force' to discard them and start over
```

Use `init --force` only when you genuinely want to discard everything. It backs the old file up first.

### Values

Surrounding whitespace is always removed from a value, so `set k "  v  "` stores `v`. A value that is empty after trimming is rejected rather than stored:

```text
error: value must not be empty: surrounding whitespace is removed, so a blank value is not allowed
```

This applies to both `akc set` and interactive `set`, so the two behave identically.

### Interactive mode

Run `akc secrets.akc` without a command to enter interactive mode. The password is requested once, then commands can be run repeatedly:

```text
akc> list
akc> get api_key
akc> set another_key another-value
akc> delete another_key
akc> change-password
akc> exit
```

Interactive mode honours `AKC_PASSWORD`/`--password` for every command, so it can be scripted:

```bash
printf 'set k v\nexit\n' | AKC_PASSWORD=... akc secrets.akc
```

There is no interactive `init`: interactive mode only ever runs against an existing keychain, so discarding its contents is a deliberate CLI-only act (`akc secrets.akc init --force`). Interactive `change-password` always asks you to type the new password twice, since there is no subcommand there to carry `--new-password`.

`change-password` and `init --force` back up the existing file to `<keychain>_<YYYYMMDD_HHMMSS>.bak` before replacing it.

### Upgrade

```powershell
akc upgrade                           # check for and install updates
akc upgrade --yes                     # skip confirmation prompt
akc upgrade --force                   # force upgrade even if already on latest
akc upgrade --version 1.0.0           # upgrade to a specific version
```

Each release includes a `checksums.txt` file with SHA256 hashes, and akc verifies the downloaded binary against it before installing. This protects against a corrupted or truncated download. It is not a signature: `checksums.txt` is fetched from the same release as the binary, so it cannot protect against a compromised release.

## Security

- Wrong password fails via GCM authentication; truncated or tampered files are rejected.
- Argon2 parameters read from a file header are range-checked before use, so a hostile file cannot request an enormous allocation.
- Writes go to a temp file in the same directory, then atomically replace the target. Re-initialization backs up the existing file first.
- Secret material is zeroized from memory when dropped.
- An empty data file is ~111 bytes.

## Development

- **Source code:** `src/`
- **Testing:** See [test/README.md](test/README.md)
- **Releases:** `releases/` (archive of all versions)
- **Changelog:** [CHANGELOG.md](CHANGELOG.md)
- **Checksums:** `cargo run --example generate_checksums -- releases/<dir>`

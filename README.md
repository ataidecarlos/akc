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
| `akc-macos-aarch64` | macOS Apple Silicon |

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

**macOS (Apple Silicon):**
```bash
chmod +x akc-macos-aarch64
sudo mv akc-macos-aarch64 /usr/local/bin/akc
akc --version
```

Because macOS binaries are unsigned, Gatekeeper quarantines a binary that was
downloaded through a browser. `akc --update` writes the file directly and does
not set the quarantine attribute, so installing that way avoids it; after a
manual download, clear it with
`xattr -d com.apple.quarantine /usr/local/bin/akc`.

The installed binary can be named `akc` regardless of the asset name. Or browse all releases in [releases/](releases/).

### Build from Source

```powershell
cargo build --release
# Binary: target/release/akc.exe
```

### Build Release Binaries

Release assets are architecture-explicit, so each script builds one platform.

```powershell
# Windows (both architectures) -> target\release-dist
.\scripts\build-release.ps1
```

```bash
# Linux (both architectures, static musl) -> target/release-dist
# Run inside WSL. Uses zig as the musl cross-compiler when available:
ZIG=/path/to/zig ./scripts/build-release.sh
```

```bash
# All five assets from macOS -> target/release-dist
# Cross-compiles the Windows and Linux binaries, so only the macOS one can be
# run on the build host. One-time setup:
#   brew install zig llvm && cargo install cargo-xwin cargo-zigbuild
#   export PATH="$(brew --prefix llvm)/bin:$PATH"   # keg-only; llvm-lib
./scripts/build-release-macos.sh
```

A release must carry **every** asset: `--update` reads `releases/latest` and then
downloads the file for the current platform, so a release missing one turns
`akc --update` on that platform into a 404 rather than a no-op.

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

## Configuration

Set this up once and no command ever needs a path or a password:

```text
# ~/.config/akc/config
keychain = ~/homelab/secrets.akc
key_file = ~/.config/akc/key
```

The format is `key = value`, one per line; `#` starts a comment and a leading
`~` is expanded. It is deliberately **not** TOML — two settings do not justify a
parser dependency, so anything fancier than `key = value` is rejected rather
than half-understood. An unknown setting is an error rather than being silently
ignored, and the message names the line. `AKC_CONFIG_DIR` relocates the whole
directory, which is useful in tests. Both paths can be overridden per invocation
with `--keychain` and `--key-file`.

The master key file is read with only its trailing newline removed, so a key
written by `printf` behaves identically to one written by `echo`, and a key with
meaningful surrounding spaces keeps them.

## Usage

```bash
akc --init                    # create an empty keychain
akc api_key --set sk-123      # add or update
akc api_key                   # print the value
akc --list                    # list key names only
akc api_key --delete          # remove
akc --rotate                  # re-key, keeping every secret
akc --update                  # check for updates
```

`--set` with no value reads the secret from **stdin**, so it never appears in
`argv` where any local process could read it from `ps`:

```bash
printf '%s' "$TOKEN" | akc api_key --set
```

| | |
|---|---|
| `akc NAME...` | print the value of each name, one per line |
| `--set [VALUE]` | store NAME, overwriting it (reads stdin when VALUE is omitted) |
| `--list [NAME...]` | list names, or only those given |
| `--delete NAME...` | remove (aliases: `--drop`, `--remove`, `--rm`) |
| `--init` | create an empty keychain |
| `--rotate` | re-key under a new master key |
| `--sync` | pull, merge, and push changes with the sync server |
| `--sync-init` | create a sync vault and print its token |
| `--sync-join TOKEN` | adopt an existing sync vault on this machine |
| `--update` | check for and install updates |
| `--json` | machine-readable output for reads |
| `--keychain PATH` | override the configured keychain |
| `--key-file PATH` | override the configured key file |
| `--password PW` | literal master key, for one-off scripted use |
| `--new-password PW` | replacement key for `--init` / `--rotate` |
| `--new-key-file PATH` | replacement key file for `--init` / `--rotate` |
| `--force` | with `--init` / `--update`: proceed although it discards something |
| `--yes` | with `--update`: skip the confirmation prompt |
| `--to VERSION` | with `--update`: install a specific version |

`--password`, `--keychain` and `--key-file` are global, so they may appear
anywhere on the line.

Every command is a flag and the bare positional is only ever a secret name, so a
secret can never collide with a command word.

### Passwords and keys

A keychain is unlocked by a master key, taken from the first of these that applies:

| source | |
|---|---|
| `--password` | a literal value, for one-off scripted use |
| `--key-file` | a file path given on the command line |
| `AKC_KEY_FILE` | the same, from the environment |
| `key_file` in the config | the configured key file |
| `~/.config/akc/key` | the default location, when it exists |
| the console | you are prompted, with hidden input |

A **named** key file must exist. A typo'd path fails rather than silently falling
through to a prompt, which would leave an agent waiting on input that never comes.

`--init` and `--rotate` take a *new* key. `--new-password` / `AKC_NEW_PASSWORD`
and `--new-key-file` name it directly; `--init` additionally falls back to the
key file it would read anyway, so a fresh machine can be set up with the key
already in place. `--rotate` does not fall back — rotating onto the key it is
already using would be a no-op re-encrypt, so it asks instead.

A **typed** key is asked for twice, because the confirmation is what catches a
typo before it locks your secrets away. A key supplied by flag, file or
environment is used as-is: it cannot contain a typo, and asking twice would break
scripted use.

`--init` refuses to overwrite an existing keychain, so a mistyped path cannot
silently replace a vault you already had:

```text
error: secrets.akc already exists: use --rotate to keep its secrets, or
       --init --force to discard them and start over
```

Use `--init --force` only when you genuinely want to discard everything. It backs
the old file up first.

### Values

Surrounding whitespace is always removed, so `--set "  v  "` stores `v`. A value
that is empty after trimming is rejected rather than stored:

```text
error: value must not be empty: surrounding whitespace is removed, so a blank value is not allowed
```

`--set` with no value reads stdin; `--set ""` is an empty value and is rejected.
The two are different things, and both are different from `--set "   "`.

### Rotating the master key

```bash
akc --rotate --new-key-file ~/new.key
# or: akc --rotate --new-password <new>
# or: AKC_NEW_PASSWORD=<new> akc --rotate
```

The vault is a single blob, so this is **one re-encrypt, not a loop over
secrets** — it costs the same whether the vault holds one key or a thousand. The
current key is verified first, so a wrong one changes nothing, and the old file is
backed up before it is replaced.

What rotation cannot do: if the vault is committed to git, the retired key still
opens every commit made before the rotation. Rotating protects what you write
afterwards, and nothing earlier.

### Backups

`--rotate` and `--init --force` back the existing file up to
`<keychain>_<YYYYMMDD_HHMMSS>.bak` before replacing it.

### Team use: a shared vault in git

A keychain can be committed to a repository and shared by a team. Everyone uses
the same master key, kept in a file outside the repository and handed over the way
you would hand over any other shared secret.

Two writers need care, because the vault is a single encrypted blob and git
cannot merge two versions of it. `scripts/akc-git.sh` wraps the pull, write and
push sequence so a second writer re-reads the first writer's change instead of
colliding with it:

```bash
scripts/akc-git.sh secrets.akc api_key --set sk-123
scripts/akc-git.sh secrets.akc api_key
```

Two things protect a shared vault:

- **A write lock.** Every write takes an exclusive lock on `<vault>.lock` and
  reloads the vault inside it, so two writers on one machine cannot clobber each
  other — a caller holding state it read earlier cannot save that stale copy over
  someone else's work. The lock file is a sidecar: add `*.lock` to your
  `.gitignore`.
- **Pull before write, push after.** The script does this for you. Writing by
  hand, do the same: `git pull` before the command and `git push` straight after.

If two writers do collide, the script says so rather than silently picking a
side, and prints the recovery. Nothing is lost either way: your change is still a
commit.

The shared master key is a bearer secret with no expiry. Anyone who has ever held
it can decrypt every secret in the repository's history, including ones deleted
since, so handing it over cannot be taken back. Rotating it protects only what is
written afterwards.

### Cloud sync

Sync keeps one vault across machines through a server that only ever sees
ciphertext. It needs Cloudflare's free tier: one Worker and one D1 database. The
deployment is in [`worker/README.md`](worker/README.md), and the whole server is
around a hundred lines.

```bash
# First machine: create the vault key and print the only copy of the token
akc --sync-init --sync-url https://akc-sync.you.workers.dev

# Every other machine: adopt the token
akc --sync-join 'akcsync://akc-sync.you.workers.dev#<64 hex chars>'

# Exchange changes (pull, merge, push)
akc --sync
```

The vault file stays exactly what it was: sync decrypts it in memory, merges,
and saves it again. `--sync` is explicit, so no command ever blocks on the
network; run it when you want, or from a scheduled task.

- **The token is the vault.** `--sync-init` derives the encryption key, the item
  ids, and the bearer token from one 32-byte secret. It is printed once and
  never uploaded in a recoverable form. Lose it and the synced copy is
  unreadable; the local vault is unaffected.
- **The server learns nothing.** Entries are AES-256-GCM blobs keyed by a PRF of
  the secret name. The server sees sizes, timestamps, and how often you sync.
- **Rotation stays local.** `--rotate` changes only the vault password. The sync
  key is independent, so no re-upload and no other device is disturbed.
- **Conflicts lose nothing.** If the same secret changed on two machines, the
  server's value keeps the name and the other is kept as
  `name (conflict <date>)`. Both machines converge on the same winner.
- **Deletes propagate as tombstones** and are kept forever: they are a few dozen
  bytes, and a machine that was offline for months still converges.

The sync credential lives in `~/.config/akc/sync-key` (0600) by default, or in
`AKC_SYNC_KEY`. The cursor and hashes live beside it in `sync-state.json`. Use
`sync_key_file` in the config to move the key.

### Upgrade

```bash
akc --update                  # check for and install updates
akc --update --yes            # skip the confirmation prompt
akc --update --force          # install even if already current
akc --update --to 1.0.0       # install a specific version
```

Each release includes a `checksums.txt` file with SHA256 hashes, and akc verifies the downloaded binary against it before installing. This protects against a corrupted or truncated download. It is not a signature: `checksums.txt` is fetched from the same release as the binary, so it cannot protect against a compromised release.

## Security

- Wrong key fails via GCM authentication; truncated or tampered files are rejected.
- Argon2 parameters read from a file header are range-checked before use, so a hostile file cannot request an enormous allocation.
- Writes go to a temp file in the same directory, then atomically replace the target. Re-initialization backs up the existing file first.
- Secret material is zeroized from memory when dropped.
- An empty data file is 110 bytes.

The vault file is safe to put anywhere, including a public repository — its
contents are encrypted client-side and the key names are encrypted too. **The key
file is therefore the whole security boundary.** Wherever it lives, anything that
can read it can read every secret, and anything running as you can read it. Keep
it out of the repository and out of anything you would not trust with the vault
itself.

## Development

- **Source code:** `src/`
- **Testing:** See [test/README.md](test/README.md)
- **Releases:** `releases/` (archive of all versions)
- **Changelog:** [CHANGELOG.md](CHANGELOG.md)
- **Checksums:** `cargo run --example generate_checksums -- releases/<dir>`

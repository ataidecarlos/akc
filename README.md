# akc

A minimal encrypted secret store. CLI only: no server or network listener. Single executable, single portable data file.

- **Encrypted file:** a cleartext header (`magic(8) + version(4) + nonce(12) + KDF params(12) + salt(32)`) followed by the AES-256-GCM ciphertext of a JSON store. Both key names and values are encrypted.
- **KDF:** Argon2id (64 MiB, t=3, p=4). The parameters are recorded in the header, so a vault keeps working if the defaults change.
- **Portable:** the data file works on any OS and is safe to keep in OneDrive/Dropbox (atomic temp-file writes, never partially synced states). Use any filename; `.akc` is only a suggested convention.
- **Tiny:** single static binary, no runtime.

## Compatibility

Keychains written by akc 1.x and 2.x (headerless `salt + nonce + ciphertext`) are still readable. They are migrated to the current container automatically the first time they are written, with no action required.

akc 5.0 is local-only. The encrypted sync feature added in 4.1 is removed until
it has had another stabilization pass; if a 4.x config contains `sync_url`, akc
now says so and asks you to delete that line. Vault files are unaffected.

## Installation

### Download and Install

Download the latest release from the
[GitHub releases page](https://github.com/ataidecarlos/akc/releases), then place
the binary in a directory on your `PATH`. `akc --update` installs from the same
page, so a manual download is only needed the first time.

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

The installed binary can be named `akc` regardless of the asset name.

### Build from Source

```bash
cargo build --release
# Binary: target/release/akc  (target\release\akc.exe on Windows)
```

### Build Release Binaries

All release binaries are built on GitHub-hosted runners; nothing is built or
archived locally. The `release` workflow builds each asset natively on its own
runner, writes `checksums.txt`, and attaches both to the GitHub release:

| Runner | Target | Asset |
|---|---|---|
| `ubuntu-latest` | `x86_64-unknown-linux-musl` | `akc-linux-x86_64` |
| `ubuntu-24.04-arm` | `aarch64-unknown-linux-musl` | `akc-linux-aarch64` |
| `windows-latest` | `x86_64-pc-windows-msvc` | `akc-windows-x86_64.exe` |
| `windows-11-arm` | `aarch64-pc-windows-msvc` | `akc-windows-aarch64.exe` |
| `macos-latest` | `aarch64-apple-darwin` | `akc-macos-aarch64` |

Run it from the Actions tab, or push a `v*` tag. The GitHub release is the only
copy of a binary, and `akc --update` reads it through the Releases API.

## Configuration

Set this up once and no command ever needs a path or a password:

```text
# ~/.config/akc/config
keychain = ~/homelab/secrets.akc
key_file = ~/.config/akc/key
```

The format is `key = value`, one per line; `#` starts a comment and a leading
`~` is expanded. It is deliberately **not** TOML — two settings do not
justify a parser dependency, so anything fancier than `key = value` is rejected
rather than half-understood. An unknown setting is an error rather than being silently
ignored, and the message names the line. `AKC_CONFIG_DIR` relocates the whole
directory, which is useful in tests. Both can be overridden per invocation
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
| `--delete NAME...` | remove (aliases: `--drop`, `--remove`, `--rm`); asks first on a terminal |
| `--init` | create an empty keychain |
| `--rotate` | re-key under a new master key |
| `--update` | check for and install updates |
| `--json` | machine-readable output for reads |
| `--keychain PATH` | override the configured keychain |
| `--key-file PATH` | override the configured key file |
| `--password PW` | literal master key, for one-off scripted use |
| `--new-password PW` | replacement key for `--init` / `--rotate` |
| `--new-key-file PATH` | replacement key file for `--init` / `--rotate` |
| `--force` | with `--init` / `--update`: proceed although it discards something |
| `--yes` | skip the confirmation prompt (`--update`, `--delete`) |
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
already in place. `--password` / `AKC_PASSWORD` are never reused as the new key,
because that value is meant for opening an existing vault. `--rotate` does not
fall back — rotating onto the key it is already using would be a no-op
re-encrypt, so it asks instead.

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

### Multiple machines

The vault file is portable: copy it, keep it in a synced folder, or commit it to
a repository. Only the file moves; the master key stays on each machine.

If two machines write at the same time, the later writer must re-read the other
change. The vault is a single encrypted blob that git cannot merge, so when it
lives in a repository pull before a write and push straight after it. On one
machine, every write takes an exclusive `<vault>.lock` and reloads the vault
inside it, so concurrent writers cannot clobber each other. The lock file is a
sidecar: add `*.lock` to your `.gitignore`.

A shared master key is a bearer secret with no expiry. Anyone who has ever held
it can decrypt every secret in the repository's history, including ones deleted
since, so handing it over cannot be taken back. Rotating it protects only what is
written afterwards.

### Upgrade

```bash
akc --update                  # check for and install updates
akc --update --yes            # skip the confirmation prompt
akc --update --force          # install even if already current
akc --update --to 1.0.0       # install a specific version
```

Each release includes a `checksums.txt` file with SHA256 hashes, and akc verifies the downloaded binary against it before installing. This protects against a corrupted or truncated download. It is not a signature: `checksums.txt` is fetched from the same release as the binary, so it cannot protect against a compromised release. The updater has a 60-second HTTP timeout and refuses a download over 64 MiB.

## Security

- Wrong key fails via GCM authentication; truncated or tampered files are rejected.
- Argon2 parameters read from a file header are range-checked before use; the
  memory cost is capped at 256 MiB, so a hostile file cannot request an enormous
  allocation.
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
- **Testing:** See [test/README.md](test/README.md). CI runs the unit and
  integration suites on Linux, macOS and Windows for every push and pull request.
- **Releases:** the [GitHub releases page](https://github.com/ataidecarlos/akc/releases), built by `.github/workflows/release.yml` on GitHub-hosted runners
- **Changelog:** [CHANGELOG.md](CHANGELOG.md)

# akc — 20261015_1

**Release Date:** 2026-10-15
**Version:** 4.0.0

## What's New

This release redesigns the command line around a single idea: **fetching a secret
should need nothing but the secret's name.** Set up a config file once and `akc
NAME` works from any process, on any machine, with no path and no password
prompt — which is what makes it usable by an AI agent.

- **`akc NAME` reads a secret.** Every command is now a flag and the bare
  positional is only ever a secret name, so a secret can never collide with a
  command word.
- **A config file at `~/.config/akc/config`** naming the keychain and the master
  key file. Override per-invocation with `--keychain` and `--key-file`.
- **Master key files.** `--key-file`, `AKC_KEY_FILE`, or `~/.config/akc/key`. Only
  the trailing newline is stripped, so a key written by `printf` behaves
  identically to one written by `echo`.
- **`--set` reads the value from stdin** when no value is given, so a secret never
  appears in `argv` where any local process could read it from `ps`.
- **`--rotate`** replaces `change-password`. The vault is one blob, so this is a
  single re-encrypt, not a loop over secrets — the same cost for one key or a
  thousand.
- **`--json`** for reads, and reading several names at once costs one key
  derivation rather than one each.
- **macOS (Apple Silicon) is now a published platform.** Previously the only way
  to run akc on a Mac was to build it.
- **Concurrent writes no longer lose secrets.** Every write takes an exclusive
  lock on `<vault>.lock` and reloads inside it. Before this, ten concurrent
  writes left one key standing.
- **`scripts/akc-git.sh`** wraps an akc command with the pull-before-write,
  push-after sequence a vault committed to git needs.

## Upgrade notes (breaking)

The command line changed completely. Every existing invocation needs rewriting:

| 3.x | 4.0.0 |
|---|---|
| `akc secrets.akc get NAME` | `akc --keychain secrets.akc NAME` |
| `akc secrets.akc set NAME VALUE` | `akc --keychain secrets.akc NAME --set VALUE` |
| `akc secrets.akc list` | `akc --keychain secrets.akc --list` |
| `akc secrets.akc delete NAME` | `akc --keychain secrets.akc NAME --delete` |
| `akc secrets.akc init` | `akc --keychain secrets.akc --init` |
| `akc secrets.akc change-password` | `akc --keychain secrets.akc --rotate` |
| `akc upgrade --version X` | `akc --update --to X` |
| `akc secrets.akc` (interactive) | removed |

Put the keychain and key paths in `~/.config/akc/config` and every `--keychain`
above disappears.

Interactive mode is gone: the flag-based syntax does not translate to a
line-oriented REPL, and the tool is built for agents. `--init --force` remains the
way to discard a vault.

**Existing keychains need no migration.** The container format is unchanged —
only the command line moved. macOS users had no binary before this release, so
for them it is a first install rather than an upgrade.

## Features

- **One job:** fetch a secret by name, with no prompt and no arguments
- **Strong encryption:** AES-256-GCM with Argon2id (64 MiB, t=3, p=4)
- **Portable:** single encrypted file, safe for cloud sync and safe to commit
- **Small:** ~1.8–2.5 MB static binary, no runtime dependencies
- **Agent-friendly:** a key file plus a config file is all an agent needs
- **Secure:** GCM authentication on a wrong key, tamper detection, range-checked
  header parameters, memory zeroization, and a write lock against lost updates

## Platforms

Assets are named for the platform they target. Pick the one matching your machine.

**Windows:**
- `akc-windows-x86_64.exe` — Windows x64
- `akc-windows-aarch64.exe` — Windows ARM64

**Linux:**
- `akc-linux-x86_64` — Linux x86_64 (static, musl)
- `akc-linux-aarch64` — Linux ARM64 (static, musl)

**macOS:**
- `akc-macos-aarch64` — macOS Apple Silicon

macOS binaries are unsigned, so Gatekeeper quarantines one downloaded through a
browser. `akc --update` writes the file directly and does not set the quarantine
attribute; after a manual download, clear it with
`xattr -d com.apple.quarantine /usr/local/bin/akc`.

## Checksums

See `checksums.txt` for SHA256 hashes of all binaries. `akc --update` verifies the
downloaded binary against it before installing. This protects against a corrupted
or truncated download; it is not a signature, since the checksums are fetched from
the same release as the binary.

## Usage

```bash
akc --init                    # create an empty keychain
akc api_key --set sk-123      # add or update
akc api_key                   # print the value
akc --list                    # list key names only
akc api_key --delete          # remove
akc --rotate                  # re-key, keeping every secret
```

`--set` with no value reads the secret from stdin, so it never reaches `argv`:

```bash
printf '%s' "$TOKEN" | akc api_key --set
```

## Upgrade

```bash
akc --update                  # check for and install updates
akc --update --yes            # skip the confirmation prompt
akc --update --force          # reinstall even if already current
akc --update --to 4.0.0       # a specific version
```

Note that `--update` on 3.x is `akc upgrade`, so upgrading *to* 4.0.0 from 3.0.0
works with the old command; everything after uses the new one.

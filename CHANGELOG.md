# Changelog

All notable changes to akc are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.0.0] - 2026-10-03

### Added

- **`change-password` command**, which re-keys a keychain while keeping every
  secret. It verifies the current password first, so a wrong one changes nothing,
  and backs the file up before rewriting it. Available as
  `akc <file> change-password`, with `--new-password` / `AKC_NEW_PASSWORD` for
  non-interactive use.
- `init --force` for deliberately discarding an existing keychain. It backs the
  old file up first.

### Changed

- **Breaking: `init` refuses to overwrite an existing keychain.** It previously
  replaced one silently, so a typo'd new password locked the secrets away behind a
  password the user did not have, with only the backup left to recover from. Use
  `change-password` to re-key, or `init --force` to discard on purpose.
- **Breaking: every action that upserts a password asks for it twice.** `init` and
  `change-password` both confirm a typed password. Double entry applies only to a
  typed password; one supplied via `--password`/`AKC_PASSWORD` cannot contain a typo.
- **Breaking: secret values are normalized.** Surrounding whitespace is always
  removed, and a value that is empty after trimming is rejected instead of stored.
  Previously interactive `set` silently trimmed while the CLI did not, so the same
  operation behaved differently depending on how it was invoked. Both error
  messages now state the rule.
- **Breaking: no interactive `init`.** Interactive mode only ever runs against an
  existing keychain, so wiping it is now `init --force` from the CLI. Interactive
  `change-password` replaces the old interactive `init`.

### Fixed

- **`test/test.sh` aborted the whole suite at the first expected failure.**
  `run_akc` toggled `set -e` on and left it enabled, so any later deliberately
  failing command killed the run with a bare exit code 1 and no output. The
  function no longer touches shell options, and expected failures capture their
  status explicitly.
- **Interactive `init` no longer reverts the new password.** Re-initializing from
  interactive mode replaced the file under the new password but left the session
  holding the old one, so the next `set`/`delete` silently re-encrypted the vault
  under the old password and discarded the secrets entered since. The session now
  adopts the new password.
- **Interactive `init` honours `AKC_PASSWORD`/`--password`.** It previously always
  prompted on the console, which hung indefinitely whenever stdin was a pipe and
  made interactive mode unusable from scripts and CI.
- **`upgrade` works on Windows ARM64.** Previously `upgrade` refused to run on
  `windows-aarch64`.
- **`upgrade` no longer risks leaving you without a binary.** On Windows the old
  binary was moved aside and the new one renamed into place; if that second step
  failed you were left with no `akc` at all. It now restores the previous binary
  on failure.
- **`upgrade` no longer silently leaves `akc.old.exe` behind.** The cleanup delete
  of the running image always failed and the error was discarded; it is now
  reported.
- **`upgrade` reports an unsupported platform before making any network call**
  instead of after contacting GitHub.
- **`Keychain::save` rejects oversize stores before writing**, instead of leaving a
  partially written file.
- `init` now writes with an exclusive create, so a keychain cannot be clobbered by
  a race between the existence check and the write.

### Changed

- **Breaking: release assets are architecture-explicit.** `akc.exe` was a single
  file that could only be one architecture — it was in fact ARM64 while the
  release README described it as "Windows x64", so x64 users received a binary
  they could not run. Assets are now `akc-windows-x86_64.exe`,
  `akc-windows-aarch64.exe`, `akc-linux-x86_64` and `akc-linux-aarch64`.
- **Keychain files gained a cleartext header** carrying a magic marker, a container
  version, and the Argon2id parameters. Keychains written by 1.x and 2.x are still
  readable and are migrated automatically on their next write. A vault's recorded
  KDF parameters are preserved on save, so changing the defaults never silently
  weakens or strengthens an existing vault.
- Interactive mode reads its password prompt through an injectable seam, so the
  whole session is covered by tests without a terminal.
- `scripts/generate_checksums.rs` is wired up as a Cargo example and discovers
  release assets instead of hardcoding their names, so a renamed binary can no
  longer be left out of `checksums.txt`.
- Added `scripts/build-release.ps1` and `scripts/build-release.sh` to build all four
  release assets, which were previously produced by undocumented manual steps.
- `test/test.sh` gained an `AKC=<path>` override and now fails with a clear message
  when the selected binary cannot run on the host, instead of "Exec format error".

### Removed

- The unused `create` argument of `Keychain::save`, along with its unreachable
  "file may already exist" path.
- The unused `tag_name` field on `ReleaseInfo`.

## [2.0.0] - 2026-09-23

### Added

- Interactive mode with `get`, `set`, `list`, `delete`, `init`, `help`, and `exit` commands
- `AKC_PASSWORD` environment variable
- Backups before re-initializing an existing keychain

### Changed

- Breaking CLI syntax change: keychain path now comes immediately after `akc`
- Existing keychains are backed up as `<keychain>_<YYYYMMDD_HHMMSS>.bak` before `init` replaces them

## [1.1.0] - 2026-09-19

### Added

- `upgrade` command: check for and install updates from GitHub releases
  - Checksum verification (SHA256) before replacing binary
  - Confirmation prompt (skip with `--yes`)
  - Force upgrade with `--force`
  - Upgrade to specific version with `--version <VER>` (supports downgrade and pre-release)
- Checksums published with each release (`checksums.txt`)
- Cross-platform upgrade: automatically detects OS and architecture

### Fixed

- `--password` option now visible in `akc --help` (was only in subcommand help)

## [1.0.1] - 2026-09-19

### Added

- Linux support: x86_64 and ARM64 static binaries (musl)
- Bash test script (`test/test.sh`) alongside PowerShell tests
- Reorganized release structure with date-based naming (YYYYMMDD_x)

## [1.0.0] - 2026-09-19

### Added

- Initial release
- CLI commands: `init`, `set`, `get`, `list`, `delete`
- AES-256-GCM encryption with Argon2id key derivation
- Portable encrypted file format (any filename, `.akc` suggested)
- Atomic writes for cloud sync safety (OneDrive/Dropbox)
- Memory zeroization for secrets
- Interactive password prompts with confirmation
- `--password` flag for scripting
- Comprehensive test suite (15 unit + 19 integration tests)

# Changelog

All notable changes to akc are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

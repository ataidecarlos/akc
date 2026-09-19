# Changelog

All notable changes to akc are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

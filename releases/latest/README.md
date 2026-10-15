# Latest Release

This folder mirrors the most recent published release of akc (**4.0.0**, 2026-10-15).
It is kept identical to [`releases/20261015_1/`](../20261015_1/).

**Binaries** (architecture-explicit — pick the one matching your machine):

- `akc-windows-x86_64.exe` — Windows x64
- `akc-windows-aarch64.exe` — Windows ARM64
- `akc-linux-x86_64` — Linux x86_64 (static, musl)
- `akc-linux-aarch64` — Linux ARM64 (static, musl)
- `akc-macos-aarch64` — macOS Apple Silicon

**Verification:**
- `checksums.txt` — SHA256 hashes for all binaries, verified by `akc --update`
  before installing

macOS binaries are unsigned, so Gatekeeper quarantines one downloaded through a
browser. `akc --update` writes the file directly and does not set the quarantine
attribute; after a manual download, clear it with
`xattr -d com.apple.quarantine /usr/local/bin/akc`.

For version history and detailed release notes, see the versioned folders.

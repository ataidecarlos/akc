# Latest Release

**4.1.0**, 2026-10-29. This folder mirrors
[`releases/20261029_1/`](../20261029_1/) and is the copy in the repository.

`akc --update` does not read this folder: it asks the GitHub Releases API for
the latest release and downloads the asset there. The published GitHub release
and this folder carry the same files.

**Binaries** (architecture-explicit — pick the one matching your machine):

- `akc-windows-aarch64.exe` — Windows ARM64
- `akc-linux-aarch64` — Linux ARM64 (static, musl)

There is no x86_64 or macOS asset in this release. `akc --update` on those
platforms will fail with a 404.

**Verification:**

- `checksums.txt` — SHA256 hashes for both binaries

For full release notes, see [RELEASE_NOTES.md](RELEASE_NOTES.md).

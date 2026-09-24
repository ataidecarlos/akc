# Testing akc

## Philosophy

akc uses a two-tier testing approach:

1. **Unit tests** (`cargo test`) — Fast, isolated tests of individual modules
   - Crypto: encryption/decryption, key derivation, tamper detection
   - Storage: file I/O, atomic writes, data format
   - Password: interactive prompts, confirmation, validation

2. **Integration tests** (`test/test.ps1` or `test/test.sh`) — End-to-end CLI behavior
   - All 5 commands (`init`, `set`, `get`, `list`, `delete`)
   - Error cases (wrong password, missing keys, empty password)
   - Security (tamper detection, GCM authentication)
   - Portability (file copying, cross-directory usage)
   - Atomicity (failed operations don't corrupt state)

**Core principles:**
- Test behavior, not implementation details
- Test failure modes — ensure errors are caught correctly
- Test portability — files must work after copying
- Test atomicity — failed operations must not leave partial state

## Running Tests

**Windows (PowerShell):**
```powershell
# Unit tests (15 tests)
cargo test

# Integration tests (19 tests)
.\test\test.ps1
```

**Linux (PowerShell Core or Bash):**
```bash
# Unit tests (15 tests)
cargo test

# Integration tests - PowerShell Core
pwsh test/test.ps1

# Integration tests - Bash
chmod +x test/test.sh
./test/test.sh
```

Both unit and integration tests must pass before any release.

## What's Tested

**Commands:**
- `init` — creates file, backs up existing files, rejects empty password
- `set` — adds new secrets, updates existing
- `get` — retrieves values, fails on missing keys
- `list` — shows sorted keys
- `delete` — removes keys, fails on missing keys

**Security:**
- Wrong password rejected (GCM authentication)
- Tampered files rejected
- Truncated files rejected

**Edge cases:**
- Empty password rejected
- Missing keys fail with exit code 1
- File copying preserves functionality
- Failed operations don't modify file

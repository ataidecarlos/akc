# Testing akc

## Philosophy

akc uses a two-tier testing approach:

1. **Unit tests** (`cargo test`) — Fast, isolated tests of individual modules
   - Crypto: encryption/decryption, key derivation, tamper detection, container header
   - Storage: file I/O, atomic writes, data format, legacy migration
   - Password: interactive prompts, confirmation, validation
   - Upgrade: platform asset naming, checksum verification
   - Interactive session: the whole command loop, driven from a scripted stdin and a
     scripted prompt (no terminal required)

2. **Integration tests** (`test/test.ps1` or `test/test.sh`) — End-to-end CLI behavior
   - All 5 commands (`init`, `set`, `get`, `list`, `delete`)
   - Error cases (wrong password, missing keys, empty password)
   - Security (tamper detection, GCM authentication)
   - Portability (file copying, cross-directory usage)
   - Atomicity (failed operations don't corrupt state)
   - Container format (magic marker and version byte)
   - `AKC_PASSWORD` and interactive mode, including that interactive `init` does not
     hang when the password comes from the environment

**Core principles:**
- Test behavior, not implementation details
- Test failure modes — ensure errors are caught correctly
- Test portability — files must work after copying
- Test atomicity — failed operations must not leave partial state

## Running Tests

**Windows (PowerShell):**
```powershell
# Unit tests
cargo test

# Integration tests
.\test\test.ps1
```

**Linux (PowerShell Core or Bash):**
```bash
# Unit tests
cargo test

# Integration tests - PowerShell Core
pwsh test/test.ps1

# Integration tests - Bash
chmod +x test/test.sh
./test/test.sh
```

Both unit and integration tests must pass before any release.

## Regression tests worth knowing about

These exist because the corresponding bugs shipped once already:

- `main::tests::change_password_then_set_persists_under_the_new_password` — a
  password change inside a session must leave the session writing under the new
  password.
- `main::tests::cli_change_password_needs_no_terminal_when_new_password_is_given` —
  supplying both passwords must not reach the console.
- `main::tests::interactive_change_password_always_confirms_the_new_password` — the
  double-entry guarantee for a typed password.
- `main::tests::init_refuses_to_clobber_an_existing_keychain` — `init` cannot
  replace a vault and lock its secrets behind an unverified password.
- `main::tests::set_rejects_a_blank_value_without_touching_the_vault` — a rejected
  value leaves the vault unchanged, and the error states the rule.
- `storage::tests::legacy_v1_keychain_is_migrated_on_save` — a 1.x keychain stays
  readable and is rewritten in the current container.
- `crypto::tests::out_of_range_params_are_rejected_before_allocating` — a hostile
  header cannot request an unbounded Argon2 allocation.

When changing the interactive session, keep the injected-prompt seam. Driving the
real console prompt is what let the `init` password bug go unnoticed: it needs a
terminal, so it could not be tested.

## What's Tested

**Commands:**
- `init` — creates file, refuses to overwrite an existing one, `init --force`
  replaces it with a backup, rejects empty password
- `set` — adds new secrets, updates existing, strips padding, rejects blank values
- `get` — retrieves values, fails on missing keys
- `list` — shows sorted keys
- `delete` — removes keys, fails on missing keys
- `change-password` — keeps secrets, requires the current password, creates a backup

**Password handling:**
- `init` and `change-password` confirm a typed password twice
- A wrong current password changes nothing and creates no backup
- Passwords can be changed repeatedly

**Interactive mode:**
- Multiple commands in one session
- `set` is persisted to disk, padding stripped, blank values rejected
- `change-password` adopts the new password for later writes, and keeps secrets
- `change-password` backs up the existing keychain, and a failed one leaves it intact
- `init` is not a session command
- Unknown or malformed commands do not abort the session

**Security:**
- Wrong password rejected (GCM authentication)
- Tampered files rejected
- Truncated files rejected
- Out-of-range KDF parameters in a header rejected before allocating

**Compatibility:**
- Headerless 1.x keychains load, and are migrated on the next write
- KDF parameters are preserved across saves

**Edge cases:**
- Empty password rejected
- Missing keys fail with exit code 1
- File copying preserves functionality
- Failed operations don't modify file

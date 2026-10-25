# Testing akc

## Philosophy

akc uses a two-tier testing approach:

1. **Unit tests** (`cargo test`) — Fast, isolated tests of individual modules
   - Crypto: encryption/decryption, key derivation, tamper detection, container header
   - Storage: file I/O, atomic writes, data format, legacy migration
   - Config: parsing, tilde expansion, key-file reading, key resolution order
   - Password: prompts, confirmation, validation
   - Upgrade: platform asset naming, checksum verification
   - Commands: each flag, and the errors it can produce

2. **Integration tests** (`test/test.ps1` or `test/test.sh`) — End-to-end CLI behavior
   - Every command (`--init`, `--set`, `--list`, `--delete`, `--rotate`, bare `NAME`)
   - Error cases (wrong password, missing keys, missing key file, conflicting flags)
   - Security (tamper detection, GCM authentication)
   - Portability (file copying, cross-directory usage)
   - Atomicity (failed operations don't corrupt state)
   - Container format (magic marker and version byte)
   - `AKC_PASSWORD`, the config file, and `--key-file` resolution
   - Concurrent writers losing no secrets

**Core principles:**
- Test behavior, not implementation details
- Test failure modes — ensure errors are caught correctly
- Test portability — files must work after copying
- Test atomicity — failed operations must not leave partial state

## Running Tests

**Windows (PowerShell):**
```powershell
cargo test
.\test\test.ps1
```

**Linux / macOS (Bash):**
```bash
cargo test
./test/test.sh
```

Both unit and integration tests must pass before any release. CI runs
`cargo test` plus the platform's integration suite on Linux, macOS and Windows
(`.github/workflows/ci.yml`).

## Regression tests worth knowing about

These exist because the corresponding bugs shipped once already, or because the
failure mode is silent:

- `main::tests::concurrent_writers_do_not_lose_secrets` — the vault is one blob
  rewritten wholesale, so two writers that both read before either saved would
  clobber each other. Ten concurrent writes left one key standing.
- `main::tests::mutate_reloads_before_writing` — a caller holding stale state
  must not save it over another process's write.
- `main::tests::a_rejected_mutation_leaves_the_vault_alone` — a failed write must
  not rewrite the file.
- `main::tests::init_refuses_to_clobber_an_existing_keychain` — `--init` cannot
  replace a vault and lock its secrets behind an unverified key.
- `main::tests::rotate_with_a_wrong_key_changes_nothing` — a wrong current key
  must fail before anything is written or backed up.
- `main::tests::delete_of_a_missing_name_removes_nothing` — delete is all or
  nothing, so a missing name cannot take the others with it.
- `main::tests::get_with_a_missing_name_prints_nothing` — a partial read must not
  reach stdout for a caller to misread.
- `main::tests::set_rejects_a_blank_value_without_touching_the_vault` — a rejected
  value leaves the vault unchanged, and the error states the rule.
- `config::tests::unknown_settings_are_rejected_with_a_location` — a typo'd
  setting must not be silently ignored.
- `main::tests::a_missing_named_key_file_is_an_error` — a typo'd key path must
  fail rather than fall through to a prompt an agent would hang on.
- `storage::tests::legacy_v1_keychain_is_migrated_on_save` — a 1.x keychain stays
  readable and is rewritten in the current container.
- `crypto::tests::out_of_range_params_are_rejected_before_allocating` — a hostile
  header cannot request an unbounded Argon2 allocation.

- `crypto::tests::short_magic_prefixed_file_is_rejected_without_panicking` — a
  truncated container used to index past the end of the slice and panic.
- `main::tests::exit_codes_are_distinct` — a caller must be able to tell a wrong
  key from a missing name.
- `upgrade::tests::oversized_downloads_are_rejected_before_reading` — a bad URL
  or hostile response must not be buffered into memory.

## What's Tested

**Commands:**
- `--init` — creates the file, refuses to overwrite an existing one, `--init
  --force` replaces it with a backup
- `--set` — adds new secrets, updates existing, strips padding, rejects blank
  values, reads a value from stdin when none is given
- bare `NAME` — retrieves one or more values, fails on missing keys, `--json`
- `--list` — shows sorted keys, filters to named keys, `--json`
- `--delete` — removes keys, fails on missing keys, all-or-nothing across several
  names, `--drop`/`--remove`/`--rm` aliases
- `--rotate` — keeps secrets, requires the current key, creates a backup

**Key handling:**
- Resolution order: `--password`, then a key file, then the console
- A named key file that does not exist is an error; the default one is optional
- A key file is read with only its trailing newline stripped
- `--init` and `--rotate` confirm a *typed* new key twice, and never confirm one
  supplied by flag, file or environment
- A wrong current key changes nothing and creates no backup

**Config:**
- Reads `keychain` and `key_file`, expands `~`, ignores comments and blanks
- An unknown setting or a line without `=` is rejected with its location
- A missing config file is not an error
- `--keychain` and `--key-file` override it

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

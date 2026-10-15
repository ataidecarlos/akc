#!/usr/bin/env bash
# Post-build feature tests for akc.
# Run from the project root: ./test/test.sh
# Exit code 0 = all passed, 1 = one or more failures.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Candidate binaries, most specific first. The first one that exists wins.
CANDIDATES=(
    "target/x86_64-unknown-linux-musl/release/akc"
    "target/aarch64-unknown-linux-musl/release/akc"
    "target/release/akc"
    "target/release/akc.exe"
    "target/debug/akc"
)

# The first candidate that exists AND runs here. Cross-built artifacts land
# under target/ too, and a Linux binary on macOS exists but cannot execute, so
# presence alone is not enough to choose one.
find_akc() {
    local candidate
    for candidate in "${CANDIDATES[@]}"; do
        if [[ -f "$candidate" ]] && "$candidate" --version >/dev/null 2>&1; then
            printf '%s' "$candidate"
            return 0
        fi
    done
    return 1
}

# Set AKC to test a specific build, e.g.
#   AKC=target/aarch64-unknown-linux-musl/release/akc ./test/test.sh
# Useful when several cross-built artifacts exist and only one can run here.
AKC="${AKC:-}"

if [[ -z "$AKC" ]]; then
    AKC="$(find_akc || true)"
fi

if [[ -z "$AKC" ]]; then
    echo "No build found; running cargo build --release..."
    if ! cargo build --release; then
        echo "FAIL: build"
        exit 1
    fi
    AKC="$(find_akc || true)"
fi

if [[ -z "$AKC" ]]; then
    echo "FAIL: no binary found after build"
    exit 1
fi

# A candidate built for another architecture exists but cannot run here; fail
# with a clear message instead of an opaque "Exec format error".
if ! "$AKC" --version >/dev/null 2>&1; then
    echo "FAIL: $AKC cannot run on this host ($(uname -m))"
    echo "      set AKC= to a build matching this architecture"
    exit 1
fi

echo "Using binary: $AKC"

PASS=0
FAIL=0

assert_true() {
    local name="$1"
    local condition="$2"
    local detail="${3:-}"
    if [[ "$condition" -eq 0 ]]; then
        PASS=$((PASS + 1))
        echo "PASS  $name"
    else
        FAIL=$((FAIL + 1))
        echo "FAIL  $name  $detail"
    fi
}

WORK=$(mktemp -d)

# The config file is what lets every command below run with no path and no
# prompt, which is the whole point of the tool. Tests that need a different
# keychain or key pass --keychain / --key-file explicitly.
export AKC_CONFIG_DIR="$WORK/config"
mkdir -p "$AKC_CONFIG_DIR"
KC="$WORK/secrets.akc"
KEY="$AKC_CONFIG_DIR/key"
printf 'test-pass-123\n' > "$KEY"
printf '# test config\nkeychain = %s\nkey_file = %s\n' "$KC" "$KEY" > "$AKC_CONFIG_DIR/config"

cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# Runs akc with no password flag, so the key comes from the config's key file.
# Deliberately failing invocations are expected here, so capture the status
# without tripping errexit. This function must NOT toggle `set -e`: doing so
# would silently abort the whole suite at the first expected failure.
run_akc() {
    local tmpfile; tmpfile=$(mktemp)
    EXIT_CODE=0
    "$AKC" "$@" > "$tmpfile" 2>&1 || EXIT_CODE=$?
    AKC_OUTPUT=$(cat "$tmpfile")
    rm -f "$tmpfile"
}

# Runs akc with an explicit literal password instead of the key file.
run_akc_pw() {
    local pw="$1"; shift
    local tmpfile; tmpfile=$(mktemp)
    EXIT_CODE=0
    "$AKC" --password "$pw" "$@" > "$tmpfile" 2>&1 || EXIT_CODE=$?
    AKC_OUTPUT=$(cat "$tmpfile")
    rm -f "$tmpfile"
}

# Portable file size: GNU stat first, then BSD/macOS stat.
file_size() { stat -c%s "$1" 2>/dev/null || stat -f%z "$1"; }

# --- init ---
run_akc --init
assert_true "init creates a keychain" "$([[ $EXIT_CODE -eq 0 && -f "$KC" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --init
assert_true "init refuses to clobber an existing keychain" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"already exists"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- set / get ---
run_akc zeta --set "last-value"
assert_true "set stores a value" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc alpha --set "first-value"
run_akc alpha --set "updated-value"
run_akc alpha
assert_true "set overwrites and get returns the latest" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc missing_key
assert_true "get of a missing key fails" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"key not found"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc
assert_true "get with no name fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- value normalization ---
run_akc padded --set "  padded-value  "
run_akc padded
assert_true "set strips surrounding whitespace" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "padded-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc blank --set "   "
assert_true "blank value is rejected with an explanation" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"whitespace is removed"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc blank
assert_true "blank value was not stored" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- value from stdin: the agent path, and it stays out of argv ---
printf 'piped-value\n' | "$AKC" piped --set > "$WORK/piped-out" 2>&1
assert_true "set reads the value from stdin" "$([[ $? -eq 0 ]] && echo 0 || echo 1)" "$(cat "$WORK/piped-out")"
run_akc piped
assert_true "stdin value round-trips" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "piped-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- get several names in one open ---
run_akc alpha zeta
assert_true "get accepts several names" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == $'updated-value\nlast-value' ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc alpha missing_key
assert_true "one missing name aborts the whole read" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"key not found"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- json ---
run_akc alpha --json
assert_true "get --json emits an object" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == *'"alpha"'* && "$AKC_OUTPUT" == *'"updated-value"'* ]] && echo 0 || echo 1)" \
    "$AKC_OUTPUT"

# --- list ---
run_akc --list
assert_true "list shows key names" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == *"alpha"* && "$AKC_OUTPUT" == *"zeta"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
assert_true "list never shows values" "$([[ "$AKC_OUTPUT" != *"updated-value"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --list alpha absent
assert_true "list filters to the names given" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "alpha" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --list --json
assert_true "list --json emits an array" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == *'"alpha"'* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- delete, including the aliases ---
run_akc zeta --delete
assert_true "delete removes a key" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
run_akc zeta
assert_true "deleted key is gone" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
run_akc zeta --delete
assert_true "delete of a missing key fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc piped --drop
assert_true "--drop is an alias for --delete" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
run_akc padded --remove
assert_true "--remove is an alias for --delete" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- command flags are mutually exclusive ---
run_akc --list --set
assert_true "conflicting command flags are rejected" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc a b --set
assert_true "--set takes exactly one name" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"exactly one NAME"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- wrong password ---
run_akc_pw "wrong-pass" alpha
assert_true "wrong password fails" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"wrong password"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc_pw "wrong-pass" --list
assert_true "wrong password list fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc_pw "wrong-pass" evil --set x
assert_true "wrong password set fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- the config file is what makes the path-free form work ---
run_akc alpha
assert_true "config supplies the keychain and key with no flags" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --keychain "$KC" --key-file "$KEY" alpha
assert_true "--keychain and --key-file override the config" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --keychain "$WORK/absent.akc" --key-file "$KEY" alpha
assert_true "a keychain that does not exist fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# A named key file that is missing must fail rather than silently prompt.
run_akc --key-file "$WORK/no-such-key" alpha
assert_true "a missing key file fails clearly" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"key file not found"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- environment password ---
AKC_PASSWORD="test-pass-123" "$AKC" alpha > "$WORK/env-out" 2>&1
assert_true "AKC_PASSWORD works" "$([[ $? -eq 0 ]] && echo 0 || echo 1)" "$(cat "$WORK/env-out")"

AKC_PASSWORD="wrong-pass" "$AKC" alpha > "$WORK/env-bad" 2>&1
assert_true "AKC_PASSWORD wrong value fails" "$([[ $? -ne 0 ]] && echo 0 || echo 1)" "$(cat "$WORK/env-bad")"

# --- portability ---
COPY="$WORK/copied.akc"
cp "$KC" "$COPY"
run_akc --keychain "$COPY" alpha
assert_true "copied file still works" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- tamper detection ---
LAST_BYTE=$(xxd -p -l 1 -s -1 "$COPY")
LAST_BYTE_XOR=$(printf '%02x' $((0x$LAST_BYTE ^ 0xFF)))
printf "\\x$LAST_BYTE_XOR" | dd of="$COPY" bs=1 seek=$(($(file_size "$COPY") - 1)) count=1 conv=notrunc 2>/dev/null
run_akc --keychain "$COPY" alpha
assert_true "tampered file rejected" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- container format ---
MAGIC=$(head -c 8 "$KC")
assert_true "keychain starts with the AKCSTORE magic" "$([[ "$MAGIC" == "AKCSTORE" ]] && echo 0 || echo 1)" "magic=$MAGIC"

VERSION_HEX=$(xxd -p -l 4 -s 8 "$KC")
# Little-endian u32 2 == 02 00 00 00
assert_true "keychain records container version 2" "$([[ "$VERSION_HEX" == "02000000" ]] && echo 0 || echo 1)" "version_hex=$VERSION_HEX"

# --- rotate ---
BACKUPS_BEFORE=$(find "$WORK" -maxdepth 1 -name '*.bak' | wc -l)

# --password supplies the CURRENT key, --new-password the replacement. A wrong
# current key cannot decrypt, so nothing may be written or backed up.
run_akc_pw "wrong-pass" --rotate --new-password "irrelevant"
assert_true "rotate with a wrong current key fails" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"wrong password"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc alpha
assert_true "a failed rotate leaves the vault readable" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

BACKUPS_AFTER_FAIL=$(find "$WORK" -maxdepth 1 -name '*.bak' | wc -l)
assert_true "a failed rotate creates no backup" \
    "$([[ $BACKUPS_AFTER_FAIL -eq $BACKUPS_BEFORE ]] && echo 0 || echo 1)" "$BACKUPS_BEFORE -> $BACKUPS_AFTER_FAIL"

run_akc --rotate --new-password "rotated-456"
assert_true "rotate succeeds" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc alpha
assert_true "the old key no longer opens the vault" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc_pw "rotated-456" alpha
assert_true "secrets survive a rotation" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

BACKUPS_AFTER=$(find "$WORK" -maxdepth 1 -name '*.bak' | wc -l)
assert_true "rotate created a backup" \
    "$([[ $BACKUPS_AFTER -gt $BACKUPS_BEFORE ]] && echo 0 || echo 1)" "$BACKUPS_BEFORE -> $BACKUPS_AFTER"

# --- failed ops leave the file untouched ---
run_akc_pw "rotated-456" missing_key
run_akc_pw "rotated-456" missing_key --delete
run_akc_pw "rotated-456" blank --set ""
SIZE_AFTER=$(file_size "$KC")
run_akc_pw "rotated-456" blank --set "   "
SIZE_AFTER2=$(file_size "$KC")
assert_true "failed ops leave file untouched" \
    "$([[ $SIZE_AFTER -eq $SIZE_AFTER2 ]] && echo 0 || echo 1)" "$SIZE_AFTER -> $SIZE_AFTER2"

# --- concurrent writers ---
# Regression: the vault is one blob that every write rewrites wholesale, so two
# writers that both read before either saved would clobber each other. Before
# the write lock, ten concurrent `set` calls left a single key standing.
RACE="$WORK/race.akc"
run_akc --keychain "$RACE" --init

for i in $(seq 1 10); do
    env AKC_PASSWORD="test-pass-123" "$AKC" --keychain "$RACE" "race$i" --set "value$i" &
done
wait

run_akc --keychain "$RACE" --list
RACE_KEYS=$(printf '%s\n' "$AKC_OUTPUT" | grep -c '^race')
assert_true "concurrent writers lose no secrets" \
    "$([[ "$RACE_KEYS" -eq 10 ]] && echo 0 || echo 1)" "stored $RACE_KEYS of 10"

assert_true "a write lock is kept beside the vault" \
    "$([[ -f "$RACE.lock" ]] && echo 0 || echo 1)" "no $RACE.lock"

echo ""
echo "Results: $PASS passed, $FAIL failed"
if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
exit 0

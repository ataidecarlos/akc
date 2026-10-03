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

find_akc() {
    local candidate
    for candidate in "${CANDIDATES[@]}"; do
        if [[ -f "$candidate" ]]; then
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

run_akc() {
    local pw="test-pass-123"
    if [[ "${1:-}" == "--password" ]]; then pw="$2"; shift 2; fi
    local tmpfile; tmpfile=$(mktemp)
    # Deliberately failing invocations are expected here, so capture the status
    # without tripping errexit. This function must NOT toggle `set -e`: doing so
    # would silently abort the whole suite at the first expected failure.
    EXIT_CODE=0
    "$AKC" "$@" --password "$pw" > "$tmpfile" 2>&1 || EXIT_CODE=$?
    AKC_OUTPUT=$(cat "$tmpfile")
    rm -f "$tmpfile"
}

run_akc_env() {
    local pw="$1"; shift
    local tmpfile; tmpfile=$(mktemp)
    local code=0
    AKC_PASSWORD="$pw" "$AKC" "$@" > "$tmpfile" 2>&1 || code=$?
    EXIT_CODE=$code
    AKC_OUTPUT=$(cat "$tmpfile")
    rm -f "$tmpfile"
}

WORK=$(mktemp -d)
KC="$WORK/secrets.akc"

cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# --- init ---
run_akc "$KC" init
assert_true "init creates file" "$([[ $EXIT_CODE -eq 0 && -f "$KC" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

SIZE=$(stat -c%s "$KC" 2>/dev/null || stat -f%z "$KC" 2>/dev/null)
assert_true "init file is small binary" "$([[ $SIZE -gt 40 && $SIZE -lt 200 ]] && echo 0 || echo 1)" "size=$SIZE"

run_akc "$KC" init
assert_true "init refuses an existing keychain" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"already exists"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
assert_true "init refusal points at change-password" \
    "$([[ "$AKC_OUTPUT" == *"change-password"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" init --force
assert_true "init --force replaces and backs up" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == *"Backed up"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --password "" "$KC" init "x"
assert_true "init rejects empty password" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- set ---
run_akc "$KC" set zeta "last-value"
assert_true "set adds secret" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" set alpha "first-value"
assert_true "set adds second secret" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" set alpha "updated-value"
assert_true "set updates existing secret" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- value normalization ---
run_akc "$KC" set padded "  padded-value  "
assert_true "set strips padding from values" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
run_akc "$KC" get padded
assert_true "stored value has no padding" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "padded-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" set blank ""
assert_true "set rejects an empty value" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"must not be empty"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
assert_true "empty-value error explains the rule" \
    "$([[ "$AKC_OUTPUT" == *"whitespace is removed"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" set blank "   "
assert_true "set rejects a whitespace-only value" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
run_akc "$KC" get blank
assert_true "rejected value was not stored" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- get ---
run_akc "$KC" get alpha
assert_true "get returns value" "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" get missing
assert_true "get missing key fails" "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"not found"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- list ---
run_akc "$KC" list
LINE_COUNT=$(echo "$AKC_OUTPUT" | wc -l)
FIRST=$(echo "$AKC_OUTPUT" | head -1)
SECOND=$(echo "$AKC_OUTPUT" | sed -n 2p)
THIRD=$(echo "$AKC_OUTPUT" | tail -1)
assert_true "list shows sorted keys" \
    "$([[ $EXIT_CODE -eq 0 && "$FIRST" == "alpha" && "$SECOND" == "padded" && "$THIRD" == "zeta" && $LINE_COUNT -eq 3 ]] && echo 0 || echo 1)" \
    "$AKC_OUTPUT"

# --- delete ---
run_akc "$KC" delete zeta
assert_true "delete removes key" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" get zeta
assert_true "deleted key is gone" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" delete zeta
assert_true "delete missing key fails" "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"not found"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- wrong password ---
run_akc --password "wrong-pass" "$KC" get alpha
assert_true "wrong password fails" "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"wrong password"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --password "wrong-pass" "$KC" list
assert_true "wrong password list fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --password "wrong-pass" "$KC" set evil "x"
assert_true "wrong password set fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- portability ---
COPY="$WORK/copied.akc"
cp "$KC" "$COPY"
run_akc "$COPY" get alpha
assert_true "copied file still works" "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- tamper detection ---
LAST_BYTE=$(xxd -p -l 1 -s -1 "$COPY")
LAST_BYTE_XOR=$(printf '%02x' $((0x$LAST_BYTE ^ 0xFF)))
printf "\\x$LAST_BYTE_XOR" | dd of="$COPY" bs=1 seek=$(($(stat -c%s "$COPY") - 1)) count=1 conv=notrunc 2>/dev/null
run_akc "$COPY" get alpha
assert_true "tampered file rejected" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- container format ---
MAGIC=$(head -c 8 "$KC")
assert_true "keychain starts with the AKCSTORE magic" "$([[ "$MAGIC" == "AKCSTORE" ]] && echo 0 || echo 1)" "magic=$MAGIC"

VERSION_HEX=$(xxd -p -l 4 -s 8 "$KC")
# Little-endian u32 2 == 02 00 00 00
assert_true "keychain records container version 2" "$([[ "$VERSION_HEX" == "02000000" ]] && echo 0 || echo 1)" "version_hex=$VERSION_HEX"

# --- environment password ---
run_akc_env "test-pass-123" "$KC" get alpha
assert_true "AKC_PASSWORD environment variable works" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc_env "wrong-pass" "$KC" get alpha
assert_true "AKC_PASSWORD wrong value fails" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"wrong password"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- interactive mode ---
# `timeout` guards against interactive mode blocking forever on a console
# password prompt when stdin is a pipe.
printf 'set from-interactive interactive-value\nget from-interactive\nlist\nexit\n' |
    timeout 30 env AKC_PASSWORD="test-pass-123" "$AKC" "$KC" > "$WORK/interactive-output" 2>&1
INTERACTIVE_EXIT=$?
INTERACTIVE_OUTPUT=$(cat "$WORK/interactive-output")
assert_true "interactive mode runs multiple commands" \
    "$([[ $INTERACTIVE_EXIT -eq 0 && "$INTERACTIVE_OUTPUT" == *"interactive-value"* ]] && echo 0 || echo 1)" \
    "$INTERACTIVE_OUTPUT"
# 124 from `timeout` means the process hung and had to be killed.
assert_true "interactive mode does not hang" "$([[ $INTERACTIVE_EXIT -ne 124 ]] && echo 0 || echo 1)" "exit=$INTERACTIVE_EXIT"

run_akc "$KC" get from-interactive
assert_true "interactive set is persisted" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "interactive-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

printf 'set ipad   padded-interactive  \nset iblank    \nexit\n' |
    timeout 30 env AKC_PASSWORD="test-pass-123" "$AKC" "$KC" > "$WORK/interactive-norm" 2>&1
NORM_EXIT=$?
NORM_OUTPUT=$(cat "$WORK/interactive-norm")
assert_true "interactive set strips padding" "$([[ $NORM_EXIT -eq 0 ]] && echo 0 || echo 1)" "$NORM_OUTPUT"
assert_true "interactive blank value is rejected with an explanation" \
    "$([[ "$NORM_OUTPUT" == *"whitespace is removed"* ]] && echo 0 || echo 1)" "$NORM_OUTPUT"

run_akc "$KC" get ipad
assert_true "interactive stored value has no padding" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "padded-interactive" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" get iblank
assert_true "interactive rejected a blank value" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

printf 'init\nexit\n' |
    timeout 30 env AKC_PASSWORD="test-pass-123" "$AKC" "$KC" > "$WORK/interactive-init" 2>&1
INITCMD_OUTPUT=$(cat "$WORK/interactive-init")
assert_true "interactive init is no longer a command" \
    "$([[ "$INITCMD_OUTPUT" == *"unknown command: init"* ]] && echo 0 || echo 1)" "$INITCMD_OUTPUT"

# --- change-password ---
BACKUPS_BEFORE=$(find "$WORK" -maxdepth 1 -name '*.bak' | wc -l)

run_akc --password "wrong-pass" "$KC" change-password --new-password "newpass-456"
assert_true "change-password rejects a wrong current password" \
    "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"wrong password"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" get alpha
assert_true "failed change-password leaves the vault readable" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"
BACKUPS_AFTER_FAIL=$(find "$WORK" -maxdepth 1 -name '*.bak' | wc -l)
assert_true "failed change-password creates no backup" \
    "$([[ $BACKUPS_AFTER_FAIL -eq $BACKUPS_BEFORE ]] && echo 0 || echo 1)" \
    "$BACKUPS_BEFORE -> $BACKUPS_AFTER_FAIL"

run_akc "$KC" change-password --new-password "newpass-456"
assert_true "change-password succeeds" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == *"Changed the password"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --password "newpass-456" "$KC" get alpha
assert_true "secrets survive a password change" \
    "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc "$KC" list
assert_true "old password no longer works" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

BACKUPS_AFTER=$(find "$WORK" -maxdepth 1 -name '*.bak' | wc -l)
assert_true "change-password created a backup" \
    "$([[ $BACKUPS_AFTER -eq $((BACKUPS_BEFORE + 1)) ]] && echo 0 || echo 1)" \
    "$BACKUPS_BEFORE -> $BACKUPS_AFTER"

run_akc --password "newpass-456" "$KC" change-password --new-password "another-789"
assert_true "password can be changed again" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- failed ops leave file untouched ---
SIZE_BEFORE=$(stat -c%s "$KC" 2>/dev/null || stat -f%z "$KC" 2>/dev/null)
run_akc --password "newpass-456" "$KC" get missing
run_akc --password "newpass-456" "$KC" delete missing
run_akc --password "newpass-456" "$KC" set blank ""
SIZE_AFTER=$(stat -c%s "$KC" 2>/dev/null || stat -f%z "$KC" 2>/dev/null)
assert_true "failed ops leave file untouched" \
    "$([[ $SIZE_BEFORE -eq $SIZE_AFTER ]] && echo 0 || echo 1)" "$SIZE_BEFORE -> $SIZE_AFTER"

echo ""
echo "Results: $PASS passed, $FAIL failed"
if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
exit 0
#!/usr/bin/env bash
# Post-build feature tests for akc.
# Run from the project root: ./test/test.sh
# Exit code 0 = all passed, 1 = one or more failures.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Find binary
AKC=""
for candidate in \
    "target/x86_64-unknown-linux-musl/release/akc" \
    "target/aarch64-unknown-linux-musl/release/akc" \
    "target/release/akc" \
    "target/debug/akc"; do
    if [[ -f "$candidate" ]]; then
        AKC="$candidate"
        break
    fi
done

if [[ -z "$AKC" ]]; then
    echo "No build found; running cargo build --release..."
    cargo build --release
    if [[ $? -ne 0 ]]; then
        echo "FAIL: build"
        exit 1
    fi
    for candidate in \
        "target/x86_64-unknown-linux-musl/release/akc" \
        "target/aarch64-unknown-linux-musl/release/akc" \
        "target/release/akc"; do
        if [[ -f "$candidate" ]]; then
            AKC="$candidate"
            break
        fi
    done
fi

if [[ -z "$AKC" ]]; then
    echo "FAIL: no binary found after build"
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
    local tmpfile
    tmpfile=$(mktemp)
    if [[ "${1:-}" == "--password" ]]; then
        pw="$2"
        shift 2
    fi
    "$AKC" "$@" --password "$pw" > "$tmpfile" 2>&1
    EXIT_CODE=$?
    AKC_OUTPUT=$(cat "$tmpfile")
    rm -f "$tmpfile"
}

WORK=$(mktemp -d)
KC="$WORK/secrets.akc"

trap 'rm -rf "$WORK"' EXIT

# --- init ---
run_akc init "$KC"
assert_true "init creates file" "$([[ $EXIT_CODE -eq 0 && -f "$KC" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

SIZE=$(stat -c%s "$KC" 2>/dev/null || stat -f%z "$KC" 2>/dev/null)
assert_true "init file is small binary" "$([[ $SIZE -gt 40 && $SIZE -lt 200 ]] && echo 0 || echo 1)" "size=$SIZE"

run_akc init "$KC"
assert_true "init refuses existing file" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --password "" init "$KC" "x"
assert_true "init rejects empty password" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- set ---
run_akc set "$KC" zeta "last-value"
assert_true "set adds secret" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc set "$KC" alpha "first-value"
assert_true "set adds second secret" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc set "$KC" alpha "updated-value"
assert_true "set updates existing secret" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- get ---
run_akc get "$KC" alpha
assert_true "get returns value" "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc get "$KC" missing
assert_true "get missing key fails" "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"not found"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- list ---
run_akc list "$KC"
LINE_COUNT=$(echo "$AKC_OUTPUT" | wc -l)
FIRST=$(echo "$AKC_OUTPUT" | head -1)
SECOND=$(echo "$AKC_OUTPUT" | tail -1)
assert_true "list shows sorted keys" "$([[ $EXIT_CODE -eq 0 && "$FIRST" == "alpha" && "$SECOND" == "zeta" && $LINE_COUNT -eq 2 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- delete ---
run_akc delete "$KC" zeta
assert_true "delete removes key" "$([[ $EXIT_CODE -eq 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc get "$KC" zeta
assert_true "deleted key is gone" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc delete "$KC" zeta
assert_true "delete missing key fails" "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"not found"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- wrong password ---
run_akc --password "wrong-pass" get "$KC" alpha
assert_true "wrong password fails" "$([[ $EXIT_CODE -ne 0 && "$AKC_OUTPUT" == *"wrong password"* ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --password "wrong-pass" list "$KC"
assert_true "wrong password list fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

run_akc --password "wrong-pass" set "$KC" evil "x"
assert_true "wrong password set fails" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- portability ---
COPY="$WORK/copied.akc"
cp "$KC" "$COPY"
run_akc get "$COPY" alpha
assert_true "copied file still works" "$([[ $EXIT_CODE -eq 0 && "$AKC_OUTPUT" == "updated-value" ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- tamper detection ---
LAST_BYTE=$(xxd -p -l 1 -s -1 "$COPY")
LAST_BYTE_XOR=$(printf '%02x' $((0x$LAST_BYTE ^ 0xFF)))
printf "\\x$LAST_BYTE_XOR" | dd of="$COPY" bs=1 seek=$(($(stat -c%s "$COPY") - 1)) count=1 conv=notrunc 2>/dev/null
run_akc get "$COPY" alpha
assert_true "tampered file rejected" "$([[ $EXIT_CODE -ne 0 ]] && echo 0 || echo 1)" "$AKC_OUTPUT"

# --- failed ops leave file untouched ---
BEFORE=$(stat -c%s "$KC" 2>/dev/null || stat -f%z "$KC" 2>/dev/null)
run_akc get "$KC" missing
run_akc delete "$KC" missing
AFTER=$(stat -c%s "$KC" 2>/dev/null || stat -f%z "$KC" 2>/dev/null)
assert_true "failed ops leave file untouched" "$([[ $BEFORE -eq $AFTER ]] && echo 0 || echo 1)" "$BEFORE -> $AFTER"

echo ""
echo "Results: $PASS passed, $FAIL failed"
if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
exit 0

#!/usr/bin/env bash
# Build the Linux release binaries for every supported architecture.
#
# Run from the project root, in WSL or any Linux environment:
#   ./scripts/build-release.sh [output-dir]
#
# The binaries are statically linked against musl so they run on any Linux,
# including distributions built against glibc or musl libc.
#
# Cross-compiler selection:
#   * zig  - preferred. Needs no root and ships every musl target. Set ZIG=/path/to/zig.
#   * musl-gcc - used when the host can build the target directly.
#
# Windows binaries are built separately on Windows; see scripts/build-release.ps1.

set -euo pipefail

OUT="${1:-target/release-dist}"
ZIG="${ZIG:-zig}"
TARGETS=(x86_64 aarch64)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found on PATH" >&2
    exit 1
fi

HAVE_ZIG=0
if command -v "$ZIG" >/dev/null 2>&1; then
    HAVE_ZIG=1
    echo "using zig: $("$ZIG" version)"
else
    echo "note: zig not found (set ZIG=/path/to/zig); relying on system musl toolchain" >&2
fi

mkdir -p "$OUT"
WRAPPER_DIR="$(mktemp -d)"
trap 'rm -rf "$WRAPPER_DIR"' EXIT

# cc-rs passes Rust-style target triples such as
# --target=x86_64-unknown-linux-musl, which zig cannot parse. This wrapper
# rewrites them to zig triples and pins the real target.
write_wrapper() {
    local arch="$1" zig_target="$2" path="$3"
    cat > "$path" <<WRAPPER
#!/usr/bin/env bash
out=()
for a in "\$@"; do
    case "\$a" in
        --target=*)
            t="\${a#--target=}"
            t="\${t%-unknown-linux-gnu}"
            t="\${t%-unknown-linux-musl}"
            t="\${t%-pc-windows-msvc}"
            case "\$t" in
                *-linux-musl) out+=(-target "\${t}-linux-musl") ;;
                *-linux-gnu)  out+=(-target "\${t}-linux-gnu") ;;
                # A bare architecture carries no OS; drop it and let the
                # target pinned below decide.
                *-*) out+=(-target "\$t") ;;
                *) : ;;
            esac
            ;;
        # zig bundles an LLD build that rejects this Cortex-A53 erratum
        # workaround, which rustc emits for aarch64. Dropping it only affects
        # instruction scheduling on that one core, not correctness of the build.
        -Wl,--fix-cortex-a53-*) : ;;
        *) out+=("\$a") ;;
    esac
done
exec $ZIG cc -target $zig_target "\${out[@]}"
WRAPPER
    chmod +x "$path"
}

config="$ROOT/.cargo/config.toml"
config_existed=0
[ -f "$config" ] && config_existed=1
mkdir -p "$ROOT/.cargo"

build_target() {
    local arch="$1"
    local rust_target="${arch}-unknown-linux-musl"
    local zig_target="${arch}-linux-musl"

    if ! rustup target list --installed 2>/dev/null | grep -qx "$rust_target"; then
        echo "error: rust target $rust_target is not installed; run: rustup target add $rust_target" >&2
        exit 1
    fi

    if [ "$HAVE_ZIG" = "1" ]; then
        write_wrapper "$arch" "$zig_target" "$WRAPPER_DIR/zigcc-$arch"
        cat > "$config" <<CONFIG
[target.$rust_target]
linker = "$WRAPPER_DIR/zigcc-$arch"
# Rust and zig would otherwise each supply musl's crt1.o, giving a duplicate
# _start. Let zig own the CRT objects and the libc.
rustflags = ["-C", "link-self-contained=no"]

[env]
CC_${rust_target//-/_} = { value = "$WRAPPER_DIR/zigcc-$arch", force = true }
AR_${rust_target//-/_} = { value = "$ZIG ar", force = true }
CONFIG
    fi

    echo "building $rust_target"
    cargo build --release --target "$rust_target"

    local built="target/$rust_target/release/akc"
    [ -f "$built" ] || { echo "error: expected $built" >&2; exit 1; }
    cp "$built" "$OUT/akc-linux-$arch"
    chmod +x "$OUT/akc-linux-$arch"
    echo "  -> $OUT/akc-linux-$arch ($(stat -c%s "$OUT/akc-linux-$arch" 2>/dev/null || stat -f%z "$OUT/akc-linux-$arch") bytes)"
}

for arch in "${TARGETS[@]}"; do
    build_target "$arch"
done

# Do not clobber a pre-existing .cargo/config.toml.
if [ "$config_existed" = "0" ]; then
    rm -f "$config"
    rmdir "$ROOT/.cargo" 2>/dev/null || true
fi

echo
echo "Linux binaries in $OUT:"
ls -la "$OUT"

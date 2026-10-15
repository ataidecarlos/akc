#!/usr/bin/env bash
# Cross-build every published release asset from macOS.
#
# The other two scripts build their own platform natively: build-release.ps1 on
# Windows (MSVC) and build-release.sh under WSL (static musl). This one produces
# all five from a Mac, which is the only way to get the Windows and Linux
# binaries without those machines.
#
#   cargo-xwin     fetches Microsoft's CRT and SDK, so the Windows binaries are
#                  genuine -msvc targets. Building them with mingw instead would
#                  link differently and stop them being self-contained.
#   cargo-zigbuild uses zig as the musl cross-linker for the Linux binaries.
#
# Output: target/release-dist/, one file per asset name.
#
# Setup, once:
#   brew install zig llvm
#   cargo install cargo-xwin cargo-zigbuild
#   rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc \
#                     x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
#                     aarch64-apple-darwin
#
# Homebrew's llvm is keg-only, so `llvm-lib` is not on the PATH and cargo-xwin
# fails with "failed to find tool \"llvm-lib\"" without this:
#   export PATH="$(brew --prefix llvm)/bin:$PATH"
#
# The binaries are cross-compiled, so only the host's own architecture can be
# run here. Everything else needs a check on the real platform.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT="${1:-target/release-dist}"
mkdir -p "$OUT"

build() {
    local target="$1" asset="$2" builder="$3" shim="${4:-}"
    echo "==> $asset  ($target)"

    local cargo_cmd=(cargo build)
    case "$builder" in
        xwin) cargo_cmd=(cargo xwin build) ;;
        zig)  cargo_cmd=(cargo zigbuild) ;;
    esac

    if [[ -n "$shim" ]]; then
        # ring's build.rs forces the compiler *named* "clang" for
        # windows-aarch64, while cargo-xwin supplies MSVC-style /imsvc include
        # flags that only clang-cl reads -- plain clang treats /imsvc as a
        # filename and fails. Pointing the name "clang" at clang-cl for this one
        # build resolves it, and no arguments are rewritten: the flags are
        # clang-cl's, so clang-cl is the driver they were written for.
        mkdir -p "$SHIM_DIR"
        ln -sf "$shim" "$SHIM_DIR/clang"
        env PATH="$SHIM_DIR:$PATH" "${cargo_cmd[@]}" --release --target "$target"
    else
        "${cargo_cmd[@]}" --release --target "$target"
    fi

    local built="target/$target/release/akc"
    [[ "$asset" == *.exe ]] && built="$built.exe"
    cp "$built" "$OUT/$asset"
    chmod +x "$OUT/$asset"
}

SHIM_DIR="$(mktemp -d)"
trap 'rm -rf "$SHIM_DIR"' EXIT

build x86_64-pc-windows-msvc   akc-windows-x86_64.exe  xwin
build aarch64-pc-windows-msvc  akc-windows-aarch64.exe xwin "$(command -v clang-cl || true)"
build x86_64-unknown-linux-musl  akc-linux-x86_64  zig
build aarch64-unknown-linux-musl akc-linux-aarch64 zig
build aarch64-apple-darwin     akc-macos-aarch64  cargo

echo
echo "Built into $OUT:"
ls -1 "$OUT"
echo
echo "Next: cargo run --example generate_checksums -- $OUT"

#!/usr/bin/env bash
# Cross-compiles the benchmark to a static Linux binary for the homelab VM.
#
# No zig and no cargo-zigbuild, unlike `make build-server`: this crate is pure
# Rust with no C dependencies, so rust-lld plus `link-self-contained` links a
# static musl binary on its own. Nothing has to be installed on the VM either —
# the box being measured is a live server, and the least it is touched the
# better.
#
# Homebrew's rust is usually first on PATH and ships no musl std, so both cargo
# and rustc are pinned to the rustup toolchain. Without that pin the target
# reports as "not installed" even when `rustup target list` says otherwise.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TC="${RUSTUP_TOOLCHAIN_DIR:-$HOME/.rustup/toolchains/stable-aarch64-apple-darwin}"

if [ ! -x "$TC/bin/cargo" ]; then
    echo "no rustup toolchain at $TC" >&2
    echo "set RUSTUP_TOOLCHAIN_DIR, or install the target with:" >&2
    echo "  rustup target add x86_64-unknown-linux-musl" >&2
    exit 1
fi

export RUSTC="$TC/bin/rustc"
export PATH="$TC/bin:$PATH"
export RUSTFLAGS="-C linker=rust-lld -C link-self-contained=yes"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}"

cd "$HERE"
"$TC/bin/cargo" build --release --target x86_64-unknown-linux-musl

BIN="$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/argon2-bench"
ls -la "$BIN"
file "$BIN"

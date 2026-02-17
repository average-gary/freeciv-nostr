#!/usr/bin/env bash
# Build the freeciv-nostr Rust crates and generate the C FFI header.
#
# Usage: scripts/build-rust.sh <output-dir>

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: $0 <output-dir>" >&2
    exit 1
fi

OUTPUT_DIR="$1"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

mkdir -p "$OUTPUT_DIR"

# Build all Rust crates in release mode.
cargo build --release \
    --manifest-path "$ROOT_DIR/rust/Cargo.toml" \
    --target-dir "$OUTPUT_DIR/rust-target"

# Generate the C header from the FFI crate using cbindgen.
cbindgen \
    --config "$ROOT_DIR/rust/crates/freeciv-nostr-ffi/cbindgen.toml" \
    --crate freeciv-nostr-ffi \
    --output "$OUTPUT_DIR/freeciv_nostr_ffi.h" \
    "$ROOT_DIR/rust/crates/freeciv-nostr-ffi/"

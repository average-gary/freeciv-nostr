# freeciv-nostr Rust Workspace

Rust crates providing decentralized P2P networking via Nostr and Iroh for the
Freeciv game engine.

## Workspace Structure

| Crate | Purpose |
|---|---|
| `freeciv-nostr-core` | Nostr protocol types, event construction, NIP-46 remote signing, hash chains |
| `freeciv-nostr-net` | Iroh-based P2P transport, gossip channels, blob transfer |
| `freeciv-nostr-verify` | Deterministic state hashing and lockstep verification |
| `freeciv-nostr-ffi` | C FFI bindings (staticlib + cdylib) exposed to the game engine |
| `freeciv-nostr-cli` | CLI tool for testing and debugging networking |

## Prerequisites

- Rust 1.85+ (edition 2024)
- `cbindgen` (for generating the C header): `cargo install cbindgen`

## Build

```sh
# From the repository root:
cargo build --manifest-path rust/Cargo.toml

# Or from this directory:
cd rust
cargo build
```

## Test

```sh
cd rust
cargo test
```

## Lint

```sh
cd rust
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## Generating the C FFI Header

The helper script builds all crates and runs cbindgen:

```sh
scripts/build-rust.sh /tmp/freeciv-nostr-build
```

The generated header will be at `/tmp/freeciv-nostr-build/freeciv_nostr_ffi.h`.

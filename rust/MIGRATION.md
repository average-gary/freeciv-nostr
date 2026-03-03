# Freeciv-Nostr: C to Rust Migration Plan

Tracking issue: [#25](https://github.com/average-gary/freeciv-nostr/issues/25)

## Overview

This project follows a **gradual, module-by-module** migration strategy from C to Rust.
The approach is conservative: each C module is replaced behind the same C interface via FFI,
so the rest of the codebase never knows the implementation language changed. This lets us
migrate incrementally without a risky big-bang rewrite.

The Rust code lives in `rust/` as a Cargo workspace with five crates:

| Crate | Purpose |
|---|---|
| `freeciv-nostr-core` | Nostr types, event builders, keys, signer, chain, actions |
| `freeciv-nostr-net` | P2P networking: iroh transport, gossip, relay, lockstep, matchmaking |
| `freeciv-nostr-verify` | Hash chain verification, commits, checkpoints, persistence |
| `freeciv-nostr-ffi` | C FFI boundary (`staticlib`/`cdylib`), 177 exported functions |
| `freeciv-nostr-cli` | `freeciv-p2p` binary for merged client+server mode |

## Migration Order

| Step | Module | Key C Files | Complexity | Status | Est. Effort |
|------|--------|-------------|------------|--------|-------------|
| 1 | Nostr event types | `nostr_types.h` (new) | Low | **Done** | 1 week |
| 2 | Hash chain / verification | `nostr_verify.h` (new) | Low | **Done** | 1 week |
| 3 | Key management / signer | `nostr_keys.h` (new) | Medium | **Done** | 1 week |
| 4 | P2P transport (iroh) | `nostr_net.h` (new) | High | **Done** | 3 weeks |
| 5 | Lobby / matchmaking | `nostr_lobby.h` (new) | Medium | **Done** | 2 weeks |
| 6 | Lockstep protocol | `connection.c`, `packets.c` | High | Planned | 3 weeks |
| 7 | Savegame serialization | `savegame3.c`, `savemain.c` | High | Planned | 4 weeks |
| 8 | Map generator | `mapgen.c`, `height_map.c` | High | Planned | 4 weeks |
| 9 | Game rules engine | `game.c`, `unittools.c` | Very High | Planned | 8+ weeks |

Steps 1-5 are **new modules** (no C replacement needed -- pure Rust behind FFI).
Steps 6-9 replace **existing C code** and require the full migration pattern below.

## Migration Pattern

For each module, follow these six steps:

1. **Write Rust implementation with tests** -- pure Rust, no FFI concerns, full unit
   and integration test coverage.
2. **Create C FFI wrapper** matching existing C function signatures. Use `extern "C"`
   functions in the `freeciv-nostr-ffi` crate.
3. **Swap in Rust behind the same C interface** -- update the Meson build to link the
   Rust static library instead of the old C object files.
4. **Verify all tests pass** -- `cargo test --workspace`, Meson test suite, and CI.
5. **Remove old C implementation** -- delete the replaced `.c`/`.h` files.
6. **Update cbindgen headers** -- regenerate the C header from the Rust FFI crate so
   downstream C code sees the correct signatures.

## Completed Work (Phases 1-4)

### Phase 1 -- freeciv-nostr-core (111 tests)

Pure Rust Nostr protocol types and logic:

- `kinds.rs` -- Nostr event kind constants for Freeciv
- `events.rs` -- Event builders for game actions, lobby, saves
- `chain.rs` -- Hash chain data structures for move ordering
- `keys.rs` -- Key pair generation and conversion utilities
- `signer.rs` -- Nostr event signing abstraction
- `actions.rs` -- Typed game actions (unit move, city build, diplomacy, etc.)

### Phase 2 -- freeciv-nostr-net (337 tests)

P2P networking layer built on iroh + Nostr relays:

- `endpoint.rs` / `node.rs` -- iroh endpoint and node management
- `gossip.rs` -- iroh-gossip pub/sub for game state broadcast
- `blobs.rs` -- iroh-blobs for savegame/asset transfer
- `transport.rs` -- unified send/receive abstraction
- `lobby.rs` -- game lobby creation and player join
- `lockstep.rs` -- deterministic lockstep synchronization protocol
- `validation.rs` -- incoming message validation
- `desync.rs` -- desync detection and recovery
- `relay.rs` / `nostr_relay.rs` -- Nostr relay connectivity
- `replay.rs` -- game replay from event log
- `profile.rs` -- player profile via Nostr metadata
- `matchmaking.rs` -- public game discovery
- `savegame.rs` (via blobs) -- savegame upload/download
- `protocol.rs` / `message.rs` -- wire format types

### Phase 3 -- freeciv-nostr-verify (45 tests)

Anti-cheat verification:

- `hash_chain.rs` -- SHA-256 hash chain construction and validation
- `commit.rs` -- commit-reveal scheme for simultaneous moves
- `checkpoint.rs` -- periodic game state snapshots
- `persistence.rs` -- chain state serialization to disk
- `verifier.rs` -- orchestrator that ties chain + commit + checkpoint together

### Phase 4 -- freeciv-nostr-ffi (199 tests, 177 exported functions)

C-callable FFI boundary across 12 modules:

- `error.rs` -- thread-local `fcn_last_error()` pattern
- `identity.rs` -- key generation and npub/nsec conversion
- `signer.rs` -- event signing FFI
- `chain.rs` -- hash chain FFI
- `net.rs` -- 88 functions covering endpoint, gossip, blobs, transport, lobby, lockstep
- `verifier.rs` -- verification FFI
- `relay_ffi.rs` -- Nostr relay connection management
- `replay_ffi.rs` -- replay system FFI
- `profile_ffi.rs` -- player profile FFI
- `matchmaking_ffi.rs` -- matchmaking FFI
- `util.rs` -- string/memory helpers (`fcn_string_free`, etc.)
- `lib.rs` -- crate root and version

**Total: 715 tests across the workspace.**

## Guidelines

These rules apply to every migration PR:

1. **Each module migration is a separate PR.** Do not bundle unrelated modules.
2. **No module is migrated without full test coverage.** Unit tests in Rust,
   integration tests verifying the FFI boundary, and existing C test suites must pass.
3. **Cross-platform CI must pass.** Linux (x86_64), macOS (arm64), and Windows (MSVC)
   at minimum.
4. **FFI boundary kept thin.** The FFI layer is glue code only -- no business logic.
   All logic lives in the core/net/verify crates.
5. **Idiomatic Rust.** No C-style Rust. Use `Result`, `Option`, enums, traits, and
   the borrow checker properly. Avoid `unsafe` outside the FFI crate.

## Architecture Decisions

| Decision | Rationale |
|----------|-----------|
| **JSON as FFI bridge** | Passing JSON strings across FFI instead of raw C structs avoids fragile struct layout matching. Slight overhead is acceptable for game-action-rate data. |
| **Thread-local error handling** (`fcn_last_error`) | C callers check `fcn_last_error()` after any FFI call that returns a sentinel. Avoids out-parameter error strings and matches common C patterns (errno). |
| **Scale-factor integers** for float-to-fixed-point | Game values (production, movement points) use integer representations with known scale factors instead of floats, ensuring deterministic cross-platform results. |
| **New binary** (`freeciv-p2p`) | P2P mode merges client and server into one process. This avoids the complexity of retrofitting the existing client-server split for peer-to-peer play. |
| **`-Dnostr=false` by default** | The Nostr/Rust integration is opt-in. Vanilla Freeciv builds remain unchanged -- no Rust toolchain required unless `-Dnostr=true` is set in Meson. |

## Next Steps

The immediate priorities for Phase 5+ are:

1. **Lockstep protocol migration (Step 6)** -- Replace `connection.c` / `packets.c`
   networking with the Rust lockstep implementation already built in
   `freeciv-nostr-net`. This is the highest-value migration because it enables
   deterministic P2P play. Target: the `freeciv-p2p` binary.

2. **Savegame serialization (Step 7)** -- Replace `savegame3.c` with Rust-based
   serialization. The Nostr blob transfer layer already handles save distribution;
   this step handles the save format itself.

3. **Map generator (Step 8)** -- `mapgen.c` and related files. This is a good
   candidate for Rust because it is compute-heavy, largely self-contained, and
   benefits from Rust's safety guarantees on array indexing.

4. **Game rules engine (Step 9)** -- The largest and most complex migration. This
   should wait until Steps 6-8 have proven the migration pattern on real C code
   replacements. Consider splitting into sub-steps (combat, diplomacy, city management).

Each step follows the migration pattern documented above. File a tracking sub-issue
for each step before starting work.

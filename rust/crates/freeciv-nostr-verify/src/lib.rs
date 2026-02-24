//! Deterministic state verification for freeciv-nostr.
//!
//! This crate provides the verification subsystem for the lockstep protocol:
//!
//! - **`hash_chain`**: Cryptographic hash chain linking per-turn state hashes.
//!   Each entry is `H(n) = SHA-256(H(n-1) || turn || state_hash)`, producing
//!   a tamper-evident log of the entire game history.
//!
//! - **`commit`**: Turn commit collection and consensus checking. Gathers
//!   `GAME_STATE_HASH` events (kind 4203) from all players each turn and
//!   verifies they agree on the same state hash.
//!
//! - **`checkpoint`**: Periodic snapshots of the hash chain for faster
//!   desync recovery. Configurable interval and retention.
//!
//! - **`persistence`**: Save/load verification state to disk for recovery
//!   after restarts or disconnects.
//!
//! - **`verifier`**: Top-level `GameVerifier` that orchestrates all of the
//!   above for a single game session.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────┐
//! │                  GameVerifier                     │
//! │  ┌──────────────┐  ┌───────────────────────────┐ │
//! │  │ TurnHashChain│  │   TurnCommitCollector     │ │
//! │  │  H(0)→H(1)→… │  │   player_a: hash_a       │ │
//! │  │              │  │   player_b: hash_b       │ │
//! │  └──────────────┘  │   consensus? ✓/✗         │ │
//! │  ┌──────────────┐  └───────────────────────────┘ │
//! │  │ Checkpoint   │  ┌───────────────────────────┐ │
//! │  │ Manager      │  │   Persistence             │ │
//! │  │  @T0, @T10…  │  │   save/load JSON          │ │
//! │  └──────────────┘  └───────────────────────────┘ │
//! └──────────────────────────────────────────────────┘
//! ```

pub mod checkpoint;
pub mod commit;
pub mod hash_chain;
pub mod persistence;
pub mod verifier;

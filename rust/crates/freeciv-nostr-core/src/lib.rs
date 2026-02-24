//! Nostr protocol types, event construction, NIP-46 remote signing, and hash
//! chain management for freeciv-nostr.
//!
//! This crate provides the core Nostr integration layer for Freeciv,
//! including:
//! - Custom event kind definitions for game actions, lobby, and state
//! - Key management (secp256k1, NIP-19 encoding)
//! - Event creation and validation for all custom kinds
//! - NIP-46 remote signer support (via `nostr-connect`)

pub mod events;
pub mod keys;
pub mod kinds;
pub mod signer;

//! Iroh-based P2P transport layer for freeciv-nostr. Manages per-game
//! ephemeral endpoints, gossip channels, and blob transfer.

pub mod blobs;
pub mod endpoint;
pub mod error;
pub mod gossip;
pub mod lobby;
pub mod lockstep;
pub mod message;
pub mod protocol;

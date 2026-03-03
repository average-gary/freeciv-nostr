//! Iroh-based P2P transport layer for freeciv-nostr. Manages per-game
//! ephemeral endpoints, gossip channels, and blob transfer.

pub mod blobs;
pub mod desync;
pub mod endpoint;
pub mod error;
pub mod gossip;
pub mod lobby;
pub mod lockstep;
pub mod matchmaking;
pub mod message;
pub mod node;
pub mod nostr_relay;
pub mod profile;
pub mod protocol;
pub mod relay;
pub mod replay;
pub mod savegame;
pub mod transport;
pub mod validation;

//! Game action events for the Nostr event chain.

use serde::{Deserialize, Serialize};

/// Represents a single game action that will be published as a Nostr event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameAction {
    /// Turn number when this action occurred.
    pub turn: u64,
    /// Opaque action payload (to be refined in later phases).
    pub payload: Vec<u8>,
}

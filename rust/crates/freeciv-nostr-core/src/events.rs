//! Game action events for the Nostr event chain.

use serde::{Deserialize, Serialize};

/// Represents a single game action that will be published as a Nostr event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameAction {
    /// Turn number when this action occurred.
    pub turn: u64,
    /// Opaque action payload (to be refined in later phases).
    pub payload: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_action_roundtrip_serialization() {
        let action = GameAction {
            turn: 42,
            payload: vec![1, 2, 3, 4],
        };
        let json = serde_json::to_string(&action).expect("serialize");
        let deserialized: GameAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(action, deserialized);
    }
}

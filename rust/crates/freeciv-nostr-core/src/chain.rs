//! Event chain management for action sequencing.
//!
//! Provides `PlayerChain` and `GameChain` types that track the linked-list
//! event chain for each player in a game session. Each action event references
//! the previous event in that player's chain, enabling ordering verification
//! and fork detection.

use std::collections::HashMap;

use nostr::prelude::*;

use crate::actions::PlayerAction;
use crate::kinds;

/// Tracks the event chain for a single player in a game session.
#[derive(Debug, Clone)]
pub struct PlayerChain {
    /// The game's root event ID (Game Start / Lobby event).
    pub game_event_id: EventId,
    /// The player's public key.
    pub player_pubkey: PublicKey,
    /// The most recent event ID in this player's chain.
    pub head_event_id: Option<EventId>,
    /// Next sequence number to use.
    pub next_sequence: u64,
}

impl PlayerChain {
    /// Create a new player chain for the given game and player.
    pub fn new(game_event_id: EventId, player_pubkey: PublicKey) -> Self {
        Self {
            game_event_id,
            player_pubkey,
            head_event_id: None,
            next_sequence: 0,
        }
    }

    /// Build a Nostr event for the given action, advancing the chain.
    /// Returns the EventBuilder (caller must sign it).
    ///
    /// This constructs the event with:
    /// - `e` tag referencing the game event ID
    /// - `seq` tag with the sequence number
    /// - `turn` tag with the turn number
    /// - `phase` tag with the phase number
    /// - `prev` tag referencing previous event ID in chain (empty string for first)
    /// - Content is the JSON-serialized `PlayerAction`
    ///
    /// The action's `sequence` and `prev_event_id` fields are overwritten
    /// with the chain's current values.
    pub fn build_action_event(&mut self, action: &PlayerAction) -> EventBuilder {
        let mut action = action.clone();
        action.sequence = self.next_sequence;
        action.prev_event_id = self.head_event_id.map(|id| id.to_hex()).unwrap_or_default();

        let content =
            serde_json::to_string(&action).expect("PlayerAction serialization should not fail");

        let prev_str = &action.prev_event_id;

        let tags = vec![
            Tag::event(self.game_event_id),
            Tag::custom(TagKind::custom("seq"), vec![action.sequence.to_string()]),
            Tag::custom(TagKind::custom("turn"), vec![action.turn.to_string()]),
            Tag::custom(TagKind::custom("phase"), vec![action.phase.to_string()]),
            Tag::custom(TagKind::custom("prev"), vec![prev_str.clone()]),
        ];

        self.next_sequence += 1;

        EventBuilder::new(kinds::GAME_ACTION, content).tags(tags)
    }

    /// Record that an event was signed and published, updating the head.
    pub fn record_published(&mut self, event_id: EventId) {
        self.head_event_id = Some(event_id);
    }

    /// Validate that an incoming event correctly extends this chain.
    /// Checks: sequence number, prev_event_id reference, player pubkey.
    pub fn validate_incoming(&self, event: &Event) -> Result<PlayerAction, ChainError> {
        // Check that the event is from the expected player
        if event.pubkey != self.player_pubkey {
            return Err(ChainError::UnknownPlayer(event.pubkey.to_hex()));
        }

        // Parse the content as a PlayerAction
        let action: PlayerAction = serde_json::from_str(&event.content)
            .map_err(|e| ChainError::InvalidContent(e.to_string()))?;

        // Check sequence number
        if action.sequence != self.next_sequence {
            return Err(ChainError::SequenceMismatch {
                expected: self.next_sequence,
                got: action.sequence,
            });
        }

        // Check prev_event_id reference
        let expected_prev = self.head_event_id.map(|id| id.to_hex()).unwrap_or_default();

        if action.prev_event_id != expected_prev {
            return Err(ChainError::ChainRefMismatch {
                expected: expected_prev,
                got: action.prev_event_id.clone(),
            });
        }

        Ok(action)
    }
}

/// Tracks all player chains for a game session.
#[derive(Debug)]
pub struct GameChain {
    /// The game's root event ID.
    pub game_event_id: EventId,
    /// Per-player chains, keyed by public key.
    chains: HashMap<PublicKey, PlayerChain>,
}

impl GameChain {
    /// Create a new game chain for the given game event.
    pub fn new(game_event_id: EventId) -> Self {
        Self {
            game_event_id,
            chains: HashMap::new(),
        }
    }

    /// Add a player to the game chain.
    pub fn add_player(&mut self, pubkey: PublicKey) {
        self.chains
            .entry(pubkey)
            .or_insert_with(|| PlayerChain::new(self.game_event_id, pubkey));
    }

    /// Get an immutable reference to a player's chain.
    pub fn get_player_chain(&self, pubkey: &PublicKey) -> Option<&PlayerChain> {
        self.chains.get(pubkey)
    }

    /// Get a mutable reference to a player's chain.
    pub fn get_player_chain_mut(&mut self, pubkey: &PublicKey) -> Option<&mut PlayerChain> {
        self.chains.get_mut(pubkey)
    }

    /// Validate and append an incoming event to the appropriate player's chain.
    pub fn append_event(&mut self, event: &Event) -> Result<PlayerAction, ChainError> {
        let chain = self
            .chains
            .get_mut(&event.pubkey)
            .ok_or_else(|| ChainError::UnknownPlayer(event.pubkey.to_hex()))?;

        // Check for fork: if we already have an event at this sequence and
        // the incoming event has the same sequence number, it's a fork.
        let action: PlayerAction = serde_json::from_str(&event.content)
            .map_err(|e| ChainError::InvalidContent(e.to_string()))?;

        if action.sequence < chain.next_sequence {
            return Err(ChainError::ForkDetected {
                player: event.pubkey.to_hex(),
                sequence: action.sequence,
            });
        }

        // Validate chain integrity
        let validated_action = chain.validate_incoming(event)?;

        // Advance the chain
        chain.next_sequence += 1;
        chain.head_event_id = Some(event.id);

        Ok(validated_action)
    }

    /// Get all chain heads (latest event per player) for turn commit verification.
    pub fn chain_heads(&self) -> Vec<(PublicKey, Option<EventId>)> {
        self.chains
            .iter()
            .map(|(pk, chain)| (*pk, chain.head_event_id))
            .collect()
    }
}

/// Errors that can occur during chain validation.
#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("unknown player: {0}")]
    UnknownPlayer(String),

    #[error("sequence mismatch: expected {expected}, got {got}")]
    SequenceMismatch { expected: u64, got: u64 },

    #[error("chain reference mismatch: expected {expected}, got {got}")]
    ChainRefMismatch { expected: String, got: String },

    #[error("invalid event content: {0}")]
    InvalidContent(String),

    #[error("fork detected: player {player} has two events at sequence {sequence}")]
    ForkDetected { player: String, sequence: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::PacketType;

    fn make_action(turn: u32, phase: u32) -> PlayerAction {
        PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn,
            phase,
            sequence: 0,                  // will be overwritten by chain
            prev_event_id: String::new(), // will be overwritten by chain
            payload: serde_json::json!({"unit_id": 1, "orders": []}),
        }
    }

    fn sign_builder(builder: EventBuilder, keys: &Keys) -> Event {
        let unsigned = builder.build(keys.public_key());
        unsigned
            .sign_with_keys(keys)
            .expect("signing should succeed")
    }

    #[test]
    fn player_chain_builds_first_event_with_seq_zero() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut chain = PlayerChain::new(game_id, keys.public_key());

        let action = make_action(1, 0);
        let builder = chain.build_action_event(&action);
        let unsigned = builder.build(keys.public_key());

        // Check sequence tag is "0"
        let tags: Vec<String> = unsigned
            .tags
            .iter()
            .map(|t| t.as_slice().join(","))
            .collect();
        assert!(
            tags.iter().any(|t| t == "seq,0"),
            "should have seq=0 tag, got: {:?}",
            tags
        );

        // Check prev tag is empty string
        assert!(
            tags.iter().any(|t| t == "prev,"),
            "should have empty prev tag, got: {:?}",
            tags
        );

        // Check content contains sequence 0
        let parsed: PlayerAction = serde_json::from_str(&unsigned.content).expect("valid JSON");
        assert_eq!(parsed.sequence, 0);
        assert_eq!(parsed.prev_event_id, "");
    }

    #[test]
    fn player_chain_builds_second_event_with_seq_one_and_prev() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut chain = PlayerChain::new(game_id, keys.public_key());

        // Build and "publish" first event
        let action1 = make_action(1, 0);
        let builder1 = chain.build_action_event(&action1);
        let event1 = sign_builder(builder1, &keys);
        chain.record_published(event1.id);

        // Build second event
        let action2 = make_action(1, 0);
        let builder2 = chain.build_action_event(&action2);
        let unsigned2 = builder2.build(keys.public_key());

        let parsed: PlayerAction = serde_json::from_str(&unsigned2.content).expect("valid JSON");
        assert_eq!(parsed.sequence, 1);
        assert_eq!(parsed.prev_event_id, event1.id.to_hex());

        // Check tags
        let tags: Vec<String> = unsigned2
            .tags
            .iter()
            .map(|t| t.as_slice().join(","))
            .collect();
        assert!(tags.iter().any(|t| t == "seq,1"));
        assert!(tags
            .iter()
            .any(|t| t.starts_with(&format!("prev,{}", event1.id.to_hex()))));
    }

    #[test]
    fn player_chain_validate_incoming_accepts_valid_event() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut chain = PlayerChain::new(game_id, keys.public_key());

        // Build and sign the first event
        let action = make_action(1, 0);
        let builder = chain.build_action_event(&action);

        // Reset chain state to validate (build_action_event advanced next_sequence)
        chain.next_sequence = 0;
        let event = sign_builder(builder, &keys);

        let result = chain.validate_incoming(&event);
        assert!(result.is_ok(), "should accept valid event: {:?}", result);
        let validated = result.unwrap();
        assert_eq!(validated.sequence, 0);
        assert_eq!(validated.turn, 1);
    }

    #[test]
    fn player_chain_validate_rejects_wrong_sequence() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let chain = PlayerChain::new(game_id, keys.public_key());

        // Create an action with wrong sequence
        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 1,
            phase: 0,
            sequence: 5, // chain expects 0
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let content = serde_json::to_string(&action).unwrap();
        let builder = EventBuilder::new(kinds::GAME_ACTION, content).tags(vec![
            Tag::event(game_id),
            Tag::custom(TagKind::custom("seq"), vec!["5".to_string()]),
        ]);
        let event = sign_builder(builder, &keys);

        let result = chain.validate_incoming(&event);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChainError::SequenceMismatch { expected, got } => {
                assert_eq!(expected, 0);
                assert_eq!(got, 5);
            }
            other => panic!("expected SequenceMismatch, got: {:?}", other),
        }
    }

    #[test]
    fn player_chain_validate_rejects_wrong_prev_reference() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let chain = PlayerChain::new(game_id, keys.public_key());

        // Create an action with correct sequence but wrong prev
        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: "deadbeef".to_string(), // should be empty
            payload: serde_json::json!({}),
        };
        let content = serde_json::to_string(&action).unwrap();
        let builder = EventBuilder::new(kinds::GAME_ACTION, content);
        let event = sign_builder(builder, &keys);

        let result = chain.validate_incoming(&event);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChainError::ChainRefMismatch { expected, got } => {
                assert_eq!(expected, "");
                assert_eq!(got, "deadbeef");
            }
            other => panic!("expected ChainRefMismatch, got: {:?}", other),
        }
    }

    #[test]
    fn player_chain_validate_rejects_wrong_pubkey() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let chain = PlayerChain::new(game_id, keys_a.public_key());

        // Create event from different player
        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let content = serde_json::to_string(&action).unwrap();
        let builder = EventBuilder::new(kinds::GAME_ACTION, content);
        let event = sign_builder(builder, &keys_b); // signed by B, but chain is for A

        let result = chain.validate_incoming(&event);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ChainError::UnknownPlayer(_)));
    }

    #[test]
    fn game_chain_multi_player_scenario() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();

        let mut game_chain = GameChain::new(game_id);
        game_chain.add_player(keys_a.public_key());
        game_chain.add_player(keys_b.public_key());

        // Player A sends first action
        let action_a = make_action(1, 0);
        let chain_a = game_chain
            .get_player_chain_mut(&keys_a.public_key())
            .unwrap();
        let builder_a = chain_a.build_action_event(&action_a);
        let event_a = sign_builder(builder_a, &keys_a);

        // Reset player A's chain state before append_event validates
        let chain_a = game_chain
            .get_player_chain_mut(&keys_a.public_key())
            .unwrap();
        chain_a.next_sequence = 0;
        chain_a.head_event_id = None;

        let result_a = game_chain.append_event(&event_a);
        assert!(
            result_a.is_ok(),
            "Player A's event should be accepted: {:?}",
            result_a
        );

        // Player B sends first action
        let action_b = make_action(1, 0);
        let chain_b = game_chain
            .get_player_chain_mut(&keys_b.public_key())
            .unwrap();
        let builder_b = chain_b.build_action_event(&action_b);
        let event_b = sign_builder(builder_b, &keys_b);

        let chain_b = game_chain
            .get_player_chain_mut(&keys_b.public_key())
            .unwrap();
        chain_b.next_sequence = 0;
        chain_b.head_event_id = None;

        let result_b = game_chain.append_event(&event_b);
        assert!(
            result_b.is_ok(),
            "Player B's event should be accepted: {:?}",
            result_b
        );

        // Check chain heads
        let heads = game_chain.chain_heads();
        assert_eq!(heads.len(), 2);
        for (pk, head) in &heads {
            assert!(
                head.is_some(),
                "Player {} should have a head event",
                pk.to_hex()
            );
        }
    }

    #[test]
    fn game_chain_rejects_unknown_player() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut game_chain = GameChain::new(game_id);
        // Don't add the player

        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let content = serde_json::to_string(&action).unwrap();
        let builder = EventBuilder::new(kinds::GAME_ACTION, content);
        let event = sign_builder(builder, &keys);

        let result = game_chain.append_event(&event);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ChainError::UnknownPlayer(_)));
    }

    #[test]
    fn game_chain_fork_detection() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();

        let mut game_chain = GameChain::new(game_id);
        game_chain.add_player(keys.public_key());

        // Build and append first event
        let action1 = make_action(1, 0);
        {
            let chain = game_chain.get_player_chain_mut(&keys.public_key()).unwrap();
            let builder = chain.build_action_event(&action1);
            let event = sign_builder(builder, &keys);

            // Reset chain to validate
            let chain = game_chain.get_player_chain_mut(&keys.public_key()).unwrap();
            chain.next_sequence = 0;
            chain.head_event_id = None;

            let result = game_chain.append_event(&event);
            assert!(result.is_ok());
        }

        // Now try to submit another event at sequence 0 (fork attempt)
        let fork_action = PlayerAction {
            packet_type: PacketType::UNIT_DO_ACTION,
            turn: 1,
            phase: 0,
            sequence: 0, // same as the already-accepted event
            prev_event_id: String::new(),
            payload: serde_json::json!({"different": true}),
        };
        let content = serde_json::to_string(&fork_action).unwrap();
        let builder = EventBuilder::new(kinds::GAME_ACTION, content);
        let fork_event = sign_builder(builder, &keys);

        let result = game_chain.append_event(&fork_event);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChainError::ForkDetected { player, sequence } => {
                assert_eq!(player, keys.public_key().to_hex());
                assert_eq!(sequence, 0);
            }
            other => panic!("expected ForkDetected, got: {:?}", other),
        }
    }

    #[test]
    fn game_chain_empty_heads() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut game_chain = GameChain::new(game_id);
        game_chain.add_player(keys.public_key());

        let heads = game_chain.chain_heads();
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].0, keys.public_key());
        assert!(heads[0].1.is_none());
    }

    #[test]
    fn player_chain_sequence_advances_on_build() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut chain = PlayerChain::new(game_id, keys.public_key());

        assert_eq!(chain.next_sequence, 0);

        let action = make_action(1, 0);
        let _ = chain.build_action_event(&action);
        assert_eq!(chain.next_sequence, 1);

        let _ = chain.build_action_event(&action);
        assert_eq!(chain.next_sequence, 2);
    }

    #[test]
    fn build_action_event_tags_include_turn_and_phase() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut chain = PlayerChain::new(game_id, keys.public_key());

        let action = make_action(7, 2);
        let builder = chain.build_action_event(&action);
        let unsigned = builder.build(keys.public_key());

        let tags: Vec<String> = unsigned
            .tags
            .iter()
            .map(|t| t.as_slice().join(","))
            .collect();
        assert!(
            tags.iter().any(|t| t == "turn,7"),
            "should have turn=7, got: {:?}",
            tags
        );
        assert!(
            tags.iter().any(|t| t == "phase,2"),
            "should have phase=2, got: {:?}",
            tags
        );
    }

    #[test]
    fn game_chain_add_player_is_idempotent() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut game_chain = GameChain::new(game_id);

        game_chain.add_player(keys.public_key());
        game_chain.add_player(keys.public_key()); // should not panic or duplicate

        assert_eq!(game_chain.chain_heads().len(), 1);
    }
}

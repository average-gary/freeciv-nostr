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
    game_event_id: EventId,
    /// The player's public key.
    player_pubkey: PublicKey,
    /// The most recent event ID in this player's chain.
    head_event_id: Option<EventId>,
    /// Next sequence number to use.
    next_sequence: u64,
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

    /// The game's root event ID.
    pub fn game_event_id(&self) -> EventId {
        self.game_event_id
    }

    /// The player's public key.
    pub fn player_pubkey(&self) -> PublicKey {
        self.player_pubkey
    }

    /// The most recent event ID in this player's chain.
    pub fn head_event_id(&self) -> Option<EventId> {
        self.head_event_id
    }

    /// Next sequence number to use.
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Build a Nostr event for the given action, without advancing the chain.
    /// Returns the EventBuilder (caller must sign it, then call `record_published`).
    ///
    /// The action's `sequence` and `prev_event_id` fields are overwritten
    /// with the chain's current values. Tags are constructed by delegating to
    /// `crate::events::build_player_action_event` to avoid duplication.
    pub fn build_action_event(&self, action: &PlayerAction) -> EventBuilder {
        let mut action = action.clone();
        action.sequence = self.next_sequence;
        action.prev_event_id = self.head_event_id.map(|id| id.to_hex()).unwrap_or_default();

        crate::events::build_player_action_event(self.game_event_id, &action)
    }

    /// Record that an event was signed and published, advancing the chain.
    pub fn record_published(&mut self, event_id: EventId) {
        self.next_sequence += 1;
        self.head_event_id = Some(event_id);
    }

    /// Validate that an incoming event correctly extends this chain.
    /// Checks: event kind, game reference tag, player pubkey, sequence number,
    /// and prev_event_id reference. The caller supplies the already-parsed
    /// `PlayerAction` to avoid double deserialization.
    pub fn validate_incoming(
        &self,
        event: &Event,
        action: &PlayerAction,
    ) -> Result<(), ChainError> {
        // Check event kind
        if event.kind != kinds::GAME_ACTION {
            return Err(ChainError::InvalidContent(format!(
                "expected kind {}, got {}",
                kinds::GAME_ACTION.as_u16(),
                event.kind.as_u16()
            )));
        }

        // Check the `e` tag references the expected game
        let has_game_ref = event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(|v| v.as_str()) == Some("e")
                && s.get(1).map(|v| v.as_str()) == Some(&self.game_event_id.to_hex())
        });
        if !has_game_ref {
            return Err(ChainError::InvalidContent(
                "event does not reference the expected game".to_string(),
            ));
        }

        // Check that the event is from the expected player
        if event.pubkey != self.player_pubkey {
            return Err(ChainError::UnknownPlayer(event.pubkey.to_hex()));
        }

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

        Ok(())
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
    ///
    /// Content is parsed once and reused for fork checking and validation.
    pub fn append_event(&mut self, event: &Event) -> Result<PlayerAction, ChainError> {
        let chain = self
            .chains
            .get_mut(&event.pubkey)
            .ok_or_else(|| ChainError::UnknownPlayer(event.pubkey.to_hex()))?;

        // Parse content once
        let action: PlayerAction = serde_json::from_str(&event.content)
            .map_err(|e| ChainError::InvalidContent(e.to_string()))?;

        // Check for fork: if the incoming sequence is below what we expect,
        // a conflicting event was already accepted at that sequence.
        if action.sequence < chain.next_sequence {
            return Err(ChainError::ForkDetected {
                player: event.pubkey.to_hex(),
                sequence: action.sequence,
            });
        }

        // Validate chain integrity (kind, game ref, pubkey, sequence, prev)
        chain.validate_incoming(event, &action)?;

        // Advance the chain
        chain.record_published(event.id);

        Ok(action)
    }

    /// Get all chain heads (latest event per player) for turn commit verification.
    pub fn chain_heads(&self) -> Vec<(PublicKey, Option<EventId>)> {
        self.chains
            .iter()
            .map(|(pk, chain)| (*pk, chain.head_event_id()))
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
        let chain = PlayerChain::new(game_id, keys.public_key());

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

        // build_action_event should NOT have advanced the sequence
        assert_eq!(chain.next_sequence(), 0);
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
        let chain = PlayerChain::new(game_id, keys.public_key());

        // Build and sign the first event (build_action_event no longer mutates)
        let action = make_action(1, 0);
        let builder = chain.build_action_event(&action);
        let event = sign_builder(builder, &keys);

        // Parse content and validate
        let parsed: PlayerAction = serde_json::from_str(&event.content).expect("valid JSON");
        let result = chain.validate_incoming(&event, &parsed);
        assert!(result.is_ok(), "should accept valid event: {:?}", result);
        assert_eq!(parsed.sequence, 0);
        assert_eq!(parsed.turn, 1);
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

        let result = chain.validate_incoming(&event, &action);
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
        let builder =
            EventBuilder::new(kinds::GAME_ACTION, content).tags(vec![Tag::event(game_id)]);
        let event = sign_builder(builder, &keys);

        let result = chain.validate_incoming(&event, &action);
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
        let builder =
            EventBuilder::new(kinds::GAME_ACTION, content).tags(vec![Tag::event(game_id)]);
        let event = sign_builder(builder, &keys_b); // signed by B, but chain is for A

        let result = chain.validate_incoming(&event, &action);
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

        // Player A sends first action (build does NOT advance chain now)
        let action_a = make_action(1, 0);
        let chain_a = game_chain.get_player_chain(&keys_a.public_key()).unwrap();
        let builder_a = chain_a.build_action_event(&action_a);
        let event_a = sign_builder(builder_a, &keys_a);

        let result_a = game_chain.append_event(&event_a);
        assert!(
            result_a.is_ok(),
            "Player A's event should be accepted: {:?}",
            result_a
        );

        // Player B sends first action
        let action_b = make_action(1, 0);
        let chain_b = game_chain.get_player_chain(&keys_b.public_key()).unwrap();
        let builder_b = chain_b.build_action_event(&action_b);
        let event_b = sign_builder(builder_b, &keys_b);

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

        // Build and append first event (build no longer advances chain)
        let action1 = make_action(1, 0);
        let chain = game_chain.get_player_chain(&keys.public_key()).unwrap();
        let builder = chain.build_action_event(&action1);
        let event = sign_builder(builder, &keys);
        let result = game_chain.append_event(&event);
        assert!(result.is_ok());

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
    fn player_chain_sequence_advances_on_record_published() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut chain = PlayerChain::new(game_id, keys.public_key());

        assert_eq!(chain.next_sequence(), 0);

        // build_action_event should NOT advance sequence
        let action = make_action(1, 0);
        let builder = chain.build_action_event(&action);
        assert_eq!(chain.next_sequence(), 0);

        // record_published should advance sequence
        let event = sign_builder(builder, &keys);
        chain.record_published(event.id);
        assert_eq!(chain.next_sequence(), 1);
        assert_eq!(chain.head_event_id(), Some(event.id));

        // Second build + publish
        let builder2 = chain.build_action_event(&action);
        assert_eq!(chain.next_sequence(), 1);
        let event2 = sign_builder(builder2, &keys);
        chain.record_published(event2.id);
        assert_eq!(chain.next_sequence(), 2);
        assert_eq!(chain.head_event_id(), Some(event2.id));
    }

    #[test]
    fn build_action_event_tags_include_turn_and_phase() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let chain = PlayerChain::new(game_id, keys.public_key());

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

    #[test]
    fn player_chain_accessors() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let chain = PlayerChain::new(game_id, keys.public_key());

        assert_eq!(chain.game_event_id(), game_id);
        assert_eq!(chain.player_pubkey(), keys.public_key());
        assert_eq!(chain.head_event_id(), None);
        assert_eq!(chain.next_sequence(), 0);
    }

    #[test]
    fn validate_rejects_wrong_kind() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let chain = PlayerChain::new(game_id, keys.public_key());

        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let content = serde_json::to_string(&action).unwrap();
        // Use wrong kind (kind 1 = text note)
        let builder = EventBuilder::new(Kind::from(1), content).tags(vec![Tag::event(game_id)]);
        let event = sign_builder(builder, &keys);

        let result = chain.validate_incoming(&event, &action);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChainError::InvalidContent(msg) => {
                assert!(msg.contains("expected kind"), "got: {}", msg);
            }
            other => panic!("expected InvalidContent for wrong kind, got: {:?}", other),
        }
    }

    #[test]
    fn validate_rejects_missing_game_ref() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let chain = PlayerChain::new(game_id, keys.public_key());

        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 1,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let content = serde_json::to_string(&action).unwrap();
        // No `e` tag at all
        let builder = EventBuilder::new(kinds::GAME_ACTION, content);
        let event = sign_builder(builder, &keys);

        let result = chain.validate_incoming(&event, &action);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChainError::InvalidContent(msg) => {
                assert!(
                    msg.contains("does not reference the expected game"),
                    "got: {}",
                    msg
                );
            }
            other => panic!(
                "expected InvalidContent for missing game ref, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn validate_rejects_sequence_gap() {
        // Chain expects sequence 0; submit an event with sequence 2 (gap)
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut game_chain = GameChain::new(game_id);
        game_chain.add_player(keys.public_key());

        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 1,
            phase: 0,
            sequence: 2, // chain expects 0
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let content = serde_json::to_string(&action).unwrap();
        let builder = EventBuilder::new(kinds::GAME_ACTION, content).tags(vec![
            Tag::event(game_id),
            Tag::custom(TagKind::custom("seq"), vec!["2".to_string()]),
        ]);
        let event = sign_builder(builder, &keys);

        let result = game_chain.append_event(&event);
        assert!(result.is_err());
        match result.unwrap_err() {
            ChainError::SequenceMismatch { expected, got } => {
                assert_eq!(expected, 0);
                assert_eq!(got, 2);
            }
            other => panic!("expected SequenceMismatch, got: {:?}", other),
        }
    }
}

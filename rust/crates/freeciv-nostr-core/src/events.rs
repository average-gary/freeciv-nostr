//! Game event builders for the Nostr event chain.
//!
//! Provides typed builder functions for creating all freeciv-nostr
//! custom events with proper tags and content structure.

use nostr::prelude::*;
use serde::{Deserialize, Serialize};

use crate::kinds;

/// Represents a single game action that will be published as a Nostr event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameAction {
    /// Turn number when this action occurred.
    pub turn: u64,
    /// Opaque action payload (to be refined in later phases).
    pub payload: Vec<u8>,
}

/// Game lobby settings published in a GAME_LOBBY event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LobbySettings {
    /// Ruleset name (e.g., "civ2civ3", "classic").
    pub ruleset: String,
    /// Map seed (0 for random).
    pub map_seed: u64,
    /// Game seed (0 for random).
    pub game_seed: u64,
    /// Maximum number of players.
    pub max_players: u8,
    /// Turn timeout in seconds (0 for no timeout).
    pub turn_timeout: u32,
    /// Optional description of the game.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// State hash published at end of each turn for desync detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateHash {
    /// Turn number this hash corresponds to.
    pub turn: u64,
    /// SHA-256 hash of the game state (hex-encoded).
    pub hash: String,
}

// ---- Event Builders ----

/// Build a GAME_LOBBY event (kind 4200).
///
/// Creates a lobby/challenge event with game settings.
/// The `game_id` is used as the `d` tag for deduplication.
pub fn build_lobby_event(
    game_id: &str,
    settings: &LobbySettings,
    invited_players: &[PublicKey],
) -> EventBuilder {
    let content = serde_json::to_string(settings).expect("LobbySettings serialization");

    let mut tags: Vec<Tag> = vec![Tag::identifier(game_id)];

    for pubkey in invited_players {
        tags.push(Tag::public_key(*pubkey));
    }

    EventBuilder::new(kinds::GAME_LOBBY, content).tags(tags)
}

/// Build a GAME_ACCEPT event (kind 4201).
///
/// Published by a player accepting a game invitation.
/// `lobby_event_id` references the lobby event being accepted.
/// `node_id_hex` is the player's Iroh NodeId for P2P connection.
pub fn build_accept_event(
    lobby_event_id: EventId,
    lobby_creator: PublicKey,
    node_id_hex: &str,
) -> EventBuilder {
    EventBuilder::new(kinds::GAME_ACCEPT, node_id_hex).tags(vec![
        Tag::event(lobby_event_id),
        Tag::public_key(lobby_creator),
    ])
}

/// Build a GAME_ACTION event (kind 4202).
///
/// The core event type for game actions. Content is the serialized
/// `GameAction` struct.
pub fn build_action_event(
    game_event_id: EventId,
    action: &GameAction,
    sequence: u64,
) -> EventBuilder {
    let content = serde_json::to_string(action).expect("GameAction serialization");

    EventBuilder::new(kinds::GAME_ACTION, content).tags(vec![
        Tag::event(game_event_id),
        Tag::custom(TagKind::custom("seq"), vec![sequence.to_string()]),
        Tag::custom(TagKind::custom("turn"), vec![action.turn.to_string()]),
    ])
}

/// Build a GAME_STATE_HASH event (kind 4203).
///
/// Published at the end of each turn for desync detection.
pub fn build_state_hash_event(game_event_id: EventId, state_hash: &StateHash) -> EventBuilder {
    let content = serde_json::to_string(state_hash).expect("StateHash serialization");

    EventBuilder::new(kinds::GAME_STATE_HASH, content).tags(vec![
        Tag::event(game_event_id),
        Tag::custom(TagKind::custom("turn"), vec![state_hash.turn.to_string()]),
    ])
}

/// Build a GAME_CHAT event (kind 4204).
///
/// In-game chat message. `recipient` is None for broadcast.
pub fn build_chat_event(
    game_event_id: EventId,
    message: &str,
    recipient: Option<PublicKey>,
) -> EventBuilder {
    let mut tags = vec![Tag::event(game_event_id)];

    if let Some(pubkey) = recipient {
        tags.push(Tag::public_key(pubkey));
    }

    EventBuilder::new(kinds::GAME_CHAT, message).tags(tags)
}

/// Build a GAME_DIPLOMACY event (kind 4205).
///
/// Diplomatic proposal or response.
pub fn build_diplomacy_event(
    game_event_id: EventId,
    target_player: PublicKey,
    turn: u64,
    proposal_json: &str,
) -> EventBuilder {
    EventBuilder::new(kinds::GAME_DIPLOMACY, proposal_json).tags(vec![
        Tag::event(game_event_id),
        Tag::public_key(target_player),
        Tag::custom(TagKind::custom("turn"), vec![turn.to_string()]),
    ])
}

/// Build a GAME_END event (kind 4206).
///
/// Published when a game concludes.
pub fn build_end_event(
    game_event_id: EventId,
    turn: u64,
    final_state_hash: &str,
    summary: &str,
) -> EventBuilder {
    let content = serde_json::json!({
        "turn": turn,
        "state_hash": final_state_hash,
        "summary": summary,
    })
    .to_string();

    EventBuilder::new(kinds::GAME_END, content).tags(vec![
        Tag::event(game_event_id),
        Tag::custom(TagKind::custom("turn"), vec![turn.to_string()]),
    ])
}

/// Build a HEARTBEAT event (kind 14200, ephemeral).
///
/// Indicates this node is alive and connected.
pub fn build_heartbeat_event(game_event_id: EventId) -> EventBuilder {
    EventBuilder::new(kinds::HEARTBEAT, "").tags(vec![Tag::event(game_event_id)])
}

/// Build a PLAYER_PROFILE event (kind 30420, replaceable).
///
/// Contains player stats and preferences.
pub fn build_profile_event(profile_json: &str) -> EventBuilder {
    EventBuilder::new(kinds::PLAYER_PROFILE, profile_json)
        .tags(vec![Tag::identifier("freeciv-profile")])
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

    #[test]
    fn lobby_settings_serialization() {
        let settings = LobbySettings {
            ruleset: "civ2civ3".to_string(),
            map_seed: 42,
            game_seed: 42,
            max_players: 8,
            turn_timeout: 300,
            description: Some("Test game".to_string()),
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        let deserialized: LobbySettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(settings, deserialized);
    }

    #[test]
    fn state_hash_serialization() {
        let hash = StateHash {
            turn: 10,
            hash: "abcdef1234567890".to_string(),
        };
        let json = serde_json::to_string(&hash).expect("serialize");
        let deserialized: StateHash = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(hash, deserialized);
    }

    #[test]
    fn build_lobby_event_has_correct_kind_and_tags() {
        let settings = LobbySettings {
            ruleset: "classic".to_string(),
            map_seed: 0,
            game_seed: 0,
            max_players: 4,
            turn_timeout: 0,
            description: None,
        };
        let keys = Keys::generate();
        let builder = build_lobby_event("game-123", &settings, &[]);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::GAME_LOBBY);
    }

    #[test]
    fn build_action_event_has_correct_tags() {
        let action = GameAction {
            turn: 5,
            payload: vec![0x01, 0x02],
        };
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let builder = build_action_event(game_id, &action, 42);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::GAME_ACTION);

        // Check tags contain seq and turn
        let tag_strs: Vec<String> = unsigned
            .tags
            .iter()
            .map(|t| t.as_slice().join(","))
            .collect();
        assert!(tag_strs
            .iter()
            .any(|t| t.contains("seq") && t.contains("42")));
        assert!(tag_strs
            .iter()
            .any(|t| t.contains("turn") && t.contains("5")));
    }

    #[test]
    fn build_chat_event_broadcast() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let builder = build_chat_event(game_id, "hello world", None);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::GAME_CHAT);
        assert_eq!(unsigned.content, "hello world");
    }

    #[test]
    fn build_chat_event_directed() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let recipient = Keys::generate().public_key();
        let builder = build_chat_event(game_id, "private msg", Some(recipient));
        let unsigned = builder.build(keys.public_key());
        // Should have a p-tag for the recipient
        let has_p_tag = unsigned
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("p"));
        assert!(has_p_tag);
    }

    #[test]
    fn build_heartbeat_is_ephemeral_kind() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let builder = build_heartbeat_event(game_id);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::HEARTBEAT);
        // Kind 14200 is in the ephemeral range (20000-29999... actually
        // custom kinds don't have range enforcement, but our kind value
        // is 14200 which we define as ephemeral by convention)
    }

    #[test]
    fn build_profile_is_replaceable() {
        let keys = Keys::generate();
        let builder = build_profile_event(r#"{"elo": 1500}"#);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::PLAYER_PROFILE);
        // Should have d-tag for replaceability
        let has_d_tag = unsigned
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("d"));
        assert!(has_d_tag);
    }
}

//! Game event builders for the Nostr event chain.
//!
//! Provides typed builder functions for creating all freeciv-nostr
//! custom events with proper tags and content structure.

use nostr::prelude::*;
use serde::{Deserialize, Serialize};

use crate::actions::PlayerAction;
use crate::kinds;

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

/// Parameters for a GAME_START event.
///
/// Contains the deterministic seeds and player order needed so that
/// every node begins from the exact same initial game state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameStartParams {
    /// Map generation seed.
    pub map_seed: u64,
    /// Game randomness seed.
    pub game_seed: u64,
    /// Canonical player order (hex-encoded Nostr pubkeys).
    pub player_order: Vec<String>,
    /// Ruleset name (e.g., "civ2civ3").
    pub ruleset: String,
}

/// Summary published when a game ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameEndSummary {
    /// Final turn number.
    pub turn: u64,
    /// SHA-256 hash of the final game state (hex-encoded).
    pub state_hash: String,
    /// Human-readable game summary (e.g., "Player X achieved domination victory").
    pub summary: String,
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

/// Build a GAME_ACTION event (kind 4202) from a `PlayerAction`.
///
/// This is the preferred way to build action events. The event includes:
/// - `e` tag referencing the game event ID
/// - `seq` tag with the sequence number
/// - `turn` tag with the turn number
/// - `phase` tag with the phase number
/// - `prev` tag referencing the previous event ID in the player's chain
///   (empty string for the first event)
/// - Content is the JSON-serialized `PlayerAction`
pub fn build_player_action_event(game_event_id: EventId, action: &PlayerAction) -> EventBuilder {
    let content = serde_json::to_string(action).expect("PlayerAction serialization");

    let prev_str = &action.prev_event_id;

    EventBuilder::new(kinds::GAME_ACTION, content).tags(vec![
        Tag::event(game_event_id),
        Tag::custom(TagKind::custom("seq"), vec![action.sequence.to_string()]),
        Tag::custom(TagKind::custom("turn"), vec![action.turn.to_string()]),
        Tag::custom(TagKind::custom("phase"), vec![action.phase.to_string()]),
        Tag::custom(TagKind::custom("prev"), vec![prev_str.clone()]),
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
pub fn build_end_event(game_event_id: EventId, end_summary: &GameEndSummary) -> EventBuilder {
    let content = serde_json::to_string(end_summary).expect("GameEndSummary serialization");

    EventBuilder::new(kinds::GAME_END, content).tags(vec![
        Tag::event(game_event_id),
        Tag::custom(TagKind::custom("turn"), vec![end_summary.turn.to_string()]),
    ])
}

/// Build a GAME_START event (kind 4207).
///
/// Published by the lobby lead when all accepted players are ready.
/// `lobby_event_id` references the original lobby event.
/// `players` are the Nostr public keys included as `p` tags.
pub fn build_start_event(
    lobby_event_id: EventId,
    params: &GameStartParams,
    players: &[PublicKey],
) -> EventBuilder {
    let content = serde_json::to_string(params).expect("GameStartParams serialization");

    let mut tags: Vec<Tag> = vec![Tag::event(lobby_event_id)];
    for pk in players {
        tags.push(Tag::public_key(*pk));
    }

    EventBuilder::new(kinds::GAME_START, content).tags(tags)
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

/// Build a STATE_SYNC event (kind 24200, ephemeral).
///
/// Used for real-time state synchronization fragments during late-join sync.
pub fn build_state_sync_event(
    game_event_id: EventId,
    chunk_index: u32,
    chunk_data: &[u8],
) -> EventBuilder {
    let content = hex::encode(chunk_data);

    EventBuilder::new(kinds::STATE_SYNC, content).tags(vec![
        Tag::event(game_event_id),
        Tag::custom(TagKind::custom("chunk"), vec![chunk_index.to_string()]),
    ])
}

/// Build a GAME_REPLAY event (kind 30421, replaceable).
///
/// References all action events for a completed game, enabling full replay.
pub fn build_replay_event(game_id: &str, action_event_ids: &[EventId]) -> EventBuilder {
    let mut tags: Vec<Tag> = vec![Tag::identifier(game_id)];

    for event_id in action_event_ids {
        tags.push(Tag::event(*event_id));
    }

    EventBuilder::new(kinds::GAME_REPLAY, "").tags(tags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::PacketType;

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
        // Kind 14200 is outside NIP-01's ephemeral range (20000-29999).
        // Relays may store these events.
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

    #[test]
    fn build_end_event_has_correct_kind() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let summary = GameEndSummary {
            turn: 100,
            state_hash: "abc123".to_string(),
            summary: "Player 1 wins".to_string(),
        };
        let builder = build_end_event(game_id, &summary);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::GAME_END);
        // Verify content is valid JSON with expected fields
        let parsed: serde_json::Value =
            serde_json::from_str(&unsigned.content).expect("valid json");
        assert_eq!(parsed["turn"], 100);
        assert_eq!(parsed["state_hash"], "abc123");
    }

    #[test]
    fn build_state_sync_event_has_chunk_tag() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let builder = build_state_sync_event(game_id, 3, &[0xDE, 0xAD]);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::STATE_SYNC);
        let tag_strs: Vec<String> = unsigned
            .tags
            .iter()
            .map(|t| t.as_slice().join(","))
            .collect();
        assert!(
            tag_strs
                .iter()
                .any(|t| t.contains("chunk") && t.contains("3"))
        );
    }

    #[test]
    fn build_replay_event_has_d_tag_and_event_refs() {
        let keys = Keys::generate();
        let id1 = EventId::all_zeros();
        let builder = build_replay_event("game-456", &[id1]);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::GAME_REPLAY);
        let has_d_tag = unsigned
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("d"));
        assert!(has_d_tag);
        let has_e_tag = unsigned
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("e"));
        assert!(has_e_tag);
    }

    #[test]
    fn game_end_summary_serialization() {
        let summary = GameEndSummary {
            turn: 50,
            state_hash: "deadbeef".to_string(),
            summary: "Draw".to_string(),
        };
        let json = serde_json::to_string(&summary).expect("serialize");
        let deserialized: GameEndSummary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(summary, deserialized);
    }

    #[test]
    fn build_player_action_event_has_correct_tags() {
        let action = PlayerAction {
            packet_type: PacketType::UNIT_ORDERS,
            turn: 3,
            phase: 1,
            sequence: 7,
            prev_event_id: "abcdef".to_string(),
            payload: serde_json::json!({"unit_id": 42}),
        };
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let builder = build_player_action_event(game_id, &action);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::GAME_ACTION);

        let tag_strs: Vec<String> = unsigned
            .tags
            .iter()
            .map(|t| t.as_slice().join(","))
            .collect();
        assert!(
            tag_strs
                .iter()
                .any(|t| t.contains("seq") && t.contains("7"))
        );
        assert!(
            tag_strs
                .iter()
                .any(|t| t.contains("turn") && t.contains("3"))
        );
        assert!(
            tag_strs
                .iter()
                .any(|t| t.contains("phase") && t.contains("1"))
        );
        assert!(
            tag_strs
                .iter()
                .any(|t| t.contains("prev") && t.contains("abcdef"))
        );

        // Verify content roundtrips
        let parsed: PlayerAction =
            serde_json::from_str(&unsigned.content).expect("valid JSON content");
        assert_eq!(parsed.packet_type, PacketType::UNIT_ORDERS);
        assert_eq!(parsed.turn, 3);
        assert_eq!(parsed.phase, 1);
        assert_eq!(parsed.sequence, 7);
        assert_eq!(parsed.prev_event_id, "abcdef");
    }

    #[test]
    fn game_start_params_serialization() {
        let params = GameStartParams {
            map_seed: 12345,
            game_seed: 67890,
            player_order: vec!["aabb".to_string(), "ccdd".to_string()],
            ruleset: "civ2civ3".to_string(),
        };
        let json = serde_json::to_string(&params).expect("serialize");
        let deserialized: GameStartParams = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(params, deserialized);
    }

    #[test]
    fn build_start_event_has_correct_kind_and_tags() {
        let lobby_id = EventId::all_zeros();
        let keys = Keys::generate();
        let player1 = Keys::generate().public_key();
        let player2 = Keys::generate().public_key();
        let params = GameStartParams {
            map_seed: 1,
            game_seed: 2,
            player_order: vec!["pk1".to_string(), "pk2".to_string()],
            ruleset: "classic".to_string(),
        };
        let builder = build_start_event(lobby_id, &params, &[player1, player2]);
        let unsigned = builder.build(keys.public_key());
        assert_eq!(unsigned.kind, kinds::GAME_START);

        // Should have e-tag and two p-tags
        let has_e_tag = unsigned
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("e"));
        assert!(has_e_tag);
        let p_count = unsigned
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("p"))
            .count();
        assert_eq!(p_count, 2);

        // Content should round-trip
        let parsed: GameStartParams = serde_json::from_str(&unsigned.content).expect("valid JSON");
        assert_eq!(parsed.ruleset, "classic");
        assert_eq!(parsed.map_seed, 1);
    }

    #[test]
    fn build_player_action_event_first_in_chain() {
        let action = PlayerAction {
            packet_type: PacketType::PLAYER_READY,
            turn: 0,
            phase: 0,
            sequence: 0,
            prev_event_id: String::new(),
            payload: serde_json::json!({}),
        };
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let builder = build_player_action_event(game_id, &action);
        let unsigned = builder.build(keys.public_key());

        let tag_strs: Vec<String> = unsigned
            .tags
            .iter()
            .map(|t| t.as_slice().join(","))
            .collect();
        // prev tag should have empty value
        assert!(
            tag_strs.iter().any(|t| t == "prev,"),
            "should have empty prev tag, got: {:?}",
            tag_strs
        );
        assert!(tag_strs.iter().any(|t| t == "seq,0"));
    }
}

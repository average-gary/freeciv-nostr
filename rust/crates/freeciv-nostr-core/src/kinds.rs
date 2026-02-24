//! Custom Nostr event kind definitions for freeciv-nostr.
//!
//! These kinds follow the Nostr NIP-01 event structure and use custom
//! kind numbers in ranges reserved for application-specific use.
//!
//! ## Kind Ranges
//! - **4200-4206**: Regular (stored) events for game coordination
//! - **14200**: Ephemeral events (not stored by relays)
//! - **24200**: Ephemeral events for real-time state
//! - **30420-30421**: Parameterized replaceable events

use nostr::Kind;

// ---- Regular Events (stored by relays) ----

/// Game Lobby / Challenge (kind 4200)
///
/// Published when a player creates a new game lobby or issues a challenge.
/// Contains game settings (ruleset, map seed, player count, etc.).
///
/// Tags: `d` (unique game ID), `p` (invited players, optional)
pub const GAME_LOBBY: Kind = Kind::Custom(4200);

/// Game Accept (kind 4201)
///
/// Published by a player accepting a game lobby invitation.
/// References the lobby event and includes the player's Iroh NodeId.
///
/// Tags: `e` (lobby event ID), `p` (lobby creator pubkey)
pub const GAME_ACCEPT: Kind = Kind::Custom(4201);

/// Game Action (kind 4202)
///
/// The core event type: represents a single game action (unit move,
/// city production change, diplomacy action, etc.) that must be
/// applied deterministically by all nodes.
///
/// Tags: `e` (game start event), `seq` (sequence number), `turn`, `phase`
pub const GAME_ACTION: Kind = Kind::Custom(4202);

/// Game State Hash (kind 4203)
///
/// Published at the end of each turn by each node. Contains the
/// SHA-256 hash of the game state after applying all actions for that turn.
/// Used for desync detection in the lockstep protocol.
///
/// Tags: `e` (game start event), `turn`
pub const GAME_STATE_HASH: Kind = Kind::Custom(4203);

/// Game Chat (kind 4204)
///
/// In-game chat messages between players.
///
/// Tags: `e` (game start event), `p` (recipient, optional for broadcast)
pub const GAME_CHAT: Kind = Kind::Custom(4204);

/// Game Diplomacy (kind 4205)
///
/// Diplomatic proposals and responses (treaty, alliance, peace, war).
///
/// Tags: `e` (game start event), `p` (target player), `turn`
pub const GAME_DIPLOMACY: Kind = Kind::Custom(4205);

/// Game End (kind 4206)
///
/// Published when a game concludes (victory, concession, or timeout).
/// Contains final state hash and game summary.
///
/// Tags: `e` (game start event), `turn`
pub const GAME_END: Kind = Kind::Custom(4206);

// ---- Ephemeral Events ----
// NOTE: Per NIP-01, only kinds 20000-29999 are treated as ephemeral by
// relays (not stored). Kind 14200 (HEARTBEAT) falls outside this range
// and MAY be stored by relays. If relay-side ephemerality is needed,
// consider moving HEARTBEAT to the 20000-29999 range in a future revision.

/// Heartbeat / Presence (kind 14200)
///
/// Indicates a player's node is alive and connected. Used for connection
/// health monitoring. Note: kind 14200 is outside NIP-01's ephemeral range
/// (20000-29999), so relays may store these events.
///
/// Tags: `e` (game start event)
pub const HEARTBEAT: Kind = Kind::Custom(14200);

/// Real-Time State Sync (kind 24200)
///
/// Ephemeral event for real-time state synchronization fragments.
/// Used during initial sync for late joiners.
///
/// Tags: `e` (game start event), `chunk` (chunk index)
pub const STATE_SYNC: Kind = Kind::Custom(24200);

// ---- Parameterized Replaceable Events ----

/// Player Profile (kind 30420)
///
/// Replaceable event containing a player's game profile: preferred
/// nation, ELO rating, game history stats. Updated as the player
/// completes games.
///
/// Tags: `d` ("freeciv-profile")
pub const PLAYER_PROFILE: Kind = Kind::Custom(30420);

/// Game Replay (kind 30421)
///
/// Replaceable event referencing all action events for a completed game.
/// Allows full game replay from the event chain.
///
/// Tags: `d` (game ID), `e` (all action event IDs)
pub const GAME_REPLAY: Kind = Kind::Custom(30421);

/// Returns the human-readable name for a freeciv-nostr event kind.
pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        k if k == GAME_LOBBY => "GameLobby",
        k if k == GAME_ACCEPT => "GameAccept",
        k if k == GAME_ACTION => "GameAction",
        k if k == GAME_STATE_HASH => "GameStateHash",
        k if k == GAME_CHAT => "GameChat",
        k if k == GAME_DIPLOMACY => "GameDiplomacy",
        k if k == GAME_END => "GameEnd",
        k if k == HEARTBEAT => "Heartbeat",
        k if k == STATE_SYNC => "StateSync",
        k if k == PLAYER_PROFILE => "PlayerProfile",
        k if k == GAME_REPLAY => "GameReplay",
        _ => "Unknown",
    }
}

/// Returns true if this kind is a freeciv-nostr custom kind.
pub fn is_freeciv_kind(kind: Kind) -> bool {
    kind == GAME_LOBBY
        || kind == GAME_ACCEPT
        || kind == GAME_ACTION
        || kind == GAME_STATE_HASH
        || kind == GAME_CHAT
        || kind == GAME_DIPLOMACY
        || kind == GAME_END
        || kind == HEARTBEAT
        || kind == STATE_SYNC
        || kind == PLAYER_PROFILE
        || kind == GAME_REPLAY
}

/// All regular (stored) game event kinds.
pub const REGULAR_KINDS: &[Kind] = &[
    GAME_LOBBY,
    GAME_ACCEPT,
    GAME_ACTION,
    GAME_STATE_HASH,
    GAME_CHAT,
    GAME_DIPLOMACY,
    GAME_END,
];

/// All kinds that should be pre-approved for NIP-46 signing
/// (to avoid per-action prompts during gameplay).
pub const PRE_APPROVED_KINDS: &[Kind] = &[
    GAME_ACTION,
    GAME_STATE_HASH,
    GAME_CHAT,
    GAME_DIPLOMACY,
    HEARTBEAT,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_values_are_correct() {
        assert_eq!(GAME_LOBBY.as_u16(), 4200);
        assert_eq!(GAME_ACCEPT.as_u16(), 4201);
        assert_eq!(GAME_ACTION.as_u16(), 4202);
        assert_eq!(GAME_STATE_HASH.as_u16(), 4203);
        assert_eq!(GAME_CHAT.as_u16(), 4204);
        assert_eq!(GAME_DIPLOMACY.as_u16(), 4205);
        assert_eq!(GAME_END.as_u16(), 4206);
        assert_eq!(HEARTBEAT.as_u16(), 14200);
        assert_eq!(STATE_SYNC.as_u16(), 24200);
        assert_eq!(PLAYER_PROFILE.as_u16(), 30420);
        assert_eq!(GAME_REPLAY.as_u16(), 30421);
    }

    #[test]
    fn kind_names_are_correct() {
        assert_eq!(kind_name(GAME_LOBBY), "GameLobby");
        assert_eq!(kind_name(GAME_ACTION), "GameAction");
        assert_eq!(kind_name(HEARTBEAT), "Heartbeat");
        assert_eq!(kind_name(Kind::Custom(9999)), "Unknown");
    }

    #[test]
    fn is_freeciv_kind_identifies_custom_kinds() {
        assert!(is_freeciv_kind(GAME_LOBBY));
        assert!(is_freeciv_kind(GAME_ACTION));
        assert!(is_freeciv_kind(HEARTBEAT));
        assert!(is_freeciv_kind(PLAYER_PROFILE));
        assert!(!is_freeciv_kind(Kind::Custom(9999)));
        assert!(!is_freeciv_kind(Kind::TextNote));
    }

    #[test]
    fn pre_approved_kinds_are_subset_of_all() {
        for kind in PRE_APPROVED_KINDS {
            assert!(
                is_freeciv_kind(*kind),
                "{:?} should be a freeciv kind",
                kind
            );
        }
    }
}

//! Lobby protocol for peer discovery and game session setup.
//!
//! A [`GameLobby`] tracks which players have accepted a game invitation,
//! stores their P2P endpoint addresses, and orchestrates the transition
//! from the "waiting for players" phase to the "game running" phase.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::endpoint::GameEndpoint;
use crate::error::NetError;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors specific to lobby operations.
#[derive(Debug, thiserror::Error)]
pub enum LobbyError {
    /// The lobby is not in the `Open` state.
    #[error("lobby is not open")]
    NotOpen,
    /// The lobby is full (max players reached).
    #[error("lobby is full")]
    Full,
    /// The player has already been accepted.
    #[error("player already accepted")]
    AlreadyAccepted,
    /// The caller is not the lobby lead.
    #[error("not the lobby lead")]
    NotLead,
    /// Cannot start with zero players.
    #[error("no players accepted")]
    NoPlayers,
    /// Networking error.
    #[error("net: {0}")]
    Net(#[from] NetError),
}

// ---------------------------------------------------------------------------
// State & player types
// ---------------------------------------------------------------------------

/// Current state of the lobby.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyState {
    /// Accepting players.
    Open,
    /// All players confirmed; waiting for lead to start.
    Ready,
    /// Game has been started.
    Started,
    /// Lobby was cancelled.
    Cancelled,
}

/// A player that has been accepted into the lobby.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptedPlayer {
    /// Hex-encoded Nostr public key.
    pub pubkey_hex: String,
    /// JSON-serialised Iroh `EndpointAddr` for P2P connection.
    pub endpoint_addr_json: String,
    /// Hex-encoded Nostr event ID of the accept event.
    pub accept_event_id_hex: String,
}

// ---------------------------------------------------------------------------
// GameLobby
// ---------------------------------------------------------------------------

/// Manages the player-acceptance phase of a game session.
///
/// The lobby tracks a unique `lobby_id`, the lead player (who created the
/// lobby), the maximum number of players, and the set of players who have
/// accepted. State transitions follow: `Open` -> `Ready` | `Started` |
/// `Cancelled`.
#[derive(Debug)]
pub struct GameLobby {
    lobby_id: String,
    lead_pubkey_hex: String,
    max_players: u8,
    state: LobbyState,
    players: HashMap<String, AcceptedPlayer>,
}

impl GameLobby {
    /// Create a new lobby.
    ///
    /// `lobby_id` is the unique game identifier (e.g., Nostr event ID).
    /// `lead_pubkey_hex` is the hex-encoded Nostr pubkey of the lobby creator.
    /// `max_players` is the maximum number of players that can join (must be >= 2).
    pub fn new(lobby_id: &str, lead_pubkey_hex: &str, max_players: u8) -> Self {
        Self {
            lobby_id: lobby_id.to_string(),
            lead_pubkey_hex: lead_pubkey_hex.to_string(),
            max_players,
            state: LobbyState::Open,
            players: HashMap::new(),
        }
    }

    /// Accept a player into the lobby.
    ///
    /// Returns `Ok(())` on success or a [`LobbyError`] if the lobby is not
    /// open, is full, or the player was already accepted.
    pub fn accept_player(
        &mut self,
        pubkey_hex: &str,
        endpoint_addr_json: &str,
        accept_event_id_hex: &str,
    ) -> Result<(), LobbyError> {
        if self.state != LobbyState::Open {
            return Err(LobbyError::NotOpen);
        }
        if self.players.len() >= self.max_players as usize {
            return Err(LobbyError::Full);
        }
        if self.players.contains_key(pubkey_hex) {
            return Err(LobbyError::AlreadyAccepted);
        }
        self.players.insert(
            pubkey_hex.to_string(),
            AcceptedPlayer {
                pubkey_hex: pubkey_hex.to_string(),
                endpoint_addr_json: endpoint_addr_json.to_string(),
                accept_event_id_hex: accept_event_id_hex.to_string(),
            },
        );
        Ok(())
    }

    /// Check whether `pubkey_hex` is the lobby lead.
    pub fn is_lead(&self, pubkey_hex: &str) -> bool {
        self.lead_pubkey_hex == pubkey_hex
    }

    /// Transition the lobby to `Started`.
    ///
    /// Only the lead may call this, and there must be at least one accepted
    /// player.
    pub fn start(&mut self, lead_pubkey_hex: &str) -> Result<(), LobbyError> {
        if !self.is_lead(lead_pubkey_hex) {
            return Err(LobbyError::NotLead);
        }
        if self.state != LobbyState::Open {
            return Err(LobbyError::NotOpen);
        }
        if self.players.is_empty() {
            return Err(LobbyError::NoPlayers);
        }
        self.state = LobbyState::Started;
        Ok(())
    }

    /// Transition the lobby to `Cancelled`.
    pub fn cancel(&mut self) {
        self.state = LobbyState::Cancelled;
    }

    /// Look up an accepted player by their hex pubkey.
    pub fn get_player(&self, pubkey_hex: &str) -> Option<&AcceptedPlayer> {
        self.players.get(pubkey_hex)
    }

    /// Return all accepted players.
    pub fn accepted_players(&self) -> Vec<&AcceptedPlayer> {
        self.players.values().collect()
    }

    /// Number of accepted players.
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Current lobby state.
    pub fn state(&self) -> LobbyState {
        self.state
    }

    /// The lobby identifier.
    pub fn lobby_id(&self) -> &str {
        &self.lobby_id
    }

    /// The lead player's hex-encoded pubkey.
    pub fn lead_pubkey_hex(&self) -> &str {
        &self.lead_pubkey_hex
    }

    /// Maximum number of players.
    pub fn max_players(&self) -> u8 {
        self.max_players
    }
}

// ---------------------------------------------------------------------------
// Peer connection helper
// ---------------------------------------------------------------------------

/// Connect our endpoint to every accepted player's Iroh endpoint.
///
/// Iterates through the lobby's accepted players (skipping `our_pubkey_hex`)
/// and calls `endpoint.connect()` for each one. Returns the number of
/// successful connections.
pub async fn connect_to_lobby_peers(
    lobby: &GameLobby,
    endpoint: &GameEndpoint,
    our_pubkey_hex: &str,
) -> Result<usize, LobbyError> {
    let mut connected = 0usize;
    for player in lobby.accepted_players() {
        if player.pubkey_hex == our_pubkey_hex {
            continue;
        }
        let addr: iroh::EndpointAddr =
            serde_json::from_str(&player.endpoint_addr_json).map_err(|e| {
                LobbyError::Net(NetError::Connect(format!(
                    "bad endpoint addr for {}: {e}",
                    player.pubkey_hex
                )))
            })?;
        endpoint.connect(addr).await?;
        connected += 1;
    }
    Ok(connected)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lobby() -> GameLobby {
        GameLobby::new("lobby-1", "lead_pk", 4)
    }

    // -- constructor -------------------------------------------------------

    #[test]
    fn new_lobby_is_open() {
        let lobby = make_lobby();
        assert_eq!(lobby.state(), LobbyState::Open);
    }

    #[test]
    fn new_lobby_has_zero_players() {
        let lobby = make_lobby();
        assert_eq!(lobby.player_count(), 0);
    }

    #[test]
    fn new_lobby_stores_metadata() {
        let lobby = make_lobby();
        assert_eq!(lobby.lobby_id(), "lobby-1");
        assert_eq!(lobby.lead_pubkey_hex(), "lead_pk");
        assert_eq!(lobby.max_players(), 4);
    }

    // -- accept_player -----------------------------------------------------

    #[test]
    fn accept_player_succeeds() {
        let mut lobby = make_lobby();
        let res = lobby.accept_player("pk1", r#"{"id":"abc"}"#, "evt1");
        assert!(res.is_ok());
        assert_eq!(lobby.player_count(), 1);
    }

    #[test]
    fn accept_player_stores_fields() {
        let mut lobby = make_lobby();
        lobby
            .accept_player("pk1", r#"{"id":"abc"}"#, "evt1")
            .unwrap();
        let p = lobby.get_player("pk1").unwrap();
        assert_eq!(p.pubkey_hex, "pk1");
        assert_eq!(p.endpoint_addr_json, r#"{"id":"abc"}"#);
        assert_eq!(p.accept_event_id_hex, "evt1");
    }

    #[test]
    fn accept_player_duplicate_fails() {
        let mut lobby = make_lobby();
        lobby.accept_player("pk1", "{}", "e1").unwrap();
        let res = lobby.accept_player("pk1", "{}", "e2");
        assert!(matches!(res, Err(LobbyError::AlreadyAccepted)));
    }

    #[test]
    fn accept_player_full_fails() {
        let mut lobby = GameLobby::new("id", "lead", 2);
        lobby.accept_player("pk1", "{}", "e1").unwrap();
        lobby.accept_player("pk2", "{}", "e2").unwrap();
        let res = lobby.accept_player("pk3", "{}", "e3");
        assert!(matches!(res, Err(LobbyError::Full)));
    }

    #[test]
    fn accept_player_not_open_fails() {
        let mut lobby = make_lobby();
        lobby.cancel();
        let res = lobby.accept_player("pk1", "{}", "e1");
        assert!(matches!(res, Err(LobbyError::NotOpen)));
    }

    // -- is_lead -----------------------------------------------------------

    #[test]
    fn is_lead_true_for_lead() {
        let lobby = make_lobby();
        assert!(lobby.is_lead("lead_pk"));
    }

    #[test]
    fn is_lead_false_for_other() {
        let lobby = make_lobby();
        assert!(!lobby.is_lead("other_pk"));
    }

    // -- start -------------------------------------------------------------

    #[test]
    fn start_by_lead_succeeds() {
        let mut lobby = make_lobby();
        lobby.accept_player("pk1", "{}", "e1").unwrap();
        let res = lobby.start("lead_pk");
        assert!(res.is_ok());
        assert_eq!(lobby.state(), LobbyState::Started);
    }

    #[test]
    fn start_by_non_lead_fails() {
        let mut lobby = make_lobby();
        lobby.accept_player("pk1", "{}", "e1").unwrap();
        let res = lobby.start("other");
        assert!(matches!(res, Err(LobbyError::NotLead)));
    }

    #[test]
    fn start_with_no_players_fails() {
        let mut lobby = make_lobby();
        let res = lobby.start("lead_pk");
        assert!(matches!(res, Err(LobbyError::NoPlayers)));
    }

    #[test]
    fn start_when_cancelled_fails() {
        let mut lobby = make_lobby();
        lobby.accept_player("pk1", "{}", "e1").unwrap();
        lobby.cancel();
        let res = lobby.start("lead_pk");
        assert!(matches!(res, Err(LobbyError::NotOpen)));
    }

    #[test]
    fn start_twice_fails() {
        let mut lobby = make_lobby();
        lobby.accept_player("pk1", "{}", "e1").unwrap();
        lobby.start("lead_pk").unwrap();
        let res = lobby.start("lead_pk");
        assert!(matches!(res, Err(LobbyError::NotOpen)));
    }

    // -- cancel ------------------------------------------------------------

    #[test]
    fn cancel_sets_state() {
        let mut lobby = make_lobby();
        lobby.cancel();
        assert_eq!(lobby.state(), LobbyState::Cancelled);
    }

    // -- accepted_players / get_player -------------------------------------

    #[test]
    fn accepted_players_returns_all() {
        let mut lobby = make_lobby();
        lobby.accept_player("pk1", "{}", "e1").unwrap();
        lobby.accept_player("pk2", "{}", "e2").unwrap();
        let players = lobby.accepted_players();
        assert_eq!(players.len(), 2);
    }

    #[test]
    fn get_player_missing_returns_none() {
        let lobby = make_lobby();
        assert!(lobby.get_player("nonexistent").is_none());
    }
}

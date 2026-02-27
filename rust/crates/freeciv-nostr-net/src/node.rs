//! P2P game node: merged client+server for decentralized gameplay.
//!
//! In the P2P model, each player runs a full game node that contains both
//! client and server logic. The `GameNode` orchestrates the lobby, lockstep
//! protocol, validation, desync detection, and gossip layers.

use serde::{Deserialize, Serialize};

use crate::desync::{DesyncConfig, DesyncDetector};
use crate::lobby::GameLobby;
use crate::lockstep::{LockstepConfig, LockstepProtocol, PhaseMode};
use crate::relay::{ConnectionMonitor, RelayConfig};
use crate::validation::ConsensusValidator;

/// State of the game node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    /// Node is initializing (creating endpoint, joining gossip).
    Initializing,
    /// In the lobby, waiting for players.
    InLobby,
    /// Game is starting (connecting to all peers).
    Connecting,
    /// Game is in progress.
    Playing,
    /// Game has ended.
    Finished,
    /// Node encountered an error.
    Error,
}

/// Configuration for a game node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// This player's public key (hex).
    pub player_pubkey: String,
    /// Whether this node is the game lead (lobby creator).
    pub is_lead: bool,
    /// Game's Nostr event ID (hex, set after lobby creation).
    pub game_event_id: Option<String>,
    /// Phase mode for the lockstep protocol.
    pub phase_mode: PhaseMode,
    /// Turn timeout in seconds (0 = no timeout).
    pub turn_timeout_secs: u32,
    /// Checkpoint interval for desync recovery (0 = disabled).
    pub checkpoint_interval: u32,
    /// Relay configuration.
    pub relay_config: RelayConfig,
}

/// The core P2P game node that orchestrates all subsystems.
///
/// This is the top-level type for the `freeciv-p2p` binary concept.
/// It owns or references all the subsystems:
/// - Lobby management
/// - Lockstep protocol
/// - Action validation (consensus)
/// - Desync detection
/// - Connection monitoring
#[derive(Debug)]
pub struct GameNode {
    config: NodeConfig,
    state: NodeState,
    lobby: Option<GameLobby>,
    lockstep: Option<LockstepProtocol>,
    consensus: Option<ConsensusValidator>,
    desync: Option<DesyncDetector>,
    connection_monitor: ConnectionMonitor,
    /// All player pubkeys in the game (set when game starts).
    players: Vec<String>,
    /// Current turn number.
    current_turn: u32,
}

impl GameNode {
    /// Create a new game node.
    pub fn new(config: NodeConfig) -> Self {
        Self {
            config,
            state: NodeState::Initializing,
            lobby: None,
            lockstep: None,
            consensus: None,
            desync: None,
            connection_monitor: ConnectionMonitor::new(),
            players: Vec::new(),
            current_turn: 0,
        }
    }

    /// Get the current node state.
    pub fn state(&self) -> NodeState {
        self.state
    }

    /// Get this node's player pubkey.
    pub fn player_pubkey(&self) -> &str {
        &self.config.player_pubkey
    }

    /// Whether this node is the game lead.
    pub fn is_lead(&self) -> bool {
        self.config.is_lead
    }

    /// Get the current turn.
    pub fn current_turn(&self) -> u32 {
        self.current_turn
    }

    /// Get the list of players.
    pub fn players(&self) -> &[String] {
        &self.players
    }

    /// Transition to lobby state and create a lobby.
    pub fn create_lobby(&mut self, lobby_event_id: &str, max_players: u8) -> Result<(), NodeError> {
        if self.state != NodeState::Initializing {
            return Err(NodeError::InvalidState {
                expected: "Initializing",
                got: format!("{:?}", self.state),
            });
        }
        self.lobby = Some(GameLobby::new(
            lobby_event_id,
            &self.config.player_pubkey,
            max_players,
        ));
        self.state = NodeState::InLobby;
        Ok(())
    }

    /// Join an existing lobby (non-lead player).
    pub fn join_lobby(
        &mut self,
        lobby_event_id: &str,
        lead_pubkey: &str,
        max_players: u8,
    ) -> Result<(), NodeError> {
        if self.state != NodeState::Initializing {
            return Err(NodeError::InvalidState {
                expected: "Initializing",
                got: format!("{:?}", self.state),
            });
        }
        self.lobby = Some(GameLobby::new(lobby_event_id, lead_pubkey, max_players));
        self.state = NodeState::InLobby;
        Ok(())
    }

    /// Start the game (transitions from InLobby to Connecting, then Playing).
    pub fn start_game(&mut self, player_pubkeys: Vec<String>) -> Result<(), NodeError> {
        if self.state != NodeState::InLobby {
            return Err(NodeError::InvalidState {
                expected: "InLobby",
                got: format!("{:?}", self.state),
            });
        }

        self.players = player_pubkeys.clone();

        // Initialize subsystems
        let timeout = if self.config.turn_timeout_secs > 0 {
            Some(std::time::Duration::from_secs(
                self.config.turn_timeout_secs as u64,
            ))
        } else {
            None
        };

        self.lockstep = Some(LockstepProtocol::new(LockstepConfig {
            phase_mode: self.config.phase_mode,
            turn_timeout: timeout,
            player_pubkeys: player_pubkeys.clone(),
        }));

        self.consensus = Some(ConsensusValidator::new(player_pubkeys.len()));

        self.desync = Some(DesyncDetector::new(DesyncConfig {
            checkpoint_interval: self.config.checkpoint_interval,
            max_checkpoints: 10,
            player_pubkeys,
        }));

        self.state = NodeState::Connecting;
        Ok(())
    }

    /// Mark connections as established, transition to Playing.
    pub fn connections_ready(&mut self) -> Result<(), NodeError> {
        if self.state != NodeState::Connecting {
            return Err(NodeError::InvalidState {
                expected: "Connecting",
                got: format!("{:?}", self.state),
            });
        }
        self.state = NodeState::Playing;
        Ok(())
    }

    /// Begin a new turn.
    pub fn begin_turn(&mut self, turn: u32) -> Result<(), NodeError> {
        if self.state != NodeState::Playing {
            return Err(NodeError::InvalidState {
                expected: "Playing",
                got: format!("{:?}", self.state),
            });
        }
        self.current_turn = turn;
        if let Some(ref mut ls) = self.lockstep {
            ls.begin_turn(turn);
        }
        Ok(())
    }

    /// End the game.
    pub fn end_game(&mut self) {
        self.state = NodeState::Finished;
    }

    /// Get a reference to the lockstep protocol.
    pub fn lockstep(&self) -> Option<&LockstepProtocol> {
        self.lockstep.as_ref()
    }

    /// Get a mutable reference to the lockstep protocol.
    pub fn lockstep_mut(&mut self) -> Option<&mut LockstepProtocol> {
        self.lockstep.as_mut()
    }

    /// Get a reference to the desync detector.
    pub fn desync(&self) -> Option<&DesyncDetector> {
        self.desync.as_ref()
    }

    /// Get a mutable reference to the desync detector.
    pub fn desync_mut(&mut self) -> Option<&mut DesyncDetector> {
        self.desync.as_mut()
    }

    /// Get a reference to the connection monitor.
    pub fn connection_monitor(&self) -> &ConnectionMonitor {
        &self.connection_monitor
    }

    /// Get a mutable reference to the connection monitor.
    pub fn connection_monitor_mut(&mut self) -> &mut ConnectionMonitor {
        &mut self.connection_monitor
    }

    /// Get a reference to the consensus validator.
    pub fn consensus(&self) -> Option<&ConsensusValidator> {
        self.consensus.as_ref()
    }

    /// Get a mutable reference to the consensus validator.
    pub fn consensus_mut(&mut self) -> Option<&mut ConsensusValidator> {
        self.consensus.as_mut()
    }
}

/// Errors from the game node.
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// Invalid state transition.
    #[error("invalid state transition: expected {expected}, got {got}")]
    InvalidState {
        /// The expected state.
        expected: &'static str,
        /// The actual state.
        got: String,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config(is_lead: bool) -> NodeConfig {
        NodeConfig {
            player_pubkey: "player_abc".to_string(),
            is_lead,
            game_event_id: None,
            phase_mode: PhaseMode::Concurrent,
            turn_timeout_secs: 0,
            checkpoint_interval: 5,
            relay_config: RelayConfig::default(),
        }
    }

    // -- Construction -----------------------------------------------------

    #[test]
    fn new_node_is_initializing() {
        let node = GameNode::new(default_config(true));
        assert_eq!(node.state(), NodeState::Initializing);
        assert_eq!(node.player_pubkey(), "player_abc");
        assert!(node.is_lead());
        assert_eq!(node.current_turn(), 0);
        assert!(node.players().is_empty());
    }

    #[test]
    fn new_node_non_lead() {
        let node = GameNode::new(default_config(false));
        assert!(!node.is_lead());
    }

    // -- Create lobby (lead path) -----------------------------------------

    #[test]
    fn create_lobby_transitions_to_in_lobby() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_event_1", 4).unwrap();
        assert_eq!(node.state(), NodeState::InLobby);
    }

    #[test]
    fn create_lobby_from_wrong_state_fails() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_event_1", 4).unwrap();
        // Already in InLobby, should fail.
        let err = node.create_lobby("lobby_event_2", 4).unwrap_err();
        assert!(err.to_string().contains("Initializing"));
    }

    // -- Join lobby (non-lead path) ---------------------------------------

    #[test]
    fn join_lobby_transitions_to_in_lobby() {
        let mut node = GameNode::new(default_config(false));
        node.join_lobby("lobby_event_1", "lead_pk", 4).unwrap();
        assert_eq!(node.state(), NodeState::InLobby);
    }

    #[test]
    fn join_lobby_from_wrong_state_fails() {
        let mut node = GameNode::new(default_config(false));
        node.join_lobby("lobby_event_1", "lead_pk", 4).unwrap();
        let err = node.join_lobby("lobby_event_2", "lead_pk", 4).unwrap_err();
        assert!(err.to_string().contains("Initializing"));
    }

    // -- Start game -------------------------------------------------------

    #[test]
    fn start_game_transitions_to_connecting() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_1", 4).unwrap();
        node.start_game(vec!["alice".into(), "bob".into()]).unwrap();
        assert_eq!(node.state(), NodeState::Connecting);
        assert_eq!(node.players(), &["alice", "bob"]);
    }

    #[test]
    fn start_game_initializes_subsystems() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_1", 4).unwrap();
        node.start_game(vec!["alice".into(), "bob".into()]).unwrap();

        assert!(node.lockstep().is_some());
        assert!(node.consensus().is_some());
        assert!(node.desync().is_some());
    }

    #[test]
    fn start_game_from_wrong_state_fails() {
        let mut node = GameNode::new(default_config(true));
        // Still Initializing.
        let err = node
            .start_game(vec!["alice".into(), "bob".into()])
            .unwrap_err();
        assert!(err.to_string().contains("InLobby"));
    }

    // -- Connections ready ------------------------------------------------

    #[test]
    fn connections_ready_transitions_to_playing() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_1", 4).unwrap();
        node.start_game(vec!["alice".into(), "bob".into()]).unwrap();
        node.connections_ready().unwrap();
        assert_eq!(node.state(), NodeState::Playing);
    }

    #[test]
    fn connections_ready_from_wrong_state_fails() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_1", 4).unwrap();
        // Still InLobby.
        let err = node.connections_ready().unwrap_err();
        assert!(err.to_string().contains("Connecting"));
    }

    // -- Begin turn -------------------------------------------------------

    #[test]
    fn begin_turn_updates_turn_number() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_1", 4).unwrap();
        node.start_game(vec!["alice".into(), "bob".into()]).unwrap();
        node.connections_ready().unwrap();

        node.begin_turn(1).unwrap();
        assert_eq!(node.current_turn(), 1);

        node.begin_turn(2).unwrap();
        assert_eq!(node.current_turn(), 2);
    }

    #[test]
    fn begin_turn_from_wrong_state_fails() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_1", 4).unwrap();
        // Still InLobby.
        let err = node.begin_turn(1).unwrap_err();
        assert!(err.to_string().contains("Playing"));
    }

    // -- End game ---------------------------------------------------------

    #[test]
    fn end_game_transitions_to_finished() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_1", 4).unwrap();
        node.start_game(vec!["alice".into()]).unwrap();
        node.connections_ready().unwrap();
        node.end_game();
        assert_eq!(node.state(), NodeState::Finished);
    }

    // -- Full lifecycle ---------------------------------------------------

    #[test]
    fn full_lifecycle() {
        let mut node = GameNode::new(default_config(true));
        assert_eq!(node.state(), NodeState::Initializing);

        node.create_lobby("lobby_1", 4).unwrap();
        assert_eq!(node.state(), NodeState::InLobby);

        node.start_game(vec!["alice".into(), "bob".into()]).unwrap();
        assert_eq!(node.state(), NodeState::Connecting);

        node.connections_ready().unwrap();
        assert_eq!(node.state(), NodeState::Playing);

        node.begin_turn(1).unwrap();
        assert_eq!(node.current_turn(), 1);

        node.begin_turn(2).unwrap();
        assert_eq!(node.current_turn(), 2);

        node.end_game();
        assert_eq!(node.state(), NodeState::Finished);
    }

    // -- Subsystem accessors before start ---------------------------------

    #[test]
    fn subsystems_none_before_start() {
        let node = GameNode::new(default_config(true));
        assert!(node.lockstep().is_none());
        assert!(node.consensus().is_none());
        assert!(node.desync().is_none());
    }

    // -- Connection monitor always available ------------------------------

    #[test]
    fn connection_monitor_always_available() {
        let mut node = GameNode::new(default_config(true));
        assert_eq!(node.connection_monitor().active_count(), 0);
        // Should be usable via mutable ref too.
        let monitor = node.connection_monitor_mut();
        monitor.update(crate::relay::ConnectionQuality {
            conn_type: crate::relay::ConnectionType::Direct,
            rtt: None,
            is_active: true,
            peer_id: "peer1".to_string(),
        });
        assert_eq!(node.connection_monitor().active_count(), 1);
    }

    // -- Mutable subsystem accessors after start --------------------------

    #[test]
    fn mutable_subsystem_accessors() {
        let mut node = GameNode::new(default_config(true));
        node.create_lobby("lobby_1", 4).unwrap();
        node.start_game(vec!["alice".into(), "bob".into()]).unwrap();

        assert!(node.lockstep_mut().is_some());
        assert!(node.consensus_mut().is_some());
        assert!(node.desync_mut().is_some());
    }

    // -- NodeState serde --------------------------------------------------

    #[test]
    fn node_state_serde_roundtrip() {
        let states = vec![
            NodeState::Initializing,
            NodeState::InLobby,
            NodeState::Connecting,
            NodeState::Playing,
            NodeState::Finished,
            NodeState::Error,
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            let back: NodeState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, back);
        }
    }

    // -- NodeConfig with timeout ------------------------------------------

    #[test]
    fn start_game_with_timeout() {
        let mut config = default_config(true);
        config.turn_timeout_secs = 30;
        let mut node = GameNode::new(config);
        node.create_lobby("lobby_1", 4).unwrap();
        node.start_game(vec!["alice".into()]).unwrap();
        assert!(node.lockstep().is_some());
    }

    // -- NodeError display ------------------------------------------------

    #[test]
    fn node_error_display() {
        let err = NodeError::InvalidState {
            expected: "InLobby",
            got: "Initializing".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("InLobby"));
        assert!(msg.contains("Initializing"));
    }
}

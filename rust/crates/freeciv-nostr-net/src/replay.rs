//! Game replay system for deterministic replay and live observation.
//!
//! Enables:
//! - Reconstructing game state from a Nostr event chain
//! - Live observation of in-progress games (read-only)
//! - Replay controls (play, pause, step, jump)
//! - Verification of signatures and state hashes during replay
//! - Sharing game replays as Nostr event links

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

use freeciv_nostr_core::kinds;

// ---------------------------------------------------------------------------
// Recorded types
// ---------------------------------------------------------------------------

/// A single recorded action extracted from a Nostr event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedAction {
    /// Turn number when this action occurred.
    pub turn: u64,
    /// Game phase within the turn.
    pub phase: u32,
    /// Sequence number for ordering within (turn, phase).
    pub sequence: u64,
    /// Hex-encoded public key of the player who performed the action.
    pub player_pubkey: String,
    /// The action payload as a JSON value.
    pub action: serde_json::Value,
    /// Hex-encoded Nostr event ID that contained this action.
    pub event_id: String,
    /// Whether the event's Nostr signature was valid.
    pub signature_valid: bool,
}

/// A recorded state hash from a player at the end of a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedStateHash {
    /// Turn number this hash corresponds to.
    pub turn: u64,
    /// Hex-encoded public key of the player who submitted the hash.
    pub player_pubkey: String,
    /// The state hash value (hex-encoded SHA-256).
    pub hash: String,
    /// Hex-encoded Nostr event ID that contained this hash.
    pub event_id: String,
}

/// A complete recording of a game, assembled from Nostr events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRecording {
    /// The game's unique identifier (hex-encoded event ID).
    pub game_id: String,
    /// Game start parameters (seeds, player order, ruleset, etc.).
    pub start_params: serde_json::Value,
    /// Hex-encoded public keys of all players.
    pub players: Vec<String>,
    /// All actions sorted by (turn, phase, sequence).
    pub actions: Vec<RecordedAction>,
    /// State hashes submitted at the end of each turn.
    pub state_hashes: Vec<RecordedStateHash>,
    /// Optional game end summary.
    pub end_summary: Option<serde_json::Value>,
    /// Total number of turns in the game.
    pub total_turns: u64,
}

/// Result of verifying a game recording's integrity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayVerification {
    /// Whether all Nostr event signatures were valid.
    pub signatures_valid: bool,
    /// Whether state hashes are internally consistent per turn.
    pub state_hashes_consistent: bool,
    /// Number of actions that were verified.
    pub actions_verified: u64,
    /// List of issues found during verification.
    pub issues: Vec<String>,
}

// ---------------------------------------------------------------------------
// Replay state machine
// ---------------------------------------------------------------------------

/// State of the replay controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayState {
    /// Replay is stopped (at the beginning).
    Stopped,
    /// Replay is playing automatically.
    Playing,
    /// Replay is paused.
    Paused,
    /// Replay has reached the end.
    Finished,
}

// ---------------------------------------------------------------------------
// GameRecording implementation
// ---------------------------------------------------------------------------

impl GameRecording {
    /// Construct a `GameRecording` from a set of raw Nostr event JSON strings.
    ///
    /// Events are parsed, sorted, and categorised by kind:
    /// - Kind 4207 (GAME_START): provides start_params and player list
    /// - Kind 4202 (GAME_ACTION): recorded as actions
    /// - Kind 4203 (GAME_STATE_HASH): recorded as state hashes
    /// - Kind 4206 (GAME_END): provides end summary
    ///
    /// Returns `None` if no valid events could be parsed.
    pub fn from_events(game_id: &str, event_jsons: &[String]) -> Option<Self> {
        let mut start_params = serde_json::Value::Null;
        let mut players: Vec<String> = Vec::new();
        let mut actions: Vec<RecordedAction> = Vec::new();
        let mut state_hashes: Vec<RecordedStateHash> = Vec::new();
        let mut end_summary: Option<serde_json::Value> = None;
        let mut max_turn: u64 = 0;

        for event_json in event_jsons {
            let event: serde_json::Value = match serde_json::from_str(event_json) {
                Ok(v) => v,
                Err(e) => {
                    warn!("failed to parse event JSON: {e}");
                    continue;
                }
            };

            let kind = event.get("kind").and_then(|k| k.as_u64()).unwrap_or(0);
            let event_id = event
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let pubkey = event
                .get("pubkey")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let content_str = event.get("content").and_then(|v| v.as_str()).unwrap_or("");

            // Check signature presence (basic check: sig field exists and is non-empty)
            let sig_valid = event
                .get("sig")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());

            if kind == kinds::GAME_START.as_u16() as u64 {
                // Parse start params from content
                if let Ok(params) = serde_json::from_str::<serde_json::Value>(content_str) {
                    start_params = params;
                }
                // Extract player list from p-tags
                if let Some(tags) = event.get("tags").and_then(|t| t.as_array()) {
                    for tag in tags {
                        if let Some(arr) = tag.as_array()
                            && arr.first().and_then(|v| v.as_str()) == Some("p")
                            && let Some(pk) = arr.get(1).and_then(|v| v.as_str())
                            && !players.contains(&pk.to_string())
                        {
                            players.push(pk.to_string());
                        }
                    }
                }
            } else if kind == kinds::GAME_ACTION.as_u16() as u64 {
                // Parse action content
                let action_value: serde_json::Value =
                    serde_json::from_str(content_str).unwrap_or(serde_json::Value::Null);

                // Extract turn, phase, sequence from tags
                let (turn, phase, sequence) = extract_action_tags(&event);

                if turn > max_turn {
                    max_turn = turn;
                }

                actions.push(RecordedAction {
                    turn,
                    phase,
                    sequence,
                    player_pubkey: pubkey,
                    action: action_value,
                    event_id,
                    signature_valid: sig_valid,
                });
            } else if kind == kinds::GAME_STATE_HASH.as_u16() as u64 {
                // Parse state hash from content
                let hash_value: serde_json::Value =
                    serde_json::from_str(content_str).unwrap_or(serde_json::Value::Null);
                let hash = hash_value
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let turn = hash_value.get("turn").and_then(|v| v.as_u64()).unwrap_or(0);

                if turn > max_turn {
                    max_turn = turn;
                }

                state_hashes.push(RecordedStateHash {
                    turn,
                    player_pubkey: pubkey,
                    hash,
                    event_id,
                });
            } else if kind == kinds::GAME_END.as_u16() as u64
                && let Ok(summary) = serde_json::from_str::<serde_json::Value>(content_str)
            {
                end_summary = Some(summary);
            }
        }

        // Sort actions by (turn, phase, sequence)
        actions.sort_by(|a, b| {
            a.turn
                .cmp(&b.turn)
                .then(a.phase.cmp(&b.phase))
                .then(a.sequence.cmp(&b.sequence))
        });

        // Sort state hashes by turn
        state_hashes.sort_by_key(|h| h.turn);

        // Return None if we have absolutely no data
        if actions.is_empty() && state_hashes.is_empty() && start_params.is_null() {
            return None;
        }

        Some(GameRecording {
            game_id: game_id.to_string(),
            start_params,
            players,
            actions,
            state_hashes,
            end_summary,
            total_turns: max_turn,
        })
    }

    /// Get all actions for a specific turn.
    pub fn actions_for_turn(&self, turn: u64) -> Vec<&RecordedAction> {
        self.actions.iter().filter(|a| a.turn == turn).collect()
    }

    /// Get all state hashes for a specific turn.
    pub fn state_hash_for_turn(&self, turn: u64) -> Vec<&RecordedStateHash> {
        self.state_hashes
            .iter()
            .filter(|h| h.turn == turn)
            .collect()
    }

    /// Verify the integrity of the recording.
    ///
    /// Checks:
    /// 1. All action signatures are marked as valid
    /// 2. State hashes are consistent per turn (all players agree)
    /// 3. Actions are properly ordered
    pub fn verify(&self) -> ReplayVerification {
        let mut issues = Vec::new();
        let mut signatures_valid = true;
        let mut state_hashes_consistent = true;

        // 1. Check all action signatures
        for action in &self.actions {
            if !action.signature_valid {
                signatures_valid = false;
                issues.push(format!(
                    "invalid signature on action event {} (turn {}, seq {})",
                    action.event_id, action.turn, action.sequence
                ));
            }
        }

        // 2. Check state hash consistency per turn
        let mut turns_seen: std::collections::HashMap<u64, Vec<&RecordedStateHash>> =
            std::collections::HashMap::new();
        for hash in &self.state_hashes {
            turns_seen.entry(hash.turn).or_default().push(hash);
        }
        for (turn, hashes) in &turns_seen {
            if hashes.len() > 1 {
                let first_hash = &hashes[0].hash;
                for h in &hashes[1..] {
                    if h.hash != *first_hash {
                        state_hashes_consistent = false;
                        issues.push(format!(
                            "state hash mismatch at turn {turn}: {} ({}) vs {} ({})",
                            first_hash, hashes[0].player_pubkey, h.hash, h.player_pubkey
                        ));
                    }
                }
            }
        }

        // 3. Check action ordering
        for window in self.actions.windows(2) {
            let a = &window[0];
            let b = &window[1];
            let ordering = a
                .turn
                .cmp(&b.turn)
                .then(a.phase.cmp(&b.phase))
                .then(a.sequence.cmp(&b.sequence));
            if ordering == std::cmp::Ordering::Greater {
                issues.push(format!(
                    "actions out of order: ({},{},{}) before ({},{},{})",
                    a.turn, a.phase, a.sequence, b.turn, b.phase, b.sequence
                ));
            }
        }

        ReplayVerification {
            signatures_valid,
            state_hashes_consistent,
            actions_verified: self.actions.len() as u64,
            issues,
        }
    }

    /// Generate a Nostr share link for this game replay.
    ///
    /// Returns a `nostr:naddr`-style reference using the game_id as the
    /// `d` tag identifier for the kind 30421 (GAME_REPLAY) event.
    pub fn to_nostr_share_link(&self) -> String {
        // Use a deterministic hash of game_id as a simple share identifier.
        // In a full implementation this would encode a proper NIP-19 naddr.
        let mut hasher = Sha256::new();
        hasher.update(self.game_id.as_bytes());
        let hash = hex::encode(hasher.finalize());
        format!(
            "nostr:naddr:{}:{}:{}",
            kinds::GAME_REPLAY.as_u16(),
            self.game_id,
            &hash[..16]
        )
    }
}

// ---------------------------------------------------------------------------
// ReplayController
// ---------------------------------------------------------------------------

/// Controls playback of a game recording.
///
/// Provides play/pause/stop/step/jump controls for navigating through
/// the recorded actions.
pub struct ReplayController {
    recording: GameRecording,
    state: ReplayState,
    current_index: usize,
    current_turn: u64,
    speed: f64,
}

impl ReplayController {
    /// Create a new replay controller for the given recording.
    pub fn new(recording: GameRecording) -> Self {
        Self {
            recording,
            state: ReplayState::Stopped,
            current_index: 0,
            current_turn: 0,
            speed: 1.0,
        }
    }

    /// Start or resume playback.
    pub fn play(&mut self) {
        if self.state != ReplayState::Finished {
            self.state = ReplayState::Playing;
        }
    }

    /// Pause playback.
    pub fn pause(&mut self) {
        if self.state == ReplayState::Playing {
            self.state = ReplayState::Paused;
        }
    }

    /// Stop playback and reset to the beginning.
    pub fn stop(&mut self) {
        self.state = ReplayState::Stopped;
        self.current_index = 0;
        self.current_turn = 0;
    }

    /// Advance one action forward.
    ///
    /// Returns the action that was stepped over, or `None` if at the end.
    pub fn step_forward(&mut self) -> Option<&RecordedAction> {
        if self.current_index >= self.recording.actions.len() {
            self.state = ReplayState::Finished;
            return None;
        }

        let action = &self.recording.actions[self.current_index];
        self.current_turn = action.turn;
        self.current_index += 1;

        if self.current_index >= self.recording.actions.len() {
            self.state = ReplayState::Finished;
        } else {
            // Only transition to Paused if we were Stopped (manual step);
            // if already Playing or Paused, keep that state.
            if self.state == ReplayState::Stopped {
                self.state = ReplayState::Paused;
            }
        }

        Some(action)
    }

    /// Step one action backward.
    ///
    /// Returns the action at the new position, or `None` if at the beginning.
    pub fn step_backward(&mut self) -> Option<&RecordedAction> {
        if self.current_index == 0 {
            return None;
        }

        self.current_index -= 1;
        let action = &self.recording.actions[self.current_index];
        self.current_turn = action.turn;

        if self.state == ReplayState::Finished {
            self.state = ReplayState::Paused;
        }

        Some(action)
    }

    /// Jump to the first action of a specific turn.
    ///
    /// Returns `true` if the turn was found and the position updated.
    pub fn jump_to_turn(&mut self, turn: u64) -> bool {
        // Find the first action at or after the given turn
        if let Some(idx) = self.recording.actions.iter().position(|a| a.turn >= turn) {
            self.current_index = idx;
            self.current_turn = self.recording.actions[idx].turn;
            if self.state == ReplayState::Finished || self.state == ReplayState::Stopped {
                self.state = ReplayState::Paused;
            }
            true
        } else if turn > self.recording.total_turns {
            // Beyond the end — go to finished
            self.current_index = self.recording.actions.len();
            self.current_turn = self.recording.total_turns;
            self.state = ReplayState::Finished;
            false
        } else {
            // Turn exists but has no actions; jump to end of actions before it
            self.current_index = self.recording.actions.len();
            self.current_turn = turn;
            self.state = ReplayState::Finished;
            false
        }
    }

    /// Get the current turn number.
    pub fn current_turn(&self) -> u64 {
        self.current_turn
    }

    /// Get the current action index.
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// Get the current replay state.
    pub fn state(&self) -> ReplayState {
        self.state
    }

    /// Get the playback speed multiplier.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Set the playback speed multiplier.
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed;
    }

    /// Get the replay progress as a fraction in [0.0, 1.0].
    pub fn progress(&self) -> f64 {
        if self.recording.actions.is_empty() {
            return 0.0;
        }
        self.current_index as f64 / self.recording.actions.len() as f64
    }

    /// Get a reference to the underlying recording.
    pub fn recording(&self) -> &GameRecording {
        &self.recording
    }
}

// ---------------------------------------------------------------------------
// GameObserver
// ---------------------------------------------------------------------------

/// Observes a live game in progress, collecting actions in real-time.
///
/// Provides a read-only view of the game as events arrive.
pub struct GameObserver {
    game_id: String,
    actions: Vec<RecordedAction>,
    current_turn: u64,
    active: bool,
}

impl GameObserver {
    /// Create a new observer for the given game.
    pub fn new(game_id: &str) -> Self {
        Self {
            game_id: game_id.to_string(),
            actions: Vec::new(),
            current_turn: 0,
            active: true,
        }
    }

    /// Process an incoming event JSON string.
    ///
    /// Returns `true` if the event was a game action and was recorded.
    pub fn receive_event(&mut self, event_json: &str) -> bool {
        if !self.active {
            return false;
        }

        let event: serde_json::Value = match serde_json::from_str(event_json) {
            Ok(v) => v,
            Err(_) => return false,
        };

        let kind = event.get("kind").and_then(|k| k.as_u64()).unwrap_or(0);

        if kind == kinds::GAME_END.as_u16() as u64 {
            self.active = false;
            return false;
        }

        if kind != kinds::GAME_ACTION.as_u16() as u64 {
            return false;
        }

        let event_id = event
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let pubkey = event
            .get("pubkey")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let content_str = event.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let action_value: serde_json::Value =
            serde_json::from_str(content_str).unwrap_or(serde_json::Value::Null);

        let sig_valid = event
            .get("sig")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());

        let (turn, phase, sequence) = extract_action_tags(&event);

        if turn > self.current_turn {
            self.current_turn = turn;
        }

        self.actions.push(RecordedAction {
            turn,
            phase,
            sequence,
            player_pubkey: pubkey,
            action: action_value,
            event_id,
            signature_valid: sig_valid,
        });

        true
    }

    /// Get the current turn number based on received events.
    pub fn current_turn(&self) -> u64 {
        self.current_turn
    }

    /// Get the total number of recorded actions.
    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    /// Check if the observer is still active (game not ended).
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Stop observing the game.
    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Get the game ID being observed.
    pub fn game_id(&self) -> &str {
        &self.game_id
    }

    /// Get a reference to all recorded actions.
    pub fn actions(&self) -> &[RecordedAction] {
        &self.actions
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Extract turn, phase, and sequence from a Nostr event's tags.
fn extract_action_tags(event: &serde_json::Value) -> (u64, u32, u64) {
    let mut turn = 0u64;
    let mut phase = 0u32;
    let mut sequence = 0u64;

    if let Some(tags) = event.get("tags").and_then(|t| t.as_array()) {
        for tag in tags {
            if let Some(arr) = tag.as_array() {
                let key = arr.first().and_then(|v| v.as_str()).unwrap_or("");
                let val = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
                match key {
                    "turn" => turn = val.parse().unwrap_or(0),
                    "phase" => phase = val.parse().unwrap_or(0),
                    "seq" => sequence = val.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
    }

    (turn, phase, sequence)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Helper to build fake event JSON ----------------------------------

    fn make_action_event(
        id: &str,
        pubkey: &str,
        turn: u64,
        phase: u32,
        seq: u64,
        payload: serde_json::Value,
    ) -> String {
        let content = serde_json::to_string(&serde_json::json!({
            "packet_type": 84,
            "turn": turn,
            "phase": phase,
            "sequence": seq,
            "prev_event_id": "",
            "payload": payload
        }))
        .unwrap();

        serde_json::to_string(&serde_json::json!({
            "id": id,
            "pubkey": pubkey,
            "kind": kinds::GAME_ACTION.as_u16(),
            "content": content,
            "tags": [
                ["e", "game0000"],
                ["turn", turn.to_string()],
                ["phase", phase.to_string()],
                ["seq", seq.to_string()]
            ],
            "sig": "abcdef1234567890",
            "created_at": 1700000000u64
        }))
        .unwrap()
    }

    fn make_state_hash_event(id: &str, pubkey: &str, turn: u64, hash: &str) -> String {
        let content = serde_json::to_string(&serde_json::json!({
            "turn": turn,
            "hash": hash
        }))
        .unwrap();

        serde_json::to_string(&serde_json::json!({
            "id": id,
            "pubkey": pubkey,
            "kind": kinds::GAME_STATE_HASH.as_u16(),
            "content": content,
            "tags": [
                ["e", "game0000"],
                ["turn", turn.to_string()]
            ],
            "sig": "sig123",
            "created_at": 1700000000u64
        }))
        .unwrap()
    }

    fn make_start_event(players: &[&str]) -> String {
        let content = serde_json::to_string(&serde_json::json!({
            "map_seed": 42,
            "game_seed": 99,
            "player_order": players,
            "ruleset": "classic"
        }))
        .unwrap();

        let mut tags: Vec<serde_json::Value> = vec![serde_json::json!(["e", "lobby0000"])];
        for p in players {
            tags.push(serde_json::json!(["p", p]));
        }

        serde_json::to_string(&serde_json::json!({
            "id": "start0000",
            "pubkey": players[0],
            "kind": kinds::GAME_START.as_u16(),
            "content": content,
            "tags": tags,
            "sig": "startsig",
            "created_at": 1700000000u64
        }))
        .unwrap()
    }

    fn make_end_event() -> String {
        let content = serde_json::to_string(&serde_json::json!({
            "turn": 10,
            "state_hash": "finalhash",
            "summary": "Player A wins"
        }))
        .unwrap();

        serde_json::to_string(&serde_json::json!({
            "id": "end0000",
            "pubkey": "playerA",
            "kind": kinds::GAME_END.as_u16(),
            "content": content,
            "tags": [["e", "game0000"], ["turn", "10"]],
            "sig": "endsig",
            "created_at": 1700000000u64
        }))
        .unwrap()
    }

    fn make_invalid_sig_action_event(turn: u64, seq: u64) -> String {
        let content = serde_json::to_string(&serde_json::json!({
            "packet_type": 84,
            "turn": turn,
            "phase": 0,
            "sequence": seq,
            "prev_event_id": "",
            "payload": {}
        }))
        .unwrap();

        serde_json::to_string(&serde_json::json!({
            "id": format!("nosig{seq}"),
            "pubkey": "playerA",
            "kind": kinds::GAME_ACTION.as_u16(),
            "content": content,
            "tags": [
                ["e", "game0000"],
                ["turn", turn.to_string()],
                ["phase", "0"],
                ["seq", seq.to_string()]
            ],
            "sig": "",
            "created_at": 1700000000u64
        }))
        .unwrap()
    }

    // =====================================================================
    // GameRecording::from_events tests
    // =====================================================================

    #[test]
    fn from_events_with_actions() {
        let events = vec![
            make_start_event(&["playerA", "playerB"]),
            make_action_event("e1", "playerA", 1, 0, 0, serde_json::json!({"unit_id": 1})),
            make_action_event("e2", "playerB", 1, 0, 1, serde_json::json!({"unit_id": 2})),
            make_action_event("e3", "playerA", 2, 0, 2, serde_json::json!({"unit_id": 3})),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert_eq!(recording.game_id, "game0000");
        assert_eq!(recording.players.len(), 2);
        assert_eq!(recording.actions.len(), 3);
        assert_eq!(recording.total_turns, 2);
        assert!(recording.end_summary.is_none());
    }

    #[test]
    fn from_events_with_state_hashes() {
        let events = vec![
            make_state_hash_event("h1", "playerA", 1, "hash_a_1"),
            make_state_hash_event("h2", "playerB", 1, "hash_b_1"),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert_eq!(recording.state_hashes.len(), 2);
        assert_eq!(recording.total_turns, 1);
    }

    #[test]
    fn from_events_with_end_summary() {
        let events = vec![
            make_action_event("e1", "playerA", 1, 0, 0, serde_json::json!({})),
            make_end_event(),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert!(recording.end_summary.is_some());
        let summary = recording.end_summary.as_ref().unwrap();
        assert_eq!(summary["summary"], "Player A wins");
    }

    #[test]
    fn from_events_empty_returns_none() {
        let events: Vec<String> = vec![];
        assert!(GameRecording::from_events("game0000", &events).is_none());
    }

    #[test]
    fn from_events_invalid_json_skipped() {
        let events = vec![
            "not valid json".to_string(),
            make_action_event("e1", "playerA", 1, 0, 0, serde_json::json!({})),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert_eq!(recording.actions.len(), 1);
    }

    #[test]
    fn from_events_actions_sorted_by_turn_phase_seq() {
        let events = vec![
            make_action_event("e3", "playerA", 2, 0, 0, serde_json::json!({})),
            make_action_event("e1", "playerA", 1, 0, 0, serde_json::json!({})),
            make_action_event("e2", "playerA", 1, 1, 0, serde_json::json!({})),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert_eq!(recording.actions[0].event_id, "e1");
        assert_eq!(recording.actions[1].event_id, "e2");
        assert_eq!(recording.actions[2].event_id, "e3");
    }

    // =====================================================================
    // GameRecording query methods
    // =====================================================================

    #[test]
    fn actions_for_turn_filters_correctly() {
        let events = vec![
            make_action_event("e1", "playerA", 1, 0, 0, serde_json::json!({})),
            make_action_event("e2", "playerB", 1, 0, 1, serde_json::json!({})),
            make_action_event("e3", "playerA", 2, 0, 2, serde_json::json!({})),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert_eq!(recording.actions_for_turn(1).len(), 2);
        assert_eq!(recording.actions_for_turn(2).len(), 1);
        assert_eq!(recording.actions_for_turn(3).len(), 0);
    }

    #[test]
    fn state_hash_for_turn_filters_correctly() {
        let events = vec![
            make_state_hash_event("h1", "playerA", 1, "aaa"),
            make_state_hash_event("h2", "playerB", 1, "aaa"),
            make_state_hash_event("h3", "playerA", 2, "bbb"),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert_eq!(recording.state_hash_for_turn(1).len(), 2);
        assert_eq!(recording.state_hash_for_turn(2).len(), 1);
        assert_eq!(recording.state_hash_for_turn(3).len(), 0);
    }

    // =====================================================================
    // GameRecording::verify tests
    // =====================================================================

    #[test]
    fn verify_all_valid() {
        let events = vec![
            make_action_event("e1", "playerA", 1, 0, 0, serde_json::json!({})),
            make_action_event("e2", "playerB", 1, 0, 1, serde_json::json!({})),
            make_state_hash_event("h1", "playerA", 1, "aaa"),
            make_state_hash_event("h2", "playerB", 1, "aaa"),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        let verification = recording.verify();
        assert!(verification.signatures_valid);
        assert!(verification.state_hashes_consistent);
        assert_eq!(verification.actions_verified, 2);
        assert!(verification.issues.is_empty());
    }

    #[test]
    fn verify_invalid_signature() {
        let events = vec![make_invalid_sig_action_event(1, 0)];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        let verification = recording.verify();
        assert!(!verification.signatures_valid);
        assert_eq!(verification.issues.len(), 1);
        assert!(verification.issues[0].contains("invalid signature"));
    }

    #[test]
    fn verify_inconsistent_state_hashes() {
        let events = vec![
            make_state_hash_event("h1", "playerA", 1, "aaa"),
            make_state_hash_event("h2", "playerB", 1, "bbb"), // different!
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        let verification = recording.verify();
        assert!(!verification.state_hashes_consistent);
        assert!(verification.issues.iter().any(|i| i.contains("mismatch")));
    }

    #[test]
    fn verify_empty_recording() {
        let events = vec![make_start_event(&["playerA"])];
        let recording = GameRecording::from_events("game0000", &events).unwrap();
        let verification = recording.verify();
        assert!(verification.signatures_valid);
        assert!(verification.state_hashes_consistent);
        assert_eq!(verification.actions_verified, 0);
        assert!(verification.issues.is_empty());
    }

    // =====================================================================
    // GameRecording::to_nostr_share_link tests
    // =====================================================================

    #[test]
    fn share_link_format() {
        let events = vec![make_action_event(
            "e1",
            "playerA",
            1,
            0,
            0,
            serde_json::json!({}),
        )];
        let recording = GameRecording::from_events("game0000", &events).unwrap();
        let link = recording.to_nostr_share_link();
        assert!(link.starts_with("nostr:naddr:"));
        assert!(link.contains("30421"));
        assert!(link.contains("game0000"));
    }

    #[test]
    fn share_link_deterministic() {
        let events = vec![make_action_event(
            "e1",
            "playerA",
            1,
            0,
            0,
            serde_json::json!({}),
        )];
        let recording = GameRecording::from_events("game0000", &events).unwrap();
        let link1 = recording.to_nostr_share_link();
        let link2 = recording.to_nostr_share_link();
        assert_eq!(link1, link2);
    }

    // =====================================================================
    // ReplayController tests
    // =====================================================================

    fn make_test_recording(n_actions: usize) -> GameRecording {
        let mut actions = Vec::new();
        for i in 0..n_actions {
            actions.push(RecordedAction {
                turn: (i / 3) as u64 + 1, // 3 actions per turn
                phase: 0,
                sequence: i as u64,
                player_pubkey: "playerA".to_string(),
                action: serde_json::json!({"seq": i}),
                event_id: format!("evt{i}"),
                signature_valid: true,
            });
        }
        GameRecording {
            game_id: "test_game".to_string(),
            start_params: serde_json::json!({}),
            players: vec!["playerA".to_string()],
            actions,
            state_hashes: vec![],
            end_summary: None,
            total_turns: ((n_actions.max(1) - 1) / 3 + 1) as u64,
        }
    }

    #[test]
    fn controller_new_starts_stopped() {
        let recording = make_test_recording(5);
        let ctrl = ReplayController::new(recording);
        assert_eq!(ctrl.state(), ReplayState::Stopped);
        assert_eq!(ctrl.current_turn(), 0);
        assert_eq!(ctrl.current_index(), 0);
    }

    #[test]
    fn controller_play_and_pause() {
        let recording = make_test_recording(5);
        let mut ctrl = ReplayController::new(recording);

        ctrl.play();
        assert_eq!(ctrl.state(), ReplayState::Playing);

        ctrl.pause();
        assert_eq!(ctrl.state(), ReplayState::Paused);
    }

    #[test]
    fn controller_stop_resets() {
        let recording = make_test_recording(5);
        let mut ctrl = ReplayController::new(recording);

        ctrl.step_forward();
        ctrl.step_forward();
        assert!(ctrl.current_index() > 0);

        ctrl.stop();
        assert_eq!(ctrl.state(), ReplayState::Stopped);
        assert_eq!(ctrl.current_index(), 0);
        assert_eq!(ctrl.current_turn(), 0);
    }

    #[test]
    fn controller_step_forward() {
        let recording = make_test_recording(3);
        let mut ctrl = ReplayController::new(recording);

        let a1 = ctrl.step_forward().unwrap();
        assert_eq!(a1.sequence, 0);
        assert_eq!(ctrl.current_turn(), 1);

        let a2 = ctrl.step_forward().unwrap();
        assert_eq!(a2.sequence, 1);

        let a3 = ctrl.step_forward().unwrap();
        assert_eq!(a3.sequence, 2);
        assert_eq!(ctrl.state(), ReplayState::Finished);

        // Stepping beyond end returns None
        assert!(ctrl.step_forward().is_none());
        assert_eq!(ctrl.state(), ReplayState::Finished);
    }

    #[test]
    fn controller_step_backward() {
        let recording = make_test_recording(3);
        let mut ctrl = ReplayController::new(recording);

        // Step backward at beginning returns None
        assert!(ctrl.step_backward().is_none());

        ctrl.step_forward();
        ctrl.step_forward();
        assert_eq!(ctrl.current_index(), 2);

        let a = ctrl.step_backward().unwrap();
        assert_eq!(a.sequence, 1);
        assert_eq!(ctrl.current_index(), 1);

        let a = ctrl.step_backward().unwrap();
        assert_eq!(a.sequence, 0);
        assert_eq!(ctrl.current_index(), 0);
    }

    #[test]
    fn controller_step_backward_from_finished() {
        let recording = make_test_recording(2);
        let mut ctrl = ReplayController::new(recording);

        ctrl.step_forward();
        ctrl.step_forward();
        assert_eq!(ctrl.state(), ReplayState::Finished);

        let a = ctrl.step_backward().unwrap();
        assert_eq!(a.sequence, 1);
        assert_eq!(ctrl.state(), ReplayState::Paused);
    }

    #[test]
    fn controller_jump_to_turn() {
        let recording = make_test_recording(9); // 3 turns, 3 actions each
        let mut ctrl = ReplayController::new(recording);

        // Jump to turn 2 (actions at indices 3, 4, 5)
        assert!(ctrl.jump_to_turn(2));
        assert_eq!(ctrl.current_turn(), 2);
        assert_eq!(ctrl.current_index(), 3);

        // Jump to turn 1
        assert!(ctrl.jump_to_turn(1));
        assert_eq!(ctrl.current_turn(), 1);
        assert_eq!(ctrl.current_index(), 0);

        // Jump to turn 3
        assert!(ctrl.jump_to_turn(3));
        assert_eq!(ctrl.current_turn(), 3);
        assert_eq!(ctrl.current_index(), 6);
    }

    #[test]
    fn controller_jump_beyond_end() {
        let recording = make_test_recording(3); // 1 turn
        let mut ctrl = ReplayController::new(recording);

        assert!(!ctrl.jump_to_turn(999));
        assert_eq!(ctrl.state(), ReplayState::Finished);
    }

    #[test]
    fn controller_progress() {
        let recording = make_test_recording(4);
        let mut ctrl = ReplayController::new(recording);

        assert_eq!(ctrl.progress(), 0.0);

        ctrl.step_forward();
        assert_eq!(ctrl.progress(), 0.25);

        ctrl.step_forward();
        assert_eq!(ctrl.progress(), 0.5);

        ctrl.step_forward();
        assert_eq!(ctrl.progress(), 0.75);

        ctrl.step_forward();
        assert_eq!(ctrl.progress(), 1.0);
    }

    #[test]
    fn controller_progress_empty_recording() {
        let recording = make_test_recording(0);
        let ctrl = ReplayController::new(recording);
        assert_eq!(ctrl.progress(), 0.0);
    }

    #[test]
    fn controller_speed() {
        let recording = make_test_recording(1);
        let mut ctrl = ReplayController::new(recording);
        assert_eq!(ctrl.speed(), 1.0);

        ctrl.set_speed(2.5);
        assert_eq!(ctrl.speed(), 2.5);
    }

    #[test]
    fn controller_play_at_finished_is_noop() {
        let recording = make_test_recording(1);
        let mut ctrl = ReplayController::new(recording);

        ctrl.step_forward(); // finishes
        assert_eq!(ctrl.state(), ReplayState::Finished);

        ctrl.play(); // should not change from Finished
        assert_eq!(ctrl.state(), ReplayState::Finished);
    }

    // =====================================================================
    // GameObserver tests
    // =====================================================================

    #[test]
    fn observer_new() {
        let obs = GameObserver::new("game_abc");
        assert_eq!(obs.game_id(), "game_abc");
        assert_eq!(obs.action_count(), 0);
        assert_eq!(obs.current_turn(), 0);
        assert!(obs.is_active());
    }

    #[test]
    fn observer_receive_action_event() {
        let mut obs = GameObserver::new("game0000");
        let event = make_action_event("e1", "playerA", 3, 0, 0, serde_json::json!({}));
        assert!(obs.receive_event(&event));
        assert_eq!(obs.action_count(), 1);
        assert_eq!(obs.current_turn(), 3);
    }

    #[test]
    fn observer_receive_multiple_events() {
        let mut obs = GameObserver::new("game0000");

        obs.receive_event(&make_action_event(
            "e1",
            "playerA",
            1,
            0,
            0,
            serde_json::json!({}),
        ));
        obs.receive_event(&make_action_event(
            "e2",
            "playerB",
            1,
            0,
            1,
            serde_json::json!({}),
        ));
        obs.receive_event(&make_action_event(
            "e3",
            "playerA",
            2,
            0,
            2,
            serde_json::json!({}),
        ));

        assert_eq!(obs.action_count(), 3);
        assert_eq!(obs.current_turn(), 2);
    }

    #[test]
    fn observer_ignores_non_action_events() {
        let mut obs = GameObserver::new("game0000");
        let hash_event = make_state_hash_event("h1", "playerA", 1, "aaa");
        assert!(!obs.receive_event(&hash_event));
        assert_eq!(obs.action_count(), 0);
    }

    #[test]
    fn observer_stops_on_game_end() {
        let mut obs = GameObserver::new("game0000");

        obs.receive_event(&make_action_event(
            "e1",
            "playerA",
            1,
            0,
            0,
            serde_json::json!({}),
        ));
        assert!(obs.is_active());

        obs.receive_event(&make_end_event());
        assert!(!obs.is_active());

        // Further events are ignored
        assert!(!obs.receive_event(&make_action_event(
            "e2",
            "playerA",
            2,
            0,
            1,
            serde_json::json!({})
        )));
        assert_eq!(obs.action_count(), 1);
    }

    #[test]
    fn observer_manual_stop() {
        let mut obs = GameObserver::new("game0000");
        obs.stop();
        assert!(!obs.is_active());
        assert!(!obs.receive_event(&make_action_event(
            "e1",
            "playerA",
            1,
            0,
            0,
            serde_json::json!({})
        )));
    }

    #[test]
    fn observer_invalid_json_returns_false() {
        let mut obs = GameObserver::new("game0000");
        assert!(!obs.receive_event("not json at all"));
        assert_eq!(obs.action_count(), 0);
    }

    // =====================================================================
    // Edge cases
    // =====================================================================

    #[test]
    fn single_action_recording() {
        let events = vec![make_action_event(
            "e1",
            "playerA",
            1,
            0,
            0,
            serde_json::json!({}),
        )];
        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert_eq!(recording.actions.len(), 1);
        assert_eq!(recording.total_turns, 1);

        let mut ctrl = ReplayController::new(recording);
        assert!(ctrl.step_forward().is_some());
        assert_eq!(ctrl.state(), ReplayState::Finished);
        assert!(ctrl.step_forward().is_none());
    }

    #[test]
    fn recording_only_start_event() {
        let events = vec![make_start_event(&["playerA", "playerB"])];
        let recording = GameRecording::from_events("game0000", &events).unwrap();
        assert_eq!(recording.players.len(), 2);
        assert_eq!(recording.actions.len(), 0);
        assert_eq!(recording.total_turns, 0);
    }

    #[test]
    fn controller_with_empty_actions() {
        let recording = make_test_recording(0);
        let mut ctrl = ReplayController::new(recording);
        assert!(ctrl.step_forward().is_none());
        assert!(ctrl.step_backward().is_none());
        assert!(!ctrl.jump_to_turn(1));
    }

    #[test]
    fn observer_current_turn_tracks_maximum() {
        let mut obs = GameObserver::new("game0000");
        obs.receive_event(&make_action_event(
            "e1",
            "playerA",
            5,
            0,
            0,
            serde_json::json!({}),
        ));
        obs.receive_event(&make_action_event(
            "e2",
            "playerA",
            3,
            0,
            1,
            serde_json::json!({}),
        ));
        // Turn should stay at 5 (maximum seen)
        assert_eq!(obs.current_turn(), 5);
    }

    #[test]
    fn verify_multiple_issues() {
        let events = vec![
            make_invalid_sig_action_event(1, 0),
            make_invalid_sig_action_event(1, 1),
            make_state_hash_event("h1", "playerA", 1, "aaa"),
            make_state_hash_event("h2", "playerB", 1, "bbb"),
        ];

        let recording = GameRecording::from_events("game0000", &events).unwrap();
        let verification = recording.verify();
        assert!(!verification.signatures_valid);
        assert!(!verification.state_hashes_consistent);
        // 2 invalid sigs + 1 hash mismatch = at least 3 issues
        assert!(verification.issues.len() >= 3);
    }

    #[test]
    fn from_events_only_unknown_kinds_returns_none() {
        let event = serde_json::to_string(&serde_json::json!({
            "id": "unknown1",
            "pubkey": "pk",
            "kind": 9999,
            "content": "{}",
            "tags": [],
            "sig": "sig",
            "created_at": 1700000000u64
        }))
        .unwrap();

        assert!(GameRecording::from_events("game0000", &[event]).is_none());
    }
}

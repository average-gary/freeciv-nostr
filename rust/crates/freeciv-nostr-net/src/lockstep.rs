//! Lockstep protocol for turn-based P2P game synchronization.
//!
//! Implements a commit-reveal scheme for concurrent turns, ensuring all
//! players submit their actions before any are revealed. For alternating
//! modes the commit/reveal phases are skipped — actions are applied
//! directly.
//!
//! The protocol state machine progresses through phases:
//! `Commit` → `Reveal` → `Apply` → `Verify` → `Complete` (concurrent)
//! or `Apply` → `Verify` → `Complete` (alternating).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Configuration enums
// ---------------------------------------------------------------------------

/// How actions are submitted within a turn.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseMode {
    /// All players act simultaneously (commit-reveal).
    Concurrent = 0,
    /// Players take turns one at a time.
    PlayersAlternate = 1,
    /// Teams take turns.
    TeamsAlternate = 2,
}

/// Current phase within a single turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnPhase {
    /// Waiting for all players to submit action commitments.
    Commit,
    /// Waiting for all players to reveal their actions.
    Reveal,
    /// Actions are being applied to the game state.
    Apply,
    /// Waiting for all players to submit state hashes.
    Verify,
    /// Turn is complete and verified.
    Complete,
}

// ---------------------------------------------------------------------------
// Protocol messages
// ---------------------------------------------------------------------------

/// A hashed commitment to a set of actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionCommitment {
    /// SHA-256 hex digest of the actions JSON.
    pub hash: String,
    /// Turn number this commitment applies to.
    pub turn: u32,
    /// Hex-encoded Nostr pubkey of the committing player.
    pub player_pubkey: String,
}

/// The revealed actions corresponding to a prior commitment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionReveal {
    /// JSON-serialised list of actions.
    pub actions_json: String,
    /// Turn number this reveal applies to.
    pub turn: u32,
    /// Hex-encoded Nostr pubkey of the revealing player.
    pub player_pubkey: String,
}

/// A player's hash of the post-apply game state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateHashSubmission {
    /// SHA-256 hex digest of the game state after applying actions.
    pub state_hash: String,
    /// Turn number this hash applies to.
    pub turn: u32,
    /// Hex-encoded Nostr pubkey of the submitting player.
    pub player_pubkey: String,
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for a lockstep session.
pub struct LockstepConfig {
    /// Action-submission mode.
    pub phase_mode: PhaseMode,
    /// Optional per-phase timeout. `None` disables timeouts.
    pub turn_timeout: Option<Duration>,
    /// Hex-encoded Nostr pubkeys of all players in the game.
    pub player_pubkeys: Vec<String>,
}

// ---------------------------------------------------------------------------
// Result & error types
// ---------------------------------------------------------------------------

/// Outcome of submitting data during a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnAdvanceResult {
    /// Still waiting for other players.
    Waiting {
        /// Number of players that have not yet submitted.
        remaining: usize,
    },
    /// All players have submitted; phase may advance.
    Ready,
    /// A reveal did not match its commitment.
    RevealMismatch {
        /// Pubkey of the player whose reveal mismatched.
        player_pubkey: String,
    },
    /// Players disagree on the post-apply state hash.
    DesyncDetected {
        /// Map of player pubkey to their submitted state hash.
        hashes: HashMap<String, String>,
    },
    /// The phase has timed out; some players did not submit in time.
    Timeout {
        /// Pubkeys of players that did not submit.
        missing_players: Vec<String>,
    },
}

/// Errors returned by lockstep operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LockstepError {
    /// The protocol is not in the expected phase.
    #[error("wrong phase: expected {expected:?}, got {actual:?}")]
    WrongPhase {
        /// The phase the caller expected.
        expected: TurnPhase,
        /// The actual current phase.
        actual: TurnPhase,
    },
    /// The submitted turn number does not match the current turn.
    #[error("wrong turn: expected {expected}, got {actual}")]
    WrongTurn {
        /// Expected turn number.
        expected: u32,
        /// Submitted turn number.
        actual: u32,
    },
    /// The submitting player is not a member of this game.
    #[error("unknown player: {0}")]
    UnknownPlayer(String),
    /// A reveal was submitted but no commitment exists for that player.
    #[error("no commitment found for player: {0}")]
    NoCommitment(String),
}

// ---------------------------------------------------------------------------
// LockstepProtocol
// ---------------------------------------------------------------------------

/// Main state machine for the lockstep protocol.
///
/// Tracks per-turn commitments, reveals, and state hashes, and drives
/// phase transitions.
pub struct LockstepProtocol {
    config: LockstepConfig,
    turn: u32,
    phase: TurnPhase,
    commitments: HashMap<String, ActionCommitment>,
    reveals: HashMap<String, ActionReveal>,
    state_hashes: HashMap<String, StateHashSubmission>,
    phase_start: Option<Instant>,
}

impl LockstepProtocol {
    /// Create a new lockstep protocol instance.
    pub fn new(config: LockstepConfig) -> Self {
        Self {
            turn: 0,
            phase: TurnPhase::Complete,
            commitments: HashMap::new(),
            reveals: HashMap::new(),
            state_hashes: HashMap::new(),
            phase_start: None,
            config,
        }
    }

    /// The current turn number.
    pub fn current_turn(&self) -> u32 {
        self.turn
    }

    /// The current phase within the turn.
    pub fn current_phase(&self) -> TurnPhase {
        self.phase
    }

    /// Begin a new turn, resetting all per-turn state.
    ///
    /// In concurrent mode the first phase is `Commit`; in alternating
    /// modes it is `Apply` (commit/reveal are skipped).
    pub fn begin_turn(&mut self, turn: u32) {
        self.turn = turn;
        self.commitments.clear();
        self.reveals.clear();
        self.state_hashes.clear();
        self.phase_start = Some(Instant::now());

        self.phase = match self.config.phase_mode {
            PhaseMode::Concurrent => TurnPhase::Commit,
            PhaseMode::PlayersAlternate | PhaseMode::TeamsAlternate => TurnPhase::Apply,
        };
    }

    /// Compute a SHA-256 commitment hash for the given actions JSON.
    pub fn compute_commitment(actions_json: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(actions_json.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Submit a commitment for a player.
    ///
    /// Returns [`TurnAdvanceResult::Ready`] when all players have
    /// committed, advancing the phase to `Reveal`.
    pub fn submit_commitment(
        &mut self,
        commitment: ActionCommitment,
    ) -> Result<TurnAdvanceResult, LockstepError> {
        if self.phase != TurnPhase::Commit {
            return Err(LockstepError::WrongPhase {
                expected: TurnPhase::Commit,
                actual: self.phase,
            });
        }
        if commitment.turn != self.turn {
            return Err(LockstepError::WrongTurn {
                expected: self.turn,
                actual: commitment.turn,
            });
        }
        if !self.is_known_player(&commitment.player_pubkey) {
            return Err(LockstepError::UnknownPlayer(
                commitment.player_pubkey.clone(),
            ));
        }

        self.commitments
            .insert(commitment.player_pubkey.clone(), commitment);

        let remaining = self.config.player_pubkeys.len() - self.commitments.len();
        if remaining == 0 {
            self.phase = TurnPhase::Reveal;
            self.phase_start = Some(Instant::now());
            Ok(TurnAdvanceResult::Ready)
        } else {
            Ok(TurnAdvanceResult::Waiting { remaining })
        }
    }

    /// Submit a reveal for a player.
    ///
    /// Validates that the reveal hashes to the player's prior commitment.
    /// Returns [`TurnAdvanceResult::Ready`] when all players have revealed,
    /// advancing the phase to `Apply`.
    pub fn submit_reveal(
        &mut self,
        reveal: ActionReveal,
    ) -> Result<TurnAdvanceResult, LockstepError> {
        if self.phase != TurnPhase::Reveal {
            return Err(LockstepError::WrongPhase {
                expected: TurnPhase::Reveal,
                actual: self.phase,
            });
        }
        if reveal.turn != self.turn {
            return Err(LockstepError::WrongTurn {
                expected: self.turn,
                actual: reveal.turn,
            });
        }
        if !self.is_known_player(&reveal.player_pubkey) {
            return Err(LockstepError::UnknownPlayer(reveal.player_pubkey.clone()));
        }

        // Verify against commitment.
        let commitment = self
            .commitments
            .get(&reveal.player_pubkey)
            .ok_or_else(|| LockstepError::NoCommitment(reveal.player_pubkey.clone()))?;

        let expected_hash = Self::compute_commitment(&reveal.actions_json);
        if expected_hash != commitment.hash {
            return Ok(TurnAdvanceResult::RevealMismatch {
                player_pubkey: reveal.player_pubkey,
            });
        }

        self.reveals.insert(reveal.player_pubkey.clone(), reveal);

        let remaining = self.config.player_pubkeys.len() - self.reveals.len();
        if remaining == 0 {
            self.phase = TurnPhase::Apply;
            self.phase_start = Some(Instant::now());
            Ok(TurnAdvanceResult::Ready)
        } else {
            Ok(TurnAdvanceResult::Waiting { remaining })
        }
    }

    /// Get all revealed actions in deterministic (sorted-by-pubkey) order.
    ///
    /// Only valid during or after the `Apply` phase.
    pub fn ordered_actions(&self) -> Result<Vec<&ActionReveal>, LockstepError> {
        if self.phase != TurnPhase::Apply
            && self.phase != TurnPhase::Verify
            && self.phase != TurnPhase::Complete
        {
            return Err(LockstepError::WrongPhase {
                expected: TurnPhase::Apply,
                actual: self.phase,
            });
        }
        let mut keys: Vec<&String> = self.reveals.keys().collect();
        keys.sort();
        Ok(keys
            .into_iter()
            .map(|k| self.reveals.get(k).expect("key exists"))
            .collect())
    }

    /// Signal that all actions have been applied to the game state.
    ///
    /// Transitions from `Apply` to `Verify`.
    pub fn actions_applied(&mut self) {
        if self.phase == TurnPhase::Apply {
            self.phase = TurnPhase::Verify;
            self.phase_start = Some(Instant::now());
        }
    }

    /// Submit a state hash for consensus verification.
    ///
    /// When all players have submitted, checks that every hash matches.
    /// Returns [`TurnAdvanceResult::Ready`] on consensus (advancing to
    /// `Complete`) or [`TurnAdvanceResult::DesyncDetected`] on mismatch.
    pub fn submit_state_hash(
        &mut self,
        submission: StateHashSubmission,
    ) -> Result<TurnAdvanceResult, LockstepError> {
        if self.phase != TurnPhase::Verify {
            return Err(LockstepError::WrongPhase {
                expected: TurnPhase::Verify,
                actual: self.phase,
            });
        }
        if submission.turn != self.turn {
            return Err(LockstepError::WrongTurn {
                expected: self.turn,
                actual: submission.turn,
            });
        }
        if !self.is_known_player(&submission.player_pubkey) {
            return Err(LockstepError::UnknownPlayer(
                submission.player_pubkey.clone(),
            ));
        }

        self.state_hashes
            .insert(submission.player_pubkey.clone(), submission);

        let remaining = self.config.player_pubkeys.len() - self.state_hashes.len();
        if remaining == 0 {
            // Check consensus.
            let mut hashes_iter = self.state_hashes.values().map(|s| &s.state_hash);
            let first = hashes_iter.next().expect("at least one player");
            if hashes_iter.all(|h| h == first) {
                self.phase = TurnPhase::Complete;
                Ok(TurnAdvanceResult::Ready)
            } else {
                let hashes: HashMap<String, String> = self
                    .state_hashes
                    .iter()
                    .map(|(k, v)| (k.clone(), v.state_hash.clone()))
                    .collect();
                Ok(TurnAdvanceResult::DesyncDetected { hashes })
            }
        } else {
            Ok(TurnAdvanceResult::Waiting { remaining })
        }
    }

    /// Check whether the current phase has timed out.
    ///
    /// Returns `Some(TurnAdvanceResult::Timeout { .. })` with the list of
    /// players who have not yet submitted, or `None` if no timeout is
    /// configured or the deadline has not been reached.
    pub fn check_timeout(&self) -> Option<TurnAdvanceResult> {
        let timeout = self.config.turn_timeout?;
        let start = self.phase_start?;
        if start.elapsed() < timeout {
            return None;
        }

        let submitted: std::collections::HashSet<&String> = match self.phase {
            TurnPhase::Commit => self.commitments.keys().collect(),
            TurnPhase::Reveal => self.reveals.keys().collect(),
            TurnPhase::Verify => self.state_hashes.keys().collect(),
            _ => return None,
        };

        let missing: Vec<String> = self
            .config
            .player_pubkeys
            .iter()
            .filter(|pk| !submitted.contains(pk))
            .cloned()
            .collect();

        Some(TurnAdvanceResult::Timeout {
            missing_players: missing,
        })
    }

    /// The consensus state hash, if the turn is `Complete`.
    pub fn consensus_state_hash(&self) -> Option<&str> {
        if self.phase != TurnPhase::Complete {
            return None;
        }
        self.state_hashes
            .values()
            .next()
            .map(|s| s.state_hash.as_str())
    }

    // -- helpers -----------------------------------------------------------

    fn is_known_player(&self, pubkey: &str) -> bool {
        self.config.player_pubkeys.iter().any(|pk| pk == pubkey)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn two_player_config() -> LockstepConfig {
        LockstepConfig {
            phase_mode: PhaseMode::Concurrent,
            turn_timeout: None,
            player_pubkeys: vec!["alice".into(), "bob".into()],
        }
    }

    fn three_player_config() -> LockstepConfig {
        LockstepConfig {
            phase_mode: PhaseMode::Concurrent,
            turn_timeout: None,
            player_pubkeys: vec!["alice".into(), "bob".into(), "carol".into()],
        }
    }

    // -- full concurrent lifecycle -----------------------------------------

    #[test]
    fn full_concurrent_lifecycle() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        assert_eq!(proto.current_turn(), 1);
        assert_eq!(proto.current_phase(), TurnPhase::Commit);

        let hash_a = LockstepProtocol::compute_commitment(r#"["move_north"]"#);
        let hash_b = LockstepProtocol::compute_commitment(r#"["build_farm"]"#);

        // Commit phase
        let r = proto
            .submit_commitment(ActionCommitment {
                hash: hash_a.clone(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Waiting { remaining: 1 });

        let r = proto
            .submit_commitment(ActionCommitment {
                hash: hash_b.clone(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Ready);
        assert_eq!(proto.current_phase(), TurnPhase::Reveal);

        // Reveal phase
        let r = proto
            .submit_reveal(ActionReveal {
                actions_json: r#"["move_north"]"#.into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Waiting { remaining: 1 });

        let r = proto
            .submit_reveal(ActionReveal {
                actions_json: r#"["build_farm"]"#.into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Ready);
        assert_eq!(proto.current_phase(), TurnPhase::Apply);

        // Apply + Verify
        let actions = proto.ordered_actions().unwrap();
        assert_eq!(actions.len(), 2);

        proto.actions_applied();
        assert_eq!(proto.current_phase(), TurnPhase::Verify);

        let r = proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "abc123".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Waiting { remaining: 1 });

        let r = proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "abc123".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Ready);
        assert_eq!(proto.current_phase(), TurnPhase::Complete);
        assert_eq!(proto.consensus_state_hash(), Some("abc123"));
    }

    // -- commit-reveal mismatch -------------------------------------------

    #[test]
    fn reveal_mismatch_detected() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);

        let hash_a = LockstepProtocol::compute_commitment(r#"["move_north"]"#);
        let hash_b = LockstepProtocol::compute_commitment(r#"["build_farm"]"#);

        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();

        // Alice reveals different actions than committed.
        let r = proto
            .submit_reveal(ActionReveal {
                actions_json: r#"["CHEAT"]"#.into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        assert_eq!(
            r,
            TurnAdvanceResult::RevealMismatch {
                player_pubkey: "alice".into()
            }
        );
    }

    // -- state hash consensus success -------------------------------------

    #[test]
    fn state_hash_consensus_success() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);

        // Fast-forward through commit/reveal.
        let hash_a = LockstepProtocol::compute_commitment("a");
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        proto
            .submit_reveal(ActionReveal {
                actions_json: "a".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_reveal(ActionReveal {
                actions_json: "b".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        proto.actions_applied();

        let r = proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "same_hash".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Waiting { remaining: 1 });

        let r = proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "same_hash".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Ready);
        assert_eq!(proto.consensus_state_hash(), Some("same_hash"));
    }

    // -- desync detection -------------------------------------------------

    #[test]
    fn desync_detected_different_hashes() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);

        let hash_a = LockstepProtocol::compute_commitment("a");
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        proto
            .submit_reveal(ActionReveal {
                actions_json: "a".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_reveal(ActionReveal {
                actions_json: "b".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        proto.actions_applied();

        proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "hash_alice".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        let r = proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "hash_bob".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();

        match r {
            TurnAdvanceResult::DesyncDetected { ref hashes } => {
                assert_eq!(hashes.get("alice").unwrap(), "hash_alice");
                assert_eq!(hashes.get("bob").unwrap(), "hash_bob");
            }
            other => panic!("expected DesyncDetected, got {other:?}"),
        }
    }

    // -- timeout: commit phase -------------------------------------------

    #[test]
    fn timeout_commit_phase() {
        let config = LockstepConfig {
            phase_mode: PhaseMode::Concurrent,
            turn_timeout: Some(Duration::from_millis(0)),
            player_pubkeys: vec!["alice".into(), "bob".into()],
        };
        let mut proto = LockstepProtocol::new(config);
        proto.begin_turn(1);

        // Only alice commits.
        let hash_a = LockstepProtocol::compute_commitment("a");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();

        // Force timeout by sleeping a tiny bit (0ms timeout already expired).
        std::thread::sleep(Duration::from_millis(1));

        let result = proto.check_timeout();
        match result {
            Some(TurnAdvanceResult::Timeout { missing_players }) => {
                assert_eq!(missing_players, vec!["bob".to_string()]);
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    // -- timeout: reveal phase -------------------------------------------

    #[test]
    fn timeout_reveal_phase() {
        let config = LockstepConfig {
            phase_mode: PhaseMode::Concurrent,
            turn_timeout: Some(Duration::from_millis(0)),
            player_pubkeys: vec!["alice".into(), "bob".into()],
        };
        let mut proto = LockstepProtocol::new(config);
        proto.begin_turn(1);

        let hash_a = LockstepProtocol::compute_commitment("a");
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();

        // Only alice reveals.
        proto
            .submit_reveal(ActionReveal {
                actions_json: "a".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();

        std::thread::sleep(Duration::from_millis(1));

        let result = proto.check_timeout();
        match result {
            Some(TurnAdvanceResult::Timeout { missing_players }) => {
                assert_eq!(missing_players, vec!["bob".to_string()]);
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    // -- timeout: verify phase -------------------------------------------

    #[test]
    fn timeout_verify_phase() {
        let config = LockstepConfig {
            phase_mode: PhaseMode::Concurrent,
            turn_timeout: Some(Duration::from_millis(0)),
            player_pubkeys: vec!["alice".into(), "bob".into()],
        };
        let mut proto = LockstepProtocol::new(config);
        proto.begin_turn(1);

        let hash_a = LockstepProtocol::compute_commitment("a");
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        proto
            .submit_reveal(ActionReveal {
                actions_json: "a".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_reveal(ActionReveal {
                actions_json: "b".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        proto.actions_applied();

        // Only alice submits state hash.
        proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "hash".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();

        std::thread::sleep(Duration::from_millis(1));

        let result = proto.check_timeout();
        match result {
            Some(TurnAdvanceResult::Timeout { missing_players }) => {
                assert_eq!(missing_players, vec!["bob".to_string()]);
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    // -- timeout: not expired ---------------------------------------------

    #[test]
    fn timeout_not_expired() {
        let config = LockstepConfig {
            phase_mode: PhaseMode::Concurrent,
            turn_timeout: Some(Duration::from_secs(600)),
            player_pubkeys: vec!["alice".into(), "bob".into()],
        };
        let mut proto = LockstepProtocol::new(config);
        proto.begin_turn(1);
        assert!(proto.check_timeout().is_none());
    }

    // -- timeout: disabled ------------------------------------------------

    #[test]
    fn timeout_disabled() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        assert!(proto.check_timeout().is_none());
    }

    // -- deterministic action ordering ------------------------------------

    #[test]
    fn deterministic_action_ordering() {
        let mut proto = LockstepProtocol::new(three_player_config());
        proto.begin_turn(1);

        // Commit in non-alphabetical order.
        for pk in &["carol", "alice", "bob"] {
            let json = format!(r#"["action_{pk}"]"#);
            let hash = LockstepProtocol::compute_commitment(&json);
            proto
                .submit_commitment(ActionCommitment {
                    hash,
                    turn: 1,
                    player_pubkey: pk.to_string(),
                })
                .unwrap();
        }

        for pk in &["bob", "carol", "alice"] {
            let json = format!(r#"["action_{pk}"]"#);
            proto
                .submit_reveal(ActionReveal {
                    actions_json: json,
                    turn: 1,
                    player_pubkey: pk.to_string(),
                })
                .unwrap();
        }

        let actions = proto.ordered_actions().unwrap();
        let order: Vec<&str> = actions.iter().map(|a| a.player_pubkey.as_str()).collect();
        assert_eq!(order, vec!["alice", "bob", "carol"]);
    }

    // -- alternating mode (skip commit/reveal) ----------------------------

    #[test]
    fn alternating_mode_skips_commit_reveal() {
        let config = LockstepConfig {
            phase_mode: PhaseMode::PlayersAlternate,
            turn_timeout: None,
            player_pubkeys: vec!["alice".into(), "bob".into()],
        };
        let mut proto = LockstepProtocol::new(config);
        proto.begin_turn(1);
        assert_eq!(proto.current_phase(), TurnPhase::Apply);

        // Directly apply and verify.
        proto.actions_applied();
        assert_eq!(proto.current_phase(), TurnPhase::Verify);

        proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "hash1".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        let r = proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "hash1".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        assert_eq!(r, TurnAdvanceResult::Ready);
        assert_eq!(proto.current_phase(), TurnPhase::Complete);
    }

    // -- teams alternate mode starts at Apply -----------------------------

    #[test]
    fn teams_alternate_starts_at_apply() {
        let config = LockstepConfig {
            phase_mode: PhaseMode::TeamsAlternate,
            turn_timeout: None,
            player_pubkeys: vec!["alice".into()],
        };
        let mut proto = LockstepProtocol::new(config);
        proto.begin_turn(1);
        assert_eq!(proto.current_phase(), TurnPhase::Apply);
    }

    // -- wrong phase errors -----------------------------------------------

    #[test]
    fn commit_in_wrong_phase() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);

        // Fast-forward to Reveal.
        let hash_a = LockstepProtocol::compute_commitment("a");
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();

        // Try to commit again during Reveal phase.
        let err = proto
            .submit_commitment(ActionCommitment {
                hash: "x".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap_err();
        assert!(matches!(
            err,
            LockstepError::WrongPhase {
                expected: TurnPhase::Commit,
                actual: TurnPhase::Reveal,
            }
        ));
    }

    #[test]
    fn reveal_in_wrong_phase() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        // Phase is Commit, not Reveal.
        let err = proto
            .submit_reveal(ActionReveal {
                actions_json: "x".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap_err();
        assert!(matches!(
            err,
            LockstepError::WrongPhase {
                expected: TurnPhase::Reveal,
                actual: TurnPhase::Commit,
            }
        ));
    }

    #[test]
    fn state_hash_in_wrong_phase() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        let err = proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "x".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap_err();
        assert!(matches!(
            err,
            LockstepError::WrongPhase {
                expected: TurnPhase::Verify,
                actual: TurnPhase::Commit,
            }
        ));
    }

    // -- wrong turn errors ------------------------------------------------

    #[test]
    fn commit_wrong_turn() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        let err = proto
            .submit_commitment(ActionCommitment {
                hash: "x".into(),
                turn: 99,
                player_pubkey: "alice".into(),
            })
            .unwrap_err();
        assert!(matches!(
            err,
            LockstepError::WrongTurn {
                expected: 1,
                actual: 99,
            }
        ));
    }

    #[test]
    fn reveal_wrong_turn() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        // Advance to Reveal.
        let hash_a = LockstepProtocol::compute_commitment("a");
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();

        let err = proto
            .submit_reveal(ActionReveal {
                actions_json: "a".into(),
                turn: 42,
                player_pubkey: "alice".into(),
            })
            .unwrap_err();
        assert!(matches!(
            err,
            LockstepError::WrongTurn {
                expected: 1,
                actual: 42,
            }
        ));
    }

    // -- unknown player errors --------------------------------------------

    #[test]
    fn commit_unknown_player() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        let err = proto
            .submit_commitment(ActionCommitment {
                hash: "x".into(),
                turn: 1,
                player_pubkey: "mallory".into(),
            })
            .unwrap_err();
        assert!(matches!(err, LockstepError::UnknownPlayer(ref pk) if pk == "mallory"));
    }

    #[test]
    fn reveal_unknown_player() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        let hash_a = LockstepProtocol::compute_commitment("a");
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        let err = proto
            .submit_reveal(ActionReveal {
                actions_json: "x".into(),
                turn: 1,
                player_pubkey: "mallory".into(),
            })
            .unwrap_err();
        assert!(matches!(err, LockstepError::UnknownPlayer(ref pk) if pk == "mallory"));
    }

    #[test]
    fn state_hash_unknown_player() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        let hash_a = LockstepProtocol::compute_commitment("a");
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        proto
            .submit_reveal(ActionReveal {
                actions_json: "a".into(),
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        proto
            .submit_reveal(ActionReveal {
                actions_json: "b".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();
        proto.actions_applied();
        let err = proto
            .submit_state_hash(StateHashSubmission {
                state_hash: "x".into(),
                turn: 1,
                player_pubkey: "mallory".into(),
            })
            .unwrap_err();
        assert!(matches!(err, LockstepError::UnknownPlayer(ref pk) if pk == "mallory"));
    }

    // -- edge: no commitment for reveal -----------------------------------

    #[test]
    fn reveal_without_commitment() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);

        // Only alice commits.
        let hash_a = LockstepProtocol::compute_commitment("a");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        // Manually push bob's commitment so we reach Reveal.
        let hash_b = LockstepProtocol::compute_commitment("b");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_b,
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap();

        // Remove bob's commitment to simulate a missing one.
        proto.commitments.remove("bob");

        let err = proto
            .submit_reveal(ActionReveal {
                actions_json: "b".into(),
                turn: 1,
                player_pubkey: "bob".into(),
            })
            .unwrap_err();
        assert!(matches!(err, LockstepError::NoCommitment(ref pk) if pk == "bob"));
    }

    // -- edge: ordered_actions in wrong phase -----------------------------

    #[test]
    fn ordered_actions_wrong_phase() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        // Phase is Commit.
        let err = proto.ordered_actions().unwrap_err();
        assert!(matches!(err, LockstepError::WrongPhase { .. }));
    }

    // -- edge: actions_applied is no-op in wrong phase --------------------

    #[test]
    fn actions_applied_noop_in_wrong_phase() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        // Phase is Commit.
        proto.actions_applied();
        assert_eq!(proto.current_phase(), TurnPhase::Commit);
    }

    // -- edge: consensus_state_hash returns None before Complete ----------

    #[test]
    fn consensus_hash_none_before_complete() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);
        assert!(proto.consensus_state_hash().is_none());
    }

    // -- edge: begin_turn resets state ------------------------------------

    #[test]
    fn begin_turn_resets_state() {
        let mut proto = LockstepProtocol::new(two_player_config());
        proto.begin_turn(1);

        // Add a commitment.
        let hash_a = LockstepProtocol::compute_commitment("a");
        proto
            .submit_commitment(ActionCommitment {
                hash: hash_a,
                turn: 1,
                player_pubkey: "alice".into(),
            })
            .unwrap();
        assert_eq!(proto.commitments.len(), 1);

        // Begin turn 2 — everything should reset.
        proto.begin_turn(2);
        assert_eq!(proto.current_turn(), 2);
        assert_eq!(proto.current_phase(), TurnPhase::Commit);
        assert!(proto.commitments.is_empty());
        assert!(proto.reveals.is_empty());
        assert!(proto.state_hashes.is_empty());
    }

    // -- compute_commitment is deterministic -------------------------------

    #[test]
    fn compute_commitment_deterministic() {
        let h1 = LockstepProtocol::compute_commitment("test");
        let h2 = LockstepProtocol::compute_commitment("test");
        assert_eq!(h1, h2);
        // Different input -> different hash.
        let h3 = LockstepProtocol::compute_commitment("other");
        assert_ne!(h1, h3);
    }

    // -- timeout: check_timeout in Apply/Complete returns None ------------

    #[test]
    fn check_timeout_apply_phase_returns_none() {
        let config = LockstepConfig {
            phase_mode: PhaseMode::PlayersAlternate,
            turn_timeout: Some(Duration::from_millis(0)),
            player_pubkeys: vec!["alice".into()],
        };
        let mut proto = LockstepProtocol::new(config);
        proto.begin_turn(1);
        assert_eq!(proto.current_phase(), TurnPhase::Apply);
        std::thread::sleep(Duration::from_millis(1));
        // Apply phase does not timeout via check_timeout.
        assert!(proto.check_timeout().is_none());
    }

    // -- PhaseMode serde round-trip ---------------------------------------

    #[test]
    fn phase_mode_serde_roundtrip() {
        let json = serde_json::to_string(&PhaseMode::Concurrent).unwrap();
        let back: PhaseMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, PhaseMode::Concurrent);
    }

    // -- three-player commit-reveal with all matching ---------------------

    #[test]
    fn three_player_lifecycle() {
        let mut proto = LockstepProtocol::new(three_player_config());
        proto.begin_turn(1);

        for pk in &["alice", "bob", "carol"] {
            let json = format!("action_{pk}");
            let hash = LockstepProtocol::compute_commitment(&json);
            proto
                .submit_commitment(ActionCommitment {
                    hash,
                    turn: 1,
                    player_pubkey: pk.to_string(),
                })
                .unwrap();
        }
        assert_eq!(proto.current_phase(), TurnPhase::Reveal);

        for pk in &["alice", "bob", "carol"] {
            let json = format!("action_{pk}");
            proto
                .submit_reveal(ActionReveal {
                    actions_json: json,
                    turn: 1,
                    player_pubkey: pk.to_string(),
                })
                .unwrap();
        }
        assert_eq!(proto.current_phase(), TurnPhase::Apply);

        proto.actions_applied();
        for pk in &["alice", "bob", "carol"] {
            proto
                .submit_state_hash(StateHashSubmission {
                    state_hash: "consensus".into(),
                    turn: 1,
                    player_pubkey: pk.to_string(),
                })
                .unwrap();
        }
        assert_eq!(proto.current_phase(), TurnPhase::Complete);
        assert_eq!(proto.consensus_state_hash(), Some("consensus"));
    }
}

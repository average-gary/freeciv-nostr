//! Desync detection, diagnosis, and recovery for P2P game sessions.
//!
//! Detects state divergence via hash comparison, diagnoses the divergence
//! point via binary search, and recovers using checkpoint replay or
//! majority vote.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Desync detection result after comparing state hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesyncStatus {
    /// All nodes agree on the state hash.
    InSync,
    /// Desync detected: nodes disagree on state hash.
    Desynced {
        /// Turn where desync was detected.
        turn: u32,
        /// Map of player_pubkey -> their state hash.
        hashes: HashMap<String, String>,
        /// The majority hash (if any).
        majority_hash: Option<String>,
        /// Players whose hash differs from majority.
        divergent_players: Vec<String>,
    },
}

/// A state snapshot for diagnosis/recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// Turn number this snapshot was taken at.
    pub turn: u32,
    /// The state hash (SHA-256 hex).
    pub state_hash: String,
    /// Blob hash of the full state data (BLAKE3 hex).
    pub blob_hash: String,
    /// Size of the state data in bytes.
    pub size: u64,
}

/// A checkpoint for faster desync recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryCheckpoint {
    /// Turn number of this checkpoint.
    pub turn: u32,
    /// State hash at this turn.
    pub state_hash: String,
    /// Blob hash of the full state snapshot.
    pub blob_hash: String,
    /// Number of players who agreed on this hash.
    pub agreement_count: usize,
}

/// Recovery strategy determined after desync diagnosis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryStrategy {
    /// Replay from the last matching checkpoint.
    ReplayFromCheckpoint {
        /// Turn to replay from.
        checkpoint_turn: u32,
        /// Blob hash of the checkpoint state.
        checkpoint_blob_hash: String,
    },
    /// Use the majority state (for 3+ players).
    MajorityState {
        /// The majority hash.
        state_hash: String,
        /// Blob hash of the majority state.
        blob_hash: String,
        /// Players who need to resync.
        resync_players: Vec<String>,
    },
    /// Two-player tie-break: lower player ID wins.
    TwoPlayerTieBreak {
        /// The winning player's pubkey.
        winner_pubkey: String,
        /// Blob hash of the winning state.
        blob_hash: String,
    },
    /// Cannot recover automatically.
    ManualIntervention {
        /// Reason why automatic recovery is not possible.
        reason: String,
    },
}

/// Configuration for desync detection and recovery.
#[derive(Debug, Clone)]
pub struct DesyncConfig {
    /// How often to create checkpoints (every N turns). 0 = disabled.
    pub checkpoint_interval: u32,
    /// Maximum number of checkpoints to retain.
    pub max_checkpoints: usize,
    /// Player public keys in the game.
    pub player_pubkeys: Vec<String>,
}

/// Manages desync detection and recovery for a game session.
#[derive(Debug)]
pub struct DesyncDetector {
    config: DesyncConfig,
    /// Checkpoints stored for recovery, keyed by turn.
    checkpoints: HashMap<u32, RecoveryCheckpoint>,
    /// Last turn where all nodes were in sync.
    last_sync_turn: u32,
    /// History of state hashes per turn for diagnosis.
    /// Key: turn, Value: map of player_pubkey -> state_hash
    hash_history: HashMap<u32, HashMap<String, String>>,
}

impl DesyncDetector {
    /// Create a new desync detector with the given configuration.
    pub fn new(config: DesyncConfig) -> Self {
        Self {
            config,
            checkpoints: HashMap::new(),
            last_sync_turn: 0,
            hash_history: HashMap::new(),
        }
    }

    /// Record a state hash from a player for a given turn.
    pub fn record_hash(&mut self, turn: u32, player_pubkey: &str, state_hash: &str) {
        self.hash_history
            .entry(turn)
            .or_default()
            .insert(player_pubkey.to_string(), state_hash.to_string());
    }

    /// Check if all players have submitted hashes for a turn and compare them.
    pub fn check_turn(&self, turn: u32) -> DesyncStatus {
        let hashes = match self.hash_history.get(&turn) {
            Some(h) => h,
            None => return DesyncStatus::InSync, // No data yet
        };

        if hashes.len() < self.config.player_pubkeys.len() {
            return DesyncStatus::InSync; // Not all players reported yet
        }

        // Count occurrences of each hash
        let mut hash_counts: HashMap<&str, usize> = HashMap::new();
        for hash in hashes.values() {
            *hash_counts.entry(hash.as_str()).or_insert(0) += 1;
        }

        if hash_counts.len() == 1 {
            return DesyncStatus::InSync;
        }

        // Find majority hash
        let majority = hash_counts
            .iter()
            .max_by_key(|(_, count)| **count)
            .map(|(hash, _)| hash.to_string());

        let divergent = if let Some(ref maj) = majority {
            hashes
                .iter()
                .filter(|(_, h)| h.as_str() != maj.as_str())
                .map(|(pk, _)| pk.clone())
                .collect()
        } else {
            vec![]
        };

        DesyncStatus::Desynced {
            turn,
            hashes: hashes.clone(),
            majority_hash: majority,
            divergent_players: divergent,
        }
    }

    /// Record that a turn was in sync and update `last_sync_turn`.
    pub fn mark_in_sync(&mut self, turn: u32) {
        if turn > self.last_sync_turn {
            self.last_sync_turn = turn;
        }
    }

    /// Get the last turn where all nodes agreed.
    pub fn last_sync_turn(&self) -> u32 {
        self.last_sync_turn
    }

    /// Store a checkpoint for a turn.
    pub fn store_checkpoint(&mut self, checkpoint: RecoveryCheckpoint) {
        let turn = checkpoint.turn;
        self.checkpoints.insert(turn, checkpoint);

        // Prune old checkpoints if over limit
        if self.checkpoints.len() > self.config.max_checkpoints
            && let Some(oldest) = self.checkpoints.keys().min().copied()
        {
            self.checkpoints.remove(&oldest);
        }
    }

    /// Check if a checkpoint should be created at this turn.
    pub fn should_checkpoint(&self, turn: u32) -> bool {
        self.config.checkpoint_interval > 0
            && turn > 0
            && turn.is_multiple_of(self.config.checkpoint_interval)
    }

    /// Get the most recent checkpoint at or before the given turn.
    pub fn latest_checkpoint_before(&self, turn: u32) -> Option<&RecoveryCheckpoint> {
        self.checkpoints
            .iter()
            .filter(|(t, _)| **t <= turn)
            .max_by_key(|(t, _)| **t)
            .map(|(_, cp)| cp)
    }

    /// Determine the best recovery strategy for a desync.
    pub fn determine_recovery(&self, desync: &DesyncStatus) -> RecoveryStrategy {
        match desync {
            DesyncStatus::InSync => RecoveryStrategy::ManualIntervention {
                reason: "no desync detected".to_string(),
            },
            DesyncStatus::Desynced {
                turn,
                majority_hash,
                divergent_players,
                ..
            } => {
                let num_players = self.config.player_pubkeys.len();

                // Try checkpoint replay first
                if let Some(cp) = self.latest_checkpoint_before(*turn) {
                    return RecoveryStrategy::ReplayFromCheckpoint {
                        checkpoint_turn: cp.turn,
                        checkpoint_blob_hash: cp.blob_hash.clone(),
                    };
                }

                // For 3+ players, use majority
                if num_players >= 3
                    && let Some(maj_hash) = majority_hash
                {
                    return RecoveryStrategy::MajorityState {
                        state_hash: maj_hash.clone(),
                        blob_hash: String::new(), // Needs to be filled from state transfer
                        resync_players: divergent_players.clone(),
                    };
                }

                // For 2 players, tie-break by lower pubkey
                if num_players == 2 {
                    let mut sorted = self.config.player_pubkeys.clone();
                    sorted.sort();
                    return RecoveryStrategy::TwoPlayerTieBreak {
                        winner_pubkey: sorted[0].clone(),
                        blob_hash: String::new(),
                    };
                }

                RecoveryStrategy::ManualIntervention {
                    reason: "unable to determine recovery strategy".to_string(),
                }
            }
        }
    }

    /// Get the number of stored checkpoints.
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    /// Binary search for the divergence turn given a range.
    ///
    /// Returns the earliest turn where hashes diverge, or `None` if
    /// all turns in the range are in sync.
    pub fn find_divergence_turn(&self, start: u32, end: u32) -> Option<u32> {
        let mut lo = start;
        let mut hi = end;
        let mut result = None;

        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            match self.check_turn(mid) {
                DesyncStatus::Desynced { .. } => {
                    result = Some(mid);
                    if mid == lo {
                        break;
                    }
                    hi = mid - 1;
                }
                DesyncStatus::InSync => {
                    lo = mid + 1;
                }
            }
        }

        result
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn two_player_config() -> DesyncConfig {
        DesyncConfig {
            checkpoint_interval: 5,
            max_checkpoints: 10,
            player_pubkeys: vec!["alice".into(), "bob".into()],
        }
    }

    fn three_player_config() -> DesyncConfig {
        DesyncConfig {
            checkpoint_interval: 5,
            max_checkpoints: 10,
            player_pubkeys: vec!["alice".into(), "bob".into(), "carol".into()],
        }
    }

    // -- In-sync detection ------------------------------------------------

    #[test]
    fn in_sync_when_all_hashes_match() {
        let mut det = DesyncDetector::new(two_player_config());
        det.record_hash(1, "alice", "aaa");
        det.record_hash(1, "bob", "aaa");
        assert_eq!(det.check_turn(1), DesyncStatus::InSync);
    }

    #[test]
    fn in_sync_when_no_data() {
        let det = DesyncDetector::new(two_player_config());
        assert_eq!(det.check_turn(1), DesyncStatus::InSync);
    }

    #[test]
    fn in_sync_when_not_all_players_reported() {
        let mut det = DesyncDetector::new(two_player_config());
        det.record_hash(1, "alice", "aaa");
        assert_eq!(det.check_turn(1), DesyncStatus::InSync);
    }

    // -- Desync detection -------------------------------------------------

    #[test]
    fn desync_detected_different_hashes() {
        let mut det = DesyncDetector::new(two_player_config());
        det.record_hash(1, "alice", "aaa");
        det.record_hash(1, "bob", "bbb");
        match det.check_turn(1) {
            DesyncStatus::Desynced {
                turn,
                hashes,
                majority_hash,
                divergent_players,
            } => {
                assert_eq!(turn, 1);
                assert_eq!(hashes.len(), 2);
                // With 2 players and different hashes, majority is either one
                assert!(majority_hash.is_some());
                assert_eq!(divergent_players.len(), 1);
            }
            other => panic!("expected Desynced, got {other:?}"),
        }
    }

    // -- Majority calculation ---------------------------------------------

    #[test]
    fn majority_hash_with_three_players() {
        let mut det = DesyncDetector::new(three_player_config());
        det.record_hash(1, "alice", "aaa");
        det.record_hash(1, "bob", "aaa");
        det.record_hash(1, "carol", "bbb");
        match det.check_turn(1) {
            DesyncStatus::Desynced {
                majority_hash,
                divergent_players,
                ..
            } => {
                assert_eq!(majority_hash, Some("aaa".to_string()));
                assert_eq!(divergent_players, vec!["carol".to_string()]);
            }
            other => panic!("expected Desynced, got {other:?}"),
        }
    }

    #[test]
    fn three_player_all_different() {
        let mut det = DesyncDetector::new(three_player_config());
        det.record_hash(1, "alice", "aaa");
        det.record_hash(1, "bob", "bbb");
        det.record_hash(1, "carol", "ccc");
        match det.check_turn(1) {
            DesyncStatus::Desynced {
                majority_hash,
                divergent_players,
                ..
            } => {
                // All hashes have count 1, so the "majority" is just whichever
                // max_by_key picks (non-deterministic order). We just check it
                // is Some and 2 players diverge from it.
                assert!(majority_hash.is_some());
                assert_eq!(divergent_players.len(), 2);
            }
            other => panic!("expected Desynced, got {other:?}"),
        }
    }

    // -- Checkpoint storage and retrieval ---------------------------------

    #[test]
    fn store_and_retrieve_checkpoint() {
        let mut det = DesyncDetector::new(two_player_config());
        det.store_checkpoint(RecoveryCheckpoint {
            turn: 5,
            state_hash: "h5".into(),
            blob_hash: "b5".into(),
            agreement_count: 2,
        });
        det.store_checkpoint(RecoveryCheckpoint {
            turn: 10,
            state_hash: "h10".into(),
            blob_hash: "b10".into(),
            agreement_count: 2,
        });

        let cp = det.latest_checkpoint_before(12).unwrap();
        assert_eq!(cp.turn, 10);

        let cp = det.latest_checkpoint_before(7).unwrap();
        assert_eq!(cp.turn, 5);

        let cp = det.latest_checkpoint_before(5).unwrap();
        assert_eq!(cp.turn, 5);

        assert!(det.latest_checkpoint_before(4).is_none());
    }

    // -- Checkpoint pruning -----------------------------------------------

    #[test]
    fn checkpoint_pruning() {
        let config = DesyncConfig {
            checkpoint_interval: 1,
            max_checkpoints: 3,
            player_pubkeys: vec!["alice".into()],
        };
        let mut det = DesyncDetector::new(config);

        for t in 1..=5 {
            det.store_checkpoint(RecoveryCheckpoint {
                turn: t,
                state_hash: format!("h{t}"),
                blob_hash: format!("b{t}"),
                agreement_count: 1,
            });
        }
        // Should have pruned down to max_checkpoints (3)
        assert_eq!(det.checkpoint_count(), 3);
        // Oldest checkpoints (1, 2) should have been removed
        assert!(det.latest_checkpoint_before(1).is_none());
        assert!(det.latest_checkpoint_before(2).is_none());
        assert!(det.latest_checkpoint_before(3).is_some());
    }

    // -- should_checkpoint logic ------------------------------------------

    #[test]
    fn should_checkpoint_at_interval() {
        let det = DesyncDetector::new(two_player_config()); // interval = 5
        assert!(!det.should_checkpoint(0));
        assert!(!det.should_checkpoint(1));
        assert!(!det.should_checkpoint(4));
        assert!(det.should_checkpoint(5));
        assert!(det.should_checkpoint(10));
        assert!(!det.should_checkpoint(11));
    }

    #[test]
    fn should_checkpoint_disabled_when_interval_zero() {
        let config = DesyncConfig {
            checkpoint_interval: 0,
            max_checkpoints: 10,
            player_pubkeys: vec!["alice".into()],
        };
        let det = DesyncDetector::new(config);
        assert!(!det.should_checkpoint(0));
        assert!(!det.should_checkpoint(5));
        assert!(!det.should_checkpoint(10));
    }

    // -- Recovery strategy: checkpoint replay -----------------------------

    #[test]
    fn recovery_replay_from_checkpoint() {
        let mut det = DesyncDetector::new(two_player_config());
        det.store_checkpoint(RecoveryCheckpoint {
            turn: 5,
            state_hash: "h5".into(),
            blob_hash: "b5".into(),
            agreement_count: 2,
        });
        det.record_hash(7, "alice", "aaa");
        det.record_hash(7, "bob", "bbb");
        let status = det.check_turn(7);

        match det.determine_recovery(&status) {
            RecoveryStrategy::ReplayFromCheckpoint {
                checkpoint_turn,
                checkpoint_blob_hash,
            } => {
                assert_eq!(checkpoint_turn, 5);
                assert_eq!(checkpoint_blob_hash, "b5");
            }
            other => panic!("expected ReplayFromCheckpoint, got {other:?}"),
        }
    }

    // -- Recovery strategy: majority --------------------------------------

    #[test]
    fn recovery_majority_state() {
        let mut det = DesyncDetector::new(three_player_config());
        // No checkpoints -> should fall through to majority
        det.record_hash(3, "alice", "aaa");
        det.record_hash(3, "bob", "aaa");
        det.record_hash(3, "carol", "bbb");
        let status = det.check_turn(3);

        match det.determine_recovery(&status) {
            RecoveryStrategy::MajorityState {
                state_hash,
                resync_players,
                ..
            } => {
                assert_eq!(state_hash, "aaa");
                assert_eq!(resync_players, vec!["carol".to_string()]);
            }
            other => panic!("expected MajorityState, got {other:?}"),
        }
    }

    // -- Recovery strategy: two-player tie-break --------------------------

    #[test]
    fn recovery_two_player_tie_break() {
        let mut det = DesyncDetector::new(two_player_config());
        // No checkpoints, 2 players -> tie-break
        det.record_hash(3, "alice", "aaa");
        det.record_hash(3, "bob", "bbb");
        let status = det.check_turn(3);

        match det.determine_recovery(&status) {
            RecoveryStrategy::TwoPlayerTieBreak { winner_pubkey, .. } => {
                assert_eq!(winner_pubkey, "alice"); // "alice" < "bob"
            }
            other => panic!("expected TwoPlayerTieBreak, got {other:?}"),
        }
    }

    // -- Recovery strategy: manual intervention ---------------------------

    #[test]
    fn recovery_manual_for_in_sync() {
        let det = DesyncDetector::new(two_player_config());
        let status = DesyncStatus::InSync;
        match det.determine_recovery(&status) {
            RecoveryStrategy::ManualIntervention { reason } => {
                assert!(reason.contains("no desync"));
            }
            other => panic!("expected ManualIntervention, got {other:?}"),
        }
    }

    // -- Binary search for divergence -------------------------------------

    #[test]
    fn find_divergence_turn_basic() {
        let mut det = DesyncDetector::new(two_player_config());
        // Turns 1-4 in sync, turn 5+ desynced
        for t in 1..=4 {
            det.record_hash(t, "alice", "same");
            det.record_hash(t, "bob", "same");
        }
        for t in 5..=10 {
            det.record_hash(t, "alice", "aaa");
            det.record_hash(t, "bob", "bbb");
        }
        assert_eq!(det.find_divergence_turn(1, 10), Some(5));
    }

    #[test]
    fn find_divergence_turn_all_in_sync() {
        let mut det = DesyncDetector::new(two_player_config());
        for t in 1..=5 {
            det.record_hash(t, "alice", "same");
            det.record_hash(t, "bob", "same");
        }
        assert_eq!(det.find_divergence_turn(1, 5), None);
    }

    #[test]
    fn find_divergence_turn_first_turn() {
        let mut det = DesyncDetector::new(two_player_config());
        det.record_hash(1, "alice", "aaa");
        det.record_hash(1, "bob", "bbb");
        assert_eq!(det.find_divergence_turn(1, 1), Some(1));
    }

    // -- Record + check flow ----------------------------------------------

    #[test]
    fn record_and_check_flow() {
        let mut det = DesyncDetector::new(two_player_config());

        // Turn 1: in sync
        det.record_hash(1, "alice", "h1");
        det.record_hash(1, "bob", "h1");
        assert_eq!(det.check_turn(1), DesyncStatus::InSync);
        det.mark_in_sync(1);
        assert_eq!(det.last_sync_turn(), 1);

        // Turn 2: desync
        det.record_hash(2, "alice", "h2a");
        det.record_hash(2, "bob", "h2b");
        match det.check_turn(2) {
            DesyncStatus::Desynced { turn, .. } => assert_eq!(turn, 2),
            other => panic!("expected Desynced, got {other:?}"),
        }
        // last_sync_turn should still be 1
        assert_eq!(det.last_sync_turn(), 1);
    }

    // -- Mark in sync updates ---------------------------------------------

    #[test]
    fn mark_in_sync_monotonic() {
        let mut det = DesyncDetector::new(two_player_config());
        det.mark_in_sync(5);
        assert_eq!(det.last_sync_turn(), 5);
        det.mark_in_sync(3); // Should not go backwards
        assert_eq!(det.last_sync_turn(), 5);
        det.mark_in_sync(10);
        assert_eq!(det.last_sync_turn(), 10);
    }

    // -- Config edge cases ------------------------------------------------

    #[test]
    fn single_player_always_in_sync() {
        let config = DesyncConfig {
            checkpoint_interval: 5,
            max_checkpoints: 10,
            player_pubkeys: vec!["solo".into()],
        };
        let mut det = DesyncDetector::new(config);
        det.record_hash(1, "solo", "hash");
        assert_eq!(det.check_turn(1), DesyncStatus::InSync);
    }

    #[test]
    fn checkpoint_count_starts_at_zero() {
        let det = DesyncDetector::new(two_player_config());
        assert_eq!(det.checkpoint_count(), 0);
    }

    // -- Recovery with checkpoint takes priority over majority -------------

    #[test]
    fn recovery_checkpoint_takes_priority_over_majority() {
        let mut det = DesyncDetector::new(three_player_config());
        det.store_checkpoint(RecoveryCheckpoint {
            turn: 2,
            state_hash: "h2".into(),
            blob_hash: "b2".into(),
            agreement_count: 3,
        });
        det.record_hash(5, "alice", "aaa");
        det.record_hash(5, "bob", "aaa");
        det.record_hash(5, "carol", "bbb");
        let status = det.check_turn(5);

        // Even though majority exists, checkpoint should be preferred
        match det.determine_recovery(&status) {
            RecoveryStrategy::ReplayFromCheckpoint {
                checkpoint_turn, ..
            } => {
                assert_eq!(checkpoint_turn, 2);
            }
            other => panic!("expected ReplayFromCheckpoint, got {other:?}"),
        }
    }
}

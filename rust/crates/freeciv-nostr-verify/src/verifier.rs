//! Top-level game verifier that orchestrates hash chain, commit collection,
//! and checkpointing for a single game session.
//!
//! `GameVerifier` is the main entry point for the verification subsystem.
//! It ties together:
//! - `TurnHashChain`: cryptographic chain of turn state hashes
//! - `TurnCommitCollector`: collecting and comparing state hashes from all players
//! - `CheckpointManager`: periodic snapshots for recovery
//! - Persistence: save/load state to disk

use std::path::Path;

use nostr::prelude::*;

use freeciv_nostr_core::events::{StateHash, build_state_hash_event};

use crate::checkpoint::{CheckpointConfig, CheckpointManager};
use crate::commit::{CommitError, ConsensusResult, TurnCommitCollector};
use crate::hash_chain::{HashChainError, TurnHashChain};
use crate::persistence::{PersistError, VerificationSnapshot};

/// Outcome of processing a complete turn.
#[derive(Debug)]
pub enum TurnOutcome {
    /// All players agree on the state hash. Turn is valid.
    Verified { turn: u64, state_hash: [u8; 32] },
    /// Players disagree. Desync detected.
    Desync {
        turn: u64,
        groups: std::collections::HashMap<[u8; 32], Vec<PublicKey>>,
    },
    /// Still waiting for commits from some players.
    Pending {
        turn: u64,
        received: usize,
        expected: usize,
    },
}

/// The main game verifier for a single game session.
///
/// Orchestrates the hash chain, commit collection, and checkpointing.
///
/// # Usage
///
/// ```ignore
/// let mut verifier = GameVerifier::new(game_event_id, player_keys, config);
///
/// // At end of each turn:
/// // 1. Compute state hash locally
/// let event_builder = verifier.create_state_hash_event(turn, state_hash);
/// // 2. Sign and publish the event
/// // 3. Record incoming commits from other players
/// verifier.record_commit(&incoming_event)?;
/// // 4. Check if turn is verified
/// let outcome = verifier.finalize_turn(turn, local_state_hash)?;
/// ```
#[derive(Debug)]
pub struct GameVerifier {
    chain: TurnHashChain,
    commits: TurnCommitCollector,
    checkpoints: CheckpointManager,
    /// Path for persistence, if configured.
    persist_path: Option<std::path::PathBuf>,
}

impl GameVerifier {
    /// Create a new verifier for a game session.
    pub fn new(
        game_event_id: EventId,
        players: impl IntoIterator<Item = PublicKey>,
        checkpoint_config: CheckpointConfig,
    ) -> Self {
        let players: Vec<PublicKey> = players.into_iter().collect();
        Self {
            chain: TurnHashChain::new(),
            commits: TurnCommitCollector::new(game_event_id, players),
            checkpoints: CheckpointManager::new(checkpoint_config),
            persist_path: None,
        }
    }

    /// Restore a verifier from a persisted snapshot.
    pub fn from_snapshot(snapshot: VerificationSnapshot) -> Self {
        Self {
            chain: snapshot.chain,
            commits: snapshot.commits,
            checkpoints: snapshot.checkpoints,
            persist_path: None,
        }
    }

    /// Set the persistence path. The verifier will auto-save after each
    /// verified turn.
    pub fn set_persist_path(&mut self, path: impl Into<std::path::PathBuf>) {
        self.persist_path = Some(path.into());
    }

    /// Load a verifier from a file, or create a new one if the file doesn't exist.
    pub fn load_or_new(
        path: &Path,
        game_event_id: EventId,
        players: impl IntoIterator<Item = PublicKey>,
        checkpoint_config: CheckpointConfig,
    ) -> Self {
        match VerificationSnapshot::load_from_file(path) {
            Ok(snapshot) => {
                tracing::info!(
                    chain_len = snapshot.chain.len(),
                    "restored verifier from snapshot"
                );
                let mut v = Self::from_snapshot(snapshot);
                v.persist_path = Some(path.to_path_buf());
                v
            }
            Err(_) => {
                tracing::info!("no snapshot found, creating new verifier");
                let mut v = Self::new(game_event_id, players, checkpoint_config);
                v.persist_path = Some(path.to_path_buf());
                v
            }
        }
    }

    /// Create an `EventBuilder` for publishing this node's state hash for a turn.
    ///
    /// The caller signs and publishes the resulting event, then feeds it
    /// back via `record_commit()`.
    pub fn create_state_hash_event(&self, turn: u64, state_hash: [u8; 32]) -> EventBuilder {
        let hash_data = StateHash {
            turn,
            hash: hex::encode(state_hash),
        };
        build_state_hash_event(self.commits.game_event_id(), &hash_data)
    }

    /// Record an incoming GAME_STATE_HASH event from any player (including self).
    pub fn record_commit(&mut self, event: &Event) -> Result<(), CommitError> {
        self.commits.record_commit(event)?;
        Ok(())
    }

    /// Finalize a turn: append the local state hash to the chain, check
    /// consensus, and optionally checkpoint and persist.
    ///
    /// Call this after the local node has computed its state hash AND
    /// all expected commits have been recorded (or after a timeout).
    pub fn finalize_turn(
        &mut self,
        turn: u64,
        local_state_hash: [u8; 32],
    ) -> Result<TurnOutcome, VerifierError> {
        // Check consensus first
        let consensus = self.commits.check_consensus(turn);

        match consensus {
            ConsensusResult::Agreed { state_hash, .. } => {
                // Verify local hash matches consensus
                if local_state_hash != state_hash {
                    return Err(VerifierError::LocalDesync {
                        turn,
                        local_hash: hex::encode(local_state_hash),
                        consensus_hash: hex::encode(state_hash),
                    });
                }

                // Append to chain
                self.chain
                    .append(turn, state_hash)
                    .map_err(VerifierError::Chain)?;

                // Maybe checkpoint
                self.checkpoints.maybe_checkpoint(turn, &self.chain);

                // Auto-persist
                self.auto_persist();

                tracing::debug!(turn, hash = %hex::encode(state_hash), "turn verified");

                Ok(TurnOutcome::Verified { turn, state_hash })
            }
            ConsensusResult::Desync { turn, groups } => {
                tracing::warn!(
                    turn,
                    groups = groups.len(),
                    "desync detected at turn {}",
                    turn
                );
                Ok(TurnOutcome::Desync { turn, groups })
            }
            ConsensusResult::Pending {
                turn,
                received,
                expected,
            } => Ok(TurnOutcome::Pending {
                turn,
                received,
                expected,
            }),
        }
    }

    /// Get a reference to the hash chain.
    pub fn chain(&self) -> &TurnHashChain {
        &self.chain
    }

    /// Get a reference to the commit collector.
    pub fn commits(&self) -> &TurnCommitCollector {
        &self.commits
    }

    /// Get a reference to the checkpoint manager.
    pub fn checkpoints(&self) -> &CheckpointManager {
        &self.checkpoints
    }

    /// Get players who haven't submitted commits for a turn.
    pub fn missing_commits(&self, turn: u64) -> Vec<PublicKey> {
        self.commits.missing_commits(turn)
    }

    /// Export the full chain for relay publishing.
    pub fn export_chain(&self) -> &[crate::hash_chain::ChainEntry] {
        self.chain.entries()
    }

    /// Create a snapshot of the current verification state.
    pub fn snapshot(&self) -> VerificationSnapshot {
        VerificationSnapshot {
            chain: self.chain.clone(),
            checkpoints: self.checkpoints.clone(),
            commits: self.commits.clone(),
        }
    }

    /// Manually trigger persistence.
    pub fn persist(&self) -> Result<(), PersistError> {
        if let Some(path) = &self.persist_path {
            self.snapshot().save_to_file(path)?;
        }
        Ok(())
    }

    fn auto_persist(&self) {
        if let Err(e) = self.persist() {
            tracing::warn!(error = %e, "failed to auto-persist verification state");
        }
    }
}

/// Errors from the game verifier.
#[derive(Debug, thiserror::Error)]
pub enum VerifierError {
    #[error("chain error: {0}")]
    Chain(#[from] HashChainError),

    #[error("local desync at turn {turn}: local={local_hash}, consensus={consensus_hash}")]
    LocalDesync {
        turn: u64,
        local_hash: String,
        consensus_hash: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn hash_bytes(seed: u8) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = seed;
        h
    }

    fn make_state_hash_event(
        keys: &Keys,
        game_id: EventId,
        turn: u64,
        state_hash: [u8; 32],
    ) -> Event {
        let hash_data = StateHash {
            turn,
            hash: hex::encode(state_hash),
        };
        let builder = build_state_hash_event(game_id, &hash_data);
        let unsigned = builder.build(keys.public_key());
        unsigned.sign_with_keys(keys).expect("signing works")
    }

    #[test]
    fn single_player_full_flow() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let config = CheckpointConfig {
            interval: 5,
            max_retained: 3,
        };

        let mut verifier = GameVerifier::new(game_id, [keys.public_key()], config);

        for turn in 0u64..10 {
            let state = hash_bytes(turn as u8);

            // Create and "publish" state hash event
            let event = make_state_hash_event(&keys, game_id, turn, state);
            verifier.record_commit(&event).unwrap();

            // Finalize
            match verifier.finalize_turn(turn, state).unwrap() {
                TurnOutcome::Verified {
                    turn: t,
                    state_hash,
                } => {
                    assert_eq!(t, turn);
                    assert_eq!(state_hash, state);
                }
                other => panic!("expected Verified at turn {}, got: {:?}", turn, other),
            }
        }

        assert_eq!(verifier.chain().len(), 10);
        assert!(verifier.chain().validate().is_ok());
        // Checkpoints at 0, 5 => 2 checkpoints
        assert_eq!(verifier.checkpoints().len(), 2);
    }

    #[test]
    fn two_player_agreement_flow() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let config = CheckpointConfig::default();

        let mut verifier =
            GameVerifier::new(game_id, [keys_a.public_key(), keys_b.public_key()], config);

        let state = hash_bytes(0x42);

        // Both players commit the same hash
        let event_a = make_state_hash_event(&keys_a, game_id, 0, state);
        let event_b = make_state_hash_event(&keys_b, game_id, 0, state);

        verifier.record_commit(&event_a).unwrap();
        verifier.record_commit(&event_b).unwrap();

        match verifier.finalize_turn(0, state).unwrap() {
            TurnOutcome::Verified { state_hash, .. } => {
                assert_eq!(state_hash, state);
            }
            other => panic!("expected Verified, got: {:?}", other),
        }
    }

    #[test]
    fn two_player_desync_detected() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let config = CheckpointConfig::default();

        let mut verifier =
            GameVerifier::new(game_id, [keys_a.public_key(), keys_b.public_key()], config);

        let state_a = hash_bytes(0xAA);
        let state_b = hash_bytes(0xBB);

        let event_a = make_state_hash_event(&keys_a, game_id, 0, state_a);
        let event_b = make_state_hash_event(&keys_b, game_id, 0, state_b);

        verifier.record_commit(&event_a).unwrap();
        verifier.record_commit(&event_b).unwrap();

        match verifier.finalize_turn(0, state_a).unwrap() {
            TurnOutcome::Desync { turn, groups } => {
                assert_eq!(turn, 0);
                assert_eq!(groups.len(), 2);
            }
            other => panic!("expected Desync, got: {:?}", other),
        }
    }

    #[test]
    fn pending_when_missing_commits() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let config = CheckpointConfig::default();

        let mut verifier =
            GameVerifier::new(game_id, [keys_a.public_key(), keys_b.public_key()], config);

        let state = hash_bytes(0x01);
        let event_a = make_state_hash_event(&keys_a, game_id, 0, state);
        verifier.record_commit(&event_a).unwrap();

        // Only A committed, B is missing
        match verifier.finalize_turn(0, state).unwrap() {
            TurnOutcome::Pending {
                received, expected, ..
            } => {
                assert_eq!(received, 1);
                assert_eq!(expected, 2);
            }
            other => panic!("expected Pending, got: {:?}", other),
        }

        let missing = verifier.missing_commits(0);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0], keys_b.public_key());
    }

    #[test]
    fn local_desync_error() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let config = CheckpointConfig::default();

        let mut verifier = GameVerifier::new(game_id, [keys.public_key()], config);

        let consensus_hash = hash_bytes(0xAA);
        let local_hash = hash_bytes(0xBB);

        let event = make_state_hash_event(&keys, game_id, 0, consensus_hash);
        verifier.record_commit(&event).unwrap();

        // Local hash doesn't match what we published
        let result = verifier.finalize_turn(0, local_hash);
        assert!(result.is_err());
        match result.unwrap_err() {
            VerifierError::LocalDesync { turn, .. } => {
                assert_eq!(turn, 0);
            }
            other => panic!("expected LocalDesync, got: {:?}", other),
        }
    }

    #[test]
    fn create_state_hash_event_builder() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let config = CheckpointConfig::default();

        let verifier = GameVerifier::new(game_id, [keys.public_key()], config);

        let state = hash_bytes(0x42);
        let builder = verifier.create_state_hash_event(5, state);
        let unsigned = builder.build(keys.public_key());

        assert_eq!(unsigned.kind, freeciv_nostr_core::kinds::GAME_STATE_HASH);

        let parsed: StateHash = serde_json::from_str(&unsigned.content).unwrap();
        assert_eq!(parsed.turn, 5);
        assert_eq!(parsed.hash, hex::encode(state));
    }

    #[test]
    fn snapshot_and_restore() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let config = CheckpointConfig {
            interval: 5,
            max_retained: 3,
        };

        let mut verifier = GameVerifier::new(game_id, [keys.public_key()], config);

        // Run 10 turns
        for turn in 0u64..10 {
            let state = hash_bytes(turn as u8);
            let event = make_state_hash_event(&keys, game_id, turn, state);
            verifier.record_commit(&event).unwrap();
            verifier.finalize_turn(turn, state).unwrap();
        }

        // Snapshot
        let snapshot = verifier.snapshot();
        let json = snapshot.to_json().unwrap();

        // Restore
        let restored_snapshot = VerificationSnapshot::from_json(&json).unwrap();
        let restored = GameVerifier::from_snapshot(restored_snapshot);

        assert_eq!(restored.chain().len(), verifier.chain().len());
        assert_eq!(restored.chain().head_hash(), verifier.chain().head_hash());
        assert!(restored.chain().validate().is_ok());
    }

    #[test]
    fn file_persistence_flow() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let config = CheckpointConfig {
            interval: 5,
            max_retained: 3,
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("verifier.json");

        let mut verifier = GameVerifier::new(game_id, [keys.public_key()], config.clone());
        verifier.set_persist_path(&path);

        for turn in 0u64..5 {
            let state = hash_bytes(turn as u8);
            let event = make_state_hash_event(&keys, game_id, turn, state);
            verifier.record_commit(&event).unwrap();
            verifier.finalize_turn(turn, state).unwrap();
        }

        // File should exist after auto-persist
        assert!(path.exists());

        // Restore from file
        let restored = GameVerifier::load_or_new(&path, game_id, [keys.public_key()], config);
        assert_eq!(restored.chain().len(), 5);
        assert!(restored.chain().validate().is_ok());
    }

    #[test]
    fn export_chain_returns_all_entries() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let config = CheckpointConfig::default();

        let mut verifier = GameVerifier::new(game_id, [keys.public_key()], config);

        for turn in 0u64..3 {
            let state = hash_bytes(turn as u8);
            let event = make_state_hash_event(&keys, game_id, turn, state);
            verifier.record_commit(&event).unwrap();
            verifier.finalize_turn(turn, state).unwrap();
        }

        let entries = verifier.export_chain();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].turn, 0);
        assert_eq!(entries[1].turn, 1);
        assert_eq!(entries[2].turn, 2);
    }
}

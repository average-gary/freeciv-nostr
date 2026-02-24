//! Turn commit collection and consensus verification.
//!
//! At the end of each turn, every node publishes a `GAME_STATE_HASH` event
//! (kind 4203) containing their computed state hash. The `TurnCommitCollector`
//! gathers these per-player commits and checks for consensus: all players
//! must agree on the same state hash for a turn to be considered valid.

use std::collections::{HashMap, HashSet};

use nostr::prelude::*;
use serde::{Deserialize, Serialize};

use freeciv_nostr_core::events::StateHash;
use freeciv_nostr_core::kinds;

/// Result of checking consensus for a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsensusResult {
    /// All players agree on the state hash.
    Agreed { turn: u64, state_hash: [u8; 32] },
    /// Players disagree -- desync detected.
    Desync {
        turn: u64,
        /// Map of state_hash -> list of players who submitted it.
        groups: HashMap<[u8; 32], Vec<PublicKey>>,
    },
    /// Still waiting for commits from some players.
    Pending {
        turn: u64,
        received: usize,
        expected: usize,
    },
}

/// A single player's state hash commit for a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCommit {
    /// The player who submitted this commit.
    pub player: PublicKey,
    /// Turn number.
    pub turn: u64,
    /// The state hash they computed.
    pub state_hash: [u8; 32],
    /// The Nostr event ID of their GAME_STATE_HASH event.
    pub event_id: EventId,
}

/// Collects state hash commits from all players and checks consensus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCommitCollector {
    /// The game event ID this collector tracks.
    game_event_id: EventId,
    /// Set of players expected to submit commits.
    players: HashSet<PublicKey>,
    /// Per-turn commits: turn -> (player -> commit).
    commits: HashMap<u64, HashMap<PublicKey, TurnCommit>>,
}

impl TurnCommitCollector {
    /// Create a new collector for the given game and player set.
    pub fn new(game_event_id: EventId, players: impl IntoIterator<Item = PublicKey>) -> Self {
        Self {
            game_event_id,
            players: players.into_iter().collect(),
            commits: HashMap::new(),
        }
    }

    /// The game event ID.
    pub fn game_event_id(&self) -> EventId {
        self.game_event_id
    }

    /// Number of players expected to commit each turn.
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Record a GAME_STATE_HASH event from a player.
    ///
    /// Validates:
    /// - Event kind is GAME_STATE_HASH (4203)
    /// - Event references the correct game
    /// - Player is a known participant
    /// - Player has not already committed for this turn
    pub fn record_commit(&mut self, event: &Event) -> Result<&TurnCommit, CommitError> {
        // Validate kind
        if event.kind != kinds::GAME_STATE_HASH {
            return Err(CommitError::WrongKind {
                expected: kinds::GAME_STATE_HASH.as_u16(),
                got: event.kind.as_u16(),
            });
        }

        // Validate game reference
        let has_game_ref = event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(|v| v.as_str()) == Some("e")
                && s.get(1).map(|v| v.as_str()) == Some(&self.game_event_id.to_hex())
        });
        if !has_game_ref {
            return Err(CommitError::WrongGame);
        }

        // Validate player
        if !self.players.contains(&event.pubkey) {
            return Err(CommitError::UnknownPlayer(event.pubkey.to_hex()));
        }

        // Parse content
        let state_hash_data: StateHash = serde_json::from_str(&event.content)
            .map_err(|e| CommitError::InvalidContent(e.to_string()))?;

        // Parse hex hash to bytes
        let hash_bytes: [u8; 32] = hex::decode(&state_hash_data.hash)
            .map_err(|e| CommitError::InvalidContent(format!("invalid hex hash: {}", e)))?
            .try_into()
            .map_err(|v: Vec<u8>| {
                CommitError::InvalidContent(format!(
                    "hash wrong length: expected 32, got {}",
                    v.len()
                ))
            })?;

        let turn = state_hash_data.turn;

        // Check for duplicate commit
        let turn_commits = self.commits.entry(turn).or_default();
        if turn_commits.contains_key(&event.pubkey) {
            return Err(CommitError::DuplicateCommit {
                player: event.pubkey.to_hex(),
                turn,
            });
        }

        let commit = TurnCommit {
            player: event.pubkey,
            turn,
            state_hash: hash_bytes,
            event_id: event.id,
        };

        turn_commits.insert(event.pubkey, commit);

        Ok(turn_commits.get(&event.pubkey).unwrap())
    }

    /// Check consensus for a given turn.
    pub fn check_consensus(&self, turn: u64) -> ConsensusResult {
        let turn_commits = match self.commits.get(&turn) {
            Some(c) => c,
            None => {
                return ConsensusResult::Pending {
                    turn,
                    received: 0,
                    expected: self.players.len(),
                };
            }
        };

        if turn_commits.len() < self.players.len() {
            return ConsensusResult::Pending {
                turn,
                received: turn_commits.len(),
                expected: self.players.len(),
            };
        }

        // All commits received -- check if they agree
        let mut groups: HashMap<[u8; 32], Vec<PublicKey>> = HashMap::new();
        for commit in turn_commits.values() {
            groups
                .entry(commit.state_hash)
                .or_default()
                .push(commit.player);
        }

        if groups.len() == 1 {
            let (hash, _) = groups.into_iter().next().unwrap();
            ConsensusResult::Agreed {
                turn,
                state_hash: hash,
            }
        } else {
            ConsensusResult::Desync { turn, groups }
        }
    }

    /// Get all commits for a specific turn.
    pub fn get_turn_commits(&self, turn: u64) -> Option<&HashMap<PublicKey, TurnCommit>> {
        self.commits.get(&turn)
    }

    /// Remove commits for turns older than `keep_from` to free memory.
    pub fn prune_before(&mut self, keep_from: u64) {
        self.commits.retain(|&turn, _| turn >= keep_from);
    }

    /// Get the set of players who have NOT yet committed for a turn.
    pub fn missing_commits(&self, turn: u64) -> Vec<PublicKey> {
        let committed: HashSet<&PublicKey> = self
            .commits
            .get(&turn)
            .map(|c| c.keys().collect())
            .unwrap_or_default();

        self.players
            .iter()
            .filter(|p| !committed.contains(p))
            .copied()
            .collect()
    }
}

/// Errors from commit operations.
#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    #[error("wrong event kind: expected {expected}, got {got}")]
    WrongKind { expected: u16, got: u16 },

    #[error("event does not reference the expected game")]
    WrongGame,

    #[error("unknown player: {0}")]
    UnknownPlayer(String),

    #[error("duplicate commit from player {player} for turn {turn}")]
    DuplicateCommit { player: String, turn: u64 },

    #[error("invalid event content: {0}")]
    InvalidContent(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeciv_nostr_core::events::build_state_hash_event;
    use nostr::Keys;

    fn make_state_hash_event(keys: &Keys, game_id: EventId, turn: u64, hash_hex: &str) -> Event {
        let state_hash = StateHash {
            turn,
            hash: hash_hex.to_string(),
        };
        let builder = build_state_hash_event(game_id, &state_hash);
        let unsigned = builder.build(keys.public_key());
        unsigned.sign_with_keys(keys).expect("signing should work")
    }

    /// Produce a deterministic 32-byte hex string from a seed.
    fn hash_hex(seed: u8) -> String {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        hex::encode(bytes)
    }

    #[test]
    fn empty_collector_returns_pending() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let collector = TurnCommitCollector::new(game_id, [keys.public_key()]);

        match collector.check_consensus(0) {
            ConsensusResult::Pending {
                turn,
                received,
                expected,
            } => {
                assert_eq!(turn, 0);
                assert_eq!(received, 0);
                assert_eq!(expected, 1);
            }
            other => panic!("expected Pending, got: {:?}", other),
        }
    }

    #[test]
    fn single_player_consensus() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut collector = TurnCommitCollector::new(game_id, [keys.public_key()]);

        let event = make_state_hash_event(&keys, game_id, 0, &hash_hex(0xAB));
        collector.record_commit(&event).unwrap();

        match collector.check_consensus(0) {
            ConsensusResult::Agreed { turn, state_hash } => {
                assert_eq!(turn, 0);
                assert_eq!(state_hash[0], 0xAB);
            }
            other => panic!("expected Agreed, got: {:?}", other),
        }
    }

    #[test]
    fn two_player_agreement() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let mut collector =
            TurnCommitCollector::new(game_id, [keys_a.public_key(), keys_b.public_key()]);

        let same_hash = hash_hex(0x42);
        let event_a = make_state_hash_event(&keys_a, game_id, 0, &same_hash);
        let event_b = make_state_hash_event(&keys_b, game_id, 0, &same_hash);

        collector.record_commit(&event_a).unwrap();

        // After first commit, still pending
        match collector.check_consensus(0) {
            ConsensusResult::Pending {
                received, expected, ..
            } => {
                assert_eq!(received, 1);
                assert_eq!(expected, 2);
            }
            other => panic!("expected Pending, got: {:?}", other),
        }

        collector.record_commit(&event_b).unwrap();

        match collector.check_consensus(0) {
            ConsensusResult::Agreed { state_hash, .. } => {
                assert_eq!(state_hash[0], 0x42);
            }
            other => panic!("expected Agreed, got: {:?}", other),
        }
    }

    #[test]
    fn two_player_desync() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let mut collector =
            TurnCommitCollector::new(game_id, [keys_a.public_key(), keys_b.public_key()]);

        let event_a = make_state_hash_event(&keys_a, game_id, 0, &hash_hex(0xAA));
        let event_b = make_state_hash_event(&keys_b, game_id, 0, &hash_hex(0xBB));

        collector.record_commit(&event_a).unwrap();
        collector.record_commit(&event_b).unwrap();

        match collector.check_consensus(0) {
            ConsensusResult::Desync { turn, groups } => {
                assert_eq!(turn, 0);
                assert_eq!(groups.len(), 2);
            }
            other => panic!("expected Desync, got: {:?}", other),
        }
    }

    #[test]
    fn rejects_unknown_player() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_unknown = Keys::generate();
        let mut collector = TurnCommitCollector::new(game_id, [keys_a.public_key()]);

        let event = make_state_hash_event(&keys_unknown, game_id, 0, &hash_hex(0x01));
        let result = collector.record_commit(&event);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CommitError::UnknownPlayer(_)));
    }

    #[test]
    fn rejects_duplicate_commit() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut collector = TurnCommitCollector::new(game_id, [keys.public_key()]);

        let event1 = make_state_hash_event(&keys, game_id, 0, &hash_hex(0x01));
        collector.record_commit(&event1).unwrap();

        let event2 = make_state_hash_event(&keys, game_id, 0, &hash_hex(0x01));
        let result = collector.record_commit(&event2);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CommitError::DuplicateCommit { .. }
        ));
    }

    #[test]
    fn rejects_wrong_game() {
        let game_id = EventId::all_zeros();
        let other_game = EventId::from_slice(&[1u8; 32]).unwrap();
        let keys = Keys::generate();
        let mut collector = TurnCommitCollector::new(game_id, [keys.public_key()]);

        let event = make_state_hash_event(&keys, other_game, 0, &hash_hex(0x01));
        let result = collector.record_commit(&event);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CommitError::WrongGame));
    }

    #[test]
    fn rejects_wrong_kind() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut collector = TurnCommitCollector::new(game_id, [keys.public_key()]);

        // Build an event with wrong kind
        let builder =
            EventBuilder::new(kinds::GAME_CHAT, "not a state hash").tags(vec![Tag::event(game_id)]);
        let unsigned = builder.build(keys.public_key());
        let event = unsigned.sign_with_keys(&keys).unwrap();

        let result = collector.record_commit(&event);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CommitError::WrongKind { .. }));
    }

    #[test]
    fn missing_commits_tracks_outstanding_players() {
        let game_id = EventId::all_zeros();
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let mut collector =
            TurnCommitCollector::new(game_id, [keys_a.public_key(), keys_b.public_key()]);

        // Before any commits, both are missing
        let missing = collector.missing_commits(0);
        assert_eq!(missing.len(), 2);

        // After A commits, only B is missing
        let event_a = make_state_hash_event(&keys_a, game_id, 0, &hash_hex(0x01));
        collector.record_commit(&event_a).unwrap();

        let missing = collector.missing_commits(0);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0], keys_b.public_key());
    }

    #[test]
    fn prune_removes_old_turns() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut collector = TurnCommitCollector::new(game_id, [keys.public_key()]);

        for turn in 0..5 {
            let event = make_state_hash_event(&keys, game_id, turn, &hash_hex(turn as u8));
            collector.record_commit(&event).unwrap();
        }

        collector.prune_before(3);

        assert!(collector.get_turn_commits(0).is_none());
        assert!(collector.get_turn_commits(1).is_none());
        assert!(collector.get_turn_commits(2).is_none());
        assert!(collector.get_turn_commits(3).is_some());
        assert!(collector.get_turn_commits(4).is_some());
    }

    #[test]
    fn multi_turn_tracking() {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();
        let mut collector = TurnCommitCollector::new(game_id, [keys.public_key()]);

        for turn in 0..3 {
            let event = make_state_hash_event(&keys, game_id, turn, &hash_hex(turn as u8));
            collector.record_commit(&event).unwrap();
        }

        for turn in 0..3 {
            match collector.check_consensus(turn) {
                ConsensusResult::Agreed { turn: t, .. } => assert_eq!(t, turn),
                other => panic!("expected Agreed for turn {}, got: {:?}", turn, other),
            }
        }
    }
}

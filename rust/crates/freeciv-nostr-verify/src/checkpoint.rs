//! Periodic checkpoint management for desync recovery.
//!
//! Checkpoints are snapshots of the turn hash chain state taken at
//! configurable intervals. They enable faster recovery by allowing
//! nodes to sync from the nearest checkpoint rather than replaying
//! from the beginning.

use serde::{Deserialize, Serialize};

use crate::hash_chain::TurnHashChain;

/// Configuration for checkpoint frequency and retention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointConfig {
    /// Create a checkpoint every N turns.
    pub interval: u64,
    /// Maximum number of checkpoints to retain. Older ones are pruned.
    /// 0 means unlimited.
    pub max_retained: usize,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            interval: 10,
            max_retained: 10,
        }
    }
}

/// A checkpoint snapshot of the hash chain at a specific turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Turn number this checkpoint was taken at.
    pub turn: u64,
    /// The chain hash at this turn.
    pub chain_hash: [u8; 32],
    /// The state hash at this turn.
    pub state_hash: [u8; 32],
}

/// Manages periodic checkpoints of the hash chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointManager {
    config: CheckpointConfig,
    checkpoints: Vec<Checkpoint>,
}

impl CheckpointManager {
    /// Create a new checkpoint manager with the given configuration.
    pub fn new(config: CheckpointConfig) -> Self {
        Self {
            config,
            checkpoints: Vec::new(),
        }
    }

    /// Check if a checkpoint should be taken at the given turn.
    pub fn should_checkpoint(&self, turn: u64) -> bool {
        self.config.interval > 0 && turn.is_multiple_of(self.config.interval)
    }

    /// Take a checkpoint from the current chain state.
    ///
    /// Returns the checkpoint if one was created, or `None` if this
    /// turn is not a checkpoint turn.
    pub fn maybe_checkpoint(&mut self, turn: u64, chain: &TurnHashChain) -> Option<&Checkpoint> {
        if !self.should_checkpoint(turn) {
            return None;
        }

        let entry = chain.get(turn)?;

        let checkpoint = Checkpoint {
            turn,
            chain_hash: entry.chain_hash,
            state_hash: entry.state_hash,
        };

        self.checkpoints.push(checkpoint);

        // Prune old checkpoints if max_retained > 0
        if self.config.max_retained > 0 && self.checkpoints.len() > self.config.max_retained {
            let excess = self.checkpoints.len() - self.config.max_retained;
            self.checkpoints.drain(0..excess);
        }

        self.checkpoints.last()
    }

    /// Get the most recent checkpoint at or before the given turn.
    pub fn latest_before(&self, turn: u64) -> Option<&Checkpoint> {
        self.checkpoints.iter().rev().find(|cp| cp.turn <= turn)
    }

    /// Get the most recent checkpoint overall.
    pub fn latest(&self) -> Option<&Checkpoint> {
        self.checkpoints.last()
    }

    /// Get all checkpoints.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Number of checkpoints stored.
    pub fn len(&self) -> usize {
        self.checkpoints.len()
    }

    /// Whether any checkpoints exist.
    pub fn is_empty(&self) -> bool {
        self.checkpoints.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_chain(turns: u64) -> TurnHashChain {
        let mut chain = TurnHashChain::new();
        for i in 0..turns {
            let mut state = [0u8; 32];
            state[0] = i as u8;
            chain.append(i, state).unwrap();
        }
        chain
    }

    #[test]
    fn default_config() {
        let config = CheckpointConfig::default();
        assert_eq!(config.interval, 10);
        assert_eq!(config.max_retained, 10);
    }

    #[test]
    fn should_checkpoint_at_intervals() {
        let mgr = CheckpointManager::new(CheckpointConfig {
            interval: 5,
            max_retained: 0,
        });

        assert!(mgr.should_checkpoint(0));
        assert!(!mgr.should_checkpoint(1));
        assert!(!mgr.should_checkpoint(4));
        assert!(mgr.should_checkpoint(5));
        assert!(mgr.should_checkpoint(10));
        assert!(mgr.should_checkpoint(100));
    }

    #[test]
    fn zero_interval_never_checkpoints() {
        let mgr = CheckpointManager::new(CheckpointConfig {
            interval: 0,
            max_retained: 0,
        });

        assert!(!mgr.should_checkpoint(0));
        assert!(!mgr.should_checkpoint(10));
    }

    #[test]
    fn creates_checkpoints_at_interval() {
        let chain = build_chain(25);
        let mut mgr = CheckpointManager::new(CheckpointConfig {
            interval: 10,
            max_retained: 0,
        });

        for turn in 0..25 {
            mgr.maybe_checkpoint(turn, &chain);
        }

        // Should have checkpoints at turns 0, 10, 20
        assert_eq!(mgr.len(), 3);
        assert_eq!(mgr.checkpoints()[0].turn, 0);
        assert_eq!(mgr.checkpoints()[1].turn, 10);
        assert_eq!(mgr.checkpoints()[2].turn, 20);
    }

    #[test]
    fn prunes_old_checkpoints() {
        let chain = build_chain(50);
        let mut mgr = CheckpointManager::new(CheckpointConfig {
            interval: 5,
            max_retained: 3,
        });

        for turn in 0..50 {
            mgr.maybe_checkpoint(turn, &chain);
        }

        // Should have at most 3 checkpoints (the most recent ones)
        assert_eq!(mgr.len(), 3);
        assert_eq!(mgr.checkpoints()[0].turn, 35);
        assert_eq!(mgr.checkpoints()[1].turn, 40);
        assert_eq!(mgr.checkpoints()[2].turn, 45);
    }

    #[test]
    fn latest_before() {
        let chain = build_chain(30);
        let mut mgr = CheckpointManager::new(CheckpointConfig {
            interval: 10,
            max_retained: 0,
        });

        for turn in 0..30 {
            mgr.maybe_checkpoint(turn, &chain);
        }

        // Checkpoints at 0, 10, 20
        assert_eq!(mgr.latest_before(5).unwrap().turn, 0);
        assert_eq!(mgr.latest_before(10).unwrap().turn, 10);
        assert_eq!(mgr.latest_before(15).unwrap().turn, 10);
        assert_eq!(mgr.latest_before(25).unwrap().turn, 20);
    }

    #[test]
    fn latest() {
        let chain = build_chain(15);
        let mut mgr = CheckpointManager::new(CheckpointConfig {
            interval: 5,
            max_retained: 0,
        });

        assert!(mgr.latest().is_none());

        for turn in 0..15 {
            mgr.maybe_checkpoint(turn, &chain);
        }

        assert_eq!(mgr.latest().unwrap().turn, 10);
    }

    #[test]
    fn checkpoint_preserves_hashes() {
        let chain = build_chain(11);
        let mut mgr = CheckpointManager::new(CheckpointConfig {
            interval: 10,
            max_retained: 0,
        });

        mgr.maybe_checkpoint(10, &chain);

        let cp = mgr.latest().unwrap();
        let entry = chain.get(10).unwrap();
        assert_eq!(cp.chain_hash, entry.chain_hash);
        assert_eq!(cp.state_hash, entry.state_hash);
    }

    #[test]
    fn serialization_roundtrip() {
        let chain = build_chain(30);
        let mut mgr = CheckpointManager::new(CheckpointConfig {
            interval: 10,
            max_retained: 5,
        });

        for turn in 0..30 {
            mgr.maybe_checkpoint(turn, &chain);
        }

        let json = serde_json::to_string(&mgr).unwrap();
        let restored: CheckpointManager = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), mgr.len());
        for (a, b) in restored.checkpoints().iter().zip(mgr.checkpoints().iter()) {
            assert_eq!(a.turn, b.turn);
            assert_eq!(a.chain_hash, b.chain_hash);
            assert_eq!(a.state_hash, b.state_hash);
        }
    }
}

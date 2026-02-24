//! Persistence for hash chain and checkpoint state.
//!
//! Provides save/load functionality for the verification state,
//! enabling recovery after restarts or disconnects.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::checkpoint::CheckpointManager;
use crate::commit::TurnCommitCollector;
use crate::hash_chain::TurnHashChain;

/// Complete verification state that can be persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSnapshot {
    /// The turn hash chain.
    pub chain: TurnHashChain,
    /// The checkpoint manager.
    pub checkpoints: CheckpointManager,
    /// The turn commit collector.
    pub commits: TurnCommitCollector,
}

impl VerificationSnapshot {
    /// Serialize the snapshot to JSON bytes.
    pub fn to_json(&self) -> Result<Vec<u8>, PersistError> {
        serde_json::to_vec(self).map_err(PersistError::Serialize)
    }

    /// Deserialize a snapshot from JSON bytes.
    pub fn from_json(data: &[u8]) -> Result<Self, PersistError> {
        serde_json::from_slice(data).map_err(PersistError::Deserialize)
    }

    /// Save the snapshot to a file.
    pub fn save_to_file(&self, path: &Path) -> Result<(), PersistError> {
        let json = self.to_json()?;

        // Write to a temp file first, then rename for atomicity
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, &json).map_err(PersistError::Io)?;
        std::fs::rename(&tmp_path, path).map_err(PersistError::Io)?;

        tracing::debug!(
            path = %path.display(),
            bytes = json.len(),
            "saved verification snapshot"
        );

        Ok(())
    }

    /// Load a snapshot from a file.
    pub fn load_from_file(path: &Path) -> Result<Self, PersistError> {
        let data = std::fs::read(path).map_err(PersistError::Io)?;
        let snapshot = Self::from_json(&data)?;

        tracing::debug!(
            path = %path.display(),
            chain_len = snapshot.chain.len(),
            checkpoints = snapshot.checkpoints.len(),
            "loaded verification snapshot"
        );

        Ok(snapshot)
    }
}

/// Errors from persistence operations.
#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("serialization failed: {0}")]
    Serialize(serde_json::Error),

    #[error("deserialization failed: {0}")]
    Deserialize(serde_json::Error),

    #[error("I/O error: {0}")]
    Io(io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::{CheckpointConfig, CheckpointManager};
    use crate::commit::TurnCommitCollector;
    use crate::hash_chain::TurnHashChain;
    use nostr::prelude::*;

    fn make_snapshot() -> VerificationSnapshot {
        let game_id = EventId::all_zeros();
        let keys = Keys::generate();

        let mut chain = TurnHashChain::new();
        for i in 0..5 {
            let mut state = [0u8; 32];
            state[0] = i as u8;
            chain.append(i, state).unwrap();
        }

        let mut checkpoints = CheckpointManager::new(CheckpointConfig {
            interval: 2,
            max_retained: 5,
        });
        for turn in 0..5 {
            checkpoints.maybe_checkpoint(turn, &chain);
        }

        let commits = TurnCommitCollector::new(game_id, [keys.public_key()]);

        VerificationSnapshot {
            chain,
            checkpoints,
            commits,
        }
    }

    #[test]
    fn json_roundtrip() {
        let snapshot = make_snapshot();
        let json = snapshot.to_json().unwrap();
        let restored = VerificationSnapshot::from_json(&json).unwrap();

        assert_eq!(restored.chain.len(), snapshot.chain.len());
        assert_eq!(restored.chain.head_hash(), snapshot.chain.head_hash());
        assert_eq!(restored.checkpoints.len(), snapshot.checkpoints.len());
    }

    #[test]
    fn file_roundtrip() {
        let snapshot = make_snapshot();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("verify.json");

        snapshot.save_to_file(&path).unwrap();
        let restored = VerificationSnapshot::load_from_file(&path).unwrap();

        assert_eq!(restored.chain.len(), snapshot.chain.len());
        assert_eq!(restored.chain.head_hash(), snapshot.chain.head_hash());
        assert!(restored.chain.validate().is_ok());
    }

    #[test]
    fn load_nonexistent_file_errors() {
        let result = VerificationSnapshot::load_from_file(Path::new("/nonexistent/file.json"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PersistError::Io(_)));
    }

    #[test]
    fn load_invalid_json_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, b"not json").unwrap();

        let result = VerificationSnapshot::load_from_file(&path);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PersistError::Deserialize(_)));
    }
}

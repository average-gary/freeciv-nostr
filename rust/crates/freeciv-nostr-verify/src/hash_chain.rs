//! Cryptographic hash chain linking turn state hashes.
//!
//! The `TurnHashChain` maintains a chain where each entry is:
//!   `H(n) = SHA-256(H(n-1) || turn_bytes || state_hash_bytes)`
//!
//! This produces a tamper-evident log: modifying any earlier state hash
//! invalidates all subsequent chain hashes.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A single entry in the turn hash chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainEntry {
    /// Turn number for this entry.
    pub turn: u64,
    /// The game state hash for this turn (SHA-256, 32 bytes).
    pub state_hash: [u8; 32],
    /// The chain hash: `SHA-256(prev_chain_hash || turn || state_hash)`.
    pub chain_hash: [u8; 32],
}

/// Maintains the cryptographic hash chain across turns.
///
/// Each turn produces a `ChainEntry` that links back to the previous turn
/// via the chain hash. The genesis entry uses a zero hash as its predecessor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnHashChain {
    /// All chain entries, ordered by turn.
    entries: Vec<ChainEntry>,
    /// The current chain head hash (last entry's chain_hash, or zeros for empty).
    head_hash: [u8; 32],
}

impl TurnHashChain {
    /// Create a new empty hash chain.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            head_hash: [0u8; 32],
        }
    }

    /// Append a new turn's state hash to the chain.
    ///
    /// Returns the new chain entry. The `turn` must be strictly sequential
    /// (next expected turn number).
    pub fn append(
        &mut self,
        turn: u64,
        state_hash: [u8; 32],
    ) -> Result<&ChainEntry, HashChainError> {
        let expected_turn = self.next_expected_turn();
        if turn != expected_turn {
            return Err(HashChainError::TurnMismatch {
                expected: expected_turn,
                got: turn,
            });
        }

        let chain_hash = compute_chain_hash(&self.head_hash, turn, &state_hash);

        let entry = ChainEntry {
            turn,
            state_hash,
            chain_hash,
        };

        self.head_hash = chain_hash;
        self.entries.push(entry);

        Ok(self.entries.last().unwrap())
    }

    /// Get the current head hash (latest chain hash).
    pub fn head_hash(&self) -> &[u8; 32] {
        &self.head_hash
    }

    /// Get the next expected turn number.
    pub fn next_expected_turn(&self) -> u64 {
        self.entries.last().map(|e| e.turn + 1).unwrap_or(0)
    }

    /// Get the total number of entries in the chain.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the chain has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get an entry by turn number.
    pub fn get(&self, turn: u64) -> Option<&ChainEntry> {
        self.entries.get(turn as usize)
    }

    /// Get all entries.
    pub fn entries(&self) -> &[ChainEntry] {
        &self.entries
    }

    /// Validate the entire chain from genesis to head.
    ///
    /// Returns `Ok(())` if every entry's chain_hash is correct given its
    /// predecessor. Returns an error at the first invalid entry.
    pub fn validate(&self) -> Result<(), HashChainError> {
        let mut prev_hash = [0u8; 32];

        for entry in &self.entries {
            let expected = compute_chain_hash(&prev_hash, entry.turn, &entry.state_hash);
            if expected != entry.chain_hash {
                return Err(HashChainError::InvalidChainHash {
                    turn: entry.turn,
                    expected: hex::encode(expected),
                    got: hex::encode(entry.chain_hash),
                });
            }
            prev_hash = entry.chain_hash;
        }

        Ok(())
    }

    /// Find the first turn where two chains diverge.
    ///
    /// Returns `None` if the chains are identical up to the shorter chain's length.
    pub fn find_divergence(&self, other: &TurnHashChain) -> Option<u64> {
        let min_len = self.entries.len().min(other.entries.len());
        for i in 0..min_len {
            if self.entries[i].state_hash != other.entries[i].state_hash {
                return Some(i as u64);
            }
        }
        None
    }
}

impl Default for TurnHashChain {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a chain hash: `SHA-256(prev_chain_hash || turn_be_bytes || state_hash)`.
fn compute_chain_hash(prev: &[u8; 32], turn: u64, state_hash: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(prev);
    hasher.update(turn.to_be_bytes());
    hasher.update(state_hash);
    hasher.finalize().into()
}

/// Errors from hash chain operations.
#[derive(Debug, thiserror::Error)]
pub enum HashChainError {
    #[error("turn mismatch: expected {expected}, got {got}")]
    TurnMismatch { expected: u64, got: u64 },

    #[error("invalid chain hash at turn {turn}: expected {expected}, got {got}")]
    InvalidChainHash {
        turn: u64,
        expected: String,
        got: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_state_hash(seed: u8) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = seed;
        h
    }

    #[test]
    fn new_chain_is_empty() {
        let chain = TurnHashChain::new();
        assert!(chain.is_empty());
        assert_eq!(chain.len(), 0);
        assert_eq!(chain.next_expected_turn(), 0);
        assert_eq!(chain.head_hash(), &[0u8; 32]);
    }

    #[test]
    fn append_first_entry() {
        let mut chain = TurnHashChain::new();
        let state = dummy_state_hash(0xAB);
        let entry = chain.append(0, state).unwrap();

        assert_eq!(entry.turn, 0);
        assert_eq!(entry.state_hash, state);
        assert_ne!(entry.chain_hash, [0u8; 32]); // chain hash should not be zeros
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.next_expected_turn(), 1);
    }

    #[test]
    fn append_sequential_entries() {
        let mut chain = TurnHashChain::new();
        for i in 0..10 {
            chain.append(i, dummy_state_hash(i as u8)).unwrap();
        }
        assert_eq!(chain.len(), 10);
        assert_eq!(chain.next_expected_turn(), 10);
    }

    #[test]
    fn append_rejects_wrong_turn() {
        let mut chain = TurnHashChain::new();
        chain.append(0, dummy_state_hash(1)).unwrap();

        let result = chain.append(5, dummy_state_hash(2));
        assert!(result.is_err());
        match result.unwrap_err() {
            HashChainError::TurnMismatch { expected, got } => {
                assert_eq!(expected, 1);
                assert_eq!(got, 5);
            }
            other => panic!("expected TurnMismatch, got: {:?}", other),
        }
    }

    #[test]
    fn validate_valid_chain() {
        let mut chain = TurnHashChain::new();
        for i in 0..20 {
            chain.append(i, dummy_state_hash(i as u8)).unwrap();
        }
        assert!(chain.validate().is_ok());
    }

    #[test]
    fn validate_detects_tampered_entry() {
        let mut chain = TurnHashChain::new();
        for i in 0..5 {
            chain.append(i, dummy_state_hash(i as u8)).unwrap();
        }

        // Tamper with turn 2's state hash
        chain.entries[2].state_hash[0] = 0xFF;

        let result = chain.validate();
        assert!(result.is_err());
        match result.unwrap_err() {
            HashChainError::InvalidChainHash { turn, .. } => {
                assert_eq!(turn, 2);
            }
            other => panic!("expected InvalidChainHash, got: {:?}", other),
        }
    }

    #[test]
    fn chain_hash_depends_on_previous() {
        // Two chains with different state at turn 0 should diverge at turn 0
        // and produce different chain hashes at turn 1 even with same state at turn 1.
        let mut chain_a = TurnHashChain::new();
        let mut chain_b = TurnHashChain::new();

        chain_a.append(0, dummy_state_hash(1)).unwrap();
        chain_b.append(0, dummy_state_hash(2)).unwrap();

        // Same state hash at turn 1
        let same_state = dummy_state_hash(99);
        chain_a.append(1, same_state).unwrap();
        chain_b.append(1, same_state).unwrap();

        // Chain hashes at turn 1 should differ because turn 0 differed
        assert_ne!(
            chain_a.get(1).unwrap().chain_hash,
            chain_b.get(1).unwrap().chain_hash,
        );
    }

    #[test]
    fn find_divergence_identical_chains() {
        let mut chain_a = TurnHashChain::new();
        let mut chain_b = TurnHashChain::new();

        for i in 0..5 {
            let state = dummy_state_hash(i as u8);
            chain_a.append(i, state).unwrap();
            chain_b.append(i, state).unwrap();
        }

        assert_eq!(chain_a.find_divergence(&chain_b), None);
    }

    #[test]
    fn find_divergence_at_specific_turn() {
        let mut chain_a = TurnHashChain::new();
        let mut chain_b = TurnHashChain::new();

        for i in 0..3 {
            let state = dummy_state_hash(i as u8);
            chain_a.append(i, state).unwrap();
            chain_b.append(i, state).unwrap();
        }

        // Diverge at turn 3
        chain_a.append(3, dummy_state_hash(0xAA)).unwrap();
        chain_b.append(3, dummy_state_hash(0xBB)).unwrap();

        assert_eq!(chain_a.find_divergence(&chain_b), Some(3));
    }

    #[test]
    fn get_by_turn() {
        let mut chain = TurnHashChain::new();
        let state = dummy_state_hash(42);
        chain.append(0, state).unwrap();

        let entry = chain.get(0).unwrap();
        assert_eq!(entry.turn, 0);
        assert_eq!(entry.state_hash, state);

        assert!(chain.get(1).is_none());
    }

    #[test]
    fn serialization_roundtrip() {
        let mut chain = TurnHashChain::new();
        for i in 0..5 {
            chain.append(i, dummy_state_hash(i as u8)).unwrap();
        }

        let json = serde_json::to_string(&chain).unwrap();
        let restored: TurnHashChain = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), chain.len());
        assert_eq!(restored.head_hash(), chain.head_hash());
        assert!(restored.validate().is_ok());

        for i in 0..5 {
            assert_eq!(
                restored.get(i).unwrap().chain_hash,
                chain.get(i).unwrap().chain_hash
            );
        }
    }

    #[test]
    fn large_chain_performance() {
        let mut chain = TurnHashChain::new();
        for i in 0..1000 {
            chain.append(i, dummy_state_hash((i % 256) as u8)).unwrap();
        }

        // Validate should complete quickly
        let start = std::time::Instant::now();
        assert!(chain.validate().is_ok());
        let elapsed = start.elapsed();

        // 1000 SHA-256 hashes should be well under 1 second
        assert!(
            elapsed.as_millis() < 1000,
            "validation of 1000-entry chain took {}ms",
            elapsed.as_millis()
        );
    }
}

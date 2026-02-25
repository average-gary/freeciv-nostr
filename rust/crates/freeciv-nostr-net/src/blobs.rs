//! Content-addressed blob storage for large game data.
//!
//! Uses `iroh-blobs` for content-addressed storage with BLAKE3 hashing.
//! Data is stored in an in-memory store and transferred between peers
//! using the game's existing QUIC streams ([`StreamId::StateSync`]).
//!
//! # Use Cases
//!
//! - **Ruleset transfer**: Host sends the ruleset blob to joining players
//!   at game start.
//! - **State snapshot**: For late joiners or desync recovery, the current
//!   game state is sent as a blob (Nostr event kind `STATE_SYNC` = 24200).
//! - **Savegame sharing**: Players can share save files via blob transfer.
//!
//! # Transfer Protocol
//!
//! Blob transfer uses a simple request/response protocol over QUIC streams:
//!
//! 1. Provider imports data → gets BLAKE3 hash.
//! 2. Provider announces hash to peers via gossip (as a `StateSync` message).
//! 3. Requester sends hash to provider over a QUIC stream.
//! 4. Provider reads from local store and streams the data back.
//! 5. Requester verifies BLAKE3 hash matches.
//!
//! This avoids the iroh version mismatch between `iroh-blobs 0.96` (which
//! uses `iroh 0.94` internally) and our `iroh 0.96` endpoint.

use bytes::Bytes;
use iroh_blobs::store::mem::MemStore;
use iroh_blobs::{BlobFormat, Hash};

use crate::error::NetError;

/// Maximum blob size we accept (50 MiB — enough for large rulesets).
pub const MAX_BLOB_SIZE: usize = 50 * 1024 * 1024;

/// Content-addressed blob store for game data.
///
/// Wraps an in-memory iroh-blobs [`MemStore`] and provides high-level
/// methods for importing, reading, and verifying game data blobs.
///
/// Actual network transfer is done via the game's QUIC streams — this
/// struct handles only the storage and verification side.
///
/// # Example
///
/// ```ignore
/// let blobs = GameBlobs::new();
///
/// // Provider side: import data
/// let hash = blobs.import_bytes(ruleset_data).await?;
/// let hash_hex = blobs.hash_to_hex(hash);
/// // Send hash_hex to peers via gossip...
///
/// // Requester side: after receiving blob data over QUIC
/// let verified_hash = blobs.import_bytes(received_data).await?;
/// assert_eq!(verified_hash, expected_hash);
/// ```
pub struct GameBlobs {
    /// The in-memory blob store.
    store: MemStore,
}

impl GameBlobs {
    /// Create a new in-memory blob store.
    pub fn new() -> Self {
        let store = MemStore::new();
        Self { store }
    }

    /// Import raw bytes into the blob store.
    ///
    /// Returns the BLAKE3 [`Hash`] of the imported data. The hash is
    /// deterministic — the same data always produces the same hash.
    pub async fn import_bytes(&self, data: impl Into<Bytes>) -> Result<Hash, NetError> {
        let tag = self
            .store
            .add_bytes(data.into())
            .await
            .map_err(|e| NetError::Blob(e.to_string()))?;
        Ok(tag.hash)
    }

    /// Check if a blob with the given hash exists locally.
    pub async fn has(&self, hash: Hash) -> Result<bool, NetError> {
        self.store
            .blobs()
            .has(hash)
            .await
            .map_err(|e| NetError::Blob(e.to_string()))
    }

    /// Read a locally stored blob's contents.
    ///
    /// Returns `None` if the blob is not in the store.
    pub async fn get_bytes(&self, hash: Hash) -> Result<Option<Bytes>, NetError> {
        if !self.has(hash).await? {
            return Ok(None);
        }
        let data = self
            .store
            .get_bytes(hash)
            .await
            .map_err(|e| NetError::Blob(e.to_string()))?;
        Ok(Some(data))
    }

    /// Import bytes and return both the hash and the data for transfer.
    ///
    /// Use the hash as an identifier when announcing the blob via gossip,
    /// and the data for streaming to peers over QUIC.
    pub async fn import_and_hash(&self, data: impl Into<Bytes>) -> Result<(Hash, Bytes), NetError> {
        let bytes: Bytes = data.into();
        let hash = self.import_bytes(bytes.clone()).await?;
        Ok((hash, bytes))
    }

    /// Verify that received data matches an expected hash.
    ///
    /// Computes the BLAKE3 hash of `data` and compares it to `expected`.
    /// Also imports the data into the local store if valid.
    ///
    /// Returns `Ok(hash)` if the data matches, or an error if it doesn't.
    pub async fn verify_and_import(
        &self,
        data: impl Into<Bytes>,
        expected: Hash,
    ) -> Result<Hash, NetError> {
        let bytes: Bytes = data.into();

        if bytes.len() > MAX_BLOB_SIZE {
            return Err(NetError::Blob(format!(
                "blob too large: {} bytes (max {})",
                bytes.len(),
                MAX_BLOB_SIZE
            )));
        }

        let computed = Hash::new(&bytes);
        if computed != expected {
            return Err(NetError::Blob(format!(
                "hash mismatch: expected {}, got {}",
                expected.to_hex(),
                computed.to_hex()
            )));
        }

        self.import_bytes(bytes).await?;
        Ok(computed)
    }

    /// Get the BLAKE3 hash of some data without importing it.
    ///
    /// Useful for checking if a blob already exists locally before importing.
    pub fn hash_bytes(data: &[u8]) -> Hash {
        Hash::new(data)
    }

    /// Convert a hash to its hex string representation.
    pub fn hash_to_hex(hash: Hash) -> String {
        hash.to_hex().to_string()
    }

    /// Parse a hex string into a [`Hash`].
    ///
    /// Expects exactly 64 hex characters (32 bytes). Returns an error
    /// if the input is invalid.
    pub fn hash_from_hex(hex: &str) -> Result<Hash, NetError> {
        // Validate length first — iroh-blobs' Hash::from_str can panic
        // on invalid input (upstream bug in data-encoding).
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(NetError::Blob(format!(
                "invalid hash hex: expected 64 hex chars, got {} chars",
                hex.len()
            )));
        }
        hex.parse()
            .map_err(|e| NetError::Blob(format!("invalid hash hex: {e}")))
    }

    /// Get the blob format (always [`BlobFormat::Raw`] for game data).
    pub fn blob_format() -> BlobFormat {
        BlobFormat::Raw
    }

    /// Get the number of blobs in the store.
    ///
    /// Useful for debugging and diagnostics.
    pub async fn count(&self) -> usize {
        // We track the count by importing — each unique hash is one blob.
        // For now, rely on the has() check for individual lookups.
        // A full listing API would require n0_future::Stream support.
        0 // TODO: implement once iroh-blobs stream API stabilizes
    }
}

impl Default for GameBlobs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn import_and_read_bytes() {
        let blobs = GameBlobs::new();
        let data = b"hello freeciv-nostr world";
        let hash = blobs.import_bytes(&data[..]).await.unwrap();

        // Hash should be deterministic (BLAKE3)
        let expected = Hash::new(data);
        assert_eq!(hash, expected);

        // Read back
        let got = blobs.get_bytes(hash).await.unwrap().unwrap();
        assert_eq!(&got[..], data);
    }

    #[tokio::test]
    async fn has_returns_false_for_missing() {
        let blobs = GameBlobs::new();
        let fake_hash = Hash::new(b"does not exist");
        assert!(!blobs.has(fake_hash).await.unwrap());
    }

    #[tokio::test]
    async fn has_returns_true_after_import() {
        let blobs = GameBlobs::new();
        let hash = blobs.import_bytes(b"test data".as_ref()).await.unwrap();
        assert!(blobs.has(hash).await.unwrap());
    }

    #[tokio::test]
    async fn get_bytes_returns_none_for_missing() {
        let blobs = GameBlobs::new();
        let fake_hash = Hash::new(b"nope");
        assert!(blobs.get_bytes(fake_hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn hash_bytes_matches_import() {
        let blobs = GameBlobs::new();
        let data = b"deterministic hash check";
        let precomputed = GameBlobs::hash_bytes(data);
        let imported = blobs.import_bytes(&data[..]).await.unwrap();
        assert_eq!(precomputed, imported);
    }

    #[tokio::test]
    async fn default_creates_valid_instance() {
        let blobs = GameBlobs::default();
        let hash = blobs.import_bytes(b"default test".as_ref()).await.unwrap();
        assert!(blobs.has(hash).await.unwrap());
    }

    #[test]
    fn max_blob_size_is_50_mib() {
        assert_eq!(MAX_BLOB_SIZE, 50 * 1024 * 1024);
    }

    #[tokio::test]
    async fn import_empty_bytes() {
        let blobs = GameBlobs::new();
        let hash = blobs.import_bytes(b"".as_ref()).await.unwrap();
        let got = blobs.get_bytes(hash).await.unwrap().unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn import_large_payload() {
        let blobs = GameBlobs::new();
        // 1 MiB of data
        let data = vec![0xABu8; 1024 * 1024];
        let hash = blobs.import_bytes(data.clone()).await.unwrap();
        let got = blobs.get_bytes(hash).await.unwrap().unwrap();
        assert_eq!(got.len(), 1024 * 1024);
        assert_eq!(&got[..], &data[..]);
    }

    #[tokio::test]
    async fn import_and_hash_returns_both() {
        let blobs = GameBlobs::new();
        let data = b"import and hash test";
        let (hash, bytes) = blobs.import_and_hash(&data[..]).await.unwrap();
        assert_eq!(&bytes[..], data);
        assert_eq!(hash, Hash::new(data));
    }

    #[tokio::test]
    async fn verify_and_import_valid() {
        let blobs = GameBlobs::new();
        let data = b"verify me";
        let expected = Hash::new(data);
        let result = blobs.verify_and_import(&data[..], expected).await.unwrap();
        assert_eq!(result, expected);
        assert!(blobs.has(expected).await.unwrap());
    }

    #[tokio::test]
    async fn verify_and_import_mismatch() {
        let blobs = GameBlobs::new();
        let data = b"actual data";
        let wrong_hash = Hash::new(b"different data");
        let result = blobs.verify_and_import(&data[..], wrong_hash).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("hash mismatch"), "error: {err}");
    }

    #[tokio::test]
    async fn verify_and_import_too_large() {
        let blobs = GameBlobs::new();
        let data = vec![0u8; MAX_BLOB_SIZE + 1];
        let hash = Hash::new(&data);
        let result = blobs.verify_and_import(data, hash).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("too large"), "error: {err}");
    }

    #[test]
    fn hash_hex_roundtrip() {
        let hash = Hash::new(b"roundtrip test");
        let hex = GameBlobs::hash_to_hex(hash);
        let parsed = GameBlobs::hash_from_hex(&hex).unwrap();
        assert_eq!(hash, parsed);
    }

    #[test]
    fn hash_from_invalid_hex() {
        // Empty string should fail to parse as a hash
        let result = GameBlobs::hash_from_hex("");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn multiple_imports_all_retrievable() {
        let blobs = GameBlobs::new();
        let h1 = blobs.import_bytes(b"data one".as_ref()).await.unwrap();
        let h2 = blobs.import_bytes(b"data two".as_ref()).await.unwrap();
        assert_ne!(h1, h2);
        assert!(blobs.has(h1).await.unwrap());
        assert!(blobs.has(h2).await.unwrap());
    }

    #[test]
    fn blob_format_is_raw() {
        assert_eq!(GameBlobs::blob_format(), BlobFormat::Raw);
    }
}

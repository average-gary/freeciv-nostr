//! Key management for freeciv-nostr.
//!
//! Provides secp256k1 key generation, NIP-19 encoding/decoding (nsec/npub),
//! and key storage utilities.

use nostr::prelude::*;
use thiserror::Error;

/// Errors that can occur during key operations.
#[derive(Debug, Error)]
pub enum KeyError {
    /// Failed to parse a NIP-19 encoded key.
    #[error("invalid NIP-19 key: {0}")]
    InvalidNip19(String),

    /// Failed to parse a hex-encoded key.
    #[error("invalid hex key: {0}")]
    InvalidHex(String),

    /// Key generation failed.
    #[error("key generation failed: {0}")]
    GenerationFailed(String),

    /// Underlying Nostr library error.
    #[error("nostr error: {0}")]
    Nostr(#[from] nostr::key::Error),
}

/// A player's Nostr identity, wrapping a secp256k1 keypair.
///
/// This is the primary identity type used throughout freeciv-nostr.
/// Each player has one long-lived identity (for reputation/profile)
/// and may also use per-game ephemeral keys.
#[derive(Debug, Clone)]
pub struct PlayerIdentity {
    keys: Keys,
}

impl PlayerIdentity {
    /// Generate a new random identity (secp256k1 keypair).
    pub fn generate() -> Self {
        Self {
            keys: Keys::generate(),
        }
    }

    /// Create an identity from an existing secret key.
    pub fn from_secret_key(secret_key: SecretKey) -> Self {
        Self {
            keys: Keys::new(secret_key),
        }
    }

    /// Parse an identity from a NIP-19 `nsec` string.
    pub fn from_nsec(nsec: &str) -> Result<Self, KeyError> {
        let secret_key =
            SecretKey::from_bech32(nsec).map_err(|e| KeyError::InvalidNip19(e.to_string()))?;
        Ok(Self::from_secret_key(secret_key))
    }

    /// Parse an identity from a hex-encoded secret key.
    pub fn from_hex(hex: &str) -> Result<Self, KeyError> {
        let secret_key =
            SecretKey::from_hex(hex).map_err(|e| KeyError::InvalidHex(e.to_string()))?;
        Ok(Self::from_secret_key(secret_key))
    }

    /// Get the public key for this identity.
    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    /// Get the secret key for this identity.
    pub fn secret_key(&self) -> &SecretKey {
        self.keys.secret_key()
    }

    /// Get the underlying `Keys` for use with nostr crate functions.
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// Export the public key as a NIP-19 `npub` string.
    pub fn to_npub(&self) -> String {
        self.keys.public_key().to_bech32().expect("npub encoding")
    }

    /// Export the secret key as a NIP-19 `nsec` string.
    pub fn to_nsec(&self) -> String {
        self.keys.secret_key().to_bech32().expect("nsec encoding")
    }

    /// Export the public key as a hex string.
    pub fn to_public_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }

    /// Export the secret key as a hex string.
    pub fn to_secret_hex(&self) -> String {
        self.keys.secret_key().to_secret_hex()
    }
}

/// Parse a public key from either npub (bech32) or hex format.
pub fn parse_public_key(input: &str) -> Result<PublicKey, KeyError> {
    if input.starts_with("npub") {
        PublicKey::from_bech32(input).map_err(|e| KeyError::InvalidNip19(e.to_string()))
    } else {
        PublicKey::from_hex(input).map_err(|e| KeyError::InvalidHex(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_identity() {
        let id = PlayerIdentity::generate();
        // Public key should be 32 bytes (64 hex chars)
        assert_eq!(id.to_public_hex().len(), 64);
        // Secret key should be 32 bytes (64 hex chars)
        assert_eq!(id.to_secret_hex().len(), 64);
    }

    #[test]
    fn nsec_roundtrip() {
        let id = PlayerIdentity::generate();
        let nsec = id.to_nsec();
        assert!(nsec.starts_with("nsec1"));

        let restored = PlayerIdentity::from_nsec(&nsec).expect("parse nsec");
        assert_eq!(id.to_public_hex(), restored.to_public_hex());
        assert_eq!(id.to_secret_hex(), restored.to_secret_hex());
    }

    #[test]
    fn npub_encoding() {
        let id = PlayerIdentity::generate();
        let npub = id.to_npub();
        assert!(npub.starts_with("npub1"));

        let parsed = parse_public_key(&npub).expect("parse npub");
        assert_eq!(id.public_key(), parsed);
    }

    #[test]
    fn hex_roundtrip() {
        let id = PlayerIdentity::generate();
        let hex = id.to_secret_hex();

        let restored = PlayerIdentity::from_hex(&hex).expect("parse hex");
        assert_eq!(id.to_public_hex(), restored.to_public_hex());
    }

    #[test]
    fn parse_public_key_hex() {
        let id = PlayerIdentity::generate();
        let hex = id.to_public_hex();
        let parsed = parse_public_key(&hex).expect("parse hex pubkey");
        assert_eq!(id.public_key(), parsed);
    }

    #[test]
    fn parse_public_key_npub() {
        let id = PlayerIdentity::generate();
        let npub = id.to_npub();
        let parsed = parse_public_key(&npub).expect("parse npub");
        assert_eq!(id.public_key(), parsed);
    }

    #[test]
    fn invalid_nsec_returns_error() {
        assert!(PlayerIdentity::from_nsec("not_a_valid_nsec").is_err());
    }

    #[test]
    fn invalid_hex_returns_error() {
        assert!(PlayerIdentity::from_hex("not_hex").is_err());
    }

    #[test]
    fn two_generated_identities_are_different() {
        let a = PlayerIdentity::generate();
        let b = PlayerIdentity::generate();
        assert_ne!(a.to_public_hex(), b.to_public_hex());
    }
}

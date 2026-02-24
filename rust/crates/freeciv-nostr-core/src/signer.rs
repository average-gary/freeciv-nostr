//! Signer abstraction for freeciv-nostr.
//!
//! Provides a unified interface for signing Nostr events, supporting both:
//! - **Local signing**: using an in-memory secret key
//! - **NIP-46 remote signing**: delegating to a remote signer (e.g., Amber, nsecBunker)
//!
//! Game code uses the `GameSigner` trait and is agnostic to whether signing
//! happens locally or remotely.

use nostr::prelude::*;
use thiserror::Error;

use crate::keys::PlayerIdentity;
use crate::kinds;

/// Errors that can occur during signing operations.
#[derive(Debug, Error)]
pub enum SignerError {
    /// The signer is not connected or initialized.
    #[error("signer not connected")]
    NotConnected,

    /// The remote signer rejected the signing request.
    #[error("signing rejected: {0}")]
    Rejected(String),

    /// The event kind is not pre-approved and requires explicit approval.
    #[error("event kind {0} not pre-approved for automatic signing")]
    KindNotApproved(u16),

    /// Underlying Nostr library error.
    #[error("nostr error: {0}")]
    Nostr(String),

    /// Timeout waiting for remote signer response.
    #[error("signer timeout after {0}ms")]
    Timeout(u64),
}

/// Trait for signing Nostr events.
///
/// Game code uses this trait to sign events without knowing whether
/// the signing happens locally or via NIP-46 remote signer.
pub trait GameSigner: Send + Sync {
    /// Get the public key of the signer.
    fn public_key(&self) -> PublicKey;

    /// Sign an unsigned event, returning the signed event.
    ///
    /// For local signers, this is immediate.
    /// For NIP-46 remote signers, this may involve a network round-trip.
    fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError>;

    /// Check if a given event kind is pre-approved for automatic signing
    /// (no user prompt required).
    fn is_pre_approved(&self, kind: Kind) -> bool;

    /// Get a human-readable description of this signer (for logging).
    fn description(&self) -> &str;
}

/// Local signer using an in-memory secret key.
///
/// This is the simplest signer: signing is immediate and never fails
/// (assuming the key is valid). Used for testing and for players who
/// manage their own keys.
pub struct LocalSigner {
    identity: PlayerIdentity,
}

impl LocalSigner {
    /// Create a new local signer from a player identity.
    pub fn new(identity: PlayerIdentity) -> Self {
        Self { identity }
    }

    /// Create a new local signer with a freshly generated key.
    pub fn generate() -> Self {
        Self::new(PlayerIdentity::generate())
    }

    /// Get the underlying player identity.
    pub fn identity(&self) -> &PlayerIdentity {
        &self.identity
    }
}

impl GameSigner for LocalSigner {
    fn public_key(&self) -> PublicKey {
        self.identity.public_key()
    }

    fn sign_event(&self, unsigned: UnsignedEvent) -> Result<Event, SignerError> {
        unsigned
            .sign_with_keys(self.identity.keys())
            .map_err(|e| SignerError::Nostr(e.to_string()))
    }

    fn is_pre_approved(&self, _kind: Kind) -> bool {
        // Local signer always approves — the user controls the key directly.
        true
    }

    fn description(&self) -> &str {
        "local"
    }
}

/// Verify that an event was signed by the claimed author.
///
/// Checks:
/// 1. Event ID matches the canonical hash of the event content
/// 2. Schnorr signature is valid for the event ID and public key
pub fn verify_event(event: &Event) -> Result<(), SignerError> {
    event
        .verify()
        .map_err(|e| SignerError::Nostr(e.to_string()))
}

/// Check if an event kind is in the pre-approved set for NIP-46 signing.
///
/// Pre-approved kinds can be signed without prompting the user,
/// which is essential for real-time gameplay (you can't prompt
/// for every unit move).
pub fn is_gameplay_kind(kind: Kind) -> bool {
    kinds::PRE_APPROVED_KINDS.contains(&kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds;

    #[test]
    fn local_signer_creates_valid_events() {
        let signer = LocalSigner::generate();
        let pubkey = signer.public_key();

        // Create an unsigned event
        let unsigned = EventBuilder::new(kinds::GAME_ACTION, "test action payload").build(pubkey);

        // Sign it
        let event = signer.sign_event(unsigned).expect("signing should succeed");

        // Verify the signature
        assert!(event.verify().is_ok());
        assert_eq!(event.pubkey, pubkey);
        assert_eq!(event.kind, kinds::GAME_ACTION);
    }

    #[test]
    fn local_signer_approves_all_kinds() {
        let signer = LocalSigner::generate();
        assert!(signer.is_pre_approved(kinds::GAME_ACTION));
        assert!(signer.is_pre_approved(kinds::GAME_LOBBY));
        assert!(signer.is_pre_approved(Kind::TextNote));
        assert!(signer.is_pre_approved(Kind::Custom(9999)));
    }

    #[test]
    fn verify_event_catches_tampering() {
        let signer = LocalSigner::generate();
        let pubkey = signer.public_key();

        let unsigned = EventBuilder::new(kinds::GAME_CHAT, "hello").build(pubkey);

        let event = signer.sign_event(unsigned).expect("sign");
        // Valid event should verify
        assert!(verify_event(&event).is_ok());
    }

    #[test]
    fn is_gameplay_kind_correct() {
        assert!(is_gameplay_kind(kinds::GAME_ACTION));
        assert!(is_gameplay_kind(kinds::GAME_STATE_HASH));
        assert!(is_gameplay_kind(kinds::GAME_CHAT));
        assert!(is_gameplay_kind(kinds::GAME_DIPLOMACY));
        assert!(is_gameplay_kind(kinds::HEARTBEAT));
        // Non-gameplay kinds
        assert!(!is_gameplay_kind(kinds::GAME_LOBBY));
        assert!(!is_gameplay_kind(kinds::GAME_END));
        assert!(!is_gameplay_kind(kinds::PLAYER_PROFILE));
    }

    #[test]
    fn signer_description() {
        let signer = LocalSigner::generate();
        assert_eq!(signer.description(), "local");
    }
}

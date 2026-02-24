//! Gossip-based message broadcasting for game sessions.
//!
//! Uses `iroh-gossip` for efficient broadcast-tree message propagation.
//! Each game gets a unique [`TopicId`] derived from its root Nostr event ID.

use iroh_gossip::net::Gossip;
use iroh_gossip::proto::TopicId;
use sha2::{Digest, Sha256};

use crate::error::NetError;

/// Derive a [`TopicId`] from a game's root Nostr event ID (hex string).
///
/// The topic is computed as `SHA-256("freeciv-nostr-game:" || event_id_hex)`.
/// This ensures all players in the same game subscribe to the same topic,
/// and different games get different topics.
pub fn game_topic(game_event_id_hex: &str) -> TopicId {
    let mut hasher = Sha256::new();
    hasher.update(b"freeciv-nostr-game:");
    hasher.update(game_event_id_hex.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    hash.into()
}

/// Wraps `iroh-gossip` for game message broadcasting.
///
/// Provides a high-level interface for broadcasting game messages
/// (actions, state sync, chat) to all peers in a game session.
pub struct GameGossip {
    /// The topic ID for this game session.
    topic: TopicId,
    /// The underlying gossip handle.
    gossip: Gossip,
}

impl GameGossip {
    /// Create a new gossip instance for a game session.
    ///
    /// The `endpoint` is used by the gossip protocol to establish connections.
    /// The `game_event_id_hex` is the hex-encoded Nostr event ID of the game's
    /// root event, used to derive the topic.
    pub fn new(endpoint: iroh::Endpoint, game_event_id_hex: &str) -> Self {
        let topic = game_topic(game_event_id_hex);
        let gossip = Gossip::builder().spawn(endpoint);
        Self { topic, gossip }
    }

    /// Get the [`TopicId`] for this game session.
    pub fn topic(&self) -> TopicId {
        self.topic
    }

    /// Get a reference to the underlying [`Gossip`] handle.
    pub fn gossip(&self) -> &Gossip {
        &self.gossip
    }

    // TODO: Add high-level subscribe/broadcast methods once the full
    // game message protocol is defined. The iroh-gossip API provides:
    // - gossip.subscribe(topic, peers) for joining a topic
    // - sender.broadcast(bytes) for sending messages
    // - receiver stream for receiving messages

    /// Shut down the gossip actor.
    pub async fn shutdown(self) -> Result<(), NetError> {
        self.gossip
            .shutdown()
            .await
            .map_err(|e| NetError::Gossip(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_topic_deterministic() {
        let event_id = "abc123def456";
        let topic1 = game_topic(event_id);
        let topic2 = game_topic(event_id);
        assert_eq!(topic1, topic2, "Same input must produce the same topic");
    }

    #[test]
    fn game_topic_different_inputs() {
        let topic_a = game_topic("event_aaa");
        let topic_b = game_topic("event_bbb");
        assert_ne!(
            topic_a, topic_b,
            "Different event IDs must produce different topics"
        );
    }

    #[test]
    fn game_topic_is_32_bytes() {
        let topic = game_topic("some_event_id");
        // TopicId is [u8; 32] under the hood — verify via as_bytes()
        assert_eq!(topic.as_bytes().len(), 32);
    }

    #[test]
    fn game_topic_known_value() {
        // Compute expected value manually:
        // SHA-256("freeciv-nostr-game:" + "deadbeef")
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"freeciv-nostr-game:");
        hasher.update(b"deadbeef");
        let expected: [u8; 32] = hasher.finalize().into();

        let topic = game_topic("deadbeef");
        assert_eq!(topic.as_bytes(), &expected);
    }
}

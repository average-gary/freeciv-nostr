//! Gossip-based message broadcasting for game sessions.
//!
//! Uses `iroh-gossip` for efficient broadcast-tree message propagation.
//! Each game gets a unique [`TopicId`] derived from its root Nostr event ID.
//!
//! # Architecture
//!
//! The gossip layer provides two main capabilities:
//! - **Broadcasting**: Send a framed message to all peers in a game session.
//! - **Receiving**: Consume incoming messages as a [`Stream`] of [`GossipEvent`]s.
//!
//! Messages are framed using the [`FramedMessage`] format (see [`crate::message`]),
//! allowing multiplexing of game actions, state sync, chat, and heartbeats over
//! a single gossip topic.

use bytes::Bytes;
use iroh::EndpointId;
use iroh_gossip::net::Gossip;
use iroh_gossip::proto::TopicId;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::error::NetError;
use crate::message::{FramedMessage, decode_message, encode_message};
use crate::protocol::StreamId;

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

/// An event received from the gossip layer.
#[derive(Debug, Clone)]
pub enum GossipEvent {
    /// A framed game message was received from a peer.
    Received {
        /// The decoded message.
        message: FramedMessage,
        /// The peer that delivered this message (may not be the original author).
        delivered_from: EndpointId,
    },
    /// A new peer joined the gossip topic.
    NeighborUp(EndpointId),
    /// A peer left the gossip topic.
    NeighborDown(EndpointId),
    /// The receiver lagged behind and missed messages.
    Lagged,
}

/// A handle for sending messages to a gossip topic.
///
/// Obtained from [`GameGossip::subscribe`]. Clone-safe — multiple senders
/// can broadcast concurrently.
#[derive(Clone)]
pub struct GameGossipSender {
    inner: iroh_gossip::api::GossipSender,
}

impl GameGossipSender {
    /// Broadcast a framed message to all peers on the topic.
    ///
    /// The message is encoded with length-prefix framing before broadcast.
    pub async fn broadcast(&self, msg: &FramedMessage) -> Result<(), NetError> {
        let encoded = encode_message(msg)?;
        self.inner
            .broadcast(Bytes::from(encoded))
            .await
            .map_err(|e: iroh_gossip::api::ApiError| NetError::Gossip(e.to_string()))
    }

    /// Broadcast raw bytes with a specific stream ID.
    ///
    /// Convenience wrapper that creates a [`FramedMessage`] and broadcasts it.
    pub async fn broadcast_raw(
        &self,
        stream_id: StreamId,
        payload: Vec<u8>,
    ) -> Result<(), NetError> {
        let msg = FramedMessage { stream_id, payload };
        self.broadcast(&msg).await
    }

    /// Notify the gossip layer to connect to additional peers.
    pub async fn join_peers(&self, peers: Vec<EndpointId>) -> Result<(), NetError> {
        self.inner
            .join_peers(peers)
            .await
            .map_err(|e: iroh_gossip::api::ApiError| NetError::Gossip(e.to_string()))
    }
}

/// A handle for receiving messages from a gossip topic.
///
/// Obtained from [`GameGossip::subscribe`]. Wraps the raw iroh-gossip
/// event stream and decodes framed messages.
pub struct GameGossipReceiver {
    rx: mpsc::Receiver<GossipEvent>,
}

impl GameGossipReceiver {
    /// Receive the next gossip event, blocking until one is available.
    ///
    /// Returns `None` when the gossip topic has been shut down.
    pub async fn recv(&mut self) -> Option<GossipEvent> {
        self.rx.recv().await
    }

    /// Try to receive a gossip event without blocking.
    ///
    /// Returns `Ok(event)` if one is available, `Err(TryRecvError)` otherwise.
    pub fn try_recv(&mut self) -> Result<GossipEvent, mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }
}

/// Wraps `iroh-gossip` for game message broadcasting.
///
/// Provides a high-level interface for broadcasting game messages
/// (actions, state sync, chat) to all peers in a game session.
///
/// # Lifecycle
///
/// 1. Create with [`GameGossip::new`] — spawns the gossip actor.
/// 2. Subscribe to the topic with [`GameGossip::subscribe`] — returns
///    a `(GameGossipSender, GameGossipReceiver)` pair.
/// 3. Use the sender to broadcast messages.
/// 4. Read from the receiver to get incoming messages.
/// 5. Shut down with [`GameGossip::shutdown`] when the game ends.
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
    ///
    /// Useful for advanced operations like handling incoming connections
    /// via `gossip.handle_connection()`.
    pub fn gossip(&self) -> &Gossip {
        &self.gossip
    }

    /// Subscribe to the game topic and join the gossip swarm.
    ///
    /// `bootstrap_peers` is the list of known peers to initially connect to.
    /// At least one peer should be provided for the first joiner; subsequent
    /// joiners can bootstrap from any existing participant.
    ///
    /// Returns a `(GameGossipSender, GameGossipReceiver)` pair. The sender
    /// can be cloned and shared across tasks. The receiver yields
    /// [`GossipEvent`]s — decoded framed messages plus neighbor notifications.
    ///
    /// The subscription remains active until both the sender and receiver are
    /// dropped, or [`GameGossip::shutdown`] is called.
    pub async fn subscribe(
        &self,
        bootstrap_peers: Vec<EndpointId>,
    ) -> Result<(GameGossipSender, GameGossipReceiver), NetError> {
        use futures_lite::StreamExt;

        let topic_handle = self
            .gossip
            .subscribe_and_join(self.topic, bootstrap_peers)
            .await
            .map_err(|e| NetError::Gossip(e.to_string()))?;

        let (iroh_sender, mut iroh_receiver) = topic_handle.split();

        // Channel buffer: enough to absorb bursts without blocking the gossip actor.
        let (tx, rx) = mpsc::channel::<GossipEvent>(256);

        // Spawn a background task to decode incoming gossip events and forward
        // them to our typed channel.
        tokio::spawn(async move {
            while let Some(event_result) = iroh_receiver.next().await {
                let gossip_event = match event_result {
                    Ok(iroh_gossip::api::Event::Received(msg)) => {
                        match decode_message(&msg.content) {
                            Ok((framed, _consumed)) => GossipEvent::Received {
                                message: framed,
                                delivered_from: msg.delivered_from,
                            },
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    from = %msg.delivered_from,
                                    len = msg.content.len(),
                                    "failed to decode gossip message, skipping"
                                );
                                continue;
                            }
                        }
                    }
                    Ok(iroh_gossip::api::Event::NeighborUp(id)) => {
                        tracing::debug!(peer = %id, "gossip neighbor up");
                        GossipEvent::NeighborUp(id)
                    }
                    Ok(iroh_gossip::api::Event::NeighborDown(id)) => {
                        tracing::debug!(peer = %id, "gossip neighbor down");
                        GossipEvent::NeighborDown(id)
                    }
                    Ok(iroh_gossip::api::Event::Lagged) => {
                        tracing::warn!("gossip receiver lagged, messages were dropped");
                        GossipEvent::Lagged
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "gossip stream error, closing receiver");
                        break;
                    }
                };

                if tx.send(gossip_event).await.is_err() {
                    // Receiver dropped — stop processing.
                    break;
                }
            }
        });

        Ok((
            GameGossipSender { inner: iroh_sender },
            GameGossipReceiver { rx },
        ))
    }

    /// Shut down the gossip actor.
    ///
    /// Sends disconnect messages to all peers and stops the background actor.
    /// Any active subscriptions will stop receiving events.
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

    #[test]
    fn gossip_sender_is_clone() {
        // Verify GameGossipSender is Clone (compile-time check)
        fn assert_clone<T: Clone>() {}
        assert_clone::<GameGossipSender>();
    }

    #[test]
    fn gossip_event_variants() {
        // Use a valid Ed25519 public key (generator point)
        let key = iroh::SecretKey::generate(&mut rand::rng()).public();

        // Verify all GossipEvent variants exist and can be constructed
        let _received = GossipEvent::Received {
            message: FramedMessage {
                stream_id: StreamId::GameActions,
                payload: vec![1, 2, 3],
            },
            delivered_from: key,
        };
        let _up = GossipEvent::NeighborUp(key);
        let _down = GossipEvent::NeighborDown(key);
        let _lagged = GossipEvent::Lagged;
    }
}

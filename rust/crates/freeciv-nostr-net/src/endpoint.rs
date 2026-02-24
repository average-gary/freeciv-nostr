//! Per-game ephemeral Iroh endpoint management.
//!
//! Each game session gets a fresh Ed25519 keypair via an ephemeral [`GameEndpoint`].
//! The keypair is never reused across sessions.

use std::collections::HashMap;
use std::sync::Arc;

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use iroh::{EndpointAddr, EndpointId, Endpoint, SecretKey};
use tokio::sync::Mutex;

use crate::error::NetError;
use crate::protocol::ALPN;

/// A per-game ephemeral P2P endpoint.
///
/// Each game session gets a fresh Ed25519 keypair (never reused).
/// The endpoint owns the underlying QUIC transport and manages
/// peer connections.
pub struct GameEndpoint {
    /// The Iroh endpoint (owns the QUIC transport).
    endpoint: Endpoint,
    /// Our ephemeral EndpointId for this game.
    endpoint_id: EndpointId,
    /// Connected peers (EndpointId -> Connection).
    peers: Arc<Mutex<HashMap<EndpointId, Connection>>>,
}

impl GameEndpoint {
    /// Create a new ephemeral endpoint for a game session.
    ///
    /// Generates a fresh Ed25519 keypair so each game session has a
    /// unique identity.
    pub async fn new() -> Result<Self, NetError> {
        let secret_key = SecretKey::generate(&mut rand::rng());
        let endpoint = Endpoint::builder()
            .alpns(vec![ALPN.to_vec()])
            .secret_key(secret_key)
            .bind()
            .await
            .map_err(|e| NetError::EndpointCreate(e.to_string()))?;

        let endpoint_id = endpoint.id();

        Ok(Self {
            endpoint,
            endpoint_id,
            peers: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Get this endpoint's [`EndpointId`] (for publishing in Game Accept events).
    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }

    /// Get this endpoint's [`EndpointAddr`] (includes relay info for NAT traversal).
    pub fn endpoint_addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// Connect to a peer by their [`EndpointAddr`].
    ///
    /// Returns the peer's [`EndpointId`] on success.
    pub async fn connect(&self, peer_addr: EndpointAddr) -> Result<EndpointId, NetError> {
        let conn = self
            .endpoint
            .connect(peer_addr, ALPN)
            .await
            .map_err(|e| NetError::Connect(e.to_string()))?;
        let peer_id = conn.remote_id();
        self.peers.lock().await.insert(peer_id, conn);
        Ok(peer_id)
    }

    /// Accept an incoming connection.
    ///
    /// Blocks until a peer connects. Returns the peer's [`EndpointId`].
    pub async fn accept(&self) -> Result<EndpointId, NetError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or(NetError::EndpointClosed)?;
        let conn = incoming
            .await
            .map_err(|e| NetError::Accept(e.to_string()))?;
        let peer_id = conn.remote_id();
        self.peers.lock().await.insert(peer_id, conn);
        Ok(peer_id)
    }

    /// Open a bidirectional QUIC stream to a connected peer.
    ///
    /// Returns a `(SendStream, RecvStream)` pair for the new stream.
    pub async fn open_stream(
        &self,
        peer: &EndpointId,
    ) -> Result<(SendStream, RecvStream), NetError> {
        let peers = self.peers.lock().await;
        let conn = peers
            .get(peer)
            .ok_or_else(|| NetError::PeerNotFound(peer.to_string()))?;
        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| NetError::Stream(e.to_string()))?;
        Ok((send, recv))
    }

    /// Get the number of connected peers.
    pub async fn peer_count(&self) -> usize {
        self.peers.lock().await.len()
    }

    /// Disconnect from all peers and close the endpoint.
    pub async fn shutdown(self) -> Result<(), NetError> {
        let peers = self.peers.lock().await;
        for (_, conn) in peers.iter() {
            conn.close(VarInt::from_u32(0), b"game ended");
        }
        drop(peers);
        self.endpoint.close().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that creating an endpoint succeeds and produces a valid EndpointId.
    #[tokio::test]
    async fn create_endpoint() {
        let ep = GameEndpoint::new().await.expect("failed to create endpoint");
        // EndpointId should be non-zero (a real public key)
        let id_str = ep.endpoint_id().to_string();
        assert!(!id_str.is_empty());
    }

    /// Test that each endpoint gets a unique ephemeral key.
    #[tokio::test]
    async fn ephemeral_keys_differ() {
        let ep1 = GameEndpoint::new().await.expect("failed to create endpoint 1");
        let ep2 = GameEndpoint::new().await.expect("failed to create endpoint 2");
        assert_ne!(
            ep1.endpoint_id(),
            ep2.endpoint_id(),
            "Two ephemeral endpoints should have different identities"
        );
        // Clean up
        ep1.shutdown().await.unwrap();
        ep2.shutdown().await.unwrap();
    }

    /// Test that peer_count starts at zero.
    #[tokio::test]
    async fn initial_peer_count_is_zero() {
        let ep = GameEndpoint::new().await.expect("failed to create endpoint");
        assert_eq!(ep.peer_count().await, 0);
        ep.shutdown().await.unwrap();
    }

    /// Test that endpoint_addr returns something.
    #[tokio::test]
    async fn endpoint_addr_contains_id() {
        let ep = GameEndpoint::new().await.expect("failed to create endpoint");
        let addr = ep.endpoint_addr();
        assert_eq!(addr.id, ep.endpoint_id());
        ep.shutdown().await.unwrap();
    }
}

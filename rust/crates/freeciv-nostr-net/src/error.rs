//! Error types for the freeciv-nostr P2P networking layer.

/// Errors that can occur in the networking layer.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    /// Failed to create an Iroh endpoint.
    #[error("failed to create endpoint: {0}")]
    EndpointCreate(String),
    /// The endpoint has been closed.
    #[error("endpoint closed")]
    EndpointClosed,
    /// A connection attempt failed.
    #[error("connection failed: {0}")]
    Connect(String),
    /// Accepting an incoming connection failed.
    #[error("accept failed: {0}")]
    Accept(String),
    /// The specified peer was not found in the connection table.
    #[error("peer not found: {0}")]
    PeerNotFound(String),
    /// A stream operation failed.
    #[error("stream error: {0}")]
    Stream(String),
    /// Not enough data to parse a complete message.
    #[error("incomplete message parse")]
    IncompleteParse,
    /// An invalid stream ID byte was encountered.
    #[error("invalid stream ID: {0}")]
    InvalidStreamId(u8),
    /// The message payload exceeds the maximum allowed size.
    #[error("message too large: {0} bytes")]
    MessageTooLarge(usize),
    /// A gossip protocol error.
    #[error("gossip error: {0}")]
    Gossip(String),
}

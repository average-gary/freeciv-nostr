//! Protocol constants for the freeciv-nostr P2P protocol.

/// ALPN protocol identifier for freeciv-nostr connections.
pub const ALPN: &[u8] = b"freeciv-nostr/v1";

/// Stream IDs for multiplexed QUIC streams.
/// Each stream type has a dedicated purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StreamId {
    /// Game actions (Nostr events carrying PlayerAction data)
    GameActions = 0,
    /// State synchronization (checkpoint data for late joiners)
    StateSync = 1,
    /// Chat and diplomacy messages
    Chat = 2,
    /// Heartbeat / presence pings
    Heartbeat = 3,
}

/// Message framing: 4-byte big-endian length prefix + payload
pub const LENGTH_PREFIX_SIZE: usize = 4;
/// Maximum message payload size (16 MiB)
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpn_constant() {
        assert_eq!(ALPN, b"freeciv-nostr/v1");
        assert!(!ALPN.is_empty());
    }

    #[test]
    fn stream_id_values() {
        assert_eq!(StreamId::GameActions as u8, 0);
        assert_eq!(StreamId::StateSync as u8, 1);
        assert_eq!(StreamId::Chat as u8, 2);
        assert_eq!(StreamId::Heartbeat as u8, 3);
    }

    #[test]
    fn stream_id_equality() {
        assert_eq!(StreamId::GameActions, StreamId::GameActions);
        assert_ne!(StreamId::GameActions, StreamId::Chat);
    }

    #[test]
    fn constants_sizes() {
        assert_eq!(LENGTH_PREFIX_SIZE, 4);
        assert_eq!(MAX_MESSAGE_SIZE, 16 * 1024 * 1024);
    }
}

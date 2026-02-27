//! NAT traversal and relay configuration for Iroh endpoints.
//!
//! Iroh provides built-in NAT traversal via QUIC holepunching and relay
//! fallback through n0's public relay infrastructure. This module provides
//! configuration and monitoring for connection quality.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Connection type between two peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionType {
    /// Direct P2P connection (QUIC holepunched or same LAN).
    Direct,
    /// Connection mediated through a relay server.
    Relayed,
    /// Connection type unknown or being determined.
    Unknown,
}

/// Connection quality metrics for a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionQuality {
    /// The connection type.
    pub conn_type: ConnectionType,
    /// Round-trip time estimate (if available).
    pub rtt: Option<Duration>,
    /// Whether the connection is currently active.
    pub is_active: bool,
    /// Peer's endpoint ID (hex).
    pub peer_id: String,
}

/// Relay configuration for a game session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    /// Whether to use n0's public relay infrastructure.
    pub use_public_relays: bool,
    /// Custom relay URLs to use in addition to (or instead of) public relays.
    pub custom_relay_urls: Vec<String>,
    /// Whether to prefer direct connections (attempt holepunch before relay).
    pub prefer_direct: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            use_public_relays: true,
            custom_relay_urls: Vec::new(),
            prefer_direct: true,
        }
    }
}

impl RelayConfig {
    /// Create a config that only uses public relays (default).
    pub fn public_only() -> Self {
        Self::default()
    }

    /// Create a config with a custom relay URL.
    pub fn with_custom_relay(mut self, url: String) -> Self {
        self.custom_relay_urls.push(url);
        self
    }

    /// Create a config that disables public relays (custom only).
    pub fn custom_only(urls: Vec<String>) -> Self {
        Self {
            use_public_relays: false,
            custom_relay_urls: urls,
            prefer_direct: true,
        }
    }
}

/// Connection monitor that tracks quality metrics for all peers.
#[derive(Debug)]
pub struct ConnectionMonitor {
    /// Quality info per peer (keyed by endpoint ID hex).
    peers: std::collections::HashMap<String, ConnectionQuality>,
}

impl ConnectionMonitor {
    /// Create a new empty connection monitor.
    pub fn new() -> Self {
        Self {
            peers: std::collections::HashMap::new(),
        }
    }

    /// Update connection quality for a peer.
    pub fn update(&mut self, quality: ConnectionQuality) {
        self.peers.insert(quality.peer_id.clone(), quality);
    }

    /// Get connection quality for a specific peer.
    pub fn get(&self, peer_id: &str) -> Option<&ConnectionQuality> {
        self.peers.get(peer_id)
    }

    /// Get all peer connection qualities.
    pub fn all_peers(&self) -> Vec<&ConnectionQuality> {
        self.peers.values().collect()
    }

    /// Get number of direct connections.
    pub fn direct_count(&self) -> usize {
        self.peers
            .values()
            .filter(|q| q.conn_type == ConnectionType::Direct && q.is_active)
            .count()
    }

    /// Get number of relayed connections.
    pub fn relayed_count(&self) -> usize {
        self.peers
            .values()
            .filter(|q| q.conn_type == ConnectionType::Relayed && q.is_active)
            .count()
    }

    /// Get the number of active connections.
    pub fn active_count(&self) -> usize {
        self.peers.values().filter(|q| q.is_active).count()
    }

    /// Remove a peer from monitoring.
    pub fn remove(&mut self, peer_id: &str) {
        self.peers.remove(peer_id);
    }
}

impl Default for ConnectionMonitor {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- RelayConfig tests ------------------------------------------------

    #[test]
    fn relay_config_default() {
        let config = RelayConfig::default();
        assert!(config.use_public_relays);
        assert!(config.custom_relay_urls.is_empty());
        assert!(config.prefer_direct);
    }

    #[test]
    fn relay_config_public_only() {
        let config = RelayConfig::public_only();
        assert!(config.use_public_relays);
        assert!(config.custom_relay_urls.is_empty());
        assert!(config.prefer_direct);
    }

    #[test]
    fn relay_config_with_custom_relay() {
        let config =
            RelayConfig::default().with_custom_relay("https://relay.example.com".to_string());
        assert!(config.use_public_relays);
        assert_eq!(config.custom_relay_urls.len(), 1);
        assert_eq!(config.custom_relay_urls[0], "https://relay.example.com");
    }

    #[test]
    fn relay_config_with_multiple_custom_relays() {
        let config = RelayConfig::default()
            .with_custom_relay("https://relay1.example.com".to_string())
            .with_custom_relay("https://relay2.example.com".to_string());
        assert_eq!(config.custom_relay_urls.len(), 2);
    }

    #[test]
    fn relay_config_custom_only() {
        let urls = vec![
            "https://relay1.example.com".to_string(),
            "https://relay2.example.com".to_string(),
        ];
        let config = RelayConfig::custom_only(urls.clone());
        assert!(!config.use_public_relays);
        assert_eq!(config.custom_relay_urls, urls);
        assert!(config.prefer_direct);
    }

    #[test]
    fn relay_config_serde_roundtrip() {
        let config =
            RelayConfig::default().with_custom_relay("https://relay.example.com".to_string());
        let json = serde_json::to_string(&config).unwrap();
        let back: RelayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.use_public_relays, back.use_public_relays);
        assert_eq!(config.custom_relay_urls, back.custom_relay_urls);
        assert_eq!(config.prefer_direct, back.prefer_direct);
    }

    // -- ConnectionType tests ---------------------------------------------

    #[test]
    fn connection_type_serde_roundtrip() {
        for ct in &[
            ConnectionType::Direct,
            ConnectionType::Relayed,
            ConnectionType::Unknown,
        ] {
            let json = serde_json::to_string(ct).unwrap();
            let back: ConnectionType = serde_json::from_str(&json).unwrap();
            assert_eq!(*ct, back);
        }
    }

    #[test]
    fn connection_type_equality() {
        assert_eq!(ConnectionType::Direct, ConnectionType::Direct);
        assert_ne!(ConnectionType::Direct, ConnectionType::Relayed);
        assert_ne!(ConnectionType::Relayed, ConnectionType::Unknown);
    }

    // -- ConnectionQuality tests ------------------------------------------

    #[test]
    fn connection_quality_serde_roundtrip() {
        let quality = ConnectionQuality {
            conn_type: ConnectionType::Direct,
            rtt: Some(Duration::from_millis(50)),
            is_active: true,
            peer_id: "abc123".to_string(),
        };
        let json = serde_json::to_string(&quality).unwrap();
        let back: ConnectionQuality = serde_json::from_str(&json).unwrap();
        assert_eq!(quality.conn_type, back.conn_type);
        assert_eq!(quality.rtt, back.rtt);
        assert_eq!(quality.is_active, back.is_active);
        assert_eq!(quality.peer_id, back.peer_id);
    }

    #[test]
    fn connection_quality_serde_no_rtt() {
        let quality = ConnectionQuality {
            conn_type: ConnectionType::Unknown,
            rtt: None,
            is_active: false,
            peer_id: "def456".to_string(),
        };
        let json = serde_json::to_string(&quality).unwrap();
        let back: ConnectionQuality = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rtt, None);
        assert!(!back.is_active);
    }

    // -- ConnectionMonitor tests ------------------------------------------

    #[test]
    fn monitor_new_is_empty() {
        let monitor = ConnectionMonitor::new();
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(monitor.direct_count(), 0);
        assert_eq!(monitor.relayed_count(), 0);
        assert!(monitor.all_peers().is_empty());
    }

    #[test]
    fn monitor_update_and_get() {
        let mut monitor = ConnectionMonitor::new();
        let quality = ConnectionQuality {
            conn_type: ConnectionType::Direct,
            rtt: Some(Duration::from_millis(10)),
            is_active: true,
            peer_id: "peer1".to_string(),
        };
        monitor.update(quality);
        let q = monitor.get("peer1").unwrap();
        assert_eq!(q.conn_type, ConnectionType::Direct);
        assert!(q.is_active);
    }

    #[test]
    fn monitor_get_missing_returns_none() {
        let monitor = ConnectionMonitor::new();
        assert!(monitor.get("nonexistent").is_none());
    }

    #[test]
    fn monitor_update_overwrites() {
        let mut monitor = ConnectionMonitor::new();
        monitor.update(ConnectionQuality {
            conn_type: ConnectionType::Relayed,
            rtt: None,
            is_active: true,
            peer_id: "peer1".to_string(),
        });
        assert_eq!(
            monitor.get("peer1").unwrap().conn_type,
            ConnectionType::Relayed
        );

        monitor.update(ConnectionQuality {
            conn_type: ConnectionType::Direct,
            rtt: Some(Duration::from_millis(5)),
            is_active: true,
            peer_id: "peer1".to_string(),
        });
        assert_eq!(
            monitor.get("peer1").unwrap().conn_type,
            ConnectionType::Direct
        );
    }

    #[test]
    fn monitor_counts_direct_and_relayed() {
        let mut monitor = ConnectionMonitor::new();
        monitor.update(ConnectionQuality {
            conn_type: ConnectionType::Direct,
            rtt: None,
            is_active: true,
            peer_id: "peer1".to_string(),
        });
        monitor.update(ConnectionQuality {
            conn_type: ConnectionType::Relayed,
            rtt: None,
            is_active: true,
            peer_id: "peer2".to_string(),
        });
        monitor.update(ConnectionQuality {
            conn_type: ConnectionType::Direct,
            rtt: None,
            is_active: true,
            peer_id: "peer3".to_string(),
        });
        // Inactive direct peer should not count.
        monitor.update(ConnectionQuality {
            conn_type: ConnectionType::Direct,
            rtt: None,
            is_active: false,
            peer_id: "peer4".to_string(),
        });

        assert_eq!(monitor.direct_count(), 2);
        assert_eq!(monitor.relayed_count(), 1);
        assert_eq!(monitor.active_count(), 3);
        assert_eq!(monitor.all_peers().len(), 4);
    }

    #[test]
    fn monitor_remove() {
        let mut monitor = ConnectionMonitor::new();
        monitor.update(ConnectionQuality {
            conn_type: ConnectionType::Direct,
            rtt: None,
            is_active: true,
            peer_id: "peer1".to_string(),
        });
        monitor.update(ConnectionQuality {
            conn_type: ConnectionType::Relayed,
            rtt: None,
            is_active: true,
            peer_id: "peer2".to_string(),
        });

        assert_eq!(monitor.active_count(), 2);
        monitor.remove("peer1");
        assert_eq!(monitor.active_count(), 1);
        assert!(monitor.get("peer1").is_none());
        assert!(monitor.get("peer2").is_some());
    }

    #[test]
    fn monitor_remove_nonexistent_is_noop() {
        let mut monitor = ConnectionMonitor::new();
        monitor.remove("ghost"); // should not panic
        assert_eq!(monitor.active_count(), 0);
    }

    #[test]
    fn monitor_default_trait() {
        let monitor = ConnectionMonitor::default();
        assert_eq!(monitor.active_count(), 0);
    }
}

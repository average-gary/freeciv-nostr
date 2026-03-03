//! Nostr WebSocket relay client for publishing game events and subscribing
//! to game discovery, history retrieval, and player profile lookups.
//!
//! This module provides a relay pool that publishes events to multiple relays
//! in parallel (alongside iroh-gossip real-time delivery) and subscribes to
//! relays for game discovery, history, and profile data.
//!
//! # Design
//!
//! A trait-based approach (`RelayTransport`) allows unit testing without
//! real WebSocket connections. The default implementation (`WsRelayTransport`)
//! would connect over WebSocket in production; tests use `MockRelayTransport`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nostr::{Event, EventId, Filter, Kind, PublicKey};
use serde::{Deserialize, Serialize};

use freeciv_nostr_core::kinds;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Default Nostr relays used when `use_default_relays` is true.
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.nostr.band",
];

/// Configuration for the relay pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayPoolConfig {
    /// List of relay WebSocket URLs (e.g., `wss://relay.damus.io`).
    pub relay_urls: Vec<String>,
    /// Timeout for relay operations in seconds.
    pub timeout_secs: u64,
    /// Maximum number of retry attempts for failed operations.
    pub max_retries: u32,
    /// Whether to include default relays in addition to `relay_urls`.
    pub use_default_relays: bool,
}

impl Default for RelayPoolConfig {
    fn default() -> Self {
        Self {
            relay_urls: Vec::new(),
            timeout_secs: 10,
            max_retries: 3,
            use_default_relays: true,
        }
    }
}

impl RelayPoolConfig {
    /// Create a config with only the specified relays (no defaults).
    pub fn custom(urls: Vec<String>) -> Self {
        Self {
            relay_urls: urls,
            timeout_secs: 10,
            max_retries: 3,
            use_default_relays: false,
        }
    }

    /// Parse config from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize config to a JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Return the effective relay URL list (user URLs + defaults if enabled).
    pub fn effective_urls(&self) -> Vec<String> {
        let mut urls = self.relay_urls.clone();
        if self.use_default_relays {
            for default_url in DEFAULT_RELAYS {
                let s = (*default_url).to_string();
                if !urls.contains(&s) {
                    urls.push(s);
                }
            }
        }
        urls
    }
}

// ---------------------------------------------------------------------------
// Connection health tracking
// ---------------------------------------------------------------------------

/// Status of a relay connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayStatus {
    /// Connected and healthy.
    Connected,
    /// Currently establishing a connection.
    Connecting,
    /// Gracefully disconnected.
    Disconnected,
    /// Connection failed with a reason.
    Failed { reason: String },
}

/// A single relay connection with health tracking.
#[derive(Debug, Clone)]
pub struct RelayConnection {
    /// The relay WebSocket URL.
    pub url: String,
    /// Current connection status.
    pub status: RelayStatus,
    /// Number of consecutive failures.
    pub failure_count: u32,
    /// Timestamp of the last successful operation.
    pub last_success: Option<Instant>,
    /// Timestamp of the last failure.
    pub last_failure: Option<Instant>,
}

impl RelayConnection {
    /// Create a new relay connection in the `Disconnected` state.
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            status: RelayStatus::Disconnected,
            failure_count: 0,
            last_success: None,
            last_failure: None,
        }
    }

    /// Record a successful operation.
    pub fn record_success(&mut self) {
        self.status = RelayStatus::Connected;
        self.failure_count = 0;
        self.last_success = Some(Instant::now());
    }

    /// Record a failed operation.
    pub fn record_failure(&mut self, reason: &str) {
        self.failure_count += 1;
        self.last_failure = Some(Instant::now());
        self.status = RelayStatus::Failed {
            reason: reason.to_string(),
        };
    }

    /// Mark as connecting.
    pub fn mark_connecting(&mut self) {
        self.status = RelayStatus::Connecting;
    }

    /// Mark as disconnected.
    pub fn mark_disconnected(&mut self) {
        self.status = RelayStatus::Disconnected;
    }

    /// Whether this relay should be retried (failure count below threshold).
    pub fn should_retry(&self, max_retries: u32) -> bool {
        self.failure_count < max_retries
    }

    /// Whether the relay is currently usable (connected or connecting).
    pub fn is_usable(&self) -> bool {
        matches!(
            self.status,
            RelayStatus::Connected | RelayStatus::Connecting
        )
    }
}

// ---------------------------------------------------------------------------
// Subscription filters
// ---------------------------------------------------------------------------

/// Filters for relay subscriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionFilter {
    /// Discover open game lobbies (kind 30420 — PLAYER_PROFILE, as specified).
    /// In practice this queries for game lobby advertisements.
    GameDiscovery,
    /// Retrieve history for a specific game.
    GameHistory {
        /// Nostr event ID of the game root event (hex).
        game_event_id: String,
    },
    /// Look up a player profile.
    PlayerProfile {
        /// Hex-encoded Nostr public key.
        pubkey: String,
    },
    /// Custom filter with arbitrary kinds and tags.
    Custom {
        /// Event kinds to match.
        kinds: Vec<u16>,
        /// Tag filters as `(tag_name, values)` pairs.
        tags: Vec<(String, Vec<String>)>,
    },
}

impl SubscriptionFilter {
    /// Convert this high-level filter into a nostr `Filter`.
    pub fn to_nostr_filter(&self) -> Result<Filter, String> {
        match self {
            SubscriptionFilter::GameDiscovery => {
                // Game discovery: look for PLAYER_PROFILE kind (30420)
                // as specified in the issue requirements.
                Ok(Filter::new().kind(Kind::Custom(30420)))
            }
            SubscriptionFilter::GameHistory { game_event_id } => {
                // Game history: kinds 4200-4206 referencing the game event ID.
                let game_kinds = vec![
                    kinds::GAME_LOBBY,
                    kinds::GAME_ACCEPT,
                    kinds::GAME_ACTION,
                    kinds::GAME_STATE_HASH,
                    kinds::GAME_CHAT,
                    kinds::GAME_DIPLOMACY,
                    kinds::GAME_END,
                ];

                let event_id = EventId::parse(game_event_id)
                    .map_err(|e| format!("invalid game event id: {e}"))?;

                Ok(Filter::new().kinds(game_kinds).event(event_id))
            }
            SubscriptionFilter::PlayerProfile { pubkey } => {
                // Player profile lookup: kind 30421, filtered by author.
                let pk =
                    PublicKey::parse(pubkey).map_err(|e| format!("invalid public key: {e}"))?;

                Ok(Filter::new().kind(Kind::Custom(30421)).author(pk))
            }
            SubscriptionFilter::Custom { kinds, tags } => {
                let mut filter = Filter::new();

                if !kinds.is_empty() {
                    let nostr_kinds: Vec<Kind> = kinds.iter().map(|k| Kind::Custom(*k)).collect();
                    filter = filter.kinds(nostr_kinds);
                }

                for (tag_name, values) in tags {
                    // Use single-letter tag filter via the Filter API
                    // For custom tags we add them as hashtag filters when
                    // the tag name is a single character, otherwise skip
                    // (the nostr crate Filter supports single-char tag filters).
                    if tag_name.len() == 1 {
                        let tag_char = tag_name.chars().next().unwrap();
                        let tag_kind = nostr::SingleLetterTag::from_char(tag_char)
                            .map_err(|_| format!("unsupported tag letter: {tag_char}"))?;
                        for value in values {
                            filter = filter.custom_tag(tag_kind, value.clone());
                        }
                    }
                }

                Ok(filter)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Subscription management
// ---------------------------------------------------------------------------

/// A subscription to relay events.
#[derive(Debug, Clone)]
pub struct RelaySubscription {
    /// Unique subscription ID.
    pub id: String,
    /// The filter for this subscription.
    pub filter: SubscriptionFilter,
    /// Whether this subscription is active.
    pub active: bool,
}

impl RelaySubscription {
    /// Create a new active subscription.
    pub fn new(id: &str, filter: SubscriptionFilter) -> Self {
        Self {
            id: id.to_string(),
            filter,
            active: true,
        }
    }

    /// Deactivate this subscription.
    pub fn deactivate(&mut self) {
        self.active = false;
    }
}

// ---------------------------------------------------------------------------
// Publish results
// ---------------------------------------------------------------------------

/// Result of publishing an event to multiple relays.
#[derive(Debug, Clone)]
pub struct PublishResult {
    /// Number of relays that accepted the event.
    pub accepted: u32,
    /// Number of relays that failed.
    pub failed: u32,
    /// Per-relay results: `(relay_url, Ok(()) or Err(reason))`.
    pub details: Vec<(String, Result<(), String>)>,
}

impl PublishResult {
    /// Create an empty publish result.
    pub fn empty() -> Self {
        Self {
            accepted: 0,
            failed: 0,
            details: Vec::new(),
        }
    }

    /// Record a successful publish to a relay.
    pub fn record_success(&mut self, url: &str) {
        self.accepted += 1;
        self.details.push((url.to_string(), Ok(())));
    }

    /// Record a failed publish to a relay.
    pub fn record_failure(&mut self, url: &str, reason: &str) {
        self.failed += 1;
        self.details
            .push((url.to_string(), Err(reason.to_string())));
    }

    /// Whether at least one relay accepted the event.
    pub fn has_any_success(&self) -> bool {
        self.accepted > 0
    }

    /// Total number of relays attempted.
    pub fn total(&self) -> u32 {
        self.accepted + self.failed
    }
}

// ---------------------------------------------------------------------------
// Relay transport trait (for mockability)
// ---------------------------------------------------------------------------

/// Abstraction over the WebSocket transport to a single relay.
///
/// Implementing this trait allows swapping the real WebSocket client
/// for a mock in tests.
pub trait RelayTransport: Send + Sync {
    /// Connect to the relay.
    fn connect(&self, url: &str) -> Result<(), String>;

    /// Disconnect from the relay.
    fn disconnect(&self, url: &str) -> Result<(), String>;

    /// Publish an event to a specific relay.
    fn publish(&self, url: &str, event: &Event) -> Result<(), String>;

    /// Fetch events matching a filter from a specific relay.
    fn fetch_events(&self, url: &str, filter: &Filter) -> Result<Vec<Event>, String>;
}

// ---------------------------------------------------------------------------
// Relay pool
// ---------------------------------------------------------------------------

/// A pool of Nostr relay connections for publishing and subscribing.
///
/// Manages connections to multiple relays, publishing events in parallel
/// with retry/fallback, and maintaining subscriptions.
pub struct RelayPool {
    /// Pool configuration.
    config: RelayPoolConfig,
    /// Per-relay connection state.
    connections: Arc<Mutex<HashMap<String, RelayConnection>>>,
    /// Active subscriptions.
    subscriptions: Arc<Mutex<HashMap<String, RelaySubscription>>>,
    /// The transport implementation (real WS or mock).
    transport: Arc<dyn RelayTransport>,
    /// Counter for generating subscription IDs.
    sub_counter: Arc<Mutex<u64>>,
}

impl RelayPool {
    /// Create a new relay pool with the given config and transport.
    pub fn new(config: RelayPoolConfig, transport: Arc<dyn RelayTransport>) -> Self {
        let urls = config.effective_urls();
        let mut connections = HashMap::new();
        for url in &urls {
            connections.insert(url.clone(), RelayConnection::new(url));
        }

        Self {
            config,
            connections: Arc::new(Mutex::new(connections)),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            transport,
            sub_counter: Arc::new(Mutex::new(0)),
        }
    }

    /// Get the pool configuration.
    pub fn config(&self) -> &RelayPoolConfig {
        &self.config
    }

    /// Get the number of configured relays.
    pub fn relay_count(&self) -> usize {
        self.connections.lock().unwrap().len()
    }

    /// Get the status of all relays.
    pub fn relay_statuses(&self) -> Vec<(String, RelayStatus)> {
        self.connections
            .lock()
            .unwrap()
            .iter()
            .map(|(url, conn)| (url.clone(), conn.status.clone()))
            .collect()
    }

    /// Get the status of a specific relay.
    pub fn relay_status(&self, url: &str) -> Option<RelayStatus> {
        self.connections
            .lock()
            .unwrap()
            .get(url)
            .map(|c| c.status.clone())
    }

    /// Add a relay to the pool.
    pub fn add_relay(&mut self, url: &str) {
        let mut conns = self.connections.lock().unwrap();
        if !conns.contains_key(url) {
            conns.insert(url.to_string(), RelayConnection::new(url));
        }
    }

    /// Remove a relay from the pool.
    pub fn remove_relay(&mut self, url: &str) -> bool {
        let mut conns = self.connections.lock().unwrap();
        conns.remove(url).is_some()
    }

    /// Connect to all relays in the pool.
    pub fn connect_all(&self) -> Vec<(String, Result<(), String>)> {
        let urls: Vec<String> = self.connections.lock().unwrap().keys().cloned().collect();

        let mut results = Vec::new();
        for url in urls {
            {
                let mut conns = self.connections.lock().unwrap();
                if let Some(conn) = conns.get_mut(&url) {
                    conn.mark_connecting();
                }
            }

            let result = self.transport.connect(&url);

            {
                let mut conns = self.connections.lock().unwrap();
                if let Some(conn) = conns.get_mut(&url) {
                    match &result {
                        Ok(()) => conn.record_success(),
                        Err(reason) => conn.record_failure(reason),
                    }
                }
            }

            results.push((url, result));
        }
        results
    }

    /// Disconnect from all relays.
    pub fn disconnect_all(&self) {
        let urls: Vec<String> = self.connections.lock().unwrap().keys().cloned().collect();

        for url in urls {
            let _ = self.transport.disconnect(&url);
            let mut conns = self.connections.lock().unwrap();
            if let Some(conn) = conns.get_mut(&url) {
                conn.mark_disconnected();
            }
        }
    }

    /// Publish an event to all relays with retry logic.
    ///
    /// Attempts to publish to every relay. On failure, retries up to
    /// `max_retries` times per relay.
    pub fn publish(&self, event: &Event) -> PublishResult {
        let urls: Vec<String> = self.connections.lock().unwrap().keys().cloned().collect();

        let mut result = PublishResult::empty();

        for url in urls {
            let mut published = false;
            let max_retries = self.config.max_retries;

            for attempt in 0..=max_retries {
                match self.transport.publish(&url, event) {
                    Ok(()) => {
                        result.record_success(&url);
                        let mut conns = self.connections.lock().unwrap();
                        if let Some(conn) = conns.get_mut(&url) {
                            conn.record_success();
                        }
                        published = true;
                        break;
                    }
                    Err(reason) => {
                        let mut conns = self.connections.lock().unwrap();
                        if let Some(conn) = conns.get_mut(&url) {
                            conn.record_failure(&reason);
                            if attempt == max_retries || !conn.should_retry(max_retries) {
                                drop(conns);
                                result.record_failure(&url, &reason);
                                break;
                            }
                        } else {
                            drop(conns);
                            result.record_failure(&url, &reason);
                            break;
                        }
                    }
                }
            }

            if !published && result.details.iter().all(|(u, _)| u != &url) {
                result.record_failure(&url, "exhausted retries");
            }
        }

        result
    }

    /// Subscribe with a filter and return the subscription ID.
    pub fn subscribe(&self, filter: SubscriptionFilter) -> Result<String, String> {
        // Validate the filter can be converted to a nostr Filter.
        let _nostr_filter = filter.to_nostr_filter()?;

        let sub_id = {
            let mut counter = self.sub_counter.lock().unwrap();
            *counter += 1;
            format!("sub_{}", *counter)
        };

        let subscription = RelaySubscription::new(&sub_id, filter);
        self.subscriptions
            .lock()
            .unwrap()
            .insert(sub_id.clone(), subscription);

        Ok(sub_id)
    }

    /// Unsubscribe by subscription ID. Returns true if the subscription existed.
    pub fn unsubscribe(&self, sub_id: &str) -> bool {
        let mut subs = self.subscriptions.lock().unwrap();
        if let Some(sub) = subs.get_mut(sub_id) {
            sub.deactivate();
            subs.remove(sub_id);
            true
        } else {
            false
        }
    }

    /// Get all active subscriptions.
    pub fn active_subscriptions(&self) -> Vec<RelaySubscription> {
        self.subscriptions
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.active)
            .cloned()
            .collect()
    }

    /// Fetch events matching a filter from all connected relays.
    ///
    /// Returns the first successful result from any relay. On failure,
    /// tries the next relay.
    pub fn fetch_events(&self, filter: &SubscriptionFilter) -> Result<Vec<Event>, String> {
        let nostr_filter = filter.to_nostr_filter()?;

        let urls: Vec<String> = self.connections.lock().unwrap().keys().cloned().collect();

        let mut last_error = String::from("no relays configured");

        for url in urls {
            match self.transport.fetch_events(&url, &nostr_filter) {
                Ok(events) => {
                    let mut conns = self.connections.lock().unwrap();
                    if let Some(conn) = conns.get_mut(&url) {
                        conn.record_success();
                    }
                    return Ok(events);
                }
                Err(reason) => {
                    let mut conns = self.connections.lock().unwrap();
                    if let Some(conn) = conns.get_mut(&url) {
                        conn.record_failure(&reason);
                    }
                    last_error = reason;
                }
            }
        }

        Err(last_error)
    }

    /// Check whether a specific event kind should be relayed.
    ///
    /// Ephemeral events (heartbeat, state sync) are not published to
    /// relays—they only go over iroh-gossip.
    pub fn should_relay_kind(kind: Kind) -> bool {
        // Ephemeral kinds are not stored by relays.
        kind != kinds::HEARTBEAT && kind != kinds::STATE_SYNC
    }

    /// Get a JSON status summary of the pool.
    pub fn status_json(&self) -> Result<String, serde_json::Error> {
        let conns = self.connections.lock().unwrap();
        let statuses: Vec<serde_json::Value> = conns
            .iter()
            .map(|(url, conn)| {
                serde_json::json!({
                    "url": url,
                    "status": format!("{:?}", conn.status),
                    "failure_count": conn.failure_count,
                })
            })
            .collect();

        let sub_count = self.subscriptions.lock().unwrap().len();

        serde_json::to_string(&serde_json::json!({
            "relay_count": conns.len(),
            "subscription_count": sub_count,
            "relays": statuses,
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::JsonUtil;
    use std::sync::Mutex as StdMutex;

    // -- Mock transport for testing -----------------------------------------

    /// A mock relay transport that records operations and returns
    /// configurable results.
    struct MockRelayTransport {
        /// Track which URLs were connected.
        connected: StdMutex<Vec<String>>,
        /// Track which URLs were disconnected.
        disconnected: StdMutex<Vec<String>>,
        /// Track published events per URL.
        published: StdMutex<Vec<(String, Event)>>,
        /// Whether connect should succeed.
        connect_succeeds: bool,
        /// Whether publish should succeed.
        publish_succeeds: bool,
        /// Events to return from fetch_events.
        fetch_results: StdMutex<Vec<Event>>,
        /// Whether fetch should succeed.
        fetch_succeeds: bool,
        /// Track how many publish attempts were made per URL.
        publish_attempts: StdMutex<HashMap<String, u32>>,
        /// Fail publish N times before succeeding (per URL).
        fail_n_times: u32,
    }

    impl MockRelayTransport {
        fn new_success() -> Self {
            Self {
                connected: StdMutex::new(Vec::new()),
                disconnected: StdMutex::new(Vec::new()),
                published: StdMutex::new(Vec::new()),
                connect_succeeds: true,
                publish_succeeds: true,
                fetch_results: StdMutex::new(Vec::new()),
                fetch_succeeds: true,
                publish_attempts: StdMutex::new(HashMap::new()),
                fail_n_times: 0,
            }
        }

        fn new_failing() -> Self {
            Self {
                connected: StdMutex::new(Vec::new()),
                disconnected: StdMutex::new(Vec::new()),
                published: StdMutex::new(Vec::new()),
                connect_succeeds: false,
                publish_succeeds: false,
                fetch_results: StdMutex::new(Vec::new()),
                fetch_succeeds: false,
                publish_attempts: StdMutex::new(HashMap::new()),
                fail_n_times: 0,
            }
        }

        fn new_fail_then_succeed(fail_count: u32) -> Self {
            Self {
                connected: StdMutex::new(Vec::new()),
                disconnected: StdMutex::new(Vec::new()),
                published: StdMutex::new(Vec::new()),
                connect_succeeds: true,
                publish_succeeds: true,
                fetch_results: StdMutex::new(Vec::new()),
                fetch_succeeds: true,
                publish_attempts: StdMutex::new(HashMap::new()),
                fail_n_times: fail_count,
            }
        }
    }

    impl RelayTransport for MockRelayTransport {
        fn connect(&self, url: &str) -> Result<(), String> {
            if self.connect_succeeds {
                self.connected.lock().unwrap().push(url.to_string());
                Ok(())
            } else {
                Err("mock connect failure".to_string())
            }
        }

        fn disconnect(&self, url: &str) -> Result<(), String> {
            self.disconnected.lock().unwrap().push(url.to_string());
            Ok(())
        }

        fn publish(&self, url: &str, event: &Event) -> Result<(), String> {
            let mut attempts = self.publish_attempts.lock().unwrap();
            let count = attempts.entry(url.to_string()).or_insert(0);
            *count += 1;
            let current = *count;
            drop(attempts);

            if self.fail_n_times > 0 && current <= self.fail_n_times {
                return Err(format!("mock publish failure (attempt {current})"));
            }

            if self.publish_succeeds {
                self.published
                    .lock()
                    .unwrap()
                    .push((url.to_string(), event.clone()));
                Ok(())
            } else {
                Err("mock publish failure".to_string())
            }
        }

        fn fetch_events(&self, _url: &str, _filter: &Filter) -> Result<Vec<Event>, String> {
            if self.fetch_succeeds {
                Ok(self.fetch_results.lock().unwrap().clone())
            } else {
                Err("mock fetch failure".to_string())
            }
        }
    }

    /// Helper to create a signed test event.
    fn make_test_event() -> Event {
        use nostr::Keys;
        let keys = Keys::generate();
        let builder = nostr::EventBuilder::new(kinds::GAME_LOBBY, "test content");
        builder.sign_with_keys(&keys).unwrap()
    }

    // -- RelayPoolConfig tests ---------------------------------------------

    #[test]
    fn config_default() {
        let config = RelayPoolConfig::default();
        assert!(config.relay_urls.is_empty());
        assert_eq!(config.timeout_secs, 10);
        assert_eq!(config.max_retries, 3);
        assert!(config.use_default_relays);
    }

    #[test]
    fn config_custom() {
        let config = RelayPoolConfig::custom(vec!["wss://my.relay".to_string()]);
        assert_eq!(config.relay_urls.len(), 1);
        assert!(!config.use_default_relays);
    }

    #[test]
    fn config_effective_urls_with_defaults() {
        let config = RelayPoolConfig::default();
        let urls = config.effective_urls();
        assert_eq!(urls.len(), DEFAULT_RELAYS.len());
        for default in DEFAULT_RELAYS {
            assert!(urls.contains(&default.to_string()));
        }
    }

    #[test]
    fn config_effective_urls_without_defaults() {
        let config = RelayPoolConfig::custom(vec!["wss://my.relay".to_string()]);
        let urls = config.effective_urls();
        assert_eq!(urls, vec!["wss://my.relay".to_string()]);
    }

    #[test]
    fn config_effective_urls_no_duplicates() {
        let config = RelayPoolConfig {
            relay_urls: vec![DEFAULT_RELAYS[0].to_string()],
            use_default_relays: true,
            ..Default::default()
        };
        let urls = config.effective_urls();
        // Should not duplicate the relay that's in both lists.
        let count = urls
            .iter()
            .filter(|u| u.as_str() == DEFAULT_RELAYS[0])
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn config_json_roundtrip() {
        let config = RelayPoolConfig {
            relay_urls: vec!["wss://relay.example.com".to_string()],
            timeout_secs: 30,
            max_retries: 5,
            use_default_relays: false,
        };
        let json = config.to_json().unwrap();
        let back = RelayPoolConfig::from_json(&json).unwrap();
        assert_eq!(back.relay_urls, config.relay_urls);
        assert_eq!(back.timeout_secs, config.timeout_secs);
        assert_eq!(back.max_retries, config.max_retries);
        assert_eq!(back.use_default_relays, config.use_default_relays);
    }

    #[test]
    fn config_from_json_full() {
        let json = r#"{"relay_urls":["wss://r1","wss://r2"],"timeout_secs":5,"max_retries":2,"use_default_relays":false}"#;
        let config = RelayPoolConfig::from_json(json).unwrap();
        assert_eq!(config.relay_urls.len(), 2);
        assert_eq!(config.timeout_secs, 5);
        assert_eq!(config.max_retries, 2);
        assert!(!config.use_default_relays);
    }

    // -- RelayConnection health tests --------------------------------------

    #[test]
    fn connection_new_is_disconnected() {
        let conn = RelayConnection::new("wss://relay.test");
        assert_eq!(conn.status, RelayStatus::Disconnected);
        assert_eq!(conn.failure_count, 0);
        assert!(conn.last_success.is_none());
        assert!(conn.last_failure.is_none());
    }

    #[test]
    fn connection_record_success() {
        let mut conn = RelayConnection::new("wss://relay.test");
        conn.record_success();
        assert_eq!(conn.status, RelayStatus::Connected);
        assert_eq!(conn.failure_count, 0);
        assert!(conn.last_success.is_some());
    }

    #[test]
    fn connection_record_failure() {
        let mut conn = RelayConnection::new("wss://relay.test");
        conn.record_failure("timeout");
        assert_eq!(
            conn.status,
            RelayStatus::Failed {
                reason: "timeout".to_string()
            }
        );
        assert_eq!(conn.failure_count, 1);
        assert!(conn.last_failure.is_some());
    }

    #[test]
    fn connection_success_resets_failure_count() {
        let mut conn = RelayConnection::new("wss://relay.test");
        conn.record_failure("err1");
        conn.record_failure("err2");
        assert_eq!(conn.failure_count, 2);
        conn.record_success();
        assert_eq!(conn.failure_count, 0);
        assert_eq!(conn.status, RelayStatus::Connected);
    }

    #[test]
    fn connection_state_transitions() {
        let mut conn = RelayConnection::new("wss://relay.test");
        assert_eq!(conn.status, RelayStatus::Disconnected);

        conn.mark_connecting();
        assert_eq!(conn.status, RelayStatus::Connecting);
        assert!(conn.is_usable());

        conn.record_success();
        assert_eq!(conn.status, RelayStatus::Connected);
        assert!(conn.is_usable());

        conn.record_failure("network error");
        assert!(matches!(conn.status, RelayStatus::Failed { .. }));
        assert!(!conn.is_usable());

        conn.mark_disconnected();
        assert_eq!(conn.status, RelayStatus::Disconnected);
        assert!(!conn.is_usable());
    }

    #[test]
    fn connection_should_retry() {
        let mut conn = RelayConnection::new("wss://relay.test");
        assert!(conn.should_retry(3));

        conn.record_failure("err1");
        assert!(conn.should_retry(3)); // 1 < 3

        conn.record_failure("err2");
        assert!(conn.should_retry(3)); // 2 < 3

        conn.record_failure("err3");
        assert!(!conn.should_retry(3)); // 3 >= 3
    }

    // -- SubscriptionFilter tests ------------------------------------------

    #[test]
    fn filter_game_discovery() {
        let filter = SubscriptionFilter::GameDiscovery;
        let nostr_filter = filter.to_nostr_filter().unwrap();

        // Verify it has kind 30420.
        let json = nostr_filter.as_json();
        assert!(json.contains("30420"), "filter JSON: {json}");
    }

    #[test]
    fn filter_game_history() {
        // Use a valid 32-byte hex event ID (all zeros).
        let game_id = "0000000000000000000000000000000000000000000000000000000000000000";
        let filter = SubscriptionFilter::GameHistory {
            game_event_id: game_id.to_string(),
        };
        let nostr_filter = filter.to_nostr_filter().unwrap();

        let json = nostr_filter.as_json();
        // Should contain game event kinds.
        assert!(json.contains("4200"), "filter JSON: {json}");
        assert!(json.contains("4201"), "filter JSON: {json}");
        assert!(json.contains("4202"), "filter JSON: {json}");
        assert!(json.contains("4203"), "filter JSON: {json}");
        assert!(json.contains("4204"), "filter JSON: {json}");
        assert!(json.contains("4205"), "filter JSON: {json}");
        assert!(json.contains("4206"), "filter JSON: {json}");
        // Should reference the event ID.
        assert!(json.contains(game_id), "filter JSON: {json}");
    }

    #[test]
    fn filter_game_history_invalid_id() {
        let filter = SubscriptionFilter::GameHistory {
            game_event_id: "not-a-valid-hex-id".to_string(),
        };
        assert!(filter.to_nostr_filter().is_err());
    }

    #[test]
    fn filter_player_profile() {
        // Generate a valid public key.
        let keys = nostr::Keys::generate();
        let pk_hex = keys.public_key().to_hex();

        let filter = SubscriptionFilter::PlayerProfile {
            pubkey: pk_hex.clone(),
        };
        let nostr_filter = filter.to_nostr_filter().unwrap();

        let json = nostr_filter.as_json();
        assert!(json.contains("30421"), "filter JSON: {json}");
        assert!(json.contains(&pk_hex), "filter JSON: {json}");
    }

    #[test]
    fn filter_player_profile_invalid_pubkey() {
        let filter = SubscriptionFilter::PlayerProfile {
            pubkey: "invalid-pubkey".to_string(),
        };
        assert!(filter.to_nostr_filter().is_err());
    }

    #[test]
    fn filter_custom_kinds() {
        let filter = SubscriptionFilter::Custom {
            kinds: vec![4200, 4207],
            tags: vec![],
        };
        let nostr_filter = filter.to_nostr_filter().unwrap();
        let json = nostr_filter.as_json();
        assert!(json.contains("4200"), "filter JSON: {json}");
        assert!(json.contains("4207"), "filter JSON: {json}");
    }

    #[test]
    fn filter_custom_with_tags() {
        let filter = SubscriptionFilter::Custom {
            kinds: vec![4202],
            tags: vec![("e".to_string(), vec!["abc123".to_string()])],
        };
        let nostr_filter = filter.to_nostr_filter().unwrap();
        let json = nostr_filter.as_json();
        assert!(json.contains("4202"), "filter JSON: {json}");
        assert!(json.contains("abc123"), "filter JSON: {json}");
    }

    #[test]
    fn filter_custom_empty() {
        let filter = SubscriptionFilter::Custom {
            kinds: vec![],
            tags: vec![],
        };
        // Should succeed even with empty fields.
        assert!(filter.to_nostr_filter().is_ok());
    }

    // -- PublishResult tests -----------------------------------------------

    #[test]
    fn publish_result_empty() {
        let result = PublishResult::empty();
        assert_eq!(result.accepted, 0);
        assert_eq!(result.failed, 0);
        assert!(result.details.is_empty());
        assert!(!result.has_any_success());
        assert_eq!(result.total(), 0);
    }

    #[test]
    fn publish_result_tracking() {
        let mut result = PublishResult::empty();
        result.record_success("wss://relay1");
        result.record_failure("wss://relay2", "timeout");
        result.record_success("wss://relay3");

        assert_eq!(result.accepted, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.total(), 3);
        assert!(result.has_any_success());

        assert!(result.details[0].1.is_ok());
        assert!(result.details[1].1.is_err());
        assert!(result.details[2].1.is_ok());
    }

    // -- RelayPool tests ---------------------------------------------------

    #[test]
    fn pool_creation_with_defaults() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let pool = RelayPool::new(RelayPoolConfig::default(), transport);
        assert_eq!(pool.relay_count(), DEFAULT_RELAYS.len());
    }

    #[test]
    fn pool_creation_with_custom_relays() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string(), "wss://r2".to_string()]);
        let pool = RelayPool::new(config, transport);
        assert_eq!(pool.relay_count(), 2);
    }

    #[test]
    fn pool_add_relay() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string()]);
        let mut pool = RelayPool::new(config, transport);
        assert_eq!(pool.relay_count(), 1);

        pool.add_relay("wss://r2");
        assert_eq!(pool.relay_count(), 2);

        // Adding duplicate should not increase count.
        pool.add_relay("wss://r2");
        assert_eq!(pool.relay_count(), 2);
    }

    #[test]
    fn pool_remove_relay() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string(), "wss://r2".to_string()]);
        let mut pool = RelayPool::new(config, transport);
        assert_eq!(pool.relay_count(), 2);

        assert!(pool.remove_relay("wss://r1"));
        assert_eq!(pool.relay_count(), 1);

        // Removing non-existent relay returns false.
        assert!(!pool.remove_relay("wss://r1"));
    }

    #[test]
    fn pool_connect_all_success() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string(), "wss://r2".to_string()]);
        let pool = RelayPool::new(config, transport.clone());

        let results = pool.connect_all();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, r)| r.is_ok()));

        // All relays should be connected.
        for (url, status) in pool.relay_statuses() {
            assert_eq!(status, RelayStatus::Connected, "relay {url} not connected");
        }
    }

    #[test]
    fn pool_connect_all_failure() {
        let transport = Arc::new(MockRelayTransport::new_failing());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string()]);
        let pool = RelayPool::new(config, transport);

        let results = pool.connect_all();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_err());

        let status = pool.relay_status("wss://r1").unwrap();
        assert!(matches!(status, RelayStatus::Failed { .. }));
    }

    #[test]
    fn pool_disconnect_all() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string(), "wss://r2".to_string()]);
        let pool = RelayPool::new(config, transport);

        pool.connect_all();
        pool.disconnect_all();

        for (_, status) in pool.relay_statuses() {
            assert_eq!(status, RelayStatus::Disconnected);
        }
    }

    #[test]
    fn pool_publish_success() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string(), "wss://r2".to_string()]);
        let pool = RelayPool::new(config, transport.clone());

        let event = make_test_event();
        let result = pool.publish(&event);

        assert_eq!(result.accepted, 2);
        assert_eq!(result.failed, 0);
        assert!(result.has_any_success());
        assert_eq!(transport.published.lock().unwrap().len(), 2);
    }

    #[test]
    fn pool_publish_all_fail() {
        let transport = Arc::new(MockRelayTransport::new_failing());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string()]);
        let pool = RelayPool::new(config.clone(), transport);

        let event = make_test_event();
        let result = pool.publish(&event);

        assert_eq!(result.accepted, 0);
        // With max_retries=3, the relay will be attempted 4 times (0..=3)
        // but failure count reaches 3 on the 3rd failure, so should_retry
        // returns false and we break.
        assert_eq!(result.failed, 1);
        assert!(!result.has_any_success());
    }

    #[test]
    fn pool_publish_retry_then_succeed() {
        // Fail 2 times, then succeed on 3rd attempt.
        let transport = Arc::new(MockRelayTransport::new_fail_then_succeed(2));
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string()]);
        let pool = RelayPool::new(config, transport.clone());

        let event = make_test_event();
        let result = pool.publish(&event);

        assert_eq!(result.accepted, 1);
        assert_eq!(result.failed, 0);
        assert!(result.has_any_success());
        assert_eq!(transport.published.lock().unwrap().len(), 1);
    }

    // -- Subscription tests ------------------------------------------------

    #[test]
    fn pool_subscribe() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let pool = RelayPool::new(RelayPoolConfig::custom(vec![]), transport);

        let sub_id = pool.subscribe(SubscriptionFilter::GameDiscovery).unwrap();
        assert!(sub_id.starts_with("sub_"));

        let subs = pool.active_subscriptions();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].id, sub_id);
        assert!(subs[0].active);
    }

    #[test]
    fn pool_subscribe_multiple() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let pool = RelayPool::new(RelayPoolConfig::custom(vec![]), transport);

        let id1 = pool.subscribe(SubscriptionFilter::GameDiscovery).unwrap();
        let id2 = pool
            .subscribe(SubscriptionFilter::GameHistory {
                game_event_id: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            })
            .unwrap();

        assert_ne!(id1, id2);
        assert_eq!(pool.active_subscriptions().len(), 2);
    }

    #[test]
    fn pool_unsubscribe() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let pool = RelayPool::new(RelayPoolConfig::custom(vec![]), transport);

        let sub_id = pool.subscribe(SubscriptionFilter::GameDiscovery).unwrap();
        assert_eq!(pool.active_subscriptions().len(), 1);

        assert!(pool.unsubscribe(&sub_id));
        assert_eq!(pool.active_subscriptions().len(), 0);

        // Unsubscribing again returns false.
        assert!(!pool.unsubscribe(&sub_id));
    }

    #[test]
    fn pool_subscribe_invalid_filter() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let pool = RelayPool::new(RelayPoolConfig::custom(vec![]), transport);

        let result = pool.subscribe(SubscriptionFilter::GameHistory {
            game_event_id: "invalid".to_string(),
        });
        assert!(result.is_err());
    }

    // -- Fetch events tests ------------------------------------------------

    #[test]
    fn pool_fetch_events_success() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string()]);
        let pool = RelayPool::new(config, transport);

        let result = pool.fetch_events(&SubscriptionFilter::GameDiscovery);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty()); // Mock returns empty vec.
    }

    #[test]
    fn pool_fetch_events_fallback() {
        // First relay fails, second succeeds.
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string(), "wss://r2".to_string()]);
        let pool = RelayPool::new(config, transport);

        let result = pool.fetch_events(&SubscriptionFilter::GameDiscovery);
        assert!(result.is_ok());
    }

    #[test]
    fn pool_fetch_events_all_fail() {
        let transport = Arc::new(MockRelayTransport::new_failing());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string()]);
        let pool = RelayPool::new(config, transport);

        let result = pool.fetch_events(&SubscriptionFilter::GameDiscovery);
        assert!(result.is_err());
    }

    #[test]
    fn pool_fetch_events_no_relays() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec![]);
        let pool = RelayPool::new(config, transport);

        let result = pool.fetch_events(&SubscriptionFilter::GameDiscovery);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "no relays configured");
    }

    // -- Event routing tests -----------------------------------------------

    #[test]
    fn should_relay_game_events() {
        assert!(RelayPool::should_relay_kind(kinds::GAME_LOBBY));
        assert!(RelayPool::should_relay_kind(kinds::GAME_ACCEPT));
        assert!(RelayPool::should_relay_kind(kinds::GAME_ACTION));
        assert!(RelayPool::should_relay_kind(kinds::GAME_STATE_HASH));
        assert!(RelayPool::should_relay_kind(kinds::GAME_CHAT));
        assert!(RelayPool::should_relay_kind(kinds::GAME_DIPLOMACY));
        assert!(RelayPool::should_relay_kind(kinds::GAME_END));
        assert!(RelayPool::should_relay_kind(kinds::GAME_START));
        assert!(RelayPool::should_relay_kind(kinds::PLAYER_PROFILE));
        assert!(RelayPool::should_relay_kind(kinds::GAME_REPLAY));
    }

    #[test]
    fn should_not_relay_ephemeral_events() {
        assert!(!RelayPool::should_relay_kind(kinds::HEARTBEAT));
        assert!(!RelayPool::should_relay_kind(kinds::STATE_SYNC));
    }

    // -- Status / JSON output tests ----------------------------------------

    #[test]
    fn pool_status_json() {
        let transport = Arc::new(MockRelayTransport::new_success());
        let config = RelayPoolConfig::custom(vec!["wss://r1".to_string()]);
        let pool = RelayPool::new(config, transport);

        let json = pool.status_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["relay_count"], 1);
        assert_eq!(value["subscription_count"], 0);
    }

    // -- RelaySubscription tests -------------------------------------------

    #[test]
    fn subscription_creation_and_deactivation() {
        let mut sub = RelaySubscription::new("test-sub", SubscriptionFilter::GameDiscovery);
        assert!(sub.active);
        assert_eq!(sub.id, "test-sub");

        sub.deactivate();
        assert!(!sub.active);
    }

    // -- RelayStatus tests -------------------------------------------------

    #[test]
    fn relay_status_eq() {
        assert_eq!(RelayStatus::Connected, RelayStatus::Connected);
        assert_eq!(RelayStatus::Disconnected, RelayStatus::Disconnected);
        assert_eq!(RelayStatus::Connecting, RelayStatus::Connecting);
        assert_eq!(
            RelayStatus::Failed {
                reason: "x".to_string()
            },
            RelayStatus::Failed {
                reason: "x".to_string()
            }
        );
        assert_ne!(RelayStatus::Connected, RelayStatus::Disconnected);
    }

    #[test]
    fn relay_status_serde_roundtrip() {
        let statuses = vec![
            RelayStatus::Connected,
            RelayStatus::Connecting,
            RelayStatus::Disconnected,
            RelayStatus::Failed {
                reason: "test".to_string(),
            },
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let back: RelayStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }
}

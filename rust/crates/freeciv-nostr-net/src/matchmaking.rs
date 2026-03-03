//! Matchmaking backend for game discovery and lobby management.
//!
//! Provides data structures and logic for browsing, creating, and joining
//! games via Nostr relays. The C GUI calls these through FFI bindings in
//! the `freeciv-nostr-ffi` crate.
//!
//! # Event Kinds
//!
//! - **Kind 4200** – Game Lobby event: published when a player creates a
//!   game and wants others to discover it.
//! - **Game Offer** – for private/invite-only games, shared by event ID.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// MapSize
// ---------------------------------------------------------------------------

/// Map size presets (or custom dimensions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapSize {
    Tiny,
    Small,
    Medium,
    Large,
    Huge,
    Custom(u32, u32),
}

impl std::fmt::Display for MapSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapSize::Tiny => write!(f, "Tiny"),
            MapSize::Small => write!(f, "Small"),
            MapSize::Medium => write!(f, "Medium"),
            MapSize::Large => write!(f, "Large"),
            MapSize::Huge => write!(f, "Huge"),
            MapSize::Custom(w, h) => write!(f, "Custom({w}x{h})"),
        }
    }
}

// ---------------------------------------------------------------------------
// ListingStatus
// ---------------------------------------------------------------------------

/// Current status of a game listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListingStatus {
    /// Accepting players.
    Open,
    /// All player slots are filled.
    Full,
    /// Game is currently being played.
    InProgress,
    /// Game has finished.
    Completed,
    /// Listing was cancelled by the creator.
    Cancelled,
}

impl std::fmt::Display for ListingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListingStatus::Open => write!(f, "Open"),
            ListingStatus::Full => write!(f, "Full"),
            ListingStatus::InProgress => write!(f, "InProgress"),
            ListingStatus::Completed => write!(f, "Completed"),
            ListingStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

// ---------------------------------------------------------------------------
// GameSettings
// ---------------------------------------------------------------------------

/// Configuration for a game session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSettings {
    /// Ruleset identifier (e.g. "civ2civ3", "classic", "multiplayer").
    pub ruleset: String,
    /// Map dimensions.
    pub map_size: MapSize,
    /// Maximum number of human players.
    pub max_players: u8,
    /// Per-turn timeout in seconds (0 = unlimited).
    pub turn_timeout: u32,
    /// Phase mode name (e.g. "concurrent", "alternating").
    pub phase_mode: String,
    /// Map generation seed (0 = random).
    pub map_seed: u64,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Number of AI-controlled players.
    pub ai_players: u8,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            ruleset: "civ2civ3".to_string(),
            map_size: MapSize::Medium,
            max_players: 4,
            turn_timeout: 300,
            phase_mode: "concurrent".to_string(),
            map_seed: 0,
            description: None,
            ai_players: 0,
        }
    }
}

impl GameSettings {
    /// Number of open (unfilled) human player slots given the current
    /// player count.
    pub fn open_slots(&self, current_players: u8) -> u8 {
        self.max_players.saturating_sub(current_players)
    }
}

// ---------------------------------------------------------------------------
// GameListing
// ---------------------------------------------------------------------------

/// A single game listing as it appears in the matchmaking browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameListing {
    /// Unique game identifier (typically a Nostr event ID).
    pub game_id: String,
    /// Hex-encoded Nostr public key of the game creator.
    pub creator_pubkey: String,
    /// Optional display name of the creator.
    pub creator_name: Option<String>,
    /// Game configuration.
    pub settings: GameSettings,
    /// Number of players currently in the game (including the creator).
    pub current_players: u8,
    /// Whether the game is invite-only.
    pub is_private: bool,
    /// Unix timestamp (seconds) when the listing was created.
    pub created_at: u64,
    /// Current status of the listing.
    pub status: ListingStatus,
}

// ---------------------------------------------------------------------------
// MatchmakingFilter
// ---------------------------------------------------------------------------

/// Filter criteria for browsing game listings.
#[derive(Debug, Clone, Default)]
pub struct MatchmakingFilter {
    /// Only show games with this ruleset.
    pub ruleset: Option<String>,
    /// Only show games with at least this many open slots.
    pub min_open_slots: Option<u8>,
    /// Only show games with at most this many max players.
    pub max_players: Option<u8>,
    /// Only show games with this map size.
    pub map_size: Option<MapSize>,
    /// If true, only show games with status `Open`.
    pub open_only: bool,
    /// If true, exclude private (invite-only) games.
    pub exclude_private: bool,
    /// Only show games created by this pubkey.
    pub creator: Option<String>,
}

impl MatchmakingFilter {
    /// Returns `true` if the given listing passes all active filters.
    pub fn matches(&self, listing: &GameListing) -> bool {
        if let Some(ref ruleset) = self.ruleset
            && listing.settings.ruleset != *ruleset
        {
            return false;
        }

        if let Some(min_slots) = self.min_open_slots {
            let open = listing.settings.open_slots(listing.current_players);
            if open < min_slots {
                return false;
            }
        }

        if let Some(max_p) = self.max_players
            && listing.settings.max_players > max_p
        {
            return false;
        }

        if let Some(ref ms) = self.map_size
            && listing.settings.map_size != *ms
        {
            return false;
        }

        if self.open_only && listing.status != ListingStatus::Open {
            return false;
        }

        if self.exclude_private && listing.is_private {
            return false;
        }

        if let Some(ref creator) = self.creator
            && listing.creator_pubkey != *creator
        {
            return false;
        }

        true
    }
}

// ---------------------------------------------------------------------------
// MatchmakingError
// ---------------------------------------------------------------------------

/// Errors produced by [`Matchmaker`] operations.
#[derive(Debug, thiserror::Error)]
pub enum MatchmakingError {
    /// The referenced game was not found.
    #[error("game not found: {0}")]
    GameNotFound(String),
    /// The game is full (no open slots).
    #[error("game is full: {0}")]
    GameFull(String),
    /// A listing with this ID already exists.
    #[error("duplicate listing: {0}")]
    DuplicateListing(String),
    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(String),
}

// ---------------------------------------------------------------------------
// Matchmaker
// ---------------------------------------------------------------------------

/// Manages the local view of available game listings.
///
/// The `Matchmaker` aggregates game lobby events received from Nostr relays
/// and provides query/filter/join operations for the GUI layer.
#[derive(Debug)]
pub struct Matchmaker {
    /// All known listings keyed by game ID.
    listings: HashMap<String, GameListing>,
    /// Game IDs that *we* created.
    my_games: Vec<String>,
    /// Game IDs that we have joined (but did not create).
    joined_games: Vec<String>,
    /// Our hex-encoded Nostr public key.
    our_pubkey: String,
}

impl Matchmaker {
    /// Create a new `Matchmaker` for the given local player.
    pub fn new(our_pubkey: &str) -> Self {
        Self {
            listings: HashMap::new(),
            my_games: Vec::new(),
            joined_games: Vec::new(),
            our_pubkey: our_pubkey.to_string(),
        }
    }

    /// Insert or replace a game listing.
    ///
    /// Returns `Err` if a listing with the same ID already exists.
    pub fn add_listing(&mut self, listing: GameListing) -> Result<(), MatchmakingError> {
        if self.listings.contains_key(&listing.game_id) {
            return Err(MatchmakingError::DuplicateListing(listing.game_id.clone()));
        }
        self.listings.insert(listing.game_id.clone(), listing);
        Ok(())
    }

    /// Remove a listing by game ID. Returns the removed listing if found.
    pub fn remove_listing(&mut self, game_id: &str) -> Option<GameListing> {
        self.listings.remove(game_id)
    }

    /// Browse listings using the given filter.
    ///
    /// Returns a vector of references to matching listings sorted by
    /// creation time (newest first).
    pub fn browse(&self, filter: &MatchmakingFilter) -> Vec<&GameListing> {
        let mut results: Vec<&GameListing> = self
            .listings
            .values()
            .filter(|l| filter.matches(l))
            .collect();
        // Newest first.
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        results
    }

    /// Look up a single listing by game ID.
    pub fn get_listing(&self, game_id: &str) -> Option<&GameListing> {
        self.listings.get(game_id)
    }

    /// Create a new game with the given settings and register it locally.
    ///
    /// Returns the generated [`GameListing`] (the caller is responsible for
    /// publishing the corresponding kind-4200 Nostr event).
    pub fn create_game(
        &mut self,
        game_id: &str,
        settings: GameSettings,
        is_private: bool,
    ) -> Result<GameListing, MatchmakingError> {
        if self.listings.contains_key(game_id) {
            return Err(MatchmakingError::DuplicateListing(game_id.to_string()));
        }
        let listing = GameListing {
            game_id: game_id.to_string(),
            creator_pubkey: self.our_pubkey.clone(),
            creator_name: None,
            settings,
            current_players: 1, // creator counts
            is_private,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            status: ListingStatus::Open,
        };
        self.listings
            .insert(listing.game_id.clone(), listing.clone());
        self.my_games.push(game_id.to_string());
        Ok(listing)
    }

    /// Join an existing game.
    ///
    /// Increments the player count and transitions to `Full` when
    /// `max_players` is reached. Returns a clone of the updated listing.
    pub fn join_game(&mut self, game_id: &str) -> Result<GameListing, MatchmakingError> {
        let listing = self
            .listings
            .get_mut(game_id)
            .ok_or_else(|| MatchmakingError::GameNotFound(game_id.to_string()))?;

        if listing.status != ListingStatus::Open {
            return Err(MatchmakingError::GameFull(game_id.to_string()));
        }

        let open = listing.settings.open_slots(listing.current_players);
        if open == 0 {
            return Err(MatchmakingError::GameFull(game_id.to_string()));
        }

        listing.current_players += 1;
        if listing.settings.open_slots(listing.current_players) == 0 {
            listing.status = ListingStatus::Full;
        }

        self.joined_games.push(game_id.to_string());
        Ok(listing.clone())
    }

    /// Total number of known listings.
    pub fn listing_count(&self) -> usize {
        self.listings.len()
    }

    /// Number of listings with `Open` status.
    pub fn open_games_count(&self) -> usize {
        self.listings
            .values()
            .filter(|l| l.status == ListingStatus::Open)
            .count()
    }

    /// Update the status of a listing.
    ///
    /// Returns `Err` if the game ID is not found.
    pub fn update_listing_status(
        &mut self,
        game_id: &str,
        status: ListingStatus,
    ) -> Result<(), MatchmakingError> {
        let listing = self
            .listings
            .get_mut(game_id)
            .ok_or_else(|| MatchmakingError::GameNotFound(game_id.to_string()))?;
        listing.status = status;
        Ok(())
    }

    /// Return IDs of games that *we* created.
    pub fn my_created_games(&self) -> &[String] {
        &self.my_games
    }

    /// Return IDs of games that we have joined (but did not create).
    pub fn my_joined_games(&self) -> &[String] {
        &self.joined_games
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers -----------------------------------------------------------

    fn default_settings() -> GameSettings {
        GameSettings::default()
    }

    fn make_listing(id: &str, ruleset: &str, max_players: u8, is_private: bool) -> GameListing {
        GameListing {
            game_id: id.to_string(),
            creator_pubkey: "creator_pk".to_string(),
            creator_name: None,
            settings: GameSettings {
                ruleset: ruleset.to_string(),
                max_players,
                ..default_settings()
            },
            current_players: 1,
            is_private,
            created_at: 1000,
            status: ListingStatus::Open,
        }
    }

    // -- GameSettings defaults --------------------------------------------

    #[test]
    fn game_settings_defaults() {
        let gs = GameSettings::default();
        assert_eq!(gs.ruleset, "civ2civ3");
        assert_eq!(gs.map_size, MapSize::Medium);
        assert_eq!(gs.max_players, 4);
        assert_eq!(gs.turn_timeout, 300);
        assert_eq!(gs.phase_mode, "concurrent");
        assert_eq!(gs.map_seed, 0);
        assert!(gs.description.is_none());
        assert_eq!(gs.ai_players, 0);
    }

    #[test]
    fn game_settings_open_slots() {
        let gs = GameSettings {
            max_players: 6,
            ..default_settings()
        };
        assert_eq!(gs.open_slots(1), 5);
        assert_eq!(gs.open_slots(6), 0);
        assert_eq!(gs.open_slots(0), 6);
        // saturating: more players than max should return 0
        assert_eq!(gs.open_slots(10), 0);
    }

    // -- ListingStatus ----------------------------------------------------

    #[test]
    fn listing_status_display() {
        assert_eq!(ListingStatus::Open.to_string(), "Open");
        assert_eq!(ListingStatus::Full.to_string(), "Full");
        assert_eq!(ListingStatus::InProgress.to_string(), "InProgress");
        assert_eq!(ListingStatus::Completed.to_string(), "Completed");
        assert_eq!(ListingStatus::Cancelled.to_string(), "Cancelled");
    }

    #[test]
    fn listing_status_serde_roundtrip() {
        for status in &[
            ListingStatus::Open,
            ListingStatus::Full,
            ListingStatus::InProgress,
            ListingStatus::Completed,
            ListingStatus::Cancelled,
        ] {
            let json = serde_json::to_string(status).unwrap();
            let back: ListingStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, back);
        }
    }

    // -- MapSize ----------------------------------------------------------

    #[test]
    fn map_size_display() {
        assert_eq!(MapSize::Tiny.to_string(), "Tiny");
        assert_eq!(MapSize::Custom(80, 50).to_string(), "Custom(80x50)");
    }

    #[test]
    fn map_size_serde_roundtrip() {
        for ms in &[
            MapSize::Tiny,
            MapSize::Small,
            MapSize::Medium,
            MapSize::Large,
            MapSize::Huge,
            MapSize::Custom(120, 80),
        ] {
            let json = serde_json::to_string(ms).unwrap();
            let back: MapSize = serde_json::from_str(&json).unwrap();
            assert_eq!(*ms, back);
        }
    }

    #[test]
    fn map_size_equality() {
        assert_eq!(MapSize::Tiny, MapSize::Tiny);
        assert_ne!(MapSize::Tiny, MapSize::Small);
        assert_eq!(MapSize::Custom(10, 20), MapSize::Custom(10, 20));
        assert_ne!(MapSize::Custom(10, 20), MapSize::Custom(20, 10));
    }

    // -- MatchmakingFilter ------------------------------------------------

    #[test]
    fn filter_default_matches_everything() {
        let filter = MatchmakingFilter::default();
        let listing = make_listing("g1", "civ2civ3", 4, false);
        assert!(filter.matches(&listing));
    }

    #[test]
    fn filter_by_ruleset() {
        let filter = MatchmakingFilter {
            ruleset: Some("classic".to_string()),
            ..Default::default()
        };
        let l1 = make_listing("g1", "classic", 4, false);
        let l2 = make_listing("g2", "civ2civ3", 4, false);
        assert!(filter.matches(&l1));
        assert!(!filter.matches(&l2));
    }

    #[test]
    fn filter_by_min_open_slots() {
        let filter = MatchmakingFilter {
            min_open_slots: Some(3),
            ..Default::default()
        };
        // max=4, current=1 => open=3
        let l1 = make_listing("g1", "civ2civ3", 4, false);
        assert!(filter.matches(&l1));

        // max=2, current=1 => open=1
        let l2 = make_listing("g2", "civ2civ3", 2, false);
        assert!(!filter.matches(&l2));
    }

    #[test]
    fn filter_by_max_players() {
        let filter = MatchmakingFilter {
            max_players: Some(4),
            ..Default::default()
        };
        let l1 = make_listing("g1", "civ2civ3", 4, false);
        let l2 = make_listing("g2", "civ2civ3", 8, false);
        assert!(filter.matches(&l1));
        assert!(!filter.matches(&l2));
    }

    #[test]
    fn filter_by_map_size() {
        let filter = MatchmakingFilter {
            map_size: Some(MapSize::Large),
            ..Default::default()
        };
        let mut listing = make_listing("g1", "civ2civ3", 4, false);
        listing.settings.map_size = MapSize::Large;
        assert!(filter.matches(&listing));

        listing.settings.map_size = MapSize::Small;
        assert!(!filter.matches(&listing));
    }

    #[test]
    fn filter_open_only() {
        let filter = MatchmakingFilter {
            open_only: true,
            ..Default::default()
        };
        let mut listing = make_listing("g1", "civ2civ3", 4, false);
        assert!(filter.matches(&listing));

        listing.status = ListingStatus::Full;
        assert!(!filter.matches(&listing));
    }

    #[test]
    fn filter_exclude_private() {
        let filter = MatchmakingFilter {
            exclude_private: true,
            ..Default::default()
        };
        let l_public = make_listing("g1", "civ2civ3", 4, false);
        let l_private = make_listing("g2", "civ2civ3", 4, true);
        assert!(filter.matches(&l_public));
        assert!(!filter.matches(&l_private));
    }

    #[test]
    fn filter_by_creator() {
        let filter = MatchmakingFilter {
            creator: Some("creator_pk".to_string()),
            ..Default::default()
        };
        let l1 = make_listing("g1", "civ2civ3", 4, false);
        assert!(filter.matches(&l1));

        let mut l2 = make_listing("g2", "civ2civ3", 4, false);
        l2.creator_pubkey = "other_pk".to_string();
        assert!(!filter.matches(&l2));
    }

    #[test]
    fn filter_combined() {
        let filter = MatchmakingFilter {
            ruleset: Some("classic".to_string()),
            open_only: true,
            exclude_private: true,
            min_open_slots: Some(2),
            ..Default::default()
        };
        // Matches: classic, open, public, max=4, current=1 => open=3 >= 2
        let l1 = make_listing("g1", "classic", 4, false);
        assert!(filter.matches(&l1));

        // Wrong ruleset
        let l2 = make_listing("g2", "civ2civ3", 4, false);
        assert!(!filter.matches(&l2));

        // Private
        let l3 = make_listing("g3", "classic", 4, true);
        assert!(!filter.matches(&l3));

        // Not enough open slots: max=2, current=1 => open=1 < 2
        let l4 = make_listing("g4", "classic", 2, false);
        assert!(!filter.matches(&l4));
    }

    // -- Matchmaker: new --------------------------------------------------

    #[test]
    fn matchmaker_new_is_empty() {
        let mm = Matchmaker::new("our_pk");
        assert_eq!(mm.listing_count(), 0);
        assert_eq!(mm.open_games_count(), 0);
        assert!(mm.my_created_games().is_empty());
        assert!(mm.my_joined_games().is_empty());
    }

    // -- Matchmaker: add_listing ------------------------------------------

    #[test]
    fn matchmaker_add_listing() {
        let mut mm = Matchmaker::new("our_pk");
        let listing = make_listing("g1", "civ2civ3", 4, false);
        assert!(mm.add_listing(listing).is_ok());
        assert_eq!(mm.listing_count(), 1);
    }

    #[test]
    fn matchmaker_add_duplicate_fails() {
        let mut mm = Matchmaker::new("our_pk");
        let l1 = make_listing("g1", "civ2civ3", 4, false);
        let l2 = make_listing("g1", "classic", 8, true);
        assert!(mm.add_listing(l1).is_ok());
        let err = mm.add_listing(l2).unwrap_err();
        assert!(matches!(err, MatchmakingError::DuplicateListing(_)));
    }

    // -- Matchmaker: remove_listing ---------------------------------------

    #[test]
    fn matchmaker_remove_listing() {
        let mut mm = Matchmaker::new("our_pk");
        mm.add_listing(make_listing("g1", "civ2civ3", 4, false))
            .unwrap();
        let removed = mm.remove_listing("g1");
        assert!(removed.is_some());
        assert_eq!(mm.listing_count(), 0);
    }

    #[test]
    fn matchmaker_remove_nonexistent_returns_none() {
        let mut mm = Matchmaker::new("our_pk");
        assert!(mm.remove_listing("ghost").is_none());
    }

    // -- Matchmaker: browse -----------------------------------------------

    #[test]
    fn matchmaker_browse_all() {
        let mut mm = Matchmaker::new("our_pk");
        let mut l1 = make_listing("g1", "civ2civ3", 4, false);
        l1.created_at = 100;
        let mut l2 = make_listing("g2", "classic", 8, false);
        l2.created_at = 200;
        mm.add_listing(l1).unwrap();
        mm.add_listing(l2).unwrap();

        let filter = MatchmakingFilter::default();
        let results = mm.browse(&filter);
        assert_eq!(results.len(), 2);
        // Newest first
        assert_eq!(results[0].game_id, "g2");
        assert_eq!(results[1].game_id, "g1");
    }

    #[test]
    fn matchmaker_browse_filtered() {
        let mut mm = Matchmaker::new("our_pk");
        mm.add_listing(make_listing("g1", "civ2civ3", 4, false))
            .unwrap();
        mm.add_listing(make_listing("g2", "classic", 4, false))
            .unwrap();

        let filter = MatchmakingFilter {
            ruleset: Some("classic".to_string()),
            ..Default::default()
        };
        let results = mm.browse(&filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].game_id, "g2");
    }

    // -- Matchmaker: get_listing ------------------------------------------

    #[test]
    fn matchmaker_get_listing_found() {
        let mut mm = Matchmaker::new("our_pk");
        mm.add_listing(make_listing("g1", "civ2civ3", 4, false))
            .unwrap();
        let l = mm.get_listing("g1");
        assert!(l.is_some());
        assert_eq!(l.unwrap().game_id, "g1");
    }

    #[test]
    fn matchmaker_get_listing_not_found() {
        let mm = Matchmaker::new("our_pk");
        assert!(mm.get_listing("ghost").is_none());
    }

    // -- Matchmaker: create_game ------------------------------------------

    #[test]
    fn matchmaker_create_game() {
        let mut mm = Matchmaker::new("our_pk");
        let listing = mm.create_game("g1", default_settings(), false).unwrap();
        assert_eq!(listing.game_id, "g1");
        assert_eq!(listing.creator_pubkey, "our_pk");
        assert_eq!(listing.current_players, 1);
        assert_eq!(listing.status, ListingStatus::Open);
        assert!(!listing.is_private);
        assert_eq!(mm.listing_count(), 1);
        assert_eq!(mm.my_created_games(), &["g1"]);
    }

    #[test]
    fn matchmaker_create_game_private() {
        let mut mm = Matchmaker::new("our_pk");
        let listing = mm.create_game("g1", default_settings(), true).unwrap();
        assert!(listing.is_private);
    }

    #[test]
    fn matchmaker_create_game_duplicate_fails() {
        let mut mm = Matchmaker::new("our_pk");
        mm.create_game("g1", default_settings(), false).unwrap();
        let err = mm.create_game("g1", default_settings(), false).unwrap_err();
        assert!(matches!(err, MatchmakingError::DuplicateListing(_)));
    }

    // -- Matchmaker: join_game --------------------------------------------

    #[test]
    fn matchmaker_join_game() {
        let mut mm = Matchmaker::new("our_pk");
        mm.add_listing(make_listing("g1", "civ2civ3", 4, false))
            .unwrap();

        let updated = mm.join_game("g1").unwrap();
        assert_eq!(updated.current_players, 2);
        assert_eq!(updated.status, ListingStatus::Open);
        assert_eq!(mm.my_joined_games(), &["g1"]);
    }

    #[test]
    fn matchmaker_join_game_fills_to_full() {
        let mut mm = Matchmaker::new("our_pk");
        // max_players=2, current=1 => 1 open slot
        mm.add_listing(make_listing("g1", "civ2civ3", 2, false))
            .unwrap();

        let updated = mm.join_game("g1").unwrap();
        assert_eq!(updated.current_players, 2);
        assert_eq!(updated.status, ListingStatus::Full);
    }

    #[test]
    fn matchmaker_join_nonexistent_game_fails() {
        let mut mm = Matchmaker::new("our_pk");
        let err = mm.join_game("ghost").unwrap_err();
        assert!(matches!(err, MatchmakingError::GameNotFound(_)));
    }

    #[test]
    fn matchmaker_join_full_game_fails() {
        let mut mm = Matchmaker::new("our_pk");
        let mut listing = make_listing("g1", "civ2civ3", 2, false);
        listing.current_players = 2;
        listing.status = ListingStatus::Full;
        mm.add_listing(listing).unwrap();

        let err = mm.join_game("g1").unwrap_err();
        assert!(matches!(err, MatchmakingError::GameFull(_)));
    }

    // -- Matchmaker: counts -----------------------------------------------

    #[test]
    fn matchmaker_listing_and_open_count() {
        let mut mm = Matchmaker::new("our_pk");
        mm.add_listing(make_listing("g1", "civ2civ3", 4, false))
            .unwrap();
        let mut full = make_listing("g2", "classic", 2, false);
        full.status = ListingStatus::Full;
        mm.add_listing(full).unwrap();

        assert_eq!(mm.listing_count(), 2);
        assert_eq!(mm.open_games_count(), 1);
    }

    // -- Matchmaker: update_listing_status --------------------------------

    #[test]
    fn matchmaker_update_listing_status() {
        let mut mm = Matchmaker::new("our_pk");
        mm.add_listing(make_listing("g1", "civ2civ3", 4, false))
            .unwrap();

        assert!(
            mm.update_listing_status("g1", ListingStatus::InProgress)
                .is_ok()
        );
        assert_eq!(
            mm.get_listing("g1").unwrap().status,
            ListingStatus::InProgress
        );
    }

    #[test]
    fn matchmaker_update_status_not_found() {
        let mut mm = Matchmaker::new("our_pk");
        let err = mm
            .update_listing_status("ghost", ListingStatus::Cancelled)
            .unwrap_err();
        assert!(matches!(err, MatchmakingError::GameNotFound(_)));
    }

    // -- Matchmaker: listing_status transitions ---------------------------

    #[test]
    fn listing_status_transition_open_to_in_progress() {
        let mut mm = Matchmaker::new("our_pk");
        mm.create_game("g1", default_settings(), false).unwrap();
        mm.update_listing_status("g1", ListingStatus::InProgress)
            .unwrap();
        assert_eq!(
            mm.get_listing("g1").unwrap().status,
            ListingStatus::InProgress
        );
    }

    #[test]
    fn listing_status_transition_in_progress_to_completed() {
        let mut mm = Matchmaker::new("our_pk");
        mm.create_game("g1", default_settings(), false).unwrap();
        mm.update_listing_status("g1", ListingStatus::InProgress)
            .unwrap();
        mm.update_listing_status("g1", ListingStatus::Completed)
            .unwrap();
        assert_eq!(
            mm.get_listing("g1").unwrap().status,
            ListingStatus::Completed
        );
    }

    #[test]
    fn listing_status_transition_open_to_cancelled() {
        let mut mm = Matchmaker::new("our_pk");
        mm.create_game("g1", default_settings(), false).unwrap();
        mm.update_listing_status("g1", ListingStatus::Cancelled)
            .unwrap();
        assert_eq!(
            mm.get_listing("g1").unwrap().status,
            ListingStatus::Cancelled
        );
    }

    // -- Matchmaker: my_created_games / my_joined_games -------------------

    #[test]
    fn matchmaker_tracks_created_and_joined() {
        let mut mm = Matchmaker::new("our_pk");
        mm.create_game("g1", default_settings(), false).unwrap();
        mm.add_listing(make_listing("g2", "civ2civ3", 4, false))
            .unwrap();
        mm.join_game("g2").unwrap();

        assert_eq!(mm.my_created_games(), &["g1"]);
        assert_eq!(mm.my_joined_games(), &["g2"]);
    }

    // -- Matchmaker: browse with multiple filters -------------------------

    #[test]
    fn browse_with_multiple_filters() {
        let mut mm = Matchmaker::new("our_pk");

        // g1: classic, public, max=4, open
        let mut l1 = make_listing("g1", "classic", 4, false);
        l1.created_at = 300;
        mm.add_listing(l1).unwrap();

        // g2: civ2civ3, private, max=8, open
        let mut l2 = make_listing("g2", "civ2civ3", 8, true);
        l2.created_at = 200;
        mm.add_listing(l2).unwrap();

        // g3: classic, public, max=2, full
        let mut l3 = make_listing("g3", "classic", 2, false);
        l3.created_at = 100;
        l3.status = ListingStatus::Full;
        mm.add_listing(l3).unwrap();

        // g4: classic, public, max=6, open, different creator
        let mut l4 = make_listing("g4", "classic", 6, false);
        l4.created_at = 400;
        l4.creator_pubkey = "other_pk".to_string();
        mm.add_listing(l4).unwrap();

        let filter = MatchmakingFilter {
            ruleset: Some("classic".to_string()),
            open_only: true,
            exclude_private: true,
            ..Default::default()
        };
        let results = mm.browse(&filter);
        assert_eq!(results.len(), 2);
        // Newest first
        assert_eq!(results[0].game_id, "g4");
        assert_eq!(results[1].game_id, "g1");
    }

    // -- GameListing serde ------------------------------------------------

    #[test]
    fn game_listing_serde_roundtrip() {
        let listing = make_listing("g1", "civ2civ3", 4, false);
        let json = serde_json::to_string(&listing).unwrap();
        let back: GameListing = serde_json::from_str(&json).unwrap();
        assert_eq!(back.game_id, "g1");
        assert_eq!(back.settings.ruleset, "civ2civ3");
        assert_eq!(back.settings.max_players, 4);
        assert!(!back.is_private);
    }

    // -- Edge case: join game that is InProgress --------------------------

    #[test]
    fn matchmaker_join_in_progress_game_fails() {
        let mut mm = Matchmaker::new("our_pk");
        let mut listing = make_listing("g1", "civ2civ3", 4, false);
        listing.status = ListingStatus::InProgress;
        mm.add_listing(listing).unwrap();

        let err = mm.join_game("g1").unwrap_err();
        assert!(matches!(err, MatchmakingError::GameFull(_)));
    }
}

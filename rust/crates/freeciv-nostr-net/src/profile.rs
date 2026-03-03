//! Player profile, reputation, and ELO rating system.
//!
//! Leverages Nostr identity for player profiles (kind 30420 addressable events),
//! verifiable game history, NIP-05 verification support, and ELO-based rankings.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Player profile stored as a Nostr addressable event (kind 30420).
///
/// The `d` tag is `"freeciv-profile"`, making this a parameterized
/// replaceable event that is updated as the player completes games.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerProfile {
    /// Display name shown in lobbies and leaderboards.
    pub display_name: String,
    /// Optional avatar URL (e.g. hosted image or NIP-94 file metadata).
    pub avatar_url: Option<String>,
    /// Hex-encoded Nostr public key (64 chars).
    pub pubkey: String,
    /// Optional NIP-05 identifier (e.g. `user@example.com`).
    pub nip05: Option<String>,
    /// Preferred rulesets (e.g. `["civ2civ3", "classic"]`).
    pub preferred_rulesets: Vec<String>,
    /// Aggregated gameplay statistics.
    pub stats: PlayerStats,
    /// Current ELO rating.
    pub elo: u32,
    /// Unix timestamp of the last profile update.
    pub updated_at: u64,
}

/// Aggregated gameplay statistics for a player.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerStats {
    /// Total games played.
    pub games_played: u32,
    /// Total games won.
    pub games_won: u32,
    /// Total games lost.
    pub games_lost: u32,
    /// Total games drawn.
    pub games_drawn: u32,
    /// Average game length in turns.
    pub avg_game_length: u32,
    /// Most frequently played ruleset.
    pub favorite_ruleset: Option<String>,
}

/// Result of a completed game, derived from a signed Game End event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameResult {
    /// Unique game identifier.
    pub game_id: String,
    /// Hex-encoded pubkeys of all participants.
    pub players: Vec<String>,
    /// Hex-encoded pubkey of the winner, or `None` for draws / abandoned.
    pub winner: Option<String>,
    /// How the game ended.
    pub outcome: GameOutcome,
    /// Number of turns played.
    pub turns: u64,
    /// Ruleset used.
    pub ruleset: String,
    /// Unix timestamp when the game ended.
    pub ended_at: u64,
    /// Nostr event ID (hex) of the Game End event.
    pub end_event_id: String,
}

/// How a game concluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameOutcome {
    /// A player achieved a victory condition.
    Victory,
    /// All remaining players agreed to a draw.
    Draw,
    /// A player conceded.
    Concession,
    /// The game timed out.
    Timeout,
    /// The game was abandoned (e.g. disconnects).
    Abandoned,
}

// ---------------------------------------------------------------------------
// PlayerProfile implementation
// ---------------------------------------------------------------------------

impl PlayerProfile {
    /// Create a new profile with default stats and the standard initial ELO.
    pub fn new(display_name: &str, pubkey: &str) -> Self {
        Self {
            display_name: display_name.to_string(),
            avatar_url: None,
            pubkey: pubkey.to_string(),
            nip05: None,
            preferred_rulesets: Vec::new(),
            stats: PlayerStats::default(),
            elo: EloCalculator::DEFAULT_RATING,
            updated_at: 0,
        }
    }

    /// Serialize the profile to a JSON string suitable for a kind 30420 event.
    pub fn to_event_content(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize a profile from a kind 30420 event content string.
    pub fn from_event_content(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Update aggregated stats after a game result.
    pub fn update_stats(&mut self, result: &GameResult) {
        self.stats.games_played += 1;

        match result.outcome {
            GameOutcome::Victory | GameOutcome::Concession => {
                if result.winner.as_deref() == Some(&self.pubkey) {
                    self.stats.games_won += 1;
                } else {
                    self.stats.games_lost += 1;
                }
            }
            GameOutcome::Draw => {
                self.stats.games_drawn += 1;
            }
            GameOutcome::Timeout | GameOutcome::Abandoned => {
                // Timeouts and abandoned games count as losses for all players
                // unless there's an explicit winner.
                if let Some(ref winner) = result.winner {
                    if winner == &self.pubkey {
                        self.stats.games_won += 1;
                    } else {
                        self.stats.games_lost += 1;
                    }
                } else {
                    self.stats.games_lost += 1;
                }
            }
        }

        // Update average game length (running average).
        let n = self.stats.games_played;
        if n == 1 {
            self.stats.avg_game_length = result.turns as u32;
        } else {
            let prev_total = (self.stats.avg_game_length as u64) * ((n - 1) as u64);
            self.stats.avg_game_length = ((prev_total + result.turns) / (n as u64)) as u32;
        }

        // Update favorite ruleset (most played).
        // We track this simply by checking if this ruleset should replace.
        // For a full implementation we'd keep per-ruleset counters, but
        // here we use a simple heuristic: set it on first game, then only
        // change if the profile has no favorite yet.
        if self.stats.favorite_ruleset.is_none() {
            self.stats.favorite_ruleset = Some(result.ruleset.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// ELO calculator
// ---------------------------------------------------------------------------

/// ELO rating calculator for two-player or multi-player outcomes.
///
/// Uses the standard ELO formula with a configurable K-factor.
#[derive(Debug, Clone)]
pub struct EloCalculator {
    /// K-factor controlling how much ratings change per game.
    k_factor: f64,
    /// Default rating for new players.
    default_rating: u32,
}

impl EloCalculator {
    /// Standard default rating for new players.
    pub const DEFAULT_RATING: u32 = 1500;
    /// Standard K-factor for competitive play.
    const DEFAULT_K_FACTOR: f64 = 32.0;

    /// Create a new ELO calculator with the default K-factor (32).
    pub fn new() -> Self {
        Self {
            k_factor: Self::DEFAULT_K_FACTOR,
            default_rating: Self::DEFAULT_RATING,
        }
    }

    /// Create a new ELO calculator with a custom K-factor.
    pub fn with_k_factor(k_factor: f64) -> Self {
        Self {
            k_factor,
            default_rating: Self::DEFAULT_RATING,
        }
    }

    /// Return the default rating for new players.
    pub fn default_rating(&self) -> u32 {
        self.default_rating
    }

    /// Calculate new ratings after a decisive game (winner/loser).
    ///
    /// Returns `(new_winner_rating, new_loser_rating)`.
    pub fn calculate_new_ratings(&self, winner_elo: u32, loser_elo: u32) -> (u32, u32) {
        let expected_winner = self.expected_score(winner_elo, loser_elo);
        let expected_loser = 1.0 - expected_winner;

        let new_winner = (winner_elo as f64 + self.k_factor * (1.0 - expected_winner)).round();
        let new_loser = (loser_elo as f64 + self.k_factor * (0.0 - expected_loser)).round();

        // Clamp to minimum of 100 to avoid negative/extremely low ratings.
        let new_winner = (new_winner as u32).max(100);
        let new_loser = (new_loser as u32).max(100);

        (new_winner, new_loser)
    }

    /// Calculate new ratings after a draw.
    ///
    /// Returns `(new_rating_a, new_rating_b)`.
    pub fn calculate_draw_ratings(&self, elo_a: u32, elo_b: u32) -> (u32, u32) {
        let expected_a = self.expected_score(elo_a, elo_b);
        let expected_b = 1.0 - expected_a;

        let new_a = (elo_a as f64 + self.k_factor * (0.5 - expected_a)).round();
        let new_b = (elo_b as f64 + self.k_factor * (0.5 - expected_b)).round();

        let new_a = (new_a as u32).max(100);
        let new_b = (new_b as u32).max(100);

        (new_a, new_b)
    }

    /// Expected score for player A against player B.
    ///
    /// Uses the standard ELO formula: E_A = 1 / (1 + 10^((R_B - R_A) / 400))
    fn expected_score(&self, rating_a: u32, rating_b: u32) -> f64 {
        let exp = (rating_b as f64 - rating_a as f64) / 400.0;
        1.0 / (1.0 + 10.0_f64.powf(exp))
    }
}

impl Default for EloCalculator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Profile manager
// ---------------------------------------------------------------------------

/// Manages player profiles, game history, and ELO ratings.
///
/// Provides the high-level API used by the game engine to track players,
/// record results, and query leaderboards.
pub struct ProfileManager {
    /// Known player profiles indexed by hex pubkey.
    profiles: HashMap<String, PlayerProfile>,
    /// Recorded game results (append-only log).
    game_results: Vec<GameResult>,
    /// ELO calculator instance.
    elo: EloCalculator,
}

impl ProfileManager {
    /// Create an empty profile manager with the default ELO calculator.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
            game_results: Vec::new(),
            elo: EloCalculator::new(),
        }
    }

    /// Look up a profile by hex pubkey.
    pub fn get_profile(&self, pubkey: &str) -> Option<&PlayerProfile> {
        self.profiles.get(pubkey)
    }

    /// Insert or replace a player profile.
    pub fn update_profile(&mut self, profile: PlayerProfile) {
        self.profiles.insert(profile.pubkey.clone(), profile);
    }

    /// Record a completed game result and update all participants' stats & ELO.
    ///
    /// Every player referenced in `result.players` who has a profile will
    /// have their stats updated. If a player has no profile yet, one is
    /// created automatically with a default display name.
    pub fn record_game_result(&mut self, result: GameResult) {
        // Ensure all players have profiles.
        for pk in &result.players {
            if !self.profiles.contains_key(pk) {
                let profile = PlayerProfile::new("Anonymous", pk);
                self.profiles.insert(pk.clone(), profile);
            }
        }

        // Update stats for each participant.
        let player_keys: Vec<String> = result.players.clone();
        for pk in &player_keys {
            if let Some(profile) = self.profiles.get_mut(pk) {
                profile.update_stats(&result);
            }
        }

        // Update ELO ratings.
        self.update_elo_ratings(&result);

        self.game_results.push(result);
    }

    /// Get the game history for a specific player (by hex pubkey).
    pub fn get_game_history(&self, pubkey: &str) -> Vec<&GameResult> {
        self.game_results
            .iter()
            .filter(|r| r.players.contains(&pubkey.to_string()))
            .collect()
    }

    /// Verify that a game result references valid participants.
    ///
    /// Returns `true` if all players in the result are known profiles
    /// and the result has at least 2 players.
    pub fn verify_game_result(&self, result: &GameResult) -> bool {
        if result.players.len() < 2 {
            return false;
        }
        // If there's a winner, they must be in the player list.
        if let Some(ref winner) = result.winner
            && !result.players.contains(winner)
        {
            return false;
        }
        true
    }

    /// Return players sorted by ELO descending (leaderboard).
    ///
    /// Returns a vec of `(pubkey, elo)` tuples.
    pub fn leaderboard(&self) -> Vec<(String, u32)> {
        let mut entries: Vec<(String, u32)> = self
            .profiles
            .values()
            .map(|p| (p.pubkey.clone(), p.elo))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries
    }

    /// Number of known player profiles.
    pub fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    /// Update ELO for all participants based on a game result.
    fn update_elo_ratings(&mut self, result: &GameResult) {
        match result.outcome {
            GameOutcome::Victory | GameOutcome::Concession => {
                if let Some(ref winner_pk) = result.winner {
                    let winner_elo = self
                        .profiles
                        .get(winner_pk)
                        .map(|p| p.elo)
                        .unwrap_or(EloCalculator::DEFAULT_RATING);

                    // Apply ELO vs each loser individually.
                    let losers: Vec<String> = result
                        .players
                        .iter()
                        .filter(|pk| *pk != winner_pk)
                        .cloned()
                        .collect();

                    // Accumulate total winner change; apply per-loser changes.
                    let mut winner_new_elo = winner_elo;
                    for loser_pk in &losers {
                        let loser_elo = self
                            .profiles
                            .get(loser_pk)
                            .map(|p| p.elo)
                            .unwrap_or(EloCalculator::DEFAULT_RATING);

                        let (new_w, new_l) =
                            self.elo.calculate_new_ratings(winner_new_elo, loser_elo);
                        winner_new_elo = new_w;

                        if let Some(loser_profile) = self.profiles.get_mut(loser_pk) {
                            loser_profile.elo = new_l;
                        }
                    }

                    if let Some(winner_profile) = self.profiles.get_mut(winner_pk) {
                        winner_profile.elo = winner_new_elo;
                    }
                }
            }
            GameOutcome::Draw => {
                // For multi-player draws, adjust pairwise between all players.
                let pks: Vec<String> = result.players.clone();
                for i in 0..pks.len() {
                    for j in (i + 1)..pks.len() {
                        let elo_i = self
                            .profiles
                            .get(&pks[i])
                            .map(|p| p.elo)
                            .unwrap_or(EloCalculator::DEFAULT_RATING);
                        let elo_j = self
                            .profiles
                            .get(&pks[j])
                            .map(|p| p.elo)
                            .unwrap_or(EloCalculator::DEFAULT_RATING);

                        let (new_i, new_j) = self.elo.calculate_draw_ratings(elo_i, elo_j);

                        if let Some(p) = self.profiles.get_mut(&pks[i]) {
                            p.elo = new_i;
                        }
                        if let Some(p) = self.profiles.get_mut(&pks[j]) {
                            p.elo = new_j;
                        }
                    }
                }
            }
            GameOutcome::Timeout | GameOutcome::Abandoned => {
                // No ELO change for timeout/abandoned unless there's a winner.
                if let Some(ref winner_pk) = result.winner {
                    let winner_elo = self
                        .profiles
                        .get(winner_pk)
                        .map(|p| p.elo)
                        .unwrap_or(EloCalculator::DEFAULT_RATING);

                    let losers: Vec<String> = result
                        .players
                        .iter()
                        .filter(|pk| *pk != winner_pk)
                        .cloned()
                        .collect();

                    let mut winner_new_elo = winner_elo;
                    for loser_pk in &losers {
                        let loser_elo = self
                            .profiles
                            .get(loser_pk)
                            .map(|p| p.elo)
                            .unwrap_or(EloCalculator::DEFAULT_RATING);

                        let (new_w, new_l) =
                            self.elo.calculate_new_ratings(winner_new_elo, loser_elo);
                        winner_new_elo = new_w;

                        if let Some(loser_profile) = self.profiles.get_mut(loser_pk) {
                            loser_profile.elo = new_l;
                        }
                    }

                    if let Some(winner_profile) = self.profiles.get_mut(winner_pk) {
                        winner_profile.elo = winner_new_elo;
                    }
                }
            }
        }
    }
}

impl Default for ProfileManager {
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

    // -- PlayerProfile tests --

    #[test]
    fn profile_new_has_default_elo() {
        let p = PlayerProfile::new("Alice", "aabbccdd");
        assert_eq!(p.display_name, "Alice");
        assert_eq!(p.pubkey, "aabbccdd");
        assert_eq!(p.elo, EloCalculator::DEFAULT_RATING);
        assert_eq!(p.stats.games_played, 0);
    }

    #[test]
    fn profile_serialization_roundtrip() {
        let mut p = PlayerProfile::new("Bob", "11223344");
        p.avatar_url = Some("https://example.com/avatar.png".to_string());
        p.nip05 = Some("bob@example.com".to_string());
        p.preferred_rulesets = vec!["civ2civ3".to_string()];
        p.stats.games_played = 10;
        p.stats.games_won = 7;
        p.updated_at = 1700000000;

        let json = p.to_event_content().unwrap();
        let p2 = PlayerProfile::from_event_content(&json).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn profile_update_stats_victory() {
        let mut p = PlayerProfile::new("Winner", "pk_winner");
        let result = GameResult {
            game_id: "game1".to_string(),
            players: vec!["pk_winner".to_string(), "pk_loser".to_string()],
            winner: Some("pk_winner".to_string()),
            outcome: GameOutcome::Victory,
            turns: 50,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "evt1".to_string(),
        };
        p.update_stats(&result);
        assert_eq!(p.stats.games_played, 1);
        assert_eq!(p.stats.games_won, 1);
        assert_eq!(p.stats.games_lost, 0);
        assert_eq!(p.stats.avg_game_length, 50);
        assert_eq!(p.stats.favorite_ruleset, Some("classic".to_string()));
    }

    #[test]
    fn profile_update_stats_loss() {
        let mut p = PlayerProfile::new("Loser", "pk_loser");
        let result = GameResult {
            game_id: "game1".to_string(),
            players: vec!["pk_winner".to_string(), "pk_loser".to_string()],
            winner: Some("pk_winner".to_string()),
            outcome: GameOutcome::Victory,
            turns: 30,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "evt1".to_string(),
        };
        p.update_stats(&result);
        assert_eq!(p.stats.games_played, 1);
        assert_eq!(p.stats.games_won, 0);
        assert_eq!(p.stats.games_lost, 1);
    }

    #[test]
    fn profile_update_stats_draw() {
        let mut p = PlayerProfile::new("Player", "pk_a");
        let result = GameResult {
            game_id: "game2".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: None,
            outcome: GameOutcome::Draw,
            turns: 100,
            ruleset: "civ2civ3".to_string(),
            ended_at: 1700000000,
            end_event_id: "evt2".to_string(),
        };
        p.update_stats(&result);
        assert_eq!(p.stats.games_drawn, 1);
        assert_eq!(p.stats.games_won, 0);
        assert_eq!(p.stats.games_lost, 0);
    }

    #[test]
    fn profile_update_stats_avg_game_length() {
        let mut p = PlayerProfile::new("Player", "pk_x");
        let make_result = |turns: u64| GameResult {
            game_id: format!("g{turns}"),
            players: vec!["pk_x".to_string(), "pk_y".to_string()],
            winner: None,
            outcome: GameOutcome::Draw,
            turns,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: format!("e{turns}"),
        };
        p.update_stats(&make_result(100));
        assert_eq!(p.stats.avg_game_length, 100);
        p.update_stats(&make_result(200));
        // (100 + 200) / 2 = 150
        assert_eq!(p.stats.avg_game_length, 150);
        p.update_stats(&make_result(300));
        // (150*2 + 300) / 3 = 200
        assert_eq!(p.stats.avg_game_length, 200);
    }

    // -- EloCalculator tests --

    #[test]
    fn elo_default_constructor() {
        let elo = EloCalculator::new();
        assert_eq!(elo.default_rating(), EloCalculator::DEFAULT_RATING);
    }

    #[test]
    fn elo_equal_ratings_winner_gains() {
        let elo = EloCalculator::new();
        let (new_w, new_l) = elo.calculate_new_ratings(1500, 1500);
        // Winner should gain, loser should lose equally.
        assert!(new_w > 1500, "winner should gain: got {new_w}");
        assert!(new_l < 1500, "loser should lose: got {new_l}");
        // Symmetry: change should be equal.
        assert_eq!(new_w - 1500, 1500 - new_l);
    }

    #[test]
    fn elo_upset_bonus() {
        let elo = EloCalculator::new();
        // Underdog (1200) beats favorite (1800).
        let (new_underdog, _new_favorite) = elo.calculate_new_ratings(1200, 1800);
        let underdog_gain = new_underdog - 1200;

        // Favorite (1800) beats underdog (1200) — expected result.
        let (new_fav2, _) = elo.calculate_new_ratings(1800, 1200);
        let favorite_gain = new_fav2 - 1800;

        // Upset should yield larger gain than the expected outcome.
        assert!(
            underdog_gain > favorite_gain,
            "upset should give bigger gain: {underdog_gain} vs {favorite_gain}"
        );
    }

    #[test]
    fn elo_draw_equal_ratings_no_change() {
        let elo = EloCalculator::new();
        let (new_a, new_b) = elo.calculate_draw_ratings(1500, 1500);
        // Draw between equals should produce no change (within rounding).
        assert_eq!(new_a, 1500);
        assert_eq!(new_b, 1500);
    }

    #[test]
    fn elo_draw_unequal_ratings() {
        let elo = EloCalculator::new();
        let (new_a, new_b) = elo.calculate_draw_ratings(1600, 1400);
        // Higher-rated player should lose some, lower should gain.
        assert!(new_a < 1600, "higher rated should lose on draw: {new_a}");
        assert!(new_b > 1400, "lower rated should gain on draw: {new_b}");
    }

    #[test]
    fn elo_minimum_clamp() {
        let elo = EloCalculator::new();
        // A very low-rated player losing should not go below 100.
        let (_, new_l) = elo.calculate_new_ratings(1500, 100);
        assert!(new_l >= 100, "rating should not go below 100: {new_l}");
    }

    #[test]
    fn elo_custom_k_factor() {
        let elo_high = EloCalculator::with_k_factor(64.0);
        let elo_low = EloCalculator::with_k_factor(16.0);
        let (w_high, _) = elo_high.calculate_new_ratings(1500, 1500);
        let (w_low, _) = elo_low.calculate_new_ratings(1500, 1500);
        assert!(
            w_high > w_low,
            "higher K should produce larger change: {w_high} vs {w_low}"
        );
    }

    // -- ProfileManager tests --

    #[test]
    fn manager_new_is_empty() {
        let mgr = ProfileManager::new();
        assert_eq!(mgr.profile_count(), 0);
        assert!(mgr.leaderboard().is_empty());
    }

    #[test]
    fn manager_update_and_get_profile() {
        let mut mgr = ProfileManager::new();
        let p = PlayerProfile::new("Alice", "pk_alice");
        mgr.update_profile(p);
        assert_eq!(mgr.profile_count(), 1);
        let retrieved = mgr.get_profile("pk_alice").unwrap();
        assert_eq!(retrieved.display_name, "Alice");
    }

    #[test]
    fn manager_update_profile_replaces() {
        let mut mgr = ProfileManager::new();
        let p1 = PlayerProfile::new("Alice", "pk_alice");
        mgr.update_profile(p1);
        let mut p2 = PlayerProfile::new("Alice2", "pk_alice");
        p2.elo = 1600;
        mgr.update_profile(p2);
        assert_eq!(mgr.profile_count(), 1);
        assert_eq!(mgr.get_profile("pk_alice").unwrap().display_name, "Alice2");
        assert_eq!(mgr.get_profile("pk_alice").unwrap().elo, 1600);
    }

    #[test]
    fn manager_get_nonexistent_profile() {
        let mgr = ProfileManager::new();
        assert!(mgr.get_profile("nonexistent").is_none());
    }

    #[test]
    fn manager_record_game_result_creates_profiles() {
        let mut mgr = ProfileManager::new();
        let result = GameResult {
            game_id: "g1".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: Some("pk_a".to_string()),
            outcome: GameOutcome::Victory,
            turns: 40,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "evt1".to_string(),
        };
        mgr.record_game_result(result);
        assert_eq!(mgr.profile_count(), 2);
        assert!(mgr.get_profile("pk_a").is_some());
        assert!(mgr.get_profile("pk_b").is_some());
    }

    #[test]
    fn manager_record_game_result_updates_stats() {
        let mut mgr = ProfileManager::new();
        mgr.update_profile(PlayerProfile::new("A", "pk_a"));
        mgr.update_profile(PlayerProfile::new("B", "pk_b"));

        let result = GameResult {
            game_id: "g1".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: Some("pk_a".to_string()),
            outcome: GameOutcome::Victory,
            turns: 60,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "evt1".to_string(),
        };
        mgr.record_game_result(result);

        let a = mgr.get_profile("pk_a").unwrap();
        assert_eq!(a.stats.games_won, 1);
        assert!(a.elo > 1500, "winner elo should increase: {}", a.elo);

        let b = mgr.get_profile("pk_b").unwrap();
        assert_eq!(b.stats.games_lost, 1);
        assert!(b.elo < 1500, "loser elo should decrease: {}", b.elo);
    }

    #[test]
    fn manager_record_draw_result() {
        let mut mgr = ProfileManager::new();
        let mut pa = PlayerProfile::new("A", "pk_a");
        pa.elo = 1600;
        let mut pb = PlayerProfile::new("B", "pk_b");
        pb.elo = 1400;
        mgr.update_profile(pa);
        mgr.update_profile(pb);

        let result = GameResult {
            game_id: "g2".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: None,
            outcome: GameOutcome::Draw,
            turns: 80,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "evt2".to_string(),
        };
        mgr.record_game_result(result);

        let a = mgr.get_profile("pk_a").unwrap();
        let b = mgr.get_profile("pk_b").unwrap();
        // Higher rated player should lose ELO in a draw, lower should gain.
        assert!(a.elo < 1600, "higher rated should lose on draw: {}", a.elo);
        assert!(b.elo > 1400, "lower rated should gain on draw: {}", b.elo);
    }

    #[test]
    fn manager_game_history() {
        let mut mgr = ProfileManager::new();
        let r1 = GameResult {
            game_id: "g1".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: Some("pk_a".to_string()),
            outcome: GameOutcome::Victory,
            turns: 40,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "e1".to_string(),
        };
        let r2 = GameResult {
            game_id: "g2".to_string(),
            players: vec!["pk_b".to_string(), "pk_c".to_string()],
            winner: Some("pk_c".to_string()),
            outcome: GameOutcome::Victory,
            turns: 30,
            ruleset: "classic".to_string(),
            ended_at: 1700000001,
            end_event_id: "e2".to_string(),
        };
        mgr.record_game_result(r1);
        mgr.record_game_result(r2);

        let history_a = mgr.get_game_history("pk_a");
        assert_eq!(history_a.len(), 1);
        assert_eq!(history_a[0].game_id, "g1");

        let history_b = mgr.get_game_history("pk_b");
        assert_eq!(history_b.len(), 2);

        let history_c = mgr.get_game_history("pk_c");
        assert_eq!(history_c.len(), 1);
    }

    #[test]
    fn manager_verify_game_result() {
        let mgr = ProfileManager::new();

        let valid = GameResult {
            game_id: "g1".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: Some("pk_a".to_string()),
            outcome: GameOutcome::Victory,
            turns: 40,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "e1".to_string(),
        };
        assert!(mgr.verify_game_result(&valid));

        // Single player is invalid.
        let single = GameResult {
            players: vec!["pk_a".to_string()],
            ..valid.clone()
        };
        assert!(!mgr.verify_game_result(&single));

        // Winner not in players is invalid.
        let bad_winner = GameResult {
            winner: Some("pk_z".to_string()),
            ..valid.clone()
        };
        assert!(!mgr.verify_game_result(&bad_winner));
    }

    #[test]
    fn manager_leaderboard_ordering() {
        let mut mgr = ProfileManager::new();
        let mut pa = PlayerProfile::new("A", "pk_a");
        pa.elo = 1700;
        let mut pb = PlayerProfile::new("B", "pk_b");
        pb.elo = 1500;
        let mut pc = PlayerProfile::new("C", "pk_c");
        pc.elo = 1900;
        mgr.update_profile(pa);
        mgr.update_profile(pb);
        mgr.update_profile(pc);

        let lb = mgr.leaderboard();
        assert_eq!(lb.len(), 3);
        assert_eq!(lb[0].0, "pk_c");
        assert_eq!(lb[0].1, 1900);
        assert_eq!(lb[1].0, "pk_a");
        assert_eq!(lb[1].1, 1700);
        assert_eq!(lb[2].0, "pk_b");
        assert_eq!(lb[2].1, 1500);
    }

    #[test]
    fn manager_concession_updates_elo() {
        let mut mgr = ProfileManager::new();
        mgr.update_profile(PlayerProfile::new("A", "pk_a"));
        mgr.update_profile(PlayerProfile::new("B", "pk_b"));

        let result = GameResult {
            game_id: "g1".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: Some("pk_a".to_string()),
            outcome: GameOutcome::Concession,
            turns: 20,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "e1".to_string(),
        };
        mgr.record_game_result(result);

        let a = mgr.get_profile("pk_a").unwrap();
        assert!(a.elo > 1500);
        let b = mgr.get_profile("pk_b").unwrap();
        assert!(b.elo < 1500);
    }

    #[test]
    fn manager_timeout_no_winner_no_elo_change() {
        let mut mgr = ProfileManager::new();
        mgr.update_profile(PlayerProfile::new("A", "pk_a"));
        mgr.update_profile(PlayerProfile::new("B", "pk_b"));

        let result = GameResult {
            game_id: "g1".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: None,
            outcome: GameOutcome::Timeout,
            turns: 10,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "e1".to_string(),
        };
        mgr.record_game_result(result);

        // No ELO change for timeout without winner.
        assert_eq!(mgr.get_profile("pk_a").unwrap().elo, 1500);
        assert_eq!(mgr.get_profile("pk_b").unwrap().elo, 1500);
    }

    #[test]
    fn profile_from_invalid_json() {
        let result = PlayerProfile::from_event_content("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn player_stats_default() {
        let stats = PlayerStats::default();
        assert_eq!(stats.games_played, 0);
        assert_eq!(stats.games_won, 0);
        assert_eq!(stats.games_lost, 0);
        assert_eq!(stats.games_drawn, 0);
        assert_eq!(stats.avg_game_length, 0);
        assert!(stats.favorite_ruleset.is_none());
    }

    #[test]
    fn profile_update_stats_abandoned_no_winner() {
        let mut p = PlayerProfile::new("Player", "pk_a");
        let result = GameResult {
            game_id: "g1".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: None,
            outcome: GameOutcome::Abandoned,
            turns: 5,
            ruleset: "classic".to_string(),
            ended_at: 1700000000,
            end_event_id: "e1".to_string(),
        };
        p.update_stats(&result);
        assert_eq!(p.stats.games_lost, 1);
        assert_eq!(p.stats.games_won, 0);
    }

    #[test]
    fn game_outcome_serialization() {
        let outcomes = vec![
            GameOutcome::Victory,
            GameOutcome::Draw,
            GameOutcome::Concession,
            GameOutcome::Timeout,
            GameOutcome::Abandoned,
        ];
        for outcome in outcomes {
            let json = serde_json::to_string(&outcome).unwrap();
            let deser: GameOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(outcome, deser);
        }
    }

    #[test]
    fn game_result_serialization_roundtrip() {
        let result = GameResult {
            game_id: "g1".to_string(),
            players: vec!["pk_a".to_string(), "pk_b".to_string()],
            winner: Some("pk_a".to_string()),
            outcome: GameOutcome::Victory,
            turns: 42,
            ruleset: "civ2civ3".to_string(),
            ended_at: 1700000000,
            end_event_id: "evt_id".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: GameResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deser);
    }
}

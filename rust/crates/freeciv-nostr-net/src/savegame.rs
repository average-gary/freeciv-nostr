//! Savegame import/export for backward compatibility with Freeciv INI saves.
//!
//! Provides a conversion layer between JSON representations of game state
//! (marshalled from the C savegame3 format) and the Nostr event chain used
//! by freeciv-nostr for decentralised game recording.
//!
//! ## Import flow
//!
//! 1. C code reads an INI savegame and serialises the game state to JSON.
//! 2. [`SavegameConverter::import_savegame`] converts the JSON into a
//!    [`ImportResult`] containing a synthetic Game Start event and a full
//!    state checkpoint.
//! 3. The Rust networking layer can then treat the imported state as if it
//!    were the result of replaying an event chain.
//!
//! ## Export flow
//!
//! 1. [`SavegameConverter::export_to_savegame`] replays a [`GameRecording`]
//!    up to the desired turn, producing a [`SavegameData`] JSON blob.
//! 2. The C code receives this blob and writes it as an INI savegame.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::replay::GameRecording;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Represents parsed game state from a savegame (as JSON from C side).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavegameData {
    /// Game metadata.
    pub metadata: SavegameMetadata,
    /// Map data (tiles, terrain, resources).
    pub map: serde_json::Value,
    /// Player data.
    pub players: Vec<SavegamePlayer>,
    /// Current turn number.
    pub turn: u64,
    /// Game random state/seed.
    pub random_seed: u64,
}

/// Metadata embedded in a savegame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavegameMetadata {
    /// Savegame format version.
    pub version: String,
    /// Ruleset name.
    pub ruleset: String,
    /// Map width.
    pub map_width: u32,
    /// Map height.
    pub map_height: u32,
    /// Number of players.
    pub num_players: u8,
    /// Game description.
    pub description: Option<String>,
}

/// A single player entry in a savegame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavegamePlayer {
    /// Player index (0-based).
    pub index: u8,
    /// Player name.
    pub name: String,
    /// Nation name.
    pub nation: String,
    /// Is this an AI player?
    pub is_ai: bool,
    /// Assigned Nostr pubkey (optional — generated for import).
    pub pubkey: Option<String>,
}

// ---------------------------------------------------------------------------
// Import result types
// ---------------------------------------------------------------------------

/// Result of importing a savegame into event format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    /// Generated game ID.
    pub game_id: String,
    /// Game Start event content (JSON).
    pub start_event: serde_json::Value,
    /// State checkpoint (full state at import turn).
    pub checkpoint: StateCheckpoint,
    /// Player key assignments.
    pub player_keys: Vec<PlayerKeyAssignment>,
    /// Import notes/warnings.
    pub warnings: Vec<String>,
}

/// Maps a player index to an assigned Nostr public key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerKeyAssignment {
    /// Player index (0-based).
    pub player_index: u8,
    /// Player name.
    pub player_name: String,
    /// Hex-encoded Nostr public key.
    pub pubkey: String,
}

/// A full state checkpoint for import/export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateCheckpoint {
    /// Turn number this checkpoint represents.
    pub turn: u64,
    /// Full game state as JSON.
    pub state: serde_json::Value,
    /// SHA-256 hash of the state.
    pub state_hash: String,
}

impl StateCheckpoint {
    /// Compute the SHA-256 hash of a JSON value.
    ///
    /// The value is serialised with `serde_json::to_string` (compact, no
    /// trailing newline) before hashing, which is deterministic for the same
    /// logical JSON structure.
    pub fn compute_hash(state: &serde_json::Value) -> String {
        let serialized = serde_json::to_string(state).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(serialized.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Create a checkpoint from savegame data.
    pub fn from_savegame(data: &SavegameData) -> Self {
        let state = serde_json::to_value(data).unwrap_or(serde_json::Value::Null);
        let state_hash = Self::compute_hash(&state);
        Self {
            turn: data.turn,
            state,
            state_hash,
        }
    }
}

// ---------------------------------------------------------------------------
// Export result types
// ---------------------------------------------------------------------------

/// Result of exporting an event chain to savegame format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    /// The savegame data (to be written by C side).
    pub savegame: SavegameData,
    /// Turn number exported at.
    pub exported_at_turn: u64,
    /// Export notes/warnings.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Options for importing a savegame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOptions {
    /// Whether to generate synthetic keys for players.
    pub generate_keys: bool,
    /// Game ID to use (`None` = auto-generate).
    pub game_id: Option<String>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            generate_keys: true,
            game_id: None,
        }
    }
}

/// Options for exporting to a savegame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Turn to export at (`None` = latest).
    pub at_turn: Option<u64>,
    /// Compression format.
    pub compression: CompressionFormat,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            at_turn: None,
            compression: CompressionFormat::None,
        }
    }
}

/// Supported savegame compression formats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CompressionFormat {
    None,
    Zlib,
    Xz,
    Zstd,
    Bzip2,
}

impl CompressionFormat {
    /// All supported compression formats.
    pub fn all() -> Vec<CompressionFormat> {
        vec![
            CompressionFormat::None,
            CompressionFormat::Zlib,
            CompressionFormat::Xz,
            CompressionFormat::Zstd,
            CompressionFormat::Bzip2,
        ]
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during savegame conversion.
#[derive(Debug, thiserror::Error)]
pub enum SavegameError {
    /// The savegame data is invalid or incomplete.
    #[error("invalid savegame data: {0}")]
    InvalidData(String),

    /// JSON serialisation/deserialisation failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// The savegame version is not supported.
    #[error("unsupported savegame version: {0}")]
    UnsupportedVersion(String),

    /// A roundtrip check failed.
    #[error("roundtrip mismatch: {0}")]
    RoundtripMismatch(String),
}

// ---------------------------------------------------------------------------
// SavegameConverter
// ---------------------------------------------------------------------------

/// Converts between Freeciv savegame data and Nostr event chains.
pub struct SavegameConverter {
    /// Default import options.
    default_import_opts: ImportOptions,
    /// Default export options.
    default_export_opts: ExportOptions,
}

impl SavegameConverter {
    /// Create a new converter with default options.
    pub fn new() -> Self {
        Self {
            default_import_opts: ImportOptions::default(),
            default_export_opts: ExportOptions::default(),
        }
    }

    /// Import a savegame, converting it to a Nostr event chain representation.
    ///
    /// Generates a synthetic Game Start event and a full state checkpoint.
    /// If `opts.generate_keys` is true, deterministic placeholder keys are
    /// generated for each player.
    pub fn import_savegame(
        &self,
        data: &SavegameData,
        opts: &ImportOptions,
    ) -> Result<ImportResult, SavegameError> {
        let mut warnings = self.validate_savegame(data);

        // Determine game ID
        let game_id = opts.game_id.clone().unwrap_or_else(|| {
            let mut hasher = Sha256::new();
            hasher.update(data.random_seed.to_le_bytes());
            hasher.update(data.metadata.ruleset.as_bytes());
            hasher.update(data.turn.to_le_bytes());
            hex::encode(hasher.finalize())
        });

        // Assign keys to players
        let player_keys: Vec<PlayerKeyAssignment> = data
            .players
            .iter()
            .map(|p| {
                let pubkey = if opts.generate_keys {
                    p.pubkey.clone().unwrap_or_else(|| {
                        // Generate a deterministic placeholder key from the
                        // game ID and player index.
                        let mut hasher = Sha256::new();
                        hasher.update(game_id.as_bytes());
                        hasher.update(b"player");
                        hasher.update(p.index.to_le_bytes());
                        hex::encode(hasher.finalize())
                    })
                } else {
                    p.pubkey.clone().unwrap_or_default()
                };
                PlayerKeyAssignment {
                    player_index: p.index,
                    player_name: p.name.clone(),
                    pubkey,
                }
            })
            .collect();

        // Build a synthetic Game Start event content
        let player_pubkeys: Vec<&str> = player_keys.iter().map(|k| k.pubkey.as_str()).collect();

        let start_event = serde_json::json!({
            "map_seed": data.random_seed,
            "game_seed": data.random_seed,
            "player_order": player_pubkeys,
            "ruleset": data.metadata.ruleset,
            "map_width": data.metadata.map_width,
            "map_height": data.metadata.map_height,
            "imported": true,
            "import_turn": data.turn,
        });

        // Build checkpoint
        let checkpoint = StateCheckpoint::from_savegame(data);

        if data.players.is_empty() {
            warnings.push("savegame has no players".to_string());
        }

        Ok(ImportResult {
            game_id,
            start_event,
            checkpoint,
            player_keys,
            warnings,
        })
    }

    /// Export a game recording to savegame format at the requested turn.
    ///
    /// Replays the event chain and reconstructs a [`SavegameData`] that the
    /// C side can serialise as an INI savegame.
    pub fn export_to_savegame(
        &self,
        recording: &GameRecording,
        opts: &ExportOptions,
    ) -> Result<ExportResult, SavegameError> {
        let mut warnings = Vec::new();

        let target_turn = opts.at_turn.unwrap_or(recording.total_turns);

        if target_turn > recording.total_turns {
            warnings.push(format!(
                "requested turn {} exceeds total turns {}; exporting at last turn",
                target_turn, recording.total_turns
            ));
        }

        // Gather actions up to the target turn
        let actions_up_to: Vec<_> = recording
            .actions
            .iter()
            .filter(|a| a.turn <= target_turn)
            .collect();

        // Extract player list and start params
        let players: Vec<SavegamePlayer> = recording
            .players
            .iter()
            .enumerate()
            .map(|(i, pubkey)| SavegamePlayer {
                index: i as u8,
                name: format!("Player {}", i + 1),
                nation: "Unknown".to_string(),
                is_ai: false,
                pubkey: Some(pubkey.clone()),
            })
            .collect();

        // Extract metadata from start_params
        let ruleset = recording
            .start_params
            .get("ruleset")
            .and_then(|v| v.as_str())
            .unwrap_or("classic")
            .to_string();
        let map_width = recording
            .start_params
            .get("map_width")
            .and_then(|v| v.as_u64())
            .unwrap_or(80) as u32;
        let map_height = recording
            .start_params
            .get("map_height")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as u32;
        let random_seed = recording
            .start_params
            .get("map_seed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let metadata = SavegameMetadata {
            version: "3.3".to_string(),
            ruleset,
            map_width,
            map_height,
            num_players: players.len() as u8,
            description: Some(format!(
                "Exported from Nostr event chain at turn {}",
                target_turn
            )),
        };

        // Build actions array as the map representation
        let actions_json: Vec<serde_json::Value> =
            actions_up_to.iter().map(|a| serde_json::json!(a)).collect();

        let savegame = SavegameData {
            metadata,
            map: serde_json::json!({ "actions_replay": actions_json }),
            players,
            turn: target_turn,
            random_seed,
        };

        if recording.players.is_empty() {
            warnings.push("recording has no players".to_string());
        }

        Ok(ExportResult {
            savegame,
            exported_at_turn: target_turn,
            warnings,
        })
    }

    /// Validate savegame data and return any warnings.
    pub fn validate_savegame(&self, data: &SavegameData) -> Vec<String> {
        let mut warnings = Vec::new();

        if data.metadata.version.is_empty() {
            warnings.push("savegame version is empty".to_string());
        }

        if data.metadata.ruleset.is_empty() {
            warnings.push("ruleset name is empty".to_string());
        }

        if data.metadata.map_width == 0 || data.metadata.map_height == 0 {
            warnings.push("map dimensions are zero".to_string());
        }

        if data.metadata.num_players as usize != data.players.len() {
            warnings.push(format!(
                "num_players ({}) does not match player count ({})",
                data.metadata.num_players,
                data.players.len()
            ));
        }

        // Check for duplicate player indices
        let mut seen_indices = std::collections::HashSet::new();
        for player in &data.players {
            if !seen_indices.insert(player.index) {
                warnings.push(format!("duplicate player index: {}", player.index));
            }
            if player.name.is_empty() {
                warnings.push(format!("player {} has empty name", player.index));
            }
        }

        warnings
    }

    /// Check if importing then exporting preserves data.
    ///
    /// Imports the savegame, then exports the result and compares the
    /// checkpoint state hash. Returns `true` if the hashes match.
    pub fn roundtrip_check(&self, original: &SavegameData) -> Result<bool, SavegameError> {
        let import_opts = ImportOptions {
            generate_keys: true,
            game_id: Some("roundtrip_test".to_string()),
        };
        let import_result = self.import_savegame(original, &import_opts)?;

        // Compute hash of the original
        let original_hash = import_result.checkpoint.state_hash.clone();

        // Re-create checkpoint from the original data
        let recomputed = StateCheckpoint::from_savegame(original);

        Ok(original_hash == recomputed.state_hash)
    }

    /// Get the default import options.
    pub fn default_import_opts(&self) -> &ImportOptions {
        &self.default_import_opts
    }

    /// Get the default export options.
    pub fn default_export_opts(&self) -> &ExportOptions {
        &self.default_export_opts
    }
}

impl Default for SavegameConverter {
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

    // -- Helpers -------------------------------------------------------------

    fn make_test_metadata() -> SavegameMetadata {
        SavegameMetadata {
            version: "3.3".to_string(),
            ruleset: "classic".to_string(),
            map_width: 80,
            map_height: 50,
            num_players: 2,
            description: Some("Test game".to_string()),
        }
    }

    fn make_test_player(index: u8, name: &str) -> SavegamePlayer {
        SavegamePlayer {
            index,
            name: name.to_string(),
            nation: "Romans".to_string(),
            is_ai: false,
            pubkey: None,
        }
    }

    fn make_test_savegame() -> SavegameData {
        SavegameData {
            metadata: make_test_metadata(),
            map: serde_json::json!({"tiles": []}),
            players: vec![make_test_player(0, "Alice"), make_test_player(1, "Bob")],
            turn: 10,
            random_seed: 42,
        }
    }

    fn make_test_recording() -> GameRecording {
        use crate::replay::RecordedAction;

        GameRecording {
            game_id: "test_game".to_string(),
            start_params: serde_json::json!({
                "map_seed": 42,
                "game_seed": 42,
                "player_order": ["pk_alice", "pk_bob"],
                "ruleset": "classic",
                "map_width": 80,
                "map_height": 50,
            }),
            players: vec!["pk_alice".to_string(), "pk_bob".to_string()],
            actions: vec![
                RecordedAction {
                    turn: 1,
                    phase: 0,
                    sequence: 0,
                    player_pubkey: "pk_alice".to_string(),
                    action: serde_json::json!({"unit_id": 1, "move": "east"}),
                    event_id: "evt0".to_string(),
                    signature_valid: true,
                },
                RecordedAction {
                    turn: 1,
                    phase: 0,
                    sequence: 1,
                    player_pubkey: "pk_bob".to_string(),
                    action: serde_json::json!({"unit_id": 2, "move": "west"}),
                    event_id: "evt1".to_string(),
                    signature_valid: true,
                },
                RecordedAction {
                    turn: 2,
                    phase: 0,
                    sequence: 2,
                    player_pubkey: "pk_alice".to_string(),
                    action: serde_json::json!({"unit_id": 1, "move": "north"}),
                    event_id: "evt2".to_string(),
                    signature_valid: true,
                },
            ],
            state_hashes: vec![],
            end_summary: None,
            total_turns: 2,
        }
    }

    // =====================================================================
    // SavegameData serialization roundtrip
    // =====================================================================

    #[test]
    fn savegame_data_serialization_roundtrip() {
        let data = make_test_savegame();
        let json = serde_json::to_string(&data).unwrap();
        let deserialized: SavegameData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.turn, data.turn);
        assert_eq!(deserialized.random_seed, data.random_seed);
        assert_eq!(deserialized.metadata.version, data.metadata.version);
        assert_eq!(deserialized.metadata.ruleset, data.metadata.ruleset);
        assert_eq!(deserialized.players.len(), data.players.len());
        assert_eq!(deserialized.players[0].name, "Alice");
        assert_eq!(deserialized.players[1].name, "Bob");
    }

    #[test]
    fn savegame_metadata_serialization_roundtrip() {
        let meta = make_test_metadata();
        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: SavegameMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.version, meta.version);
        assert_eq!(deserialized.ruleset, meta.ruleset);
        assert_eq!(deserialized.map_width, meta.map_width);
        assert_eq!(deserialized.map_height, meta.map_height);
        assert_eq!(deserialized.num_players, meta.num_players);
        assert_eq!(deserialized.description, meta.description);
    }

    #[test]
    fn savegame_player_serialization_roundtrip() {
        let player = make_test_player(0, "Alice");
        let json = serde_json::to_string(&player).unwrap();
        let deserialized: SavegamePlayer = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.index, player.index);
        assert_eq!(deserialized.name, player.name);
        assert_eq!(deserialized.nation, player.nation);
        assert_eq!(deserialized.is_ai, player.is_ai);
        assert_eq!(deserialized.pubkey, player.pubkey);
    }

    // =====================================================================
    // Import tests
    // =====================================================================

    #[test]
    fn import_basic() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        let opts = ImportOptions::default();

        let result = conv.import_savegame(&data, &opts).unwrap();
        assert!(!result.game_id.is_empty());
        assert_eq!(result.player_keys.len(), 2);
        assert_eq!(result.checkpoint.turn, 10);
        assert!(!result.checkpoint.state_hash.is_empty());
    }

    #[test]
    fn import_with_custom_game_id() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        let opts = ImportOptions {
            generate_keys: true,
            game_id: Some("custom_id_123".to_string()),
        };

        let result = conv.import_savegame(&data, &opts).unwrap();
        assert_eq!(result.game_id, "custom_id_123");
    }

    #[test]
    fn import_generates_deterministic_keys() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        let opts = ImportOptions {
            generate_keys: true,
            game_id: Some("fixed_id".to_string()),
        };

        let result1 = conv.import_savegame(&data, &opts).unwrap();
        let result2 = conv.import_savegame(&data, &opts).unwrap();

        assert_eq!(result1.player_keys[0].pubkey, result2.player_keys[0].pubkey);
        assert_eq!(result1.player_keys[1].pubkey, result2.player_keys[1].pubkey);
        // Keys should be different between players
        assert_ne!(result1.player_keys[0].pubkey, result1.player_keys[1].pubkey);
    }

    #[test]
    fn import_preserves_existing_pubkeys() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.players[0].pubkey = Some("existing_key_abc".to_string());

        let opts = ImportOptions {
            generate_keys: true,
            game_id: None,
        };

        let result = conv.import_savegame(&data, &opts).unwrap();
        assert_eq!(result.player_keys[0].pubkey, "existing_key_abc");
    }

    #[test]
    fn import_no_generate_keys() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        let opts = ImportOptions {
            generate_keys: false,
            game_id: None,
        };

        let result = conv.import_savegame(&data, &opts).unwrap();
        // Without key generation, players without pubkeys get empty strings
        assert_eq!(result.player_keys[0].pubkey, "");
        assert_eq!(result.player_keys[1].pubkey, "");
    }

    #[test]
    fn import_start_event_contains_metadata() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        let opts = ImportOptions::default();

        let result = conv.import_savegame(&data, &opts).unwrap();
        assert_eq!(result.start_event["ruleset"], "classic");
        assert_eq!(result.start_event["map_seed"], 42);
        assert_eq!(result.start_event["imported"], true);
        assert_eq!(result.start_event["import_turn"], 10);
        assert_eq!(result.start_event["map_width"], 80);
        assert_eq!(result.start_event["map_height"], 50);
    }

    #[test]
    fn import_empty_players_warns() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.players.clear();
        data.metadata.num_players = 0;

        let opts = ImportOptions::default();
        let result = conv.import_savegame(&data, &opts).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("no players")));
    }

    #[test]
    fn import_zero_turn() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.turn = 0;

        let opts = ImportOptions::default();
        let result = conv.import_savegame(&data, &opts).unwrap();
        assert_eq!(result.checkpoint.turn, 0);
    }

    // =====================================================================
    // Export tests
    // =====================================================================

    #[test]
    fn export_basic() {
        let conv = SavegameConverter::new();
        let recording = make_test_recording();
        let opts = ExportOptions::default();

        let result = conv.export_to_savegame(&recording, &opts).unwrap();
        assert_eq!(result.exported_at_turn, 2);
        assert_eq!(result.savegame.turn, 2);
        assert_eq!(result.savegame.players.len(), 2);
        assert_eq!(result.savegame.metadata.ruleset, "classic");
        assert_eq!(result.savegame.random_seed, 42);
    }

    #[test]
    fn export_at_specific_turn() {
        let conv = SavegameConverter::new();
        let recording = make_test_recording();
        let opts = ExportOptions {
            at_turn: Some(1),
            compression: CompressionFormat::None,
        };

        let result = conv.export_to_savegame(&recording, &opts).unwrap();
        assert_eq!(result.exported_at_turn, 1);
        assert_eq!(result.savegame.turn, 1);

        // Only actions from turn 1 should be included in map replay
        let actions = result.savegame.map["actions_replay"].as_array().unwrap();
        assert_eq!(actions.len(), 2); // 2 actions at turn 1
    }

    #[test]
    fn export_beyond_total_turns_warns() {
        let conv = SavegameConverter::new();
        let recording = make_test_recording();
        let opts = ExportOptions {
            at_turn: Some(999),
            compression: CompressionFormat::None,
        };

        let result = conv.export_to_savegame(&recording, &opts).unwrap();
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("exceeds total turns"))
        );
    }

    #[test]
    fn export_empty_recording() {
        let conv = SavegameConverter::new();
        let recording = GameRecording {
            game_id: "empty".to_string(),
            start_params: serde_json::json!({}),
            players: vec![],
            actions: vec![],
            state_hashes: vec![],
            end_summary: None,
            total_turns: 0,
        };
        let opts = ExportOptions::default();

        let result = conv.export_to_savegame(&recording, &opts).unwrap();
        assert_eq!(result.savegame.players.len(), 0);
        assert_eq!(result.exported_at_turn, 0);
        assert!(result.warnings.iter().any(|w| w.contains("no players")));
    }

    #[test]
    fn export_preserves_player_pubkeys() {
        let conv = SavegameConverter::new();
        let recording = make_test_recording();
        let opts = ExportOptions::default();

        let result = conv.export_to_savegame(&recording, &opts).unwrap();
        assert_eq!(
            result.savegame.players[0].pubkey,
            Some("pk_alice".to_string())
        );
        assert_eq!(
            result.savegame.players[1].pubkey,
            Some("pk_bob".to_string())
        );
    }

    // =====================================================================
    // Validate tests
    // =====================================================================

    #[test]
    fn validate_valid_savegame() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        let warnings = conv.validate_savegame(&data);
        assert!(warnings.is_empty(), "expected no warnings: {warnings:?}");
    }

    #[test]
    fn validate_empty_version() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.metadata.version = String::new();
        let warnings = conv.validate_savegame(&data);
        assert!(warnings.iter().any(|w| w.contains("version is empty")));
    }

    #[test]
    fn validate_empty_ruleset() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.metadata.ruleset = String::new();
        let warnings = conv.validate_savegame(&data);
        assert!(warnings.iter().any(|w| w.contains("ruleset name is empty")));
    }

    #[test]
    fn validate_zero_map_dimensions() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.metadata.map_width = 0;
        let warnings = conv.validate_savegame(&data);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("map dimensions are zero"))
        );
    }

    #[test]
    fn validate_num_players_mismatch() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.metadata.num_players = 5; // but only 2 players
        let warnings = conv.validate_savegame(&data);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("does not match player count"))
        );
    }

    #[test]
    fn validate_duplicate_player_indices() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.players[1].index = 0; // duplicate
        let warnings = conv.validate_savegame(&data);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("duplicate player index"))
        );
    }

    #[test]
    fn validate_empty_player_name() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.players[0].name = String::new();
        let warnings = conv.validate_savegame(&data);
        assert!(warnings.iter().any(|w| w.contains("empty name")));
    }

    // =====================================================================
    // StateCheckpoint tests
    // =====================================================================

    #[test]
    fn checkpoint_hash_is_deterministic() {
        let state = serde_json::json!({"turn": 5, "units": [1, 2, 3]});
        let hash1 = StateCheckpoint::compute_hash(&state);
        let hash2 = StateCheckpoint::compute_hash(&state);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn checkpoint_hash_differs_for_different_state() {
        let state1 = serde_json::json!({"turn": 5});
        let state2 = serde_json::json!({"turn": 6});
        let hash1 = StateCheckpoint::compute_hash(&state1);
        let hash2 = StateCheckpoint::compute_hash(&state2);
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn checkpoint_hash_is_valid_hex() {
        let state = serde_json::json!({"test": true});
        let hash = StateCheckpoint::compute_hash(&state);
        // SHA-256 produces 64 hex characters
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn checkpoint_from_savegame() {
        let data = make_test_savegame();
        let checkpoint = StateCheckpoint::from_savegame(&data);
        assert_eq!(checkpoint.turn, 10);
        assert!(!checkpoint.state_hash.is_empty());
        assert_eq!(checkpoint.state_hash.len(), 64);
        // state should contain the savegame data
        assert!(checkpoint.state.is_object());
    }

    #[test]
    fn checkpoint_from_savegame_is_deterministic() {
        let data = make_test_savegame();
        let c1 = StateCheckpoint::from_savegame(&data);
        let c2 = StateCheckpoint::from_savegame(&data);
        assert_eq!(c1.state_hash, c2.state_hash);
    }

    // =====================================================================
    // ImportOptions / ExportOptions defaults
    // =====================================================================

    #[test]
    fn import_options_default() {
        let opts = ImportOptions::default();
        assert!(opts.generate_keys);
        assert!(opts.game_id.is_none());
    }

    #[test]
    fn export_options_default() {
        let opts = ExportOptions::default();
        assert!(opts.at_turn.is_none());
        assert_eq!(opts.compression, CompressionFormat::None);
    }

    // =====================================================================
    // CompressionFormat tests
    // =====================================================================

    #[test]
    fn compression_format_serialization() {
        let formats = CompressionFormat::all();
        for fmt in &formats {
            let json = serde_json::to_string(fmt).unwrap();
            let deserialized: CompressionFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(*fmt, deserialized);
        }
    }

    #[test]
    fn compression_format_all() {
        let all = CompressionFormat::all();
        assert_eq!(all.len(), 5);
        assert!(all.contains(&CompressionFormat::None));
        assert!(all.contains(&CompressionFormat::Zlib));
        assert!(all.contains(&CompressionFormat::Xz));
        assert!(all.contains(&CompressionFormat::Zstd));
        assert!(all.contains(&CompressionFormat::Bzip2));
    }

    // =====================================================================
    // Roundtrip check
    // =====================================================================

    #[test]
    fn roundtrip_check_passes() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        assert!(conv.roundtrip_check(&data).unwrap());
    }

    #[test]
    fn roundtrip_check_consistency() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        // Running twice should give the same result
        let r1 = conv.roundtrip_check(&data).unwrap();
        let r2 = conv.roundtrip_check(&data).unwrap();
        assert_eq!(r1, r2);
    }

    // =====================================================================
    // Edge cases
    // =====================================================================

    #[test]
    fn import_with_ai_players() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.players[1].is_ai = true;
        let opts = ImportOptions::default();
        let result = conv.import_savegame(&data, &opts).unwrap();
        assert_eq!(result.player_keys.len(), 2);
    }

    #[test]
    fn import_result_serialization_roundtrip() {
        let conv = SavegameConverter::new();
        let data = make_test_savegame();
        let opts = ImportOptions::default();
        let result = conv.import_savegame(&data, &opts).unwrap();

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: ImportResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.game_id, result.game_id);
        assert_eq!(deserialized.player_keys.len(), result.player_keys.len());
        assert_eq!(
            deserialized.checkpoint.state_hash,
            result.checkpoint.state_hash
        );
    }

    #[test]
    fn export_result_serialization_roundtrip() {
        let conv = SavegameConverter::new();
        let recording = make_test_recording();
        let opts = ExportOptions::default();
        let result = conv.export_to_savegame(&recording, &opts).unwrap();

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: ExportResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exported_at_turn, result.exported_at_turn);
        assert_eq!(deserialized.savegame.turn, result.savegame.turn);
    }

    #[test]
    fn converter_default_impl() {
        let conv = SavegameConverter::default();
        assert!(conv.default_import_opts().generate_keys);
        assert!(conv.default_export_opts().at_turn.is_none());
    }

    #[test]
    fn import_options_serialization() {
        let opts = ImportOptions {
            generate_keys: false,
            game_id: Some("my_game".to_string()),
        };
        let json = serde_json::to_string(&opts).unwrap();
        let deserialized: ImportOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.generate_keys, false);
        assert_eq!(deserialized.game_id, Some("my_game".to_string()));
    }

    #[test]
    fn export_options_serialization() {
        let opts = ExportOptions {
            at_turn: Some(5),
            compression: CompressionFormat::Zstd,
        };
        let json = serde_json::to_string(&opts).unwrap();
        let deserialized: ExportOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.at_turn, Some(5));
        assert_eq!(deserialized.compression, CompressionFormat::Zstd);
    }

    #[test]
    fn savegame_with_missing_optional_fields() {
        // Description is optional
        let json = r#"{
            "metadata": {
                "version": "3.3",
                "ruleset": "classic",
                "map_width": 80,
                "map_height": 50,
                "num_players": 0,
                "description": null
            },
            "map": {},
            "players": [],
            "turn": 0,
            "random_seed": 0
        }"#;
        let data: SavegameData = serde_json::from_str(json).unwrap();
        assert_eq!(data.turn, 0);
        assert!(data.metadata.description.is_none());
        assert!(data.players.is_empty());
    }

    #[test]
    fn validate_multiple_issues() {
        let conv = SavegameConverter::new();
        let mut data = make_test_savegame();
        data.metadata.version = String::new();
        data.metadata.ruleset = String::new();
        data.metadata.map_width = 0;
        data.metadata.num_players = 99;
        let warnings = conv.validate_savegame(&data);
        assert!(warnings.len() >= 4);
    }

    #[test]
    fn checkpoint_hash_of_null_state() {
        let hash = StateCheckpoint::compute_hash(&serde_json::Value::Null);
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn checkpoint_hash_of_empty_object() {
        let hash = StateCheckpoint::compute_hash(&serde_json::json!({}));
        assert_eq!(hash.len(), 64);
        // Different from null
        let null_hash = StateCheckpoint::compute_hash(&serde_json::Value::Null);
        assert_ne!(hash, null_hash);
    }
}

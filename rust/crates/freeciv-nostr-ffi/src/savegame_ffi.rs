//! FFI bindings for savegame import/export compatibility.
//!
//! Wraps [`freeciv_nostr_net::savegame`] types for C consumption via
//! JSON-based marshalling.

use std::os::raw::{c_char, c_int};

use freeciv_nostr_net::savegame::{
    CompressionFormat, ExportOptions, ImportOptions, SavegameConverter, SavegameData,
    StateCheckpoint,
};

use crate::error::{cstr_to_str, set_last_error, string_to_c};

// ---------------------------------------------------------------------------
// SavegameConverter lifecycle
// ---------------------------------------------------------------------------

/// Create a new savegame converter.
///
/// Returns an opaque pointer, or `NULL` on allocation failure.
/// The caller must free with `fcn_savegame_converter_free()`.
///
/// # Safety
///
/// The returned pointer must be freed by the caller.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_savegame_converter_new() -> *mut SavegameConverter {
    Box::into_raw(Box::new(SavegameConverter::new()))
}

/// Free a savegame converter.
///
/// # Safety
///
/// `conv` must be a valid pointer from `fcn_savegame_converter_new()`,
/// or `NULL` (which is a no-op).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_savegame_converter_free(conv: *mut SavegameConverter) {
    if !conv.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(conv);
        }
    }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import a savegame from JSON data.
///
/// `data_json`: JSON-serialised [`SavegameData`].
/// `opts_json`: JSON-serialised [`ImportOptions`].
///
/// Returns a JSON string with the [`ImportResult`], or `NULL` on error
/// (check `fcn_last_error()`). The caller must free the returned string
/// with `fcn_string_free()`.
///
/// # Safety
///
/// - `conv` must be a valid pointer from `fcn_savegame_converter_new()`.
/// - `data_json` and `opts_json` must be valid null-terminated UTF-8 strings.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_savegame_import(
    conv: *mut SavegameConverter,
    data_json: *const c_char,
    opts_json: *const c_char,
) -> *mut c_char {
    if conv.is_null() {
        set_last_error("null converter pointer");
        return std::ptr::null_mut();
    }
    let data_str = match cstr_to_str(data_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let opts_str = match cstr_to_str(opts_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let data: SavegameData = match serde_json::from_str(data_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid savegame data JSON: {e}"));
            return std::ptr::null_mut();
        }
    };
    let opts: ImportOptions = match serde_json::from_str(opts_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid import options JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: Caller guarantees conv is valid.
    let converter = unsafe { &*conv };
    match converter.import_savegame(&data, &opts) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error(&format!("failed to serialize import result: {e}"));
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error(&format!("import failed: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Export a game recording to savegame format.
///
/// `recording_json`: JSON-serialised [`GameRecording`].
/// `opts_json`: JSON-serialised [`ExportOptions`].
///
/// Returns a JSON string with the [`ExportResult`], or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// - `conv` must be a valid pointer from `fcn_savegame_converter_new()`.
/// - `recording_json` and `opts_json` must be valid null-terminated UTF-8 strings.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_savegame_export(
    conv: *mut SavegameConverter,
    recording_json: *const c_char,
    opts_json: *const c_char,
) -> *mut c_char {
    if conv.is_null() {
        set_last_error("null converter pointer");
        return std::ptr::null_mut();
    }
    let rec_str = match cstr_to_str(recording_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let opts_str = match cstr_to_str(opts_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let recording: freeciv_nostr_net::replay::GameRecording = match serde_json::from_str(rec_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid recording JSON: {e}"));
            return std::ptr::null_mut();
        }
    };
    let opts: ExportOptions = match serde_json::from_str(opts_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid export options JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: Caller guarantees conv is valid.
    let converter = unsafe { &*conv };
    match converter.export_to_savegame(&recording, &opts) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error(&format!("failed to serialize export result: {e}"));
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error(&format!("export failed: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Validate
// ---------------------------------------------------------------------------

/// Validate savegame data.
///
/// `data_json`: JSON-serialised [`SavegameData`].
///
/// Returns a JSON array of warning strings, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// - `conv` must be a valid pointer from `fcn_savegame_converter_new()`.
/// - `data_json` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_savegame_validate(
    conv: *mut SavegameConverter,
    data_json: *const c_char,
) -> *mut c_char {
    if conv.is_null() {
        set_last_error("null converter pointer");
        return std::ptr::null_mut();
    }
    let data_str = match cstr_to_str(data_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let data: SavegameData = match serde_json::from_str(data_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid savegame data JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: Caller guarantees conv is valid.
    let converter = unsafe { &*conv };
    let warnings = converter.validate_savegame(&data);
    match serde_json::to_string(&warnings) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize warnings: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Roundtrip check
// ---------------------------------------------------------------------------

/// Check if import-then-export preserves savegame data.
///
/// `data_json`: JSON-serialised [`SavegameData`].
///
/// Returns 1 if the roundtrip preserves data, 0 if there is a mismatch,
/// -1 on error (check `fcn_last_error()`).
///
/// # Safety
///
/// - `conv` must be a valid pointer from `fcn_savegame_converter_new()`.
/// - `data_json` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_savegame_roundtrip_check(
    conv: *mut SavegameConverter,
    data_json: *const c_char,
) -> c_int {
    if conv.is_null() {
        set_last_error("null converter pointer");
        return -1;
    }
    let data_str = match cstr_to_str(data_json) {
        Some(s) => s,
        None => return -1,
    };

    let data: SavegameData = match serde_json::from_str(data_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid savegame data JSON: {e}"));
            return -1;
        }
    };

    // SAFETY: Caller guarantees conv is valid.
    let converter = unsafe { &*conv };
    match converter.roundtrip_check(&data) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(e) => {
            set_last_error(&format!("roundtrip check failed: {e}"));
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// StateCheckpoint helper
// ---------------------------------------------------------------------------

/// Create a state checkpoint from a JSON state and turn number.
///
/// `state_json`: arbitrary JSON representing the game state.
/// `turn`: the turn number for the checkpoint.
///
/// Returns a JSON-serialised [`StateCheckpoint`] with the computed hash,
/// or `NULL` on error. The caller must free with `fcn_string_free()`.
///
/// # Safety
///
/// `state_json` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_state_checkpoint_from_json(
    state_json: *const c_char,
    turn: u64,
) -> *mut c_char {
    let state_str = match cstr_to_str(state_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let state: serde_json::Value = match serde_json::from_str(state_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid state JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    let hash = StateCheckpoint::compute_hash(&state);
    let checkpoint = StateCheckpoint {
        turn,
        state,
        state_hash: hash,
    };

    match serde_json::to_string(&checkpoint) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize checkpoint: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Compression formats
// ---------------------------------------------------------------------------

/// Return the list of supported compression formats as a JSON array.
///
/// Returns a JSON array of strings, or `NULL` on error.
/// The caller must free with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_savegame_compression_formats() -> *mut c_char {
    let formats = CompressionFormat::all();
    match serde_json::to_string(&formats) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize compression formats: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::fcn_last_error;
    use std::ffi::{CStr, CString};

    fn make_savegame_json() -> String {
        serde_json::to_string(&serde_json::json!({
            "metadata": {
                "version": "3.3",
                "ruleset": "classic",
                "map_width": 80,
                "map_height": 50,
                "num_players": 2,
                "description": "Test"
            },
            "map": {"tiles": []},
            "players": [
                {"index": 0, "name": "Alice", "nation": "Romans", "is_ai": false, "pubkey": null},
                {"index": 1, "name": "Bob", "nation": "Greeks", "is_ai": false, "pubkey": null}
            ],
            "turn": 10,
            "random_seed": 42
        }))
        .unwrap()
    }

    fn make_import_opts_json() -> String {
        serde_json::to_string(&serde_json::json!({
            "generate_keys": true,
            "game_id": null
        }))
        .unwrap()
    }

    fn make_export_opts_json() -> String {
        serde_json::to_string(&serde_json::json!({
            "at_turn": null,
            "compression": "None"
        }))
        .unwrap()
    }

    fn make_recording_json() -> String {
        serde_json::to_string(&serde_json::json!({
            "game_id": "test_game",
            "start_params": {
                "map_seed": 42,
                "game_seed": 42,
                "player_order": ["pk_alice", "pk_bob"],
                "ruleset": "classic",
                "map_width": 80,
                "map_height": 50
            },
            "players": ["pk_alice", "pk_bob"],
            "actions": [
                {
                    "turn": 1,
                    "phase": 0,
                    "sequence": 0,
                    "player_pubkey": "pk_alice",
                    "action": {"unit_id": 1},
                    "event_id": "evt0",
                    "signature_valid": true
                }
            ],
            "state_hashes": [],
            "end_summary": null,
            "total_turns": 1
        }))
        .unwrap()
    }

    // =====================================================================
    // Converter lifecycle
    // =====================================================================

    #[test]
    fn converter_new_and_free() {
        let conv = fcn_savegame_converter_new();
        assert!(!conv.is_null());
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn converter_free_null_is_noop() {
        fcn_savegame_converter_free(std::ptr::null_mut());
    }

    // =====================================================================
    // Import
    // =====================================================================

    #[test]
    fn import_returns_json() {
        let conv = fcn_savegame_converter_new();
        let data = CString::new(make_savegame_json()).unwrap();
        let opts = CString::new(make_import_opts_json()).unwrap();

        let result_ptr = fcn_savegame_import(conv, data.as_ptr(), opts.as_ptr());
        assert!(!result_ptr.is_null());

        let result_json = unsafe { CStr::from_ptr(result_ptr) }.to_str().unwrap();
        let result: serde_json::Value = serde_json::from_str(result_json).unwrap();
        assert!(result.get("game_id").is_some());
        assert!(result.get("checkpoint").is_some());
        assert!(result.get("player_keys").is_some());

        crate::fcn_string_free(result_ptr);
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn import_null_converter() {
        let data = CString::new(make_savegame_json()).unwrap();
        let opts = CString::new(make_import_opts_json()).unwrap();
        assert!(fcn_savegame_import(std::ptr::null_mut(), data.as_ptr(), opts.as_ptr()).is_null());
    }

    #[test]
    fn import_null_data() {
        let conv = fcn_savegame_converter_new();
        let opts = CString::new(make_import_opts_json()).unwrap();
        assert!(fcn_savegame_import(conv, std::ptr::null(), opts.as_ptr()).is_null());
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn import_null_opts() {
        let conv = fcn_savegame_converter_new();
        let data = CString::new(make_savegame_json()).unwrap();
        assert!(fcn_savegame_import(conv, data.as_ptr(), std::ptr::null()).is_null());
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn import_invalid_data_json() {
        let conv = fcn_savegame_converter_new();
        let bad = CString::new("not json").unwrap();
        let opts = CString::new(make_import_opts_json()).unwrap();
        assert!(fcn_savegame_import(conv, bad.as_ptr(), opts.as_ptr()).is_null());
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn import_invalid_opts_json() {
        let conv = fcn_savegame_converter_new();
        let data = CString::new(make_savegame_json()).unwrap();
        let bad = CString::new("not json").unwrap();
        assert!(fcn_savegame_import(conv, data.as_ptr(), bad.as_ptr()).is_null());
        fcn_savegame_converter_free(conv);
    }

    // =====================================================================
    // Export
    // =====================================================================

    #[test]
    fn export_returns_json() {
        let conv = fcn_savegame_converter_new();
        let rec = CString::new(make_recording_json()).unwrap();
        let opts = CString::new(make_export_opts_json()).unwrap();

        let result_ptr = fcn_savegame_export(conv, rec.as_ptr(), opts.as_ptr());
        assert!(!result_ptr.is_null());

        let result_json = unsafe { CStr::from_ptr(result_ptr) }.to_str().unwrap();
        let result: serde_json::Value = serde_json::from_str(result_json).unwrap();
        assert!(result.get("savegame").is_some());
        assert!(result.get("exported_at_turn").is_some());

        crate::fcn_string_free(result_ptr);
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn export_null_converter() {
        let rec = CString::new(make_recording_json()).unwrap();
        let opts = CString::new(make_export_opts_json()).unwrap();
        assert!(fcn_savegame_export(std::ptr::null_mut(), rec.as_ptr(), opts.as_ptr()).is_null());
    }

    #[test]
    fn export_null_recording() {
        let conv = fcn_savegame_converter_new();
        let opts = CString::new(make_export_opts_json()).unwrap();
        assert!(fcn_savegame_export(conv, std::ptr::null(), opts.as_ptr()).is_null());
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn export_null_opts() {
        let conv = fcn_savegame_converter_new();
        let rec = CString::new(make_recording_json()).unwrap();
        assert!(fcn_savegame_export(conv, rec.as_ptr(), std::ptr::null()).is_null());
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn export_invalid_recording_json() {
        let conv = fcn_savegame_converter_new();
        let bad = CString::new("not json").unwrap();
        let opts = CString::new(make_export_opts_json()).unwrap();
        assert!(fcn_savegame_export(conv, bad.as_ptr(), opts.as_ptr()).is_null());
        fcn_savegame_converter_free(conv);
    }

    // =====================================================================
    // Validate
    // =====================================================================

    #[test]
    fn validate_returns_json_array() {
        let conv = fcn_savegame_converter_new();
        let data = CString::new(make_savegame_json()).unwrap();

        let result_ptr = fcn_savegame_validate(conv, data.as_ptr());
        assert!(!result_ptr.is_null());

        let result_json = unsafe { CStr::from_ptr(result_ptr) }.to_str().unwrap();
        let warnings: Vec<String> = serde_json::from_str(result_json).unwrap();
        assert!(warnings.is_empty()); // valid savegame

        crate::fcn_string_free(result_ptr);
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn validate_null_converter() {
        let data = CString::new(make_savegame_json()).unwrap();
        assert!(fcn_savegame_validate(std::ptr::null_mut(), data.as_ptr()).is_null());
    }

    #[test]
    fn validate_null_data() {
        let conv = fcn_savegame_converter_new();
        assert!(fcn_savegame_validate(conv, std::ptr::null()).is_null());
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn validate_invalid_json() {
        let conv = fcn_savegame_converter_new();
        let bad = CString::new("{{bad").unwrap();
        assert!(fcn_savegame_validate(conv, bad.as_ptr()).is_null());
        fcn_savegame_converter_free(conv);
    }

    // =====================================================================
    // Roundtrip check
    // =====================================================================

    #[test]
    fn roundtrip_check_returns_1_for_valid() {
        let conv = fcn_savegame_converter_new();
        let data = CString::new(make_savegame_json()).unwrap();
        assert_eq!(fcn_savegame_roundtrip_check(conv, data.as_ptr()), 1);
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn roundtrip_check_null_converter() {
        let data = CString::new(make_savegame_json()).unwrap();
        assert_eq!(
            fcn_savegame_roundtrip_check(std::ptr::null_mut(), data.as_ptr()),
            -1
        );
    }

    #[test]
    fn roundtrip_check_null_data() {
        let conv = fcn_savegame_converter_new();
        assert_eq!(fcn_savegame_roundtrip_check(conv, std::ptr::null()), -1);
        fcn_savegame_converter_free(conv);
    }

    #[test]
    fn roundtrip_check_invalid_json() {
        let conv = fcn_savegame_converter_new();
        let bad = CString::new("nope").unwrap();
        assert_eq!(fcn_savegame_roundtrip_check(conv, bad.as_ptr()), -1);
        fcn_savegame_converter_free(conv);
    }

    // =====================================================================
    // StateCheckpoint helper
    // =====================================================================

    #[test]
    fn checkpoint_from_json_returns_json() {
        let state = CString::new(r#"{"turn":5,"units":[1,2,3]}"#).unwrap();
        let result_ptr = fcn_state_checkpoint_from_json(state.as_ptr(), 5);
        assert!(!result_ptr.is_null());

        let json = unsafe { CStr::from_ptr(result_ptr) }.to_str().unwrap();
        let checkpoint: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(checkpoint["turn"], 5);
        assert!(checkpoint["state_hash"].is_string());
        let hash = checkpoint["state_hash"].as_str().unwrap();
        assert_eq!(hash.len(), 64);

        crate::fcn_string_free(result_ptr);
    }

    #[test]
    fn checkpoint_from_json_null_state() {
        assert!(fcn_state_checkpoint_from_json(std::ptr::null(), 0).is_null());
    }

    #[test]
    fn checkpoint_from_json_invalid_json() {
        let bad = CString::new("not json").unwrap();
        assert!(fcn_state_checkpoint_from_json(bad.as_ptr(), 0).is_null());
    }

    // =====================================================================
    // Compression formats
    // =====================================================================

    #[test]
    fn compression_formats_returns_json_array() {
        let result_ptr = fcn_savegame_compression_formats();
        assert!(!result_ptr.is_null());

        let json = unsafe { CStr::from_ptr(result_ptr) }.to_str().unwrap();
        let formats: Vec<String> = serde_json::from_str(json).unwrap();
        assert_eq!(formats.len(), 5);
        assert!(formats.contains(&"None".to_string()));
        assert!(formats.contains(&"Zlib".to_string()));
        assert!(formats.contains(&"Xz".to_string()));
        assert!(formats.contains(&"Zstd".to_string()));
        assert!(formats.contains(&"Bzip2".to_string()));

        crate::fcn_string_free(result_ptr);
    }

    // =====================================================================
    // Comprehensive null safety
    // =====================================================================

    #[test]
    fn all_null_safety() {
        // Converter
        fcn_savegame_converter_free(std::ptr::null_mut());

        // Import
        assert!(
            fcn_savegame_import(std::ptr::null_mut(), std::ptr::null(), std::ptr::null()).is_null()
        );

        // Export
        assert!(
            fcn_savegame_export(std::ptr::null_mut(), std::ptr::null(), std::ptr::null()).is_null()
        );

        // Validate
        assert!(fcn_savegame_validate(std::ptr::null_mut(), std::ptr::null()).is_null());

        // Roundtrip
        assert_eq!(
            fcn_savegame_roundtrip_check(std::ptr::null_mut(), std::ptr::null()),
            -1
        );

        // Checkpoint
        assert!(fcn_state_checkpoint_from_json(std::ptr::null(), 0).is_null());

        // Verify error was set for last call
        let err = fcn_last_error();
        assert!(!err.is_null());
    }
}

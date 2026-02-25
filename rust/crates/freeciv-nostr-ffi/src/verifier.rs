//! FFI bindings for the game verifier subsystem.
//!
//! Wraps `freeciv_nostr_verify::verifier::GameVerifier` as an opaque pointer
//! (`*mut FcnGameVerifier`). Turn outcomes are returned as a `#[repr(C)]`
//! tagged union.

use std::os::raw::c_char;

use freeciv_nostr_verify::checkpoint::CheckpointConfig;
use freeciv_nostr_verify::verifier::{GameVerifier, TurnOutcome};
use nostr::prelude::*;

use crate::error::{cstr_to_str, set_last_error, set_last_error_from, string_to_c};

/// Opaque handle to a game verifier.
///
/// Created by `fcn_verifier_new()` or `fcn_verifier_load_or_new()`.
/// Must be freed with `fcn_verifier_free()`.
pub struct FcnGameVerifier {
    inner: GameVerifier,
}

/// Outcome of a turn finalization, returned by `fcn_verifier_finalize_turn()`.
#[repr(C)]
pub enum FcnTurnOutcomeTag {
    /// All players agree on the state hash. Turn is valid.
    Verified = 0,
    /// Players disagree. Desync detected.
    Desync = 1,
    /// Still waiting for commits from some players.
    Pending = 2,
    /// An error occurred during finalization.
    Error = 3,
}

/// Create a new game verifier.
///
/// # Parameters
///
/// - `game_event_id_hex`: Hex-encoded game event ID (64 chars).
/// - `player_pubkey_hexes_json`: JSON array of hex-encoded player public keys.
/// - `checkpoint_interval`: How often to checkpoint (turns). 0 = default (10).
/// - `max_checkpoints`: Maximum checkpoints to retain. 0 = default (3).
///
/// # Safety
///
/// - String parameters must be valid null-terminated UTF-8.
/// - The returned pointer must be freed with `fcn_verifier_free()`.
/// - Returns `NULL` on failure (check `fcn_last_error()`).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_new(
    game_event_id_hex: *const c_char,
    player_pubkey_hexes_json: *const c_char,
    checkpoint_interval: u64,
    max_checkpoints: usize,
) -> *mut FcnGameVerifier {
    let game_id_str = match cstr_to_str(game_event_id_hex) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let players_json = match cstr_to_str(player_pubkey_hexes_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    // Parse game event ID
    let game_event_id = match EventId::from_hex(game_id_str) {
        Ok(id) => id,
        Err(e) => {
            set_last_error_from(e);
            return std::ptr::null_mut();
        }
    };

    // Parse player public keys from JSON array
    let pubkey_hexes: Vec<String> = match serde_json::from_str(players_json) {
        Ok(v) => v,
        Err(e) => {
            set_last_error_from(e);
            return std::ptr::null_mut();
        }
    };

    let mut players = Vec::with_capacity(pubkey_hexes.len());
    for hex in &pubkey_hexes {
        match PublicKey::from_hex(hex) {
            Ok(pk) => players.push(pk),
            Err(e) => {
                set_last_error_from(e);
                return std::ptr::null_mut();
            }
        }
    }

    let config = CheckpointConfig {
        interval: if checkpoint_interval == 0 {
            10
        } else {
            checkpoint_interval
        },
        max_retained: if max_checkpoints == 0 {
            3
        } else {
            max_checkpoints
        },
    };

    Box::into_raw(Box::new(FcnGameVerifier {
        inner: GameVerifier::new(game_event_id, players, config),
    }))
}

/// Record a state hash commit from a player (including self).
///
/// `event_json` is a signed Nostr event of kind 4203 (GAME_STATE_HASH).
///
/// Returns `0` on success, `-1` on failure (check `fcn_last_error()`).
///
/// # Safety
///
/// - `verifier` must be a valid pointer from `fcn_verifier_new()` or similar.
/// - `event_json` must be a valid null-terminated UTF-8 JSON string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_record_commit(
    verifier: *mut FcnGameVerifier,
    event_json: *const c_char,
) -> i32 {
    if verifier.is_null() {
        set_last_error("null verifier pointer");
        return -1;
    }
    let json_str = match cstr_to_str(event_json) {
        Some(s) => s,
        None => return -1,
    };

    let event: Event = match serde_json::from_str(json_str) {
        Ok(e) => e,
        Err(e) => {
            set_last_error_from(e);
            return -1;
        }
    };

    // SAFETY: Caller guarantees verifier is valid and not aliased.
    let v = unsafe { &mut *verifier };
    match v.inner.record_commit(&event) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Finalize a turn: check consensus and advance the hash chain.
///
/// `state_hash_hex` is the local node's state hash as a 64-char hex string.
///
/// Returns a `FcnTurnOutcomeTag` value:
/// - `0` (Verified): all agree
/// - `1` (Desync): players disagree
/// - `2` (Pending): still waiting for commits
/// - `3` (Error): error occurred (check `fcn_last_error()`)
///
/// # Safety
///
/// - `verifier` must be a valid pointer from `fcn_verifier_new()` or similar.
/// - `state_hash_hex` must be a valid null-terminated 64-char hex string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_finalize_turn(
    verifier: *mut FcnGameVerifier,
    turn: u64,
    state_hash_hex: *const c_char,
) -> FcnTurnOutcomeTag {
    if verifier.is_null() {
        set_last_error("null verifier pointer");
        return FcnTurnOutcomeTag::Error;
    }
    let hex_str = match cstr_to_str(state_hash_hex) {
        Some(s) => s,
        None => return FcnTurnOutcomeTag::Error,
    };

    let hash_bytes: [u8; 32] = match hex::decode(hex_str) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        Ok(bytes) => {
            set_last_error(&format!("state hash must be 32 bytes, got {}", bytes.len()));
            return FcnTurnOutcomeTag::Error;
        }
        Err(e) => {
            set_last_error_from(e);
            return FcnTurnOutcomeTag::Error;
        }
    };

    // SAFETY: Caller guarantees verifier is valid and not aliased.
    let v = unsafe { &mut *verifier };
    match v.inner.finalize_turn(turn, hash_bytes) {
        Ok(TurnOutcome::Verified { .. }) => FcnTurnOutcomeTag::Verified,
        Ok(TurnOutcome::Desync { .. }) => FcnTurnOutcomeTag::Desync,
        Ok(TurnOutcome::Pending { .. }) => FcnTurnOutcomeTag::Pending,
        Err(e) => {
            set_last_error_from(e);
            FcnTurnOutcomeTag::Error
        }
    }
}

/// Get the public key hexes of players who haven't submitted commits for a turn.
///
/// Returns a JSON array of hex strings, or `NULL` on error.
///
/// # Safety
///
/// - `verifier` must be a valid pointer.
/// - The returned string must be freed with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_missing_commits(
    verifier: *const FcnGameVerifier,
    turn: u64,
) -> *mut c_char {
    if verifier.is_null() {
        set_last_error("null verifier pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees verifier is valid.
    let v = unsafe { &*verifier };
    let missing: Vec<String> = v
        .inner
        .missing_commits(turn)
        .iter()
        .map(|pk| pk.to_hex())
        .collect();
    match serde_json::to_string(&missing) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Get the length of the hash chain.
///
/// Returns the number of verified turns, or `0` if `verifier` is null.
///
/// # Safety
///
/// `verifier` must be a valid pointer or null.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_chain_len(verifier: *const FcnGameVerifier) -> u64 {
    if verifier.is_null() {
        return 0;
    }
    // SAFETY: Caller guarantees verifier is valid.
    let v = unsafe { &*verifier };
    v.inner.chain().len() as u64
}

/// Validate the hash chain integrity.
///
/// Returns `1` if the chain is valid, `0` if invalid, `-1` on error.
///
/// # Safety
///
/// `verifier` must be a valid pointer or null.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_chain_validate(verifier: *const FcnGameVerifier) -> i32 {
    if verifier.is_null() {
        set_last_error("null verifier pointer");
        return -1;
    }
    // SAFETY: Caller guarantees verifier is valid.
    let v = unsafe { &*verifier };
    match v.inner.chain().validate() {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

/// Set the file path for automatic persistence.
///
/// The verifier will auto-save after each verified turn.
///
/// Returns `0` on success, `-1` on failure.
///
/// # Safety
///
/// - `verifier` must be a valid mutable pointer.
/// - `path` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_set_persist_path(
    verifier: *mut FcnGameVerifier,
    path: *const c_char,
) -> i32 {
    if verifier.is_null() {
        set_last_error("null verifier pointer");
        return -1;
    }
    let path_str = match cstr_to_str(path) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees verifier is valid and not aliased.
    let v = unsafe { &mut *verifier };
    v.inner.set_persist_path(path_str);
    0
}

/// Manually trigger persistence (save current state to file).
///
/// Returns `0` on success, `-1` on failure.
///
/// # Safety
///
/// `verifier` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_persist(verifier: *const FcnGameVerifier) -> i32 {
    if verifier.is_null() {
        set_last_error("null verifier pointer");
        return -1;
    }
    // SAFETY: Caller guarantees verifier is valid.
    let v = unsafe { &*verifier };
    match v.inner.persist() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Export the verifier state as a JSON snapshot.
///
/// # Safety
///
/// - `verifier` must be a valid pointer.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_snapshot_json(verifier: *const FcnGameVerifier) -> *mut c_char {
    if verifier.is_null() {
        set_last_error("null verifier pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees verifier is valid.
    let v = unsafe { &*verifier };
    let snapshot = v.inner.snapshot();
    match snapshot.to_json() {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error_from(e);
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Load a verifier from a file, or create a new one if the file doesn't exist.
///
/// # Safety
///
/// - String parameters must be valid null-terminated UTF-8.
/// - The returned pointer must be freed with `fcn_verifier_free()`.
/// - Returns `NULL` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_load_or_new(
    path: *const c_char,
    game_event_id_hex: *const c_char,
    player_pubkey_hexes_json: *const c_char,
    checkpoint_interval: u64,
    max_checkpoints: usize,
) -> *mut FcnGameVerifier {
    let path_str = match cstr_to_str(path) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let game_id_str = match cstr_to_str(game_event_id_hex) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let players_json = match cstr_to_str(player_pubkey_hexes_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let game_event_id = match EventId::from_hex(game_id_str) {
        Ok(id) => id,
        Err(e) => {
            set_last_error_from(e);
            return std::ptr::null_mut();
        }
    };

    let pubkey_hexes: Vec<String> = match serde_json::from_str(players_json) {
        Ok(v) => v,
        Err(e) => {
            set_last_error_from(e);
            return std::ptr::null_mut();
        }
    };

    let mut players = Vec::with_capacity(pubkey_hexes.len());
    for hex_val in &pubkey_hexes {
        match PublicKey::from_hex(hex_val) {
            Ok(pk) => players.push(pk),
            Err(e) => {
                set_last_error_from(e);
                return std::ptr::null_mut();
            }
        }
    }

    let config = CheckpointConfig {
        interval: if checkpoint_interval == 0 {
            10
        } else {
            checkpoint_interval
        },
        max_retained: if max_checkpoints == 0 {
            3
        } else {
            max_checkpoints
        },
    };

    let v = GameVerifier::load_or_new(
        std::path::Path::new(path_str),
        game_event_id,
        players,
        config,
    );
    Box::into_raw(Box::new(FcnGameVerifier { inner: v }))
}

/// Free a game verifier.
///
/// # Safety
///
/// `verifier` must be a valid pointer from `fcn_verifier_new()` or
/// `fcn_verifier_load_or_new()`, or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verifier_free(verifier: *mut FcnGameVerifier) {
    if !verifier.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(verifier);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeciv_nostr_core::events::{StateHash, build_state_hash_event};
    use freeciv_nostr_core::signer::{GameSigner, LocalSigner};

    fn make_game_id() -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000000"
    }

    #[test]
    fn verifier_new_and_free() {
        let signer = LocalSigner::generate();
        let pubkey_hex = signer.public_key().to_hex();
        let players_json = serde_json::to_string(&vec![pubkey_hex]).unwrap();

        let game_id = std::ffi::CString::new(make_game_id()).unwrap();
        let players = std::ffi::CString::new(players_json).unwrap();

        let ptr = fcn_verifier_new(game_id.as_ptr(), players.as_ptr(), 0, 0);
        assert!(!ptr.is_null());
        assert_eq!(fcn_verifier_chain_len(ptr), 0);
        fcn_verifier_free(ptr);
    }

    #[test]
    fn verifier_single_turn_flow() {
        let signer = LocalSigner::generate();
        let pubkey = signer.public_key();
        let pubkey_hex = pubkey.to_hex();
        let players_json = serde_json::to_string(&vec![pubkey_hex]).unwrap();

        let game_id_cstr = std::ffi::CString::new(make_game_id()).unwrap();
        let players_cstr = std::ffi::CString::new(players_json).unwrap();

        let vptr = fcn_verifier_new(game_id_cstr.as_ptr(), players_cstr.as_ptr(), 10, 3);
        assert!(!vptr.is_null());

        // Create a state hash event
        let game_event_id = EventId::all_zeros();
        let state_hash = [0x42u8; 32];
        let hash_data = StateHash {
            turn: 0,
            hash: hex::encode(state_hash),
        };
        let builder = build_state_hash_event(game_event_id, &hash_data);
        let unsigned = builder.build(pubkey);
        let event = signer.sign_event(unsigned).unwrap();
        let event_json = serde_json::to_string(&event).unwrap();
        let event_cstr = std::ffi::CString::new(event_json).unwrap();

        // Record commit
        let rc = fcn_verifier_record_commit(vptr, event_cstr.as_ptr());
        assert_eq!(rc, 0);

        // Finalize turn
        let hash_hex = hex::encode(state_hash);
        let hash_cstr = std::ffi::CString::new(hash_hex).unwrap();
        let outcome = fcn_verifier_finalize_turn(vptr, 0, hash_cstr.as_ptr());
        assert!(
            matches!(outcome, FcnTurnOutcomeTag::Verified),
            "expected Verified"
        );
        assert_eq!(fcn_verifier_chain_len(vptr), 1);
        assert_eq!(fcn_verifier_chain_validate(vptr), 1);

        fcn_verifier_free(vptr);
    }

    #[test]
    fn verifier_null_safety() {
        assert!(fcn_verifier_new(std::ptr::null(), std::ptr::null(), 0, 0).is_null());
        assert_eq!(fcn_verifier_chain_len(std::ptr::null()), 0);
        assert_eq!(fcn_verifier_chain_validate(std::ptr::null()), -1);
        assert_eq!(
            fcn_verifier_record_commit(std::ptr::null_mut(), std::ptr::null()),
            -1
        );
        assert!(matches!(
            fcn_verifier_finalize_turn(std::ptr::null_mut(), 0, std::ptr::null()),
            FcnTurnOutcomeTag::Error
        ));
        assert!(fcn_verifier_missing_commits(std::ptr::null(), 0).is_null());
        assert!(fcn_verifier_snapshot_json(std::ptr::null()).is_null());
        fcn_verifier_free(std::ptr::null_mut()); // should not crash
    }

    #[test]
    fn verifier_snapshot_roundtrip() {
        let signer = LocalSigner::generate();
        let pubkey_hex = signer.public_key().to_hex();
        let players_json = serde_json::to_string(&vec![pubkey_hex]).unwrap();

        let game_id = std::ffi::CString::new(make_game_id()).unwrap();
        let players = std::ffi::CString::new(players_json).unwrap();

        let vptr = fcn_verifier_new(game_id.as_ptr(), players.as_ptr(), 10, 3);
        let json_ptr = fcn_verifier_snapshot_json(vptr);
        assert!(!json_ptr.is_null());

        let json_str = unsafe { std::ffi::CStr::from_ptr(json_ptr) }
            .to_str()
            .unwrap();
        assert!(json_str.contains("chain"));

        crate::fcn_string_free(json_ptr);
        fcn_verifier_free(vptr);
    }
}

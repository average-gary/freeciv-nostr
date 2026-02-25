//! FFI bindings for the game chain (per-player event chain tracking).
//!
//! Wraps `freeciv_nostr_core::chain::GameChain` as an opaque pointer
//! (`*mut FcnGameChain`).

use std::os::raw::c_char;

use freeciv_nostr_core::chain::GameChain;
use nostr::prelude::*;

use crate::error::{cstr_to_str, set_last_error, set_last_error_from, string_to_c};

/// Opaque handle to a game chain tracker.
///
/// Created by `fcn_game_chain_new()`. Must be freed with `fcn_game_chain_free()`.
pub struct FcnGameChain {
    inner: GameChain,
}

/// Create a new game chain for tracking player action chains.
///
/// # Parameters
///
/// - `game_event_id_hex`: Hex-encoded game event ID (64 chars).
///
/// # Safety
///
/// - `game_event_id_hex` must be a valid null-terminated UTF-8 string.
/// - The returned pointer must be freed with `fcn_game_chain_free()`.
/// - Returns `NULL` on failure (check `fcn_last_error()`).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_chain_new(game_event_id_hex: *const c_char) -> *mut FcnGameChain {
    let game_id_str = match cstr_to_str(game_event_id_hex) {
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
    Box::into_raw(Box::new(FcnGameChain {
        inner: GameChain::new(game_event_id),
    }))
}

/// Add a player to the game chain.
///
/// # Parameters
///
/// - `chain`: A valid game chain pointer.
/// - `pubkey_hex`: Hex-encoded player public key (64 chars).
///
/// Returns `0` on success, `-1` on failure.
///
/// # Safety
///
/// - `chain` must be a valid pointer from `fcn_game_chain_new()`.
/// - `pubkey_hex` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_chain_add_player(
    chain: *mut FcnGameChain,
    pubkey_hex: *const c_char,
) -> i32 {
    if chain.is_null() {
        set_last_error("null chain pointer");
        return -1;
    }
    let hex_str = match cstr_to_str(pubkey_hex) {
        Some(s) => s,
        None => return -1,
    };
    let pubkey = match PublicKey::from_hex(hex_str) {
        Ok(pk) => pk,
        Err(e) => {
            set_last_error_from(e);
            return -1;
        }
    };
    // SAFETY: Caller guarantees chain is valid and not aliased.
    let c = unsafe { &mut *chain };
    c.inner.add_player(pubkey);
    0
}

/// Validate and append an incoming event to the appropriate player's chain.
///
/// The event is parsed, validated against the chain, and if valid, the chain
/// is advanced. Returns the parsed action as a JSON string.
///
/// # Parameters
///
/// - `chain`: A valid game chain pointer.
/// - `event_json`: A signed Nostr event of kind 4202 (GAME_ACTION) as JSON.
///
/// # Safety
///
/// - `chain` must be a valid pointer from `fcn_game_chain_new()`.
/// - `event_json` must be a valid null-terminated UTF-8 JSON string.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` on failure (check `fcn_last_error()`).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_chain_append_event(
    chain: *mut FcnGameChain,
    event_json: *const c_char,
) -> *mut c_char {
    if chain.is_null() {
        set_last_error("null chain pointer");
        return std::ptr::null_mut();
    }
    let json_str = match cstr_to_str(event_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let event: Event = match serde_json::from_str(json_str) {
        Ok(e) => e,
        Err(e) => {
            set_last_error_from(e);
            return std::ptr::null_mut();
        }
    };

    // SAFETY: Caller guarantees chain is valid and not aliased.
    let c = unsafe { &mut *chain };
    match c.inner.append_event(&event) {
        Ok(action) => match serde_json::to_string(&action) {
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

/// Get all chain heads (latest event per player) as a JSON object.
///
/// Returns a JSON object mapping player public key hex to their latest
/// event ID hex (or `null` if no events yet).
///
/// # Safety
///
/// - `chain` must be a valid pointer from `fcn_game_chain_new()`.
/// - The returned string must be freed with `fcn_string_free()`.
/// - Returns `NULL` on failure.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_chain_heads_json(chain: *const FcnGameChain) -> *mut c_char {
    if chain.is_null() {
        set_last_error("null chain pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees chain is valid.
    let c = unsafe { &*chain };
    let heads = c.inner.chain_heads();
    let map: std::collections::HashMap<String, Option<String>> = heads
        .into_iter()
        .map(|(pk, head)| (pk.to_hex(), head.map(|id| id.to_hex())))
        .collect();
    match serde_json::to_string(&map) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Free a game chain.
///
/// # Safety
///
/// `chain` must be a valid pointer from `fcn_game_chain_new()`, or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_chain_free(chain: *mut FcnGameChain) {
    if !chain.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(chain);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_game_id() -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000000"
    }

    #[test]
    fn chain_new_and_free() {
        let game_id = std::ffi::CString::new(make_game_id()).unwrap();
        let ptr = fcn_game_chain_new(game_id.as_ptr());
        assert!(!ptr.is_null());
        fcn_game_chain_free(ptr);
    }

    #[test]
    fn chain_free_null_is_noop() {
        fcn_game_chain_free(std::ptr::null_mut());
    }

    #[test]
    fn chain_add_player_and_heads() {
        let game_id = std::ffi::CString::new(make_game_id()).unwrap();
        let ptr = fcn_game_chain_new(game_id.as_ptr());

        let keys = nostr::Keys::generate();
        let pubkey_hex = std::ffi::CString::new(keys.public_key().to_hex()).unwrap();
        let rc = fcn_game_chain_add_player(ptr, pubkey_hex.as_ptr());
        assert_eq!(rc, 0);

        let heads_ptr = fcn_game_chain_heads_json(ptr);
        assert!(!heads_ptr.is_null());
        let heads_json = unsafe { std::ffi::CStr::from_ptr(heads_ptr) }
            .to_str()
            .unwrap();
        let heads: std::collections::HashMap<String, Option<String>> =
            serde_json::from_str(heads_json).unwrap();
        assert_eq!(heads.len(), 1);
        // Head should be null since no events appended yet
        let (_, head) = heads.iter().next().unwrap();
        assert!(head.is_none());

        crate::fcn_string_free(heads_ptr);
        fcn_game_chain_free(ptr);
    }

    #[test]
    fn chain_null_safety() {
        assert!(fcn_game_chain_new(std::ptr::null()).is_null());
        assert_eq!(
            fcn_game_chain_add_player(std::ptr::null_mut(), std::ptr::null()),
            -1
        );
        assert!(fcn_game_chain_append_event(std::ptr::null_mut(), std::ptr::null()).is_null());
        assert!(fcn_game_chain_heads_json(std::ptr::null()).is_null());
        fcn_game_chain_free(std::ptr::null_mut()); // should not crash
    }
}

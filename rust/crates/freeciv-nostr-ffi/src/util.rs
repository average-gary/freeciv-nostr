//! Stateless utility FFI functions for event kind inspection and verification.

use std::os::raw::c_char;

use freeciv_nostr_core::kinds;
use freeciv_nostr_core::signer;
use nostr::prelude::*;

use crate::error::{cstr_to_str, set_last_error_from, string_to_c};

/// Check if a Nostr event kind number is a gameplay kind (pre-approved for
/// NIP-46 automatic signing).
///
/// Returns `1` if the kind is gameplay, `0` otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_is_gameplay_kind(kind: u16) -> i32 {
    if signer::is_gameplay_kind(Kind::Custom(kind)) {
        1
    } else {
        0
    }
}

/// Get the human-readable name for a freeciv-nostr event kind.
///
/// Returns `"Unknown"` for unrecognized kinds.
///
/// # Safety
///
/// The returned string must be freed with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_kind_name(kind: u16) -> *mut c_char {
    string_to_c(kinds::kind_name(Kind::Custom(kind)).to_string())
}

/// Check if a Nostr event kind number is a freeciv-nostr custom kind.
///
/// Returns `1` if the kind is a freeciv-nostr kind, `0` otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_is_freeciv_kind(kind: u16) -> i32 {
    if kinds::is_freeciv_kind(Kind::Custom(kind)) {
        1
    } else {
        0
    }
}

/// Verify a signed Nostr event (passed as JSON).
///
/// Checks that:
/// 1. The event ID matches the canonical hash of the event content.
/// 2. The Schnorr signature is valid for the event ID and public key.
///
/// Returns `1` if valid, `0` if invalid. On parse error, returns `-1`
/// and sets `fcn_last_error()`.
///
/// # Safety
///
/// `event_json` must be a valid null-terminated UTF-8 JSON string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_verify_event(event_json: *const c_char) -> i32 {
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

    match signer::verify_event(&event) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeciv_nostr_core::signer::{GameSigner, LocalSigner};

    #[test]
    fn gameplay_kinds_identified() {
        assert_eq!(fcn_is_gameplay_kind(4202), 1); // GAME_ACTION
        assert_eq!(fcn_is_gameplay_kind(4203), 1); // GAME_STATE_HASH
        assert_eq!(fcn_is_gameplay_kind(4204), 1); // GAME_CHAT
        assert_eq!(fcn_is_gameplay_kind(4200), 0); // GAME_LOBBY
        assert_eq!(fcn_is_gameplay_kind(9999), 0); // Unknown
    }

    #[test]
    fn freeciv_kinds_identified() {
        assert_eq!(fcn_is_freeciv_kind(4200), 1); // GAME_LOBBY
        assert_eq!(fcn_is_freeciv_kind(4202), 1); // GAME_ACTION
        assert_eq!(fcn_is_freeciv_kind(30420), 1); // PLAYER_PROFILE
        assert_eq!(fcn_is_freeciv_kind(1), 0); // text note
        assert_eq!(fcn_is_freeciv_kind(9999), 0);
    }

    #[test]
    fn kind_names_correct() {
        let ptr = fcn_kind_name(4202);
        assert!(!ptr.is_null());
        let name = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(name, "GameAction");
        crate::fcn_string_free(ptr);

        let ptr2 = fcn_kind_name(9999);
        let name2 = unsafe { std::ffi::CStr::from_ptr(ptr2) }.to_str().unwrap();
        assert_eq!(name2, "Unknown");
        crate::fcn_string_free(ptr2);
    }

    #[test]
    fn verify_valid_event() {
        let signer = LocalSigner::generate();
        let pubkey = signer.public_key();
        let unsigned = EventBuilder::new(Kind::Custom(4202), "test").build(pubkey);
        let event = signer.sign_event(unsigned).unwrap();
        let json = serde_json::to_string(&event).unwrap();
        let c_json = std::ffi::CString::new(json).unwrap();

        assert_eq!(fcn_verify_event(c_json.as_ptr()), 1);
    }

    #[test]
    fn verify_invalid_json_returns_negative() {
        let bad = std::ffi::CString::new("not json").unwrap();
        assert_eq!(fcn_verify_event(bad.as_ptr()), -1);
    }

    #[test]
    fn verify_null_returns_negative() {
        assert_eq!(fcn_verify_event(std::ptr::null()), -1);
    }
}

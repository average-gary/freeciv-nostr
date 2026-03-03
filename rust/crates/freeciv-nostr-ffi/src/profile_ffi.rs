//! FFI bindings for player profile, reputation, and ELO rating.
//!
//! Wraps `freeciv_nostr_net::profile` types as opaque pointer types
//! for C consumption.

use std::os::raw::c_char;

use freeciv_nostr_net::profile::{EloCalculator, GameResult, PlayerProfile, ProfileManager};

use crate::error::{cstr_to_str, set_last_error, string_to_c};

/// Opaque handle to a `ProfileManager`.
///
/// Created by `fcn_profile_manager_new()`. Must be freed with
/// `fcn_profile_manager_free()`.
pub struct FcnProfileManager {
    inner: ProfileManager,
}

// ---------------------------------------------------------------------------
// ProfileManager FFI
// ---------------------------------------------------------------------------

/// Create a new empty profile manager.
///
/// # Safety
///
/// The returned pointer must be freed with `fcn_profile_manager_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_profile_manager_new() -> *mut FcnProfileManager {
    Box::into_raw(Box::new(FcnProfileManager {
        inner: ProfileManager::new(),
    }))
}

/// Free a profile manager.
///
/// # Safety
///
/// `mgr` must be a valid pointer from `fcn_profile_manager_new()`, or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_profile_manager_free(mgr: *mut FcnProfileManager) {
    if !mgr.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(mgr);
        }
    }
}

/// Insert or replace a player profile in the manager.
///
/// `profile_json` is a JSON-serialized `PlayerProfile`.
///
/// Returns 0 on success, -1 on error.
///
/// # Safety
///
/// - `mgr` must be a valid pointer from `fcn_profile_manager_new()`.
/// - `profile_json` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_profile_manager_update_profile(
    mgr: *mut FcnProfileManager,
    profile_json: *const c_char,
) -> i32 {
    if mgr.is_null() {
        set_last_error("null profile manager pointer");
        return -1;
    }
    let json_str = match cstr_to_str(profile_json) {
        Some(s) => s,
        None => return -1,
    };
    let profile: PlayerProfile = match serde_json::from_str(json_str) {
        Ok(p) => p,
        Err(e) => {
            set_last_error(&format!("invalid profile JSON: {e}"));
            return -1;
        }
    };
    // SAFETY: Caller guarantees mgr is valid.
    let mgr = unsafe { &mut *mgr };
    mgr.inner.update_profile(profile);
    0
}

/// Look up a player profile by hex pubkey.
///
/// Returns a JSON string, or `NULL` if not found.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// - `mgr` must be a valid pointer from `fcn_profile_manager_new()`.
/// - `pubkey` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_profile_manager_get_profile(
    mgr: *const FcnProfileManager,
    pubkey: *const c_char,
) -> *mut c_char {
    if mgr.is_null() {
        set_last_error("null profile manager pointer");
        return std::ptr::null_mut();
    }
    let pk = match cstr_to_str(pubkey) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    // SAFETY: Caller guarantees mgr is valid.
    let mgr = unsafe { &*mgr };
    match mgr.inner.get_profile(pk) {
        Some(profile) => match serde_json::to_string(profile) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error(&format!("failed to serialize profile: {e}"));
                std::ptr::null_mut()
            }
        },
        None => {
            set_last_error("profile not found");
            std::ptr::null_mut()
        }
    }
}

/// Record a game result and update all participants' stats and ELO.
///
/// `result_json` is a JSON-serialized `GameResult`.
///
/// Returns 0 on success, -1 on error.
///
/// # Safety
///
/// - `mgr` must be a valid pointer from `fcn_profile_manager_new()`.
/// - `result_json` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_profile_manager_record_game_result(
    mgr: *mut FcnProfileManager,
    result_json: *const c_char,
) -> i32 {
    if mgr.is_null() {
        set_last_error("null profile manager pointer");
        return -1;
    }
    let json_str = match cstr_to_str(result_json) {
        Some(s) => s,
        None => return -1,
    };
    let result: GameResult = match serde_json::from_str(json_str) {
        Ok(r) => r,
        Err(e) => {
            set_last_error(&format!("invalid game result JSON: {e}"));
            return -1;
        }
    };
    // SAFETY: Caller guarantees mgr is valid.
    let mgr = unsafe { &mut *mgr };
    mgr.inner.record_game_result(result);
    0
}

/// Get the game history for a player as a JSON array string.
///
/// Returns a JSON array of `GameResult` objects, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// - `mgr` must be a valid pointer from `fcn_profile_manager_new()`.
/// - `pubkey` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_profile_manager_get_history(
    mgr: *const FcnProfileManager,
    pubkey: *const c_char,
) -> *mut c_char {
    if mgr.is_null() {
        set_last_error("null profile manager pointer");
        return std::ptr::null_mut();
    }
    let pk = match cstr_to_str(pubkey) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    // SAFETY: Caller guarantees mgr is valid.
    let mgr = unsafe { &*mgr };
    let history = mgr.inner.get_game_history(pk);
    match serde_json::to_string(&history) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize history: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Get the leaderboard as a JSON array of `[pubkey, elo]` pairs.
///
/// Returns a JSON string, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// - `mgr` must be a valid pointer from `fcn_profile_manager_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_profile_manager_leaderboard(mgr: *const FcnProfileManager) -> *mut c_char {
    if mgr.is_null() {
        set_last_error("null profile manager pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees mgr is valid.
    let mgr = unsafe { &*mgr };
    let lb = mgr.inner.leaderboard();
    match serde_json::to_string(&lb) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize leaderboard: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Get the number of profiles in the manager.
///
/// Returns -1 on error (null pointer).
///
/// # Safety
///
/// - `mgr` must be a valid pointer from `fcn_profile_manager_new()`, or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_profile_manager_profile_count(mgr: *const FcnProfileManager) -> i32 {
    if mgr.is_null() {
        set_last_error("null profile manager pointer");
        return -1;
    }
    // SAFETY: Caller guarantees mgr is valid.
    let mgr = unsafe { &*mgr };
    mgr.inner.profile_count() as i32
}

// ---------------------------------------------------------------------------
// PlayerProfile FFI
// ---------------------------------------------------------------------------

/// Create a new player profile with default stats.
///
/// Returns a JSON-serialized `PlayerProfile` string.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// - `display_name` must be a valid null-terminated UTF-8 string.
/// - `pubkey` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_player_profile_new(
    display_name: *const c_char,
    pubkey: *const c_char,
) -> *mut c_char {
    let name = match cstr_to_str(display_name) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let pk = match cstr_to_str(pubkey) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let profile = PlayerProfile::new(name, pk);
    match serde_json::to_string(&profile) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize profile: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Parse a player profile from a Nostr event content JSON string.
///
/// Returns a JSON-serialized `PlayerProfile`, or `NULL` on parse error.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// - `event_content_json` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_player_profile_from_event(event_content_json: *const c_char) -> *mut c_char {
    let json_str = match cstr_to_str(event_content_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let profile: PlayerProfile = match serde_json::from_str(json_str) {
        Ok(p) => p,
        Err(e) => {
            set_last_error(&format!("invalid profile event content: {e}"));
            return std::ptr::null_mut();
        }
    };
    // Re-serialize to normalize.
    match serde_json::to_string(&profile) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize profile: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// ELO calculator FFI
// ---------------------------------------------------------------------------

/// Calculate new ELO ratings for a decisive game (winner/loser).
///
/// Returns a JSON string `{"winner": <u32>, "loser": <u32>}`, or `NULL`
/// on error.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// No pointer parameters; this is a pure calculation.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_elo_calculate(winner_elo: u32, loser_elo: u32, is_draw: i32) -> *mut c_char {
    let elo = EloCalculator::new();
    let (new_a, new_b) = if is_draw != 0 {
        elo.calculate_draw_ratings(winner_elo, loser_elo)
    } else {
        elo.calculate_new_ratings(winner_elo, loser_elo)
    };

    let label_a = if is_draw != 0 { "player_a" } else { "winner" };
    let label_b = if is_draw != 0 { "player_b" } else { "loser" };
    let json = format!("{{\"{label_a}\":{new_a},\"{label_b}\":{new_b}}}");
    string_to_c(json)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn manager_new_and_free() {
        let ptr = fcn_profile_manager_new();
        assert!(!ptr.is_null());
        fcn_profile_manager_free(ptr);
    }

    #[test]
    fn manager_free_null_is_noop() {
        fcn_profile_manager_free(std::ptr::null_mut());
    }

    #[test]
    fn manager_update_and_get() {
        let mgr = fcn_profile_manager_new();
        let profile_json = CString::new(
            r#"{"display_name":"Alice","avatar_url":null,"pubkey":"pk_alice","nip05":null,"preferred_rulesets":[],"stats":{"games_played":0,"games_won":0,"games_lost":0,"games_drawn":0,"avg_game_length":0,"favorite_ruleset":null},"elo":1500,"updated_at":0}"#,
        )
        .unwrap();
        let rc = fcn_profile_manager_update_profile(mgr, profile_json.as_ptr());
        assert_eq!(rc, 0);

        let pk = CString::new("pk_alice").unwrap();
        let result = fcn_profile_manager_get_profile(mgr, pk.as_ptr());
        assert!(!result.is_null());
        let json = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert!(json.contains("Alice"));
        crate::fcn_string_free(result);

        assert_eq!(fcn_profile_manager_profile_count(mgr), 1);
        fcn_profile_manager_free(mgr);
    }

    #[test]
    fn manager_record_and_history() {
        let mgr = fcn_profile_manager_new();
        let result_json = CString::new(
            r#"{"game_id":"g1","players":["pk_a","pk_b"],"winner":"pk_a","outcome":"Victory","turns":40,"ruleset":"classic","ended_at":1700000000,"end_event_id":"evt1"}"#,
        )
        .unwrap();
        let rc = fcn_profile_manager_record_game_result(mgr, result_json.as_ptr());
        assert_eq!(rc, 0);

        let pk = CString::new("pk_a").unwrap();
        let history = fcn_profile_manager_get_history(mgr, pk.as_ptr());
        assert!(!history.is_null());
        let json = unsafe { CStr::from_ptr(history) }.to_str().unwrap();
        assert!(json.contains("g1"));
        crate::fcn_string_free(history);

        fcn_profile_manager_free(mgr);
    }

    #[test]
    fn manager_leaderboard() {
        let mgr = fcn_profile_manager_new();

        // Add two profiles with different ELOs.
        let p1 = CString::new(
            r#"{"display_name":"A","avatar_url":null,"pubkey":"pk_a","nip05":null,"preferred_rulesets":[],"stats":{"games_played":0,"games_won":0,"games_lost":0,"games_drawn":0,"avg_game_length":0,"favorite_ruleset":null},"elo":1800,"updated_at":0}"#,
        )
        .unwrap();
        let p2 = CString::new(
            r#"{"display_name":"B","avatar_url":null,"pubkey":"pk_b","nip05":null,"preferred_rulesets":[],"stats":{"games_played":0,"games_won":0,"games_lost":0,"games_drawn":0,"avg_game_length":0,"favorite_ruleset":null},"elo":1600,"updated_at":0}"#,
        )
        .unwrap();
        fcn_profile_manager_update_profile(mgr, p1.as_ptr());
        fcn_profile_manager_update_profile(mgr, p2.as_ptr());

        let lb = fcn_profile_manager_leaderboard(mgr);
        assert!(!lb.is_null());
        let json = unsafe { CStr::from_ptr(lb) }.to_str().unwrap();
        // pk_a (1800) should come before pk_b (1600).
        let a_pos = json.find("pk_a").unwrap();
        let b_pos = json.find("pk_b").unwrap();
        assert!(a_pos < b_pos, "leaderboard should be ordered by ELO desc");
        crate::fcn_string_free(lb);

        fcn_profile_manager_free(mgr);
    }

    #[test]
    fn player_profile_new_ffi() {
        let name = CString::new("Bob").unwrap();
        let pk = CString::new("pk_bob").unwrap();
        let result = fcn_player_profile_new(name.as_ptr(), pk.as_ptr());
        assert!(!result.is_null());
        let json = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert!(json.contains("Bob"));
        assert!(json.contains("pk_bob"));
        assert!(json.contains("1500")); // default ELO
        crate::fcn_string_free(result);
    }

    #[test]
    fn player_profile_from_event_ffi() {
        let content = CString::new(
            r#"{"display_name":"Carol","avatar_url":null,"pubkey":"pk_carol","nip05":"carol@example.com","preferred_rulesets":["civ2civ3"],"stats":{"games_played":5,"games_won":3,"games_lost":2,"games_drawn":0,"avg_game_length":45,"favorite_ruleset":"civ2civ3"},"elo":1600,"updated_at":1700000000}"#,
        )
        .unwrap();
        let result = fcn_player_profile_from_event(content.as_ptr());
        assert!(!result.is_null());
        let json = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert!(json.contains("Carol"));
        assert!(json.contains("carol@example.com"));
        crate::fcn_string_free(result);
    }

    #[test]
    fn player_profile_from_event_invalid() {
        let bad = CString::new("not valid json").unwrap();
        let result = fcn_player_profile_from_event(bad.as_ptr());
        assert!(result.is_null());
        let err = crate::fcn_last_error();
        assert!(!err.is_null());
    }

    #[test]
    fn elo_calculate_win() {
        let result = fcn_elo_calculate(1500, 1500, 0);
        assert!(!result.is_null());
        let json = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(parsed["winner"].as_u64().unwrap() > 1500);
        assert!(parsed["loser"].as_u64().unwrap() < 1500);
        crate::fcn_string_free(result);
    }

    #[test]
    fn elo_calculate_draw() {
        let result = fcn_elo_calculate(1500, 1500, 1);
        assert!(!result.is_null());
        let json = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(parsed["player_a"].as_u64().unwrap(), 1500);
        assert_eq!(parsed["player_b"].as_u64().unwrap(), 1500);
        crate::fcn_string_free(result);
    }

    // -- Null safety tests --

    #[test]
    fn null_safety_manager_update() {
        assert_eq!(
            fcn_profile_manager_update_profile(std::ptr::null_mut(), std::ptr::null()),
            -1
        );
    }

    #[test]
    fn null_safety_manager_get() {
        assert!(fcn_profile_manager_get_profile(std::ptr::null(), std::ptr::null()).is_null());
    }

    #[test]
    fn null_safety_manager_record() {
        assert_eq!(
            fcn_profile_manager_record_game_result(std::ptr::null_mut(), std::ptr::null()),
            -1
        );
    }

    #[test]
    fn null_safety_manager_history() {
        assert!(fcn_profile_manager_get_history(std::ptr::null(), std::ptr::null()).is_null());
    }

    #[test]
    fn null_safety_manager_leaderboard() {
        assert!(fcn_profile_manager_leaderboard(std::ptr::null()).is_null());
    }

    #[test]
    fn null_safety_manager_count() {
        assert_eq!(fcn_profile_manager_profile_count(std::ptr::null()), -1);
    }

    #[test]
    fn null_safety_profile_new() {
        assert!(fcn_player_profile_new(std::ptr::null(), std::ptr::null()).is_null());
    }

    #[test]
    fn null_safety_profile_from_event() {
        assert!(fcn_player_profile_from_event(std::ptr::null()).is_null());
    }

    #[test]
    fn manager_invalid_profile_json() {
        let mgr = fcn_profile_manager_new();
        let bad = CString::new("not json").unwrap();
        assert_eq!(fcn_profile_manager_update_profile(mgr, bad.as_ptr()), -1);
        fcn_profile_manager_free(mgr);
    }

    #[test]
    fn manager_invalid_result_json() {
        let mgr = fcn_profile_manager_new();
        let bad = CString::new("not json").unwrap();
        assert_eq!(
            fcn_profile_manager_record_game_result(mgr, bad.as_ptr()),
            -1
        );
        fcn_profile_manager_free(mgr);
    }

    #[test]
    fn manager_get_nonexistent_returns_null() {
        let mgr = fcn_profile_manager_new();
        let pk = CString::new("nonexistent").unwrap();
        let result = fcn_profile_manager_get_profile(mgr, pk.as_ptr());
        assert!(result.is_null());
        fcn_profile_manager_free(mgr);
    }
}

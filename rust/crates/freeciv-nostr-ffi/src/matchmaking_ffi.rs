//! C FFI bindings for the matchmaking / game-browser subsystem.
//!
//! Wraps [`freeciv_nostr_net::matchmaking::Matchmaker`] as an opaque pointer
//! type (`*mut FcnMatchmaker`) for C consumption.

use std::os::raw::c_char;

use freeciv_nostr_net::matchmaking::{
    GameListing, GameSettings, ListingStatus, MapSize, Matchmaker, MatchmakingFilter,
};

use crate::error::{cstr_to_str, set_last_error, string_to_c};

// ---------------------------------------------------------------------------
// Opaque handle
// ---------------------------------------------------------------------------

/// Opaque handle to a [`Matchmaker`] instance.
pub struct FcnMatchmaker {
    inner: Matchmaker,
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create a new matchmaker for the given local player pubkey.
///
/// `our_pubkey` is the hex-encoded Nostr public key.
///
/// Returns an opaque handle, or `NULL` on error (check `fcn_last_error()`).
/// The caller must free with `fcn_matchmaker_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_new(our_pubkey: *const c_char) -> *mut FcnMatchmaker {
    let pk = match cstr_to_str(our_pubkey) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    Box::into_raw(Box::new(FcnMatchmaker {
        inner: Matchmaker::new(pk),
    }))
}

/// Free a matchmaker handle.
///
/// After this call the handle must not be used.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_free(mm: *mut FcnMatchmaker) {
    if !mm.is_null() {
        // SAFETY: Caller guarantees mm was returned by fcn_matchmaker_new.
        unsafe {
            let _ = Box::from_raw(mm);
        }
    }
}

// ---------------------------------------------------------------------------
// Listing management
// ---------------------------------------------------------------------------

/// Add a game listing from JSON.
///
/// `listing_json` is a JSON-serialised [`GameListing`].
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_add_listing(
    mm: *mut FcnMatchmaker,
    listing_json: *const c_char,
) -> i32 {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return -1;
    }
    let json_str = match cstr_to_str(listing_json) {
        Some(s) => s,
        None => return -1,
    };
    let listing: GameListing = match serde_json::from_str(json_str) {
        Ok(l) => l,
        Err(e) => {
            set_last_error(&format!("invalid listing JSON: {e}"));
            return -1;
        }
    };
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &mut *mm };
    match mm.inner.add_listing(listing) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(&e.to_string());
            -1
        }
    }
}

/// Remove a listing by game ID.
///
/// Returns 0 if the listing was removed, 1 if not found, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_remove_listing(
    mm: *mut FcnMatchmaker,
    game_id: *const c_char,
) -> i32 {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return -1;
    }
    let gid = match cstr_to_str(game_id) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &mut *mm };
    match mm.inner.remove_listing(gid) {
        Some(_) => 0,
        None => 1,
    }
}

// ---------------------------------------------------------------------------
// Browsing / querying
// ---------------------------------------------------------------------------

/// Browse listings with an optional JSON filter.
///
/// `filter_json` is a JSON object with optional fields matching
/// [`MatchmakingFilter`]. Pass `NULL` for no filter (returns all).
///
/// Returns a JSON array of matching [`GameListing`] objects.
/// The caller must free the returned string with `fcn_string_free()`.
/// Returns `NULL` on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_browse(
    mm: *const FcnMatchmaker,
    filter_json: *const c_char,
) -> *mut c_char {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &*mm };

    let filter = if filter_json.is_null() {
        MatchmakingFilter::default()
    } else {
        let json_str = match cstr_to_str(filter_json) {
            Some(s) => s,
            None => return std::ptr::null_mut(),
        };
        parse_filter_json(json_str)
    };

    let results = mm.inner.browse(&filter);
    match serde_json::to_string(&results) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize browse results: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Get a single listing by game ID as JSON.
///
/// Returns the JSON-serialised [`GameListing`], or `NULL` if not found.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_get_listing(
    mm: *const FcnMatchmaker,
    game_id: *const c_char,
) -> *mut c_char {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return std::ptr::null_mut();
    }
    let gid = match cstr_to_str(game_id) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &*mm };
    match mm.inner.get_listing(gid) {
        Some(listing) => match serde_json::to_string(listing) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error(&format!("failed to serialize listing: {e}"));
                std::ptr::null_mut()
            }
        },
        None => {
            set_last_error(&format!("game not found: {gid}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Create / join
// ---------------------------------------------------------------------------

/// Create a new game with the given settings.
///
/// `game_id`: unique game identifier (C string).
/// `settings_json`: JSON-serialised [`GameSettings`].
/// `is_private`: 1 for invite-only, 0 for public.
///
/// Returns the JSON-serialised [`GameListing`] on success.
/// The caller must free the returned string with `fcn_string_free()`.
/// Returns `NULL` on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_create_game(
    mm: *mut FcnMatchmaker,
    game_id: *const c_char,
    settings_json: *const c_char,
    is_private: i32,
) -> *mut c_char {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return std::ptr::null_mut();
    }
    let gid = match cstr_to_str(game_id) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let json_str = match cstr_to_str(settings_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let settings: GameSettings = match serde_json::from_str(json_str) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid settings JSON: {e}"));
            return std::ptr::null_mut();
        }
    };
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &mut *mm };
    match mm.inner.create_game(gid, settings, is_private != 0) {
        Ok(listing) => match serde_json::to_string(&listing) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error(&format!("failed to serialize listing: {e}"));
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Join an existing game by game ID.
///
/// Returns the updated JSON-serialised [`GameListing`] on success.
/// The caller must free the returned string with `fcn_string_free()`.
/// Returns `NULL` on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_join_game(
    mm: *mut FcnMatchmaker,
    game_id: *const c_char,
) -> *mut c_char {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return std::ptr::null_mut();
    }
    let gid = match cstr_to_str(game_id) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &mut *mm };
    match mm.inner.join_game(gid) {
        Ok(listing) => match serde_json::to_string(&listing) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error(&format!("failed to serialize listing: {e}"));
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Counts
// ---------------------------------------------------------------------------

/// Get the total number of known listings.
///
/// Returns -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_listing_count(mm: *const FcnMatchmaker) -> i32 {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return -1;
    }
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &*mm };
    mm.inner.listing_count() as i32
}

/// Get the number of open (joinable) games.
///
/// Returns -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_open_count(mm: *const FcnMatchmaker) -> i32 {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return -1;
    }
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &*mm };
    mm.inner.open_games_count() as i32
}

// ---------------------------------------------------------------------------
// Status update
// ---------------------------------------------------------------------------

/// Update the status of a listing.
///
/// `status`: 0=Open, 1=Full, 2=InProgress, 3=Completed, 4=Cancelled.
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_update_status(
    mm: *mut FcnMatchmaker,
    game_id: *const c_char,
    status: u8,
) -> i32 {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return -1;
    }
    let gid = match cstr_to_str(game_id) {
        Some(s) => s,
        None => return -1,
    };
    let listing_status = match status {
        0 => ListingStatus::Open,
        1 => ListingStatus::Full,
        2 => ListingStatus::InProgress,
        3 => ListingStatus::Completed,
        4 => ListingStatus::Cancelled,
        other => {
            set_last_error(&format!("invalid status: {other}"));
            return -1;
        }
    };
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &mut *mm };
    match mm.inner.update_listing_status(gid, listing_status) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(&e.to_string());
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// My games
// ---------------------------------------------------------------------------

/// Get IDs of games created by the local player, as a JSON array of strings.
///
/// The caller must free the returned string with `fcn_string_free()`.
/// Returns `NULL` on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_matchmaker_my_games(mm: *const FcnMatchmaker) -> *mut c_char {
    if mm.is_null() {
        set_last_error("null matchmaker pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees mm is valid.
    let mm = unsafe { &*mm };
    let created = mm.inner.my_created_games();
    let joined = mm.inner.my_joined_games();
    let result = serde_json::json!({
        "created": created,
        "joined": joined,
    });
    match serde_json::to_string(&result) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize my games: {e}"));
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parse a JSON object into a [`MatchmakingFilter`].
///
/// Supported fields (all optional):
/// - `"ruleset"`: string
/// - `"min_open_slots"`: u8
/// - `"max_players"`: u8
/// - `"map_size"`: string or object (e.g. `"Medium"` or `{"Custom":[80,50]}`)
/// - `"open_only"`: bool
/// - `"exclude_private"`: bool
/// - `"creator"`: string
fn parse_filter_json(json_str: &str) -> MatchmakingFilter {
    // Attempt to parse into a serde_json::Value and extract fields manually,
    // so we keep the filter struct non-Deserialize (it has Option<MapSize>
    // which works but the overall struct uses Default for booleans).
    let v: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return MatchmakingFilter::default(),
    };

    let ruleset = v.get("ruleset").and_then(|v| v.as_str()).map(String::from);
    let min_open_slots = v
        .get("min_open_slots")
        .and_then(|v| v.as_u64())
        .map(|n| n as u8);
    let max_players = v
        .get("max_players")
        .and_then(|v| v.as_u64())
        .map(|n| n as u8);
    let map_size = v
        .get("map_size")
        .and_then(|v| serde_json::from_value::<MapSize>(v.clone()).ok());
    let open_only = v
        .get("open_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let exclude_private = v
        .get("exclude_private")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let creator = v.get("creator").and_then(|v| v.as_str()).map(String::from);

    MatchmakingFilter {
        ruleset,
        min_open_slots,
        max_players,
        map_size,
        open_only,
        exclude_private,
        creator,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    // -- Null safety -------------------------------------------------------

    #[test]
    fn matchmaker_new_null_returns_null() {
        let ptr = fcn_matchmaker_new(std::ptr::null());
        assert!(ptr.is_null());
    }

    #[test]
    fn matchmaker_free_null_is_noop() {
        fcn_matchmaker_free(std::ptr::null_mut());
    }

    #[test]
    fn matchmaker_add_listing_null_mm() {
        let json = CString::new("{}").unwrap();
        assert_eq!(
            fcn_matchmaker_add_listing(std::ptr::null_mut(), json.as_ptr()),
            -1
        );
    }

    #[test]
    fn matchmaker_add_listing_null_json() {
        let pk = CString::new("our_pk").unwrap();
        let mm = fcn_matchmaker_new(pk.as_ptr());
        assert!(!mm.is_null());
        assert_eq!(fcn_matchmaker_add_listing(mm, std::ptr::null()), -1);
        fcn_matchmaker_free(mm);
    }

    #[test]
    fn matchmaker_remove_listing_null_mm() {
        let gid = CString::new("g1").unwrap();
        assert_eq!(
            fcn_matchmaker_remove_listing(std::ptr::null_mut(), gid.as_ptr()),
            -1
        );
    }

    #[test]
    fn matchmaker_browse_null_mm() {
        let ptr = fcn_matchmaker_browse(std::ptr::null(), std::ptr::null());
        assert!(ptr.is_null());
    }

    #[test]
    fn matchmaker_get_listing_null_mm() {
        let gid = CString::new("g1").unwrap();
        let ptr = fcn_matchmaker_get_listing(std::ptr::null(), gid.as_ptr());
        assert!(ptr.is_null());
    }

    #[test]
    fn matchmaker_create_game_null_mm() {
        let gid = CString::new("g1").unwrap();
        let settings = CString::new("{}").unwrap();
        let ptr =
            fcn_matchmaker_create_game(std::ptr::null_mut(), gid.as_ptr(), settings.as_ptr(), 0);
        assert!(ptr.is_null());
    }

    #[test]
    fn matchmaker_join_game_null_mm() {
        let gid = CString::new("g1").unwrap();
        let ptr = fcn_matchmaker_join_game(std::ptr::null_mut(), gid.as_ptr());
        assert!(ptr.is_null());
    }

    #[test]
    fn matchmaker_listing_count_null() {
        assert_eq!(fcn_matchmaker_listing_count(std::ptr::null()), -1);
    }

    #[test]
    fn matchmaker_open_count_null() {
        assert_eq!(fcn_matchmaker_open_count(std::ptr::null()), -1);
    }

    #[test]
    fn matchmaker_update_status_null_mm() {
        let gid = CString::new("g1").unwrap();
        assert_eq!(
            fcn_matchmaker_update_status(std::ptr::null_mut(), gid.as_ptr(), 0),
            -1
        );
    }

    #[test]
    fn matchmaker_my_games_null() {
        let ptr = fcn_matchmaker_my_games(std::ptr::null());
        assert!(ptr.is_null());
    }

    // -- End-to-end FFI flow -----------------------------------------------

    #[test]
    fn ffi_create_browse_join_flow() {
        let pk = CString::new("our_pk").unwrap();
        let mm = fcn_matchmaker_new(pk.as_ptr());
        assert!(!mm.is_null());

        // Create a game
        let gid = CString::new("game1").unwrap();
        let settings = CString::new(
            r#"{"ruleset":"classic","map_size":"Medium","max_players":4,"turn_timeout":300,"phase_mode":"concurrent","map_seed":0,"description":null,"ai_players":0}"#,
        )
        .unwrap();
        let listing_ptr = fcn_matchmaker_create_game(mm, gid.as_ptr(), settings.as_ptr(), 0);
        assert!(!listing_ptr.is_null());
        let listing_str = unsafe { CStr::from_ptr(listing_ptr) }.to_str().unwrap();
        let listing: GameListing = serde_json::from_str(listing_str).unwrap();
        assert_eq!(listing.game_id, "game1");
        assert_eq!(listing.current_players, 1);
        crate::error::fcn_string_free(listing_ptr);

        // Count
        assert_eq!(fcn_matchmaker_listing_count(mm), 1);
        assert_eq!(fcn_matchmaker_open_count(mm), 1);

        // Browse (no filter)
        let browse_ptr = fcn_matchmaker_browse(mm, std::ptr::null());
        assert!(!browse_ptr.is_null());
        let browse_str = unsafe { CStr::from_ptr(browse_ptr) }.to_str().unwrap();
        let browse_results: Vec<GameListing> = serde_json::from_str(browse_str).unwrap();
        assert_eq!(browse_results.len(), 1);
        crate::error::fcn_string_free(browse_ptr);

        // Get listing
        let get_ptr = fcn_matchmaker_get_listing(mm, gid.as_ptr());
        assert!(!get_ptr.is_null());
        crate::error::fcn_string_free(get_ptr);

        // My games
        let my_ptr = fcn_matchmaker_my_games(mm);
        assert!(!my_ptr.is_null());
        let my_str = unsafe { CStr::from_ptr(my_ptr) }.to_str().unwrap();
        let my_val: serde_json::Value = serde_json::from_str(my_str).unwrap();
        assert_eq!(my_val["created"].as_array().unwrap().len(), 1);
        crate::error::fcn_string_free(my_ptr);

        // Add another listing externally, then join it
        let ext_listing = CString::new(
            r#"{"game_id":"game2","creator_pubkey":"other_pk","creator_name":null,"settings":{"ruleset":"civ2civ3","map_size":"Medium","max_players":4,"turn_timeout":300,"phase_mode":"concurrent","map_seed":0,"description":null,"ai_players":0},"current_players":1,"is_private":false,"created_at":1000,"status":"Open"}"#,
        )
        .unwrap();
        assert_eq!(fcn_matchmaker_add_listing(mm, ext_listing.as_ptr()), 0);
        assert_eq!(fcn_matchmaker_listing_count(mm), 2);

        // Join
        let gid2 = CString::new("game2").unwrap();
        let join_ptr = fcn_matchmaker_join_game(mm, gid2.as_ptr());
        assert!(!join_ptr.is_null());
        let join_str = unsafe { CStr::from_ptr(join_ptr) }.to_str().unwrap();
        let joined: GameListing = serde_json::from_str(join_str).unwrap();
        assert_eq!(joined.current_players, 2);
        crate::error::fcn_string_free(join_ptr);

        // Update status
        assert_eq!(fcn_matchmaker_update_status(mm, gid.as_ptr(), 2), 0); // InProgress

        // Remove
        assert_eq!(fcn_matchmaker_remove_listing(mm, gid.as_ptr()), 0);
        assert_eq!(fcn_matchmaker_listing_count(mm), 1);

        // Remove non-existent
        let ghost = CString::new("ghost").unwrap();
        assert_eq!(fcn_matchmaker_remove_listing(mm, ghost.as_ptr()), 1);

        fcn_matchmaker_free(mm);
    }

    #[test]
    fn ffi_browse_with_filter() {
        let pk = CString::new("our_pk").unwrap();
        let mm = fcn_matchmaker_new(pk.as_ptr());
        assert!(!mm.is_null());

        // Add two listings
        let l1 = CString::new(
            r#"{"game_id":"g1","creator_pubkey":"pk1","creator_name":null,"settings":{"ruleset":"classic","map_size":"Medium","max_players":4,"turn_timeout":300,"phase_mode":"concurrent","map_seed":0,"description":null,"ai_players":0},"current_players":1,"is_private":false,"created_at":100,"status":"Open"}"#,
        ).unwrap();
        let l2 = CString::new(
            r#"{"game_id":"g2","creator_pubkey":"pk2","creator_name":null,"settings":{"ruleset":"civ2civ3","map_size":"Large","max_players":8,"turn_timeout":600,"phase_mode":"alternating","map_seed":42,"description":"big game","ai_players":2},"current_players":3,"is_private":true,"created_at":200,"status":"Open"}"#,
        ).unwrap();
        assert_eq!(fcn_matchmaker_add_listing(mm, l1.as_ptr()), 0);
        assert_eq!(fcn_matchmaker_add_listing(mm, l2.as_ptr()), 0);

        // Filter: classic only
        let filter = CString::new(r#"{"ruleset":"classic"}"#).unwrap();
        let browse_ptr = fcn_matchmaker_browse(mm, filter.as_ptr());
        assert!(!browse_ptr.is_null());
        let browse_str = unsafe { CStr::from_ptr(browse_ptr) }.to_str().unwrap();
        let results: Vec<GameListing> = serde_json::from_str(browse_str).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].game_id, "g1");
        crate::error::fcn_string_free(browse_ptr);

        // Filter: exclude private
        let filter2 = CString::new(r#"{"exclude_private":true}"#).unwrap();
        let browse_ptr2 = fcn_matchmaker_browse(mm, filter2.as_ptr());
        assert!(!browse_ptr2.is_null());
        let browse_str2 = unsafe { CStr::from_ptr(browse_ptr2) }.to_str().unwrap();
        let results2: Vec<GameListing> = serde_json::from_str(browse_str2).unwrap();
        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0].game_id, "g1");
        crate::error::fcn_string_free(browse_ptr2);

        fcn_matchmaker_free(mm);
    }

    #[test]
    fn ffi_update_status_invalid() {
        let pk = CString::new("our_pk").unwrap();
        let mm = fcn_matchmaker_new(pk.as_ptr());
        let gid = CString::new("g1").unwrap();
        // Invalid status code
        assert_eq!(fcn_matchmaker_update_status(mm, gid.as_ptr(), 99), -1);
        fcn_matchmaker_free(mm);
    }

    #[test]
    fn ffi_join_nonexistent_returns_null() {
        let pk = CString::new("our_pk").unwrap();
        let mm = fcn_matchmaker_new(pk.as_ptr());
        let gid = CString::new("ghost").unwrap();
        let ptr = fcn_matchmaker_join_game(mm, gid.as_ptr());
        assert!(ptr.is_null());
        fcn_matchmaker_free(mm);
    }

    #[test]
    fn ffi_create_duplicate_returns_null() {
        let pk = CString::new("our_pk").unwrap();
        let mm = fcn_matchmaker_new(pk.as_ptr());
        let gid = CString::new("g1").unwrap();
        let settings = CString::new(
            r#"{"ruleset":"classic","map_size":"Medium","max_players":4,"turn_timeout":300,"phase_mode":"concurrent","map_seed":0,"description":null,"ai_players":0}"#,
        ).unwrap();
        let ptr1 = fcn_matchmaker_create_game(mm, gid.as_ptr(), settings.as_ptr(), 0);
        assert!(!ptr1.is_null());
        crate::error::fcn_string_free(ptr1);

        // Duplicate
        let ptr2 = fcn_matchmaker_create_game(mm, gid.as_ptr(), settings.as_ptr(), 0);
        assert!(ptr2.is_null());

        fcn_matchmaker_free(mm);
    }

    #[test]
    fn ffi_add_invalid_json_returns_error() {
        let pk = CString::new("our_pk").unwrap();
        let mm = fcn_matchmaker_new(pk.as_ptr());
        let bad_json = CString::new("not valid json").unwrap();
        assert_eq!(fcn_matchmaker_add_listing(mm, bad_json.as_ptr()), -1);
        fcn_matchmaker_free(mm);
    }

    // -- parse_filter_json -------------------------------------------------

    #[test]
    fn parse_filter_json_empty_object() {
        let filter = parse_filter_json("{}");
        assert!(filter.ruleset.is_none());
        assert!(!filter.open_only);
        assert!(!filter.exclude_private);
    }

    #[test]
    fn parse_filter_json_invalid_returns_default() {
        let filter = parse_filter_json("not json");
        assert!(filter.ruleset.is_none());
    }

    #[test]
    fn parse_filter_json_all_fields() {
        let filter = parse_filter_json(
            r#"{"ruleset":"classic","min_open_slots":2,"max_players":6,"map_size":"Large","open_only":true,"exclude_private":true,"creator":"pk1"}"#,
        );
        assert_eq!(filter.ruleset.as_deref(), Some("classic"));
        assert_eq!(filter.min_open_slots, Some(2));
        assert_eq!(filter.max_players, Some(6));
        assert_eq!(filter.map_size, Some(MapSize::Large));
        assert!(filter.open_only);
        assert!(filter.exclude_private);
        assert_eq!(filter.creator.as_deref(), Some("pk1"));
    }
}

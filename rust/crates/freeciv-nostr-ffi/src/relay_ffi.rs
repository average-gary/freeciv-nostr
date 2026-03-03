//! C FFI bindings for the Nostr relay pool.
//!
//! Exposes relay pool creation, publishing, subscribing, and status
//! querying to the C game engine. All relay operations are synchronous
//! from the C perspective.
//!
//! # Memory Ownership
//!
//! - `FcnRelayPool` handles are owned by the caller and must be freed
//!   with `fcn_relay_pool_free()`.
//! - Returned `*mut c_char` strings must be freed with `fcn_string_free()`.

use std::os::raw::c_char;
use std::sync::Arc;

use freeciv_nostr_net::nostr_relay::{
    RelayPool, RelayPoolConfig, RelayTransport, SubscriptionFilter,
};
use nostr::{Event, Filter};

use crate::error::{cstr_to_str, set_last_error, string_to_c};

// ---------------------------------------------------------------------------
// Stub transport for FFI (no real WebSocket in this phase)
// ---------------------------------------------------------------------------

/// A no-op relay transport for FFI use.
///
/// In production this would be replaced by a real WebSocket transport.
/// For now, it stores events in memory and returns them on fetch.
struct StubRelayTransport;

impl RelayTransport for StubRelayTransport {
    fn connect(&self, _url: &str) -> Result<(), String> {
        Ok(())
    }

    fn disconnect(&self, _url: &str) -> Result<(), String> {
        Ok(())
    }

    fn publish(&self, _url: &str, _event: &Event) -> Result<(), String> {
        // Stub: accept all publishes.
        Ok(())
    }

    fn fetch_events(&self, _url: &str, _filter: &Filter) -> Result<Vec<Event>, String> {
        // Stub: return empty results.
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// Opaque type
// ---------------------------------------------------------------------------

/// Opaque handle to a Nostr relay pool.
pub struct FcnRelayPool {
    inner: RelayPool,
}

// ---------------------------------------------------------------------------
// FFI functions
// ---------------------------------------------------------------------------

/// Create a new Nostr relay pool from a JSON config string.
///
/// `config_json` is a JSON string matching `RelayPoolConfig`. Pass `NULL`
/// for default configuration.
///
/// Returns an opaque handle, or `NULL` on error (check `fcn_last_error()`).
/// The caller must free with `fcn_relay_pool_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_new(config_json: *const c_char) -> *mut FcnRelayPool {
    let config = if config_json.is_null() {
        RelayPoolConfig::default()
    } else {
        let json_str = match cstr_to_str(config_json) {
            Some(s) => s,
            None => return std::ptr::null_mut(),
        };
        match RelayPoolConfig::from_json(json_str) {
            Ok(c) => c,
            Err(e) => {
                set_last_error(&format!("invalid relay pool config JSON: {e}"));
                return std::ptr::null_mut();
            }
        }
    };

    let transport = Arc::new(StubRelayTransport);
    let pool = RelayPool::new(config, transport);

    Box::into_raw(Box::new(FcnRelayPool { inner: pool }))
}

/// Free a relay pool handle.
///
/// After this call the handle must not be used.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_free(pool: *mut FcnRelayPool) {
    if !pool.is_null() {
        // SAFETY: Caller guarantees pool was returned by fcn_relay_pool_new.
        unsafe {
            let _ = Box::from_raw(pool);
        }
    }
}

/// Publish a Nostr event (JSON) to all relays in the pool.
///
/// `event_json` is a JSON-serialized Nostr event.
///
/// Returns a JSON string with the publish result (`{"accepted":N,"failed":N,...}`),
/// or `NULL` on error. The caller must free the returned string with
/// `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_publish(
    pool: *const FcnRelayPool,
    event_json: *const c_char,
) -> *mut c_char {
    if pool.is_null() {
        set_last_error("null relay pool pointer");
        return std::ptr::null_mut();
    }
    let json_str = match cstr_to_str(event_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let event: Event = match serde_json::from_str(json_str) {
        Ok(e) => e,
        Err(e) => {
            set_last_error(&format!("invalid event JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: Caller guarantees pool is valid.
    let pool = unsafe { &*pool };
    let result = pool.inner.publish(&event);

    let result_json = serde_json::json!({
        "accepted": result.accepted,
        "failed": result.failed,
        "total": result.total(),
    });

    match serde_json::to_string(&result_json) {
        Ok(s) => string_to_c(s),
        Err(e) => {
            set_last_error(&format!("failed to serialize publish result: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Subscribe to relay events with a filter type.
///
/// `filter_type` selects the filter:
///   - 0 = GameDiscovery
///   - 1 = GameHistory (requires `param` = game event ID hex)
///   - 2 = PlayerProfile (requires `param` = public key hex)
///
/// `param` is an optional C string parameter (may be `NULL` for type 0).
///
/// Returns the subscription ID as a C string, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_subscribe(
    pool: *const FcnRelayPool,
    filter_type: u8,
    param: *const c_char,
) -> *mut c_char {
    if pool.is_null() {
        set_last_error("null relay pool pointer");
        return std::ptr::null_mut();
    }

    let filter = match filter_type {
        0 => SubscriptionFilter::GameDiscovery,
        1 => {
            let game_id = match cstr_to_str(param) {
                Some(s) => s,
                None => {
                    set_last_error("GameHistory filter requires a game event ID parameter");
                    return std::ptr::null_mut();
                }
            };
            SubscriptionFilter::GameHistory {
                game_event_id: game_id.to_string(),
            }
        }
        2 => {
            let pubkey = match cstr_to_str(param) {
                Some(s) => s,
                None => {
                    set_last_error("PlayerProfile filter requires a pubkey parameter");
                    return std::ptr::null_mut();
                }
            };
            SubscriptionFilter::PlayerProfile {
                pubkey: pubkey.to_string(),
            }
        }
        other => {
            set_last_error(&format!("invalid filter_type: {other}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: Caller guarantees pool is valid.
    let pool = unsafe { &*pool };
    match pool.inner.subscribe(filter) {
        Ok(sub_id) => string_to_c(sub_id),
        Err(e) => {
            set_last_error(&e);
            std::ptr::null_mut()
        }
    }
}

/// Fetch events matching a filter from relays.
///
/// `filter_type` and `param` work the same as `fcn_relay_pool_subscribe`.
///
/// Returns a JSON array of events, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_fetch_events(
    pool: *const FcnRelayPool,
    filter_type: u8,
    param: *const c_char,
) -> *mut c_char {
    if pool.is_null() {
        set_last_error("null relay pool pointer");
        return std::ptr::null_mut();
    }

    let filter = match filter_type {
        0 => SubscriptionFilter::GameDiscovery,
        1 => {
            let game_id = match cstr_to_str(param) {
                Some(s) => s,
                None => {
                    set_last_error("GameHistory filter requires a game event ID parameter");
                    return std::ptr::null_mut();
                }
            };
            SubscriptionFilter::GameHistory {
                game_event_id: game_id.to_string(),
            }
        }
        2 => {
            let pubkey = match cstr_to_str(param) {
                Some(s) => s,
                None => {
                    set_last_error("PlayerProfile filter requires a pubkey parameter");
                    return std::ptr::null_mut();
                }
            };
            SubscriptionFilter::PlayerProfile {
                pubkey: pubkey.to_string(),
            }
        }
        other => {
            set_last_error(&format!("invalid filter_type: {other}"));
            return std::ptr::null_mut();
        }
    };

    // SAFETY: Caller guarantees pool is valid.
    let pool = unsafe { &*pool };
    match pool.inner.fetch_events(&filter) {
        Ok(events) => {
            let json_arr: Vec<serde_json::Value> = events
                .iter()
                .filter_map(|e| serde_json::to_value(e).ok())
                .collect();
            match serde_json::to_string(&json_arr) {
                Ok(s) => string_to_c(s),
                Err(e) => {
                    set_last_error(&format!("failed to serialize events: {e}"));
                    std::ptr::null_mut()
                }
            }
        }
        Err(e) => {
            set_last_error(&e);
            std::ptr::null_mut()
        }
    }
}

/// Unsubscribe from a relay subscription by ID.
///
/// Returns 1 if the subscription existed and was removed, 0 if not found,
/// -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_unsubscribe(
    pool: *const FcnRelayPool,
    sub_id: *const c_char,
) -> i32 {
    if pool.is_null() {
        set_last_error("null relay pool pointer");
        return -1;
    }
    let id = match cstr_to_str(sub_id) {
        Some(s) => s,
        None => return -1,
    };

    // SAFETY: Caller guarantees pool is valid.
    let pool = unsafe { &*pool };
    if pool.inner.unsubscribe(id) { 1 } else { 0 }
}

/// Get the status of the relay pool as a JSON string.
///
/// Returns a JSON object with relay count, subscription count, and
/// per-relay status. The caller must free the returned string with
/// `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_status(pool: *const FcnRelayPool) -> *mut c_char {
    if pool.is_null() {
        set_last_error("null relay pool pointer");
        return std::ptr::null_mut();
    }

    // SAFETY: Caller guarantees pool is valid.
    let pool = unsafe { &*pool };
    match pool.inner.status_json() {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize pool status: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Add a relay URL to the pool.
///
/// Returns 0 on success, -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_add_relay(
    pool: *mut FcnRelayPool,
    relay_url: *const c_char,
) -> i32 {
    if pool.is_null() {
        set_last_error("null relay pool pointer");
        return -1;
    }
    let url = match cstr_to_str(relay_url) {
        Some(s) => s,
        None => return -1,
    };

    // SAFETY: Caller guarantees pool is valid.
    let pool = unsafe { &mut *pool };
    pool.inner.add_relay(url);
    0
}

/// Remove a relay URL from the pool.
///
/// Returns 1 if the relay was removed, 0 if not found, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_pool_remove_relay(
    pool: *mut FcnRelayPool,
    relay_url: *const c_char,
) -> i32 {
    if pool.is_null() {
        set_last_error("null relay pool pointer");
        return -1;
    }
    let url = match cstr_to_str(relay_url) {
        Some(s) => s,
        None => return -1,
    };

    // SAFETY: Caller guarantees pool is valid.
    let pool = unsafe { &mut *pool };
    if pool.inner.remove_relay(url) { 1 } else { 0 }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::fcn_last_error;
    use std::ffi::{CStr, CString};

    /// Helper to create a C string from a Rust string.
    fn to_cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    #[test]
    fn relay_pool_new_default() {
        let pool = fcn_relay_pool_new(std::ptr::null());
        assert!(!pool.is_null());
        fcn_relay_pool_free(pool);
    }

    #[test]
    fn relay_pool_new_with_config() {
        let config_json = to_cstr(
            r#"{"relay_urls":["wss://r1"],"timeout_secs":5,"max_retries":2,"use_default_relays":false}"#,
        );
        let pool = fcn_relay_pool_new(config_json.as_ptr());
        assert!(!pool.is_null());

        // Check status reflects 1 relay.
        let status_ptr = fcn_relay_pool_status(pool);
        assert!(!status_ptr.is_null());
        let status_str = unsafe { CStr::from_ptr(status_ptr) }.to_str().unwrap();
        let status: serde_json::Value = serde_json::from_str(status_str).unwrap();
        assert_eq!(status["relay_count"], 1);

        crate::fcn_string_free(status_ptr);
        fcn_relay_pool_free(pool);
    }

    #[test]
    fn relay_pool_new_invalid_json() {
        let bad_json = to_cstr("not valid json");
        let pool = fcn_relay_pool_new(bad_json.as_ptr());
        assert!(pool.is_null());

        let err = fcn_last_error();
        assert!(!err.is_null());
    }

    #[test]
    fn relay_pool_free_null_is_noop() {
        fcn_relay_pool_free(std::ptr::null_mut()); // should not crash
    }

    #[test]
    fn relay_pool_add_and_remove_relay() {
        let config_json = to_cstr(
            r#"{"relay_urls":[],"timeout_secs":5,"max_retries":2,"use_default_relays":false}"#,
        );
        let pool = fcn_relay_pool_new(config_json.as_ptr());
        assert!(!pool.is_null());

        let url = to_cstr("wss://new-relay");
        assert_eq!(fcn_relay_pool_add_relay(pool, url.as_ptr()), 0);

        // Verify relay was added via status.
        let status_ptr = fcn_relay_pool_status(pool);
        let status_str = unsafe { CStr::from_ptr(status_ptr) }.to_str().unwrap();
        let status: serde_json::Value = serde_json::from_str(status_str).unwrap();
        assert_eq!(status["relay_count"], 1);
        crate::fcn_string_free(status_ptr);

        // Remove.
        assert_eq!(fcn_relay_pool_remove_relay(pool, url.as_ptr()), 1);

        // Remove again -> not found.
        assert_eq!(fcn_relay_pool_remove_relay(pool, url.as_ptr()), 0);

        fcn_relay_pool_free(pool);
    }

    #[test]
    fn relay_pool_subscribe_game_discovery() {
        let pool = fcn_relay_pool_new(std::ptr::null());
        assert!(!pool.is_null());

        let sub_id_ptr = fcn_relay_pool_subscribe(pool, 0, std::ptr::null());
        assert!(!sub_id_ptr.is_null());

        let sub_id = unsafe { CStr::from_ptr(sub_id_ptr) }.to_str().unwrap();
        assert!(sub_id.starts_with("sub_"));

        crate::fcn_string_free(sub_id_ptr);
        fcn_relay_pool_free(pool);
    }

    #[test]
    fn relay_pool_subscribe_invalid_filter_type() {
        let pool = fcn_relay_pool_new(std::ptr::null());
        assert!(!pool.is_null());

        let result = fcn_relay_pool_subscribe(pool, 99, std::ptr::null());
        assert!(result.is_null());

        fcn_relay_pool_free(pool);
    }

    #[test]
    fn relay_pool_unsubscribe() {
        let pool = fcn_relay_pool_new(std::ptr::null());
        assert!(!pool.is_null());

        let sub_id_ptr = fcn_relay_pool_subscribe(pool, 0, std::ptr::null());
        assert!(!sub_id_ptr.is_null());

        let sub_id_cstr = unsafe { CStr::from_ptr(sub_id_ptr) };
        // Copy before freeing.
        let sub_id_owned = CString::new(sub_id_cstr.to_str().unwrap()).unwrap();
        crate::fcn_string_free(sub_id_ptr);

        // Unsubscribe should return 1 (found).
        assert_eq!(fcn_relay_pool_unsubscribe(pool, sub_id_owned.as_ptr()), 1);

        // Unsubscribe again should return 0 (not found).
        assert_eq!(fcn_relay_pool_unsubscribe(pool, sub_id_owned.as_ptr()), 0);

        fcn_relay_pool_free(pool);
    }

    #[test]
    fn relay_pool_status_null_ptr() {
        let result = fcn_relay_pool_status(std::ptr::null());
        assert!(result.is_null());
    }

    #[test]
    fn relay_pool_publish_null_ptr() {
        let event_json = to_cstr("{}");
        let result = fcn_relay_pool_publish(std::ptr::null(), event_json.as_ptr());
        assert!(result.is_null());
    }

    #[test]
    fn relay_pool_add_relay_null_pool() {
        let url = to_cstr("wss://test");
        assert_eq!(
            fcn_relay_pool_add_relay(std::ptr::null_mut(), url.as_ptr()),
            -1
        );
    }

    #[test]
    fn relay_pool_remove_relay_null_pool() {
        let url = to_cstr("wss://test");
        assert_eq!(
            fcn_relay_pool_remove_relay(std::ptr::null_mut(), url.as_ptr()),
            -1
        );
    }
}

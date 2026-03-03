//! FFI bindings for game replay, observer mode, and replay verification.
//!
//! Wraps `freeciv_nostr_net::replay` types as opaque pointer types
//! for C consumption.

use std::os::raw::c_char;

use freeciv_nostr_net::replay::{GameObserver, GameRecording, ReplayController};

use crate::error::{cstr_to_str, set_last_error, string_to_c};

// ---------------------------------------------------------------------------
// Opaque types
// ---------------------------------------------------------------------------

/// Opaque handle to a game recording.
///
/// Created by `fcn_game_recording_from_events()`.
/// Must be freed with `fcn_game_recording_free()`.
pub struct FcnGameRecording {
    inner: GameRecording,
}

/// Opaque handle to a replay controller.
///
/// Created by `fcn_replay_controller_new()`.
/// Must be freed with `fcn_replay_controller_free()`.
pub struct FcnReplayController {
    inner: ReplayController,
}

/// Opaque handle to a game observer.
///
/// Created by `fcn_game_observer_new()`.
/// Must be freed with `fcn_game_observer_free()`.
pub struct FcnGameObserver {
    inner: GameObserver,
}

// ---------------------------------------------------------------------------
// GameRecording FFI
// ---------------------------------------------------------------------------

/// Create a game recording from a JSON array of Nostr event strings.
///
/// `game_id`: hex-encoded game event ID (C string).
/// `events_json`: a JSON array of Nostr event objects serialized as strings,
///   e.g. `["<event1_json>", "<event2_json>", ...]`.
///
/// Returns an opaque handle, or `NULL` if parsing failed or no valid events
/// were found (check `fcn_last_error()`).
/// The caller must free with `fcn_game_recording_free()`.
///
/// # Safety
///
/// - `game_id` must be a valid null-terminated UTF-8 string.
/// - `events_json` must be a valid null-terminated UTF-8 string containing
///   a JSON array of strings.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_recording_from_events(
    game_id: *const c_char,
    events_json: *const c_char,
) -> *mut FcnGameRecording {
    let gid = match cstr_to_str(game_id) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let json_str = match cstr_to_str(events_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let event_strings: Vec<String> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid events JSON array: {e}"));
            return std::ptr::null_mut();
        }
    };

    match GameRecording::from_events(gid, &event_strings) {
        Some(recording) => Box::into_raw(Box::new(FcnGameRecording { inner: recording })),
        None => {
            set_last_error("no valid events found for recording");
            std::ptr::null_mut()
        }
    }
}

/// Free a game recording handle.
///
/// # Safety
///
/// `recording` must be a valid pointer from `fcn_game_recording_from_events()`,
/// or `NULL` (which is a no-op).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_recording_free(recording: *mut FcnGameRecording) {
    if !recording.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(recording);
        }
    }
}

/// Verify the integrity of a game recording.
///
/// Returns a JSON string with the `ReplayVerification` result.
/// The caller must free the returned string with `fcn_string_free()`.
/// Returns `NULL` on error.
///
/// # Safety
///
/// `recording` must be a valid pointer from `fcn_game_recording_from_events()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_recording_verify(recording: *const FcnGameRecording) -> *mut c_char {
    if recording.is_null() {
        set_last_error("null recording pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees recording is valid.
    let rec = unsafe { &*recording };
    let verification = rec.inner.verify();
    match serde_json::to_string(&verification) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize verification: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Get the total number of turns in the recording.
///
/// Returns -1 on error (null pointer).
///
/// # Safety
///
/// `recording` must be a valid pointer from `fcn_game_recording_from_events()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_recording_total_turns(recording: *const FcnGameRecording) -> i64 {
    if recording.is_null() {
        set_last_error("null recording pointer");
        return -1;
    }
    // SAFETY: Caller guarantees recording is valid.
    let rec = unsafe { &*recording };
    rec.inner.total_turns as i64
}

/// Get all actions for a specific turn as a JSON array string.
///
/// Returns a JSON array of `RecordedAction` objects, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// `recording` must be a valid pointer from `fcn_game_recording_from_events()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_recording_actions_for_turn(
    recording: *const FcnGameRecording,
    turn: u64,
) -> *mut c_char {
    if recording.is_null() {
        set_last_error("null recording pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees recording is valid.
    let rec = unsafe { &*recording };
    let actions = rec.inner.actions_for_turn(turn);
    match serde_json::to_string(&actions) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize actions: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Get a Nostr share link for the recording.
///
/// Returns a `nostr:naddr:...` string, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
///
/// # Safety
///
/// `recording` must be a valid pointer from `fcn_game_recording_from_events()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_recording_share_link(recording: *const FcnGameRecording) -> *mut c_char {
    if recording.is_null() {
        set_last_error("null recording pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees recording is valid.
    let rec = unsafe { &*recording };
    string_to_c(rec.inner.to_nostr_share_link())
}

// ---------------------------------------------------------------------------
// ReplayController FFI
// ---------------------------------------------------------------------------

/// Create a new replay controller for a game recording.
///
/// Consumes the recording — the recording pointer must NOT be used after
/// this call. The recording is moved into the controller.
///
/// Returns an opaque handle, or `NULL` on error.
/// The caller must free with `fcn_replay_controller_free()`.
///
/// # Safety
///
/// `recording` must be a valid pointer from `fcn_game_recording_from_events()`.
/// After this call, the `recording` pointer is consumed and must not be used.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_new(
    recording: *mut FcnGameRecording,
) -> *mut FcnReplayController {
    if recording.is_null() {
        set_last_error("null recording pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees recording was returned by
    // fcn_game_recording_from_events() and has not been freed.
    let rec = unsafe { Box::from_raw(recording) };
    Box::into_raw(Box::new(FcnReplayController {
        inner: ReplayController::new(rec.inner),
    }))
}

/// Free a replay controller handle.
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`,
/// or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_free(controller: *mut FcnReplayController) {
    if !controller.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(controller);
        }
    }
}

/// Start or resume playback.
///
/// Returns 0 on success, -1 on error (null pointer).
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_play(controller: *mut FcnReplayController) -> i32 {
    if controller.is_null() {
        set_last_error("null controller pointer");
        return -1;
    }
    // SAFETY: Caller guarantees controller is valid.
    let ctrl = unsafe { &mut *controller };
    ctrl.inner.play();
    0
}

/// Pause playback.
///
/// Returns 0 on success, -1 on error (null pointer).
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_pause(controller: *mut FcnReplayController) -> i32 {
    if controller.is_null() {
        set_last_error("null controller pointer");
        return -1;
    }
    // SAFETY: Caller guarantees controller is valid.
    let ctrl = unsafe { &mut *controller };
    ctrl.inner.pause();
    0
}

/// Stop playback and reset to the beginning.
///
/// Returns 0 on success, -1 on error (null pointer).
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_stop(controller: *mut FcnReplayController) -> i32 {
    if controller.is_null() {
        set_last_error("null controller pointer");
        return -1;
    }
    // SAFETY: Caller guarantees controller is valid.
    let ctrl = unsafe { &mut *controller };
    ctrl.inner.stop();
    0
}

/// Step one action forward.
///
/// Returns a JSON string of the action stepped over, or `NULL` if at end
/// or on error. The caller must free the returned string with
/// `fcn_string_free()`.
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_step_forward(
    controller: *mut FcnReplayController,
) -> *mut c_char {
    if controller.is_null() {
        set_last_error("null controller pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees controller is valid.
    let ctrl = unsafe { &mut *controller };
    match ctrl.inner.step_forward() {
        Some(action) => match serde_json::to_string(action) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error(&format!("failed to serialize action: {e}"));
                std::ptr::null_mut()
            }
        },
        None => std::ptr::null_mut(),
    }
}

/// Step one action backward.
///
/// Returns a JSON string of the action at the new position, or `NULL` if
/// at the beginning or on error. The caller must free the returned string
/// with `fcn_string_free()`.
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_step_backward(
    controller: *mut FcnReplayController,
) -> *mut c_char {
    if controller.is_null() {
        set_last_error("null controller pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees controller is valid.
    let ctrl = unsafe { &mut *controller };
    match ctrl.inner.step_backward() {
        Some(action) => match serde_json::to_string(action) {
            Ok(json) => string_to_c(json),
            Err(e) => {
                set_last_error(&format!("failed to serialize action: {e}"));
                std::ptr::null_mut()
            }
        },
        None => std::ptr::null_mut(),
    }
}

/// Jump to a specific turn.
///
/// Returns 1 if the turn was found, 0 if not found, -1 on error.
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_jump_to_turn(
    controller: *mut FcnReplayController,
    turn: u64,
) -> i32 {
    if controller.is_null() {
        set_last_error("null controller pointer");
        return -1;
    }
    // SAFETY: Caller guarantees controller is valid.
    let ctrl = unsafe { &mut *controller };
    if ctrl.inner.jump_to_turn(turn) { 1 } else { 0 }
}

/// Get the current turn number.
///
/// Returns -1 on error (null pointer).
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_current_turn(
    controller: *const FcnReplayController,
) -> i64 {
    if controller.is_null() {
        set_last_error("null controller pointer");
        return -1;
    }
    // SAFETY: Caller guarantees controller is valid.
    let ctrl = unsafe { &*controller };
    ctrl.inner.current_turn() as i64
}

/// Get replay progress as a value in [0.0, 1.0].
///
/// Returns -1.0 on error (null pointer).
///
/// # Safety
///
/// `controller` must be a valid pointer from `fcn_replay_controller_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_replay_controller_progress(controller: *const FcnReplayController) -> f64 {
    if controller.is_null() {
        set_last_error("null controller pointer");
        return -1.0;
    }
    // SAFETY: Caller guarantees controller is valid.
    let ctrl = unsafe { &*controller };
    ctrl.inner.progress()
}

// ---------------------------------------------------------------------------
// GameObserver FFI
// ---------------------------------------------------------------------------

/// Create a new game observer.
///
/// `game_id`: hex-encoded game event ID (C string).
///
/// Returns an opaque handle, or `NULL` on error.
/// The caller must free with `fcn_game_observer_free()`.
///
/// # Safety
///
/// `game_id` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_observer_new(game_id: *const c_char) -> *mut FcnGameObserver {
    let gid = match cstr_to_str(game_id) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    Box::into_raw(Box::new(FcnGameObserver {
        inner: GameObserver::new(gid),
    }))
}

/// Free a game observer handle.
///
/// # Safety
///
/// `observer` must be a valid pointer from `fcn_game_observer_new()`,
/// or `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_observer_free(observer: *mut FcnGameObserver) {
    if !observer.is_null() {
        // SAFETY: Caller guarantees this pointer was allocated by us.
        unsafe {
            let _ = Box::from_raw(observer);
        }
    }
}

/// Feed a Nostr event to the observer.
///
/// `event_json`: a JSON-serialized Nostr event (C string).
///
/// Returns 1 if the event was a game action and was recorded, 0 if it
/// was not a game action or was ignored, -1 on error (null pointer).
///
/// # Safety
///
/// - `observer` must be a valid pointer from `fcn_game_observer_new()`.
/// - `event_json` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_observer_receive_event(
    observer: *mut FcnGameObserver,
    event_json: *const c_char,
) -> i32 {
    if observer.is_null() {
        set_last_error("null observer pointer");
        return -1;
    }
    let json_str = match cstr_to_str(event_json) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees observer is valid.
    let obs = unsafe { &mut *observer };
    if obs.inner.receive_event(json_str) {
        1
    } else {
        0
    }
}

/// Get the current turn number as seen by the observer.
///
/// Returns -1 on error (null pointer).
///
/// # Safety
///
/// `observer` must be a valid pointer from `fcn_game_observer_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_observer_current_turn(observer: *const FcnGameObserver) -> i64 {
    if observer.is_null() {
        set_last_error("null observer pointer");
        return -1;
    }
    // SAFETY: Caller guarantees observer is valid.
    let obs = unsafe { &*observer };
    obs.inner.current_turn() as i64
}

/// Get the number of actions recorded by the observer.
///
/// Returns -1 on error (null pointer).
///
/// # Safety
///
/// `observer` must be a valid pointer from `fcn_game_observer_new()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_game_observer_action_count(observer: *const FcnGameObserver) -> i64 {
    if observer.is_null() {
        set_last_error("null observer pointer");
        return -1;
    }
    // SAFETY: Caller guarantees observer is valid.
    let obs = unsafe { &*observer };
    obs.inner.action_count() as i64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::fcn_last_error;
    use std::ffi::{CStr, CString};

    // -- Helper: build a minimal action event JSON for testing ---------------

    fn make_action_event_json(turn: u64, seq: u64) -> String {
        let content = serde_json::to_string(&serde_json::json!({
            "packet_type": 84,
            "turn": turn,
            "phase": 0,
            "sequence": seq,
            "prev_event_id": "",
            "payload": {}
        }))
        .unwrap();

        serde_json::to_string(&serde_json::json!({
            "id": format!("evt{seq}"),
            "pubkey": "aaaa",
            "kind": 4202,
            "content": content,
            "tags": [
                ["e", "game0000"],
                ["turn", turn.to_string()],
                ["phase", "0"],
                ["seq", seq.to_string()]
            ],
            "sig": "valid_sig_abc",
            "created_at": 1700000000u64
        }))
        .unwrap()
    }

    fn make_events_json(events: &[String]) -> String {
        serde_json::to_string(events).unwrap()
    }

    // =====================================================================
    // GameRecording FFI tests
    // =====================================================================

    #[test]
    fn recording_from_events_and_free() {
        let events = vec![make_action_event_json(1, 0), make_action_event_json(1, 1)];
        let events_json = CString::new(make_events_json(&events)).unwrap();
        let game_id = CString::new("game0000").unwrap();

        let ptr = fcn_game_recording_from_events(game_id.as_ptr(), events_json.as_ptr());
        assert!(!ptr.is_null());
        fcn_game_recording_free(ptr);
    }

    #[test]
    fn recording_from_events_null_game_id() {
        let events = vec![make_action_event_json(1, 0)];
        let events_json = CString::new(make_events_json(&events)).unwrap();

        let ptr = fcn_game_recording_from_events(std::ptr::null(), events_json.as_ptr());
        assert!(ptr.is_null());
    }

    #[test]
    fn recording_from_events_null_events() {
        let game_id = CString::new("game0000").unwrap();
        let ptr = fcn_game_recording_from_events(game_id.as_ptr(), std::ptr::null());
        assert!(ptr.is_null());
    }

    #[test]
    fn recording_from_events_invalid_json() {
        let game_id = CString::new("game0000").unwrap();
        let bad_json = CString::new("not json").unwrap();
        let ptr = fcn_game_recording_from_events(game_id.as_ptr(), bad_json.as_ptr());
        assert!(ptr.is_null());
    }

    #[test]
    fn recording_from_events_empty_array() {
        let game_id = CString::new("game0000").unwrap();
        let empty = CString::new("[]").unwrap();
        let ptr = fcn_game_recording_from_events(game_id.as_ptr(), empty.as_ptr());
        assert!(ptr.is_null());
    }

    #[test]
    fn recording_free_null_is_noop() {
        fcn_game_recording_free(std::ptr::null_mut());
    }

    #[test]
    fn recording_total_turns() {
        let events = vec![
            make_action_event_json(1, 0),
            make_action_event_json(2, 1),
            make_action_event_json(3, 2),
        ];
        let events_json = CString::new(make_events_json(&events)).unwrap();
        let game_id = CString::new("game0000").unwrap();

        let ptr = fcn_game_recording_from_events(game_id.as_ptr(), events_json.as_ptr());
        assert!(!ptr.is_null());

        let turns = fcn_game_recording_total_turns(ptr);
        assert_eq!(turns, 3);

        fcn_game_recording_free(ptr);
    }

    #[test]
    fn recording_total_turns_null() {
        assert_eq!(fcn_game_recording_total_turns(std::ptr::null()), -1);
    }

    #[test]
    fn recording_verify() {
        let events = vec![make_action_event_json(1, 0)];
        let events_json = CString::new(make_events_json(&events)).unwrap();
        let game_id = CString::new("game0000").unwrap();

        let ptr = fcn_game_recording_from_events(game_id.as_ptr(), events_json.as_ptr());
        assert!(!ptr.is_null());

        let result_ptr = fcn_game_recording_verify(ptr);
        assert!(!result_ptr.is_null());

        let result_json = unsafe { CStr::from_ptr(result_ptr) }.to_str().unwrap();
        let result: serde_json::Value = serde_json::from_str(result_json).unwrap();
        assert_eq!(result["signatures_valid"], true);
        assert_eq!(result["actions_verified"], 1);

        crate::fcn_string_free(result_ptr);
        fcn_game_recording_free(ptr);
    }

    #[test]
    fn recording_verify_null() {
        assert!(fcn_game_recording_verify(std::ptr::null()).is_null());
    }

    #[test]
    fn recording_actions_for_turn() {
        let events = vec![
            make_action_event_json(1, 0),
            make_action_event_json(1, 1),
            make_action_event_json(2, 2),
        ];
        let events_json = CString::new(make_events_json(&events)).unwrap();
        let game_id = CString::new("game0000").unwrap();

        let ptr = fcn_game_recording_from_events(game_id.as_ptr(), events_json.as_ptr());
        assert!(!ptr.is_null());

        let turn1_ptr = fcn_game_recording_actions_for_turn(ptr, 1);
        assert!(!turn1_ptr.is_null());

        let turn1_json = unsafe { CStr::from_ptr(turn1_ptr) }.to_str().unwrap();
        let turn1_actions: Vec<serde_json::Value> = serde_json::from_str(turn1_json).unwrap();
        assert_eq!(turn1_actions.len(), 2);

        crate::fcn_string_free(turn1_ptr);
        fcn_game_recording_free(ptr);
    }

    #[test]
    fn recording_actions_for_turn_null() {
        assert!(fcn_game_recording_actions_for_turn(std::ptr::null(), 1).is_null());
    }

    #[test]
    fn recording_share_link() {
        let events = vec![make_action_event_json(1, 0)];
        let events_json = CString::new(make_events_json(&events)).unwrap();
        let game_id = CString::new("game0000").unwrap();

        let ptr = fcn_game_recording_from_events(game_id.as_ptr(), events_json.as_ptr());
        assert!(!ptr.is_null());

        let link_ptr = fcn_game_recording_share_link(ptr);
        assert!(!link_ptr.is_null());

        let link = unsafe { CStr::from_ptr(link_ptr) }.to_str().unwrap();
        assert!(link.starts_with("nostr:naddr:"));

        crate::fcn_string_free(link_ptr);
        fcn_game_recording_free(ptr);
    }

    #[test]
    fn recording_share_link_null() {
        assert!(fcn_game_recording_share_link(std::ptr::null()).is_null());
    }

    // =====================================================================
    // ReplayController FFI tests
    // =====================================================================

    fn make_test_controller() -> *mut FcnReplayController {
        let events = vec![
            make_action_event_json(1, 0),
            make_action_event_json(1, 1),
            make_action_event_json(2, 2),
        ];
        let events_json = CString::new(make_events_json(&events)).unwrap();
        let game_id = CString::new("game0000").unwrap();
        let rec_ptr = fcn_game_recording_from_events(game_id.as_ptr(), events_json.as_ptr());
        assert!(!rec_ptr.is_null());
        let ctrl_ptr = fcn_replay_controller_new(rec_ptr);
        assert!(!ctrl_ptr.is_null());
        ctrl_ptr
    }

    #[test]
    fn controller_new_and_free() {
        let ctrl = make_test_controller();
        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_new_null() {
        let ptr = fcn_replay_controller_new(std::ptr::null_mut());
        assert!(ptr.is_null());
    }

    #[test]
    fn controller_free_null_is_noop() {
        fcn_replay_controller_free(std::ptr::null_mut());
    }

    #[test]
    fn controller_play_pause_stop() {
        let ctrl = make_test_controller();

        assert_eq!(fcn_replay_controller_play(ctrl), 0);
        assert_eq!(fcn_replay_controller_pause(ctrl), 0);
        assert_eq!(fcn_replay_controller_stop(ctrl), 0);

        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_play_null() {
        assert_eq!(fcn_replay_controller_play(std::ptr::null_mut()), -1);
    }

    #[test]
    fn controller_pause_null() {
        assert_eq!(fcn_replay_controller_pause(std::ptr::null_mut()), -1);
    }

    #[test]
    fn controller_stop_null() {
        assert_eq!(fcn_replay_controller_stop(std::ptr::null_mut()), -1);
    }

    #[test]
    fn controller_step_forward_returns_json() {
        let ctrl = make_test_controller();

        let action_ptr = fcn_replay_controller_step_forward(ctrl);
        assert!(!action_ptr.is_null());

        let action_json = unsafe { CStr::from_ptr(action_ptr) }.to_str().unwrap();
        let action: serde_json::Value = serde_json::from_str(action_json).unwrap();
        assert_eq!(action["sequence"], 0);

        crate::fcn_string_free(action_ptr);
        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_step_forward_null() {
        assert!(fcn_replay_controller_step_forward(std::ptr::null_mut()).is_null());
    }

    #[test]
    fn controller_step_backward_returns_json() {
        let ctrl = make_test_controller();

        // Step forward twice, then backward once
        let ptr1 = fcn_replay_controller_step_forward(ctrl);
        crate::fcn_string_free(ptr1);
        let ptr2 = fcn_replay_controller_step_forward(ctrl);
        crate::fcn_string_free(ptr2);

        let back_ptr = fcn_replay_controller_step_backward(ctrl);
        assert!(!back_ptr.is_null());

        let action_json = unsafe { CStr::from_ptr(back_ptr) }.to_str().unwrap();
        let action: serde_json::Value = serde_json::from_str(action_json).unwrap();
        assert_eq!(action["sequence"], 1);

        crate::fcn_string_free(back_ptr);
        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_step_backward_at_start_returns_null() {
        let ctrl = make_test_controller();
        assert!(fcn_replay_controller_step_backward(ctrl).is_null());
        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_step_backward_null() {
        assert!(fcn_replay_controller_step_backward(std::ptr::null_mut()).is_null());
    }

    #[test]
    fn controller_jump_to_turn() {
        let ctrl = make_test_controller();

        assert_eq!(fcn_replay_controller_jump_to_turn(ctrl, 2), 1);
        assert_eq!(fcn_replay_controller_current_turn(ctrl), 2);

        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_jump_to_turn_not_found() {
        let ctrl = make_test_controller();
        assert_eq!(fcn_replay_controller_jump_to_turn(ctrl, 999), 0);
        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_jump_to_turn_null() {
        assert_eq!(
            fcn_replay_controller_jump_to_turn(std::ptr::null_mut(), 1),
            -1
        );
    }

    #[test]
    fn controller_current_turn() {
        let ctrl = make_test_controller();

        assert_eq!(fcn_replay_controller_current_turn(ctrl), 0);

        fcn_replay_controller_step_forward(ctrl);
        assert_eq!(fcn_replay_controller_current_turn(ctrl), 1);

        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_current_turn_null() {
        assert_eq!(fcn_replay_controller_current_turn(std::ptr::null()), -1);
    }

    #[test]
    fn controller_progress() {
        let ctrl = make_test_controller();

        let p0 = fcn_replay_controller_progress(ctrl);
        assert_eq!(p0, 0.0);

        fcn_replay_controller_step_forward(ctrl);
        let p1 = fcn_replay_controller_progress(ctrl);
        assert!(p1 > 0.0 && p1 < 1.0);

        fcn_replay_controller_free(ctrl);
    }

    #[test]
    fn controller_progress_null() {
        assert_eq!(fcn_replay_controller_progress(std::ptr::null()), -1.0);
    }

    // =====================================================================
    // GameObserver FFI tests
    // =====================================================================

    #[test]
    fn observer_new_and_free() {
        let game_id = CString::new("game0000").unwrap();
        let ptr = fcn_game_observer_new(game_id.as_ptr());
        assert!(!ptr.is_null());
        fcn_game_observer_free(ptr);
    }

    #[test]
    fn observer_new_null() {
        let ptr = fcn_game_observer_new(std::ptr::null());
        assert!(ptr.is_null());
    }

    #[test]
    fn observer_free_null_is_noop() {
        fcn_game_observer_free(std::ptr::null_mut());
    }

    #[test]
    fn observer_receive_event() {
        let game_id = CString::new("game0000").unwrap();
        let ptr = fcn_game_observer_new(game_id.as_ptr());

        let event = CString::new(make_action_event_json(1, 0)).unwrap();
        let result = fcn_game_observer_receive_event(ptr, event.as_ptr());
        assert_eq!(result, 1);

        assert_eq!(fcn_game_observer_action_count(ptr), 1);
        assert_eq!(fcn_game_observer_current_turn(ptr), 1);

        fcn_game_observer_free(ptr);
    }

    #[test]
    fn observer_receive_event_null_observer() {
        let event = CString::new(make_action_event_json(1, 0)).unwrap();
        assert_eq!(
            fcn_game_observer_receive_event(std::ptr::null_mut(), event.as_ptr()),
            -1
        );
    }

    #[test]
    fn observer_receive_event_null_json() {
        let game_id = CString::new("game0000").unwrap();
        let ptr = fcn_game_observer_new(game_id.as_ptr());

        assert_eq!(fcn_game_observer_receive_event(ptr, std::ptr::null()), -1);

        fcn_game_observer_free(ptr);
    }

    #[test]
    fn observer_current_turn_null() {
        assert_eq!(fcn_game_observer_current_turn(std::ptr::null()), -1);
    }

    #[test]
    fn observer_action_count_null() {
        assert_eq!(fcn_game_observer_action_count(std::ptr::null()), -1);
    }

    #[test]
    fn all_null_safety() {
        // Recording
        assert!(fcn_game_recording_from_events(std::ptr::null(), std::ptr::null()).is_null());
        fcn_game_recording_free(std::ptr::null_mut());
        assert!(fcn_game_recording_verify(std::ptr::null()).is_null());
        assert_eq!(fcn_game_recording_total_turns(std::ptr::null()), -1);
        assert!(fcn_game_recording_actions_for_turn(std::ptr::null(), 0).is_null());
        assert!(fcn_game_recording_share_link(std::ptr::null()).is_null());

        // Controller
        assert!(fcn_replay_controller_new(std::ptr::null_mut()).is_null());
        fcn_replay_controller_free(std::ptr::null_mut());
        assert_eq!(fcn_replay_controller_play(std::ptr::null_mut()), -1);
        assert_eq!(fcn_replay_controller_pause(std::ptr::null_mut()), -1);
        assert_eq!(fcn_replay_controller_stop(std::ptr::null_mut()), -1);
        assert!(fcn_replay_controller_step_forward(std::ptr::null_mut()).is_null());
        assert!(fcn_replay_controller_step_backward(std::ptr::null_mut()).is_null());
        assert_eq!(
            fcn_replay_controller_jump_to_turn(std::ptr::null_mut(), 0),
            -1
        );
        assert_eq!(fcn_replay_controller_current_turn(std::ptr::null()), -1);
        assert_eq!(fcn_replay_controller_progress(std::ptr::null()), -1.0);

        // Observer
        assert!(fcn_game_observer_new(std::ptr::null()).is_null());
        fcn_game_observer_free(std::ptr::null_mut());
        assert_eq!(
            fcn_game_observer_receive_event(std::ptr::null_mut(), std::ptr::null()),
            -1
        );
        assert_eq!(fcn_game_observer_current_turn(std::ptr::null()), -1);
        assert_eq!(fcn_game_observer_action_count(std::ptr::null()), -1);

        // Verify error was set for last call
        let err = fcn_last_error();
        assert!(!err.is_null());
    }
}

//! C FFI bindings for P2P networking (endpoint, gossip, blobs).
//!
//! All async operations are bridged to synchronous C calls via a shared
//! tokio runtime. The runtime is created on the first call to any `fcn_net_*`
//! function and lives until `fcn_net_shutdown()`.
//!
//! # Thread Safety
//!
//! The runtime is global and thread-safe. FFI handles (`FcnEndpoint`,
//! `FcnGossip`, `FcnBlobs`) are opaque pointers that must not be shared
//! across threads without external synchronization.

use std::os::raw::c_char;
use std::sync::OnceLock;

use tokio::runtime::Runtime;

use freeciv_nostr_net::blobs::GameBlobs;
use freeciv_nostr_net::endpoint::GameEndpoint;
use freeciv_nostr_net::gossip::{GameGossip, GameGossipReceiver, GameGossipSender, GossipEvent};
use freeciv_nostr_net::message::FramedMessage;
use freeciv_nostr_net::protocol::StreamId;

use crate::error::{cstr_to_str, set_last_error, set_last_error_from, string_to_c};

// ---------------------------------------------------------------------------
// Tokio runtime
// ---------------------------------------------------------------------------

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime")
    })
}

/// Block on an async future from synchronous FFI code.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    runtime().block_on(f)
}

// ---------------------------------------------------------------------------
// Opaque types
// ---------------------------------------------------------------------------

/// Opaque handle to a per-game P2P endpoint.
pub struct FcnEndpoint {
    inner: GameEndpoint,
}

/// Opaque handle to a gossip channel for a game session.
pub struct FcnGossip {
    inner: GameGossip,
}

/// Opaque handle to a gossip sender (clone-safe).
pub struct FcnGossipSender {
    inner: GameGossipSender,
}

/// Opaque handle to a gossip receiver.
pub struct FcnGossipReceiver {
    inner: GameGossipReceiver,
}

/// Opaque handle to a blob store.
pub struct FcnBlobs {
    inner: GameBlobs,
}

/// Gossip event type tag for C.
#[repr(C)]
pub enum FcnGossipEventType {
    /// A framed message was received.
    Received = 0,
    /// A new peer joined the topic.
    NeighborUp = 1,
    /// A peer left the topic.
    NeighborDown = 2,
    /// Messages were dropped due to lag.
    Lagged = 3,
    /// No event available (try_recv returned empty).
    None = 4,
    /// An error occurred.
    Error = 5,
}

// ---------------------------------------------------------------------------
// Runtime lifecycle
// ---------------------------------------------------------------------------

/// Initialize the networking runtime.
///
/// This is called automatically on first use, but can be called explicitly
/// for deterministic initialization. Safe to call multiple times.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_net_init() {
    let _ = runtime();
}

// ---------------------------------------------------------------------------
// Endpoint FFI
// ---------------------------------------------------------------------------

/// Create a new per-game ephemeral endpoint.
///
/// Returns an opaque handle, or `NULL` on error (check `fcn_last_error()`).
/// The caller must free with `fcn_endpoint_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_endpoint_new() -> *mut FcnEndpoint {
    match block_on(GameEndpoint::new()) {
        Ok(ep) => Box::into_raw(Box::new(FcnEndpoint { inner: ep })),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Get the endpoint's ID as a hex string.
///
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_endpoint_id(ep: *const FcnEndpoint) -> *mut c_char {
    if ep.is_null() {
        set_last_error("null endpoint pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees ep is valid.
    let ep = unsafe { &*ep };
    string_to_c(ep.inner.endpoint_id().to_string())
}

/// Connect to a peer by their endpoint address string.
///
/// Returns the peer's endpoint ID as a hex string, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_endpoint_connect(
    ep: *const FcnEndpoint,
    peer_addr: *const c_char,
) -> *mut c_char {
    if ep.is_null() {
        set_last_error("null endpoint pointer");
        return std::ptr::null_mut();
    }
    let addr_str = match cstr_to_str(peer_addr) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    // SAFETY: Caller guarantees ep is valid.
    let ep = unsafe { &*ep };

    let addr: iroh::EndpointAddr = match serde_json::from_str(addr_str) {
        Ok(a) => a,
        Err(e) => {
            set_last_error(&format!("invalid endpoint address JSON: {e}"));
            return std::ptr::null_mut();
        }
    };

    match block_on(ep.inner.connect(addr)) {
        Ok(peer_id) => string_to_c(peer_id.to_string()),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Get the number of connected peers.
///
/// Returns -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_endpoint_peer_count(ep: *const FcnEndpoint) -> i32 {
    if ep.is_null() {
        set_last_error("null endpoint pointer");
        return -1;
    }
    // SAFETY: Caller guarantees ep is valid.
    let ep = unsafe { &*ep };
    block_on(ep.inner.peer_count()) as i32
}

/// Shut down the endpoint, closing all connections.
///
/// Consumes the handle — do NOT use `ep` after this call.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_endpoint_shutdown(ep: *mut FcnEndpoint) -> i32 {
    if ep.is_null() {
        set_last_error("null endpoint pointer");
        return -1;
    }
    // SAFETY: Caller guarantees ep was returned by fcn_endpoint_new()
    // and has not been freed.
    let ep = unsafe { Box::from_raw(ep) };
    match block_on(ep.inner.shutdown()) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Free an endpoint handle without shutting down cleanly.
///
/// Prefer `fcn_endpoint_shutdown()` for clean disconnect.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_endpoint_free(ep: *mut FcnEndpoint) {
    if !ep.is_null() {
        // SAFETY: Caller guarantees ep was returned by fcn_endpoint_new().
        unsafe {
            let _ = Box::from_raw(ep);
        }
    }
}

/// Get the endpoint's address as a string (includes relay info for NAT traversal).
///
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_endpoint_addr(ep: *const FcnEndpoint) -> *mut c_char {
    if ep.is_null() {
        set_last_error("null endpoint pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees ep is valid.
    let ep = unsafe { &*ep };
    match serde_json::to_string(&ep.inner.endpoint_addr()) {
        Ok(json) => string_to_c(json),
        Err(e) => {
            set_last_error(&format!("failed to serialize endpoint address: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Accept an incoming peer connection.
///
/// Blocks until a peer connects. Returns the peer's endpoint ID as a hex string.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_endpoint_accept(ep: *const FcnEndpoint) -> *mut c_char {
    if ep.is_null() {
        set_last_error("null endpoint pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees ep is valid.
    let ep = unsafe { &*ep };
    match block_on(ep.inner.accept()) {
        Ok(peer_id) => string_to_c(peer_id.to_string()),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Gossip FFI
// ---------------------------------------------------------------------------

/// Create a new gossip instance for a game session.
///
/// `game_event_id_hex` is the Nostr event ID of the game's root event.
/// The caller must free with `fcn_gossip_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_gossip_new(
    ep: *const FcnEndpoint,
    game_event_id_hex: *const c_char,
) -> *mut FcnGossip {
    if ep.is_null() {
        set_last_error("null endpoint pointer");
        return std::ptr::null_mut();
    }
    let game_id = match cstr_to_str(game_event_id_hex) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    // SAFETY: Caller guarantees ep is valid.
    let ep = unsafe { &*ep };

    // Clone the iroh endpoint (it's Arc-based internally, so this is cheap).
    let iroh_ep = ep.inner.endpoint().clone();
    // GameGossip::new() internally spawns gossip actors that require a
    // tokio runtime context, so we must run it inside block_on.
    let gossip = block_on(async { GameGossip::new(iroh_ep, game_id) });
    Box::into_raw(Box::new(FcnGossip { inner: gossip }))
}

/// Subscribe to the game topic and get sender/receiver handles.
///
/// Returns 0 on success, -1 on error. On success, `out_sender` and
/// `out_receiver` are populated with handles.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_gossip_subscribe(
    gossip: *const FcnGossip,
    out_sender: *mut *mut FcnGossipSender,
    out_receiver: *mut *mut FcnGossipReceiver,
) -> i32 {
    if gossip.is_null() || out_sender.is_null() || out_receiver.is_null() {
        set_last_error("null pointer argument");
        return -1;
    }
    // SAFETY: Caller guarantees pointers are valid.
    let gossip = unsafe { &*gossip };

    match block_on(gossip.inner.subscribe(vec![])) {
        Ok((sender, receiver)) => {
            // SAFETY: out_sender and out_receiver are valid pointers.
            unsafe {
                *out_sender = Box::into_raw(Box::new(FcnGossipSender { inner: sender }));
                *out_receiver = Box::into_raw(Box::new(FcnGossipReceiver { inner: receiver }));
            }
            0
        }
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Broadcast a message to all peers on the gossip topic.
///
/// `stream_id`: 0=GameActions, 1=StateSync, 2=Chat, 3=Heartbeat.
/// `data`/`data_len`: the payload bytes.
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_gossip_broadcast(
    sender: *const FcnGossipSender,
    stream_id: u8,
    data: *const u8,
    data_len: usize,
) -> i32 {
    if sender.is_null() {
        set_last_error("null sender pointer");
        return -1;
    }
    if data.is_null() && data_len > 0 {
        set_last_error("null data pointer with non-zero length");
        return -1;
    }

    let sid = match StreamId::try_from(stream_id) {
        Ok(s) => s,
        Err(_) => {
            set_last_error(&format!("invalid stream_id: {stream_id}"));
            return -1;
        }
    };

    // SAFETY: Caller guarantees data is valid for data_len bytes.
    let payload = if data_len > 0 {
        unsafe { std::slice::from_raw_parts(data, data_len) }.to_vec()
    } else {
        vec![]
    };

    // SAFETY: Caller guarantees sender is valid.
    let sender = unsafe { &*sender };
    let msg = FramedMessage {
        stream_id: sid,
        payload,
    };

    match block_on(sender.inner.broadcast(&msg)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Try to receive a gossip event without blocking.
///
/// Returns the event type. If `Received`, the payload is written to
/// `out_data` (up to `out_data_cap` bytes), `out_data_len` is set to
/// the actual payload length, and `out_stream_id` is set.
///
/// If `NeighborUp` or `NeighborDown`, `out_peer_id` is populated.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_gossip_try_recv(
    receiver: *mut FcnGossipReceiver,
    out_stream_id: *mut u8,
    out_data: *mut u8,
    out_data_cap: usize,
    out_data_len: *mut usize,
    out_peer_id: *mut *mut c_char,
) -> FcnGossipEventType {
    if receiver.is_null() {
        set_last_error("null receiver pointer");
        return FcnGossipEventType::Error;
    }
    // SAFETY: Caller guarantees receiver is valid.
    let receiver = unsafe { &mut *receiver };

    match receiver.inner.try_recv() {
        Ok(GossipEvent::Received {
            message,
            delivered_from,
        }) => {
            if !out_stream_id.is_null() {
                // SAFETY: Caller guarantees out_stream_id is valid.
                unsafe { *out_stream_id = message.stream_id as u8 };
            }
            if !out_data.is_null() && !out_data_len.is_null() {
                let copy_len = message.payload.len().min(out_data_cap);
                // SAFETY: Caller guarantees out_data has out_data_cap capacity.
                unsafe {
                    std::ptr::copy_nonoverlapping(message.payload.as_ptr(), out_data, copy_len);
                    *out_data_len = message.payload.len();
                }
            }
            if !out_peer_id.is_null() {
                // SAFETY: Caller guarantees out_peer_id is valid.
                unsafe { *out_peer_id = string_to_c(delivered_from.to_string()) };
            }
            FcnGossipEventType::Received
        }
        Ok(GossipEvent::NeighborUp(id)) => {
            if !out_peer_id.is_null() {
                unsafe { *out_peer_id = string_to_c(id.to_string()) };
            }
            FcnGossipEventType::NeighborUp
        }
        Ok(GossipEvent::NeighborDown(id)) => {
            if !out_peer_id.is_null() {
                unsafe { *out_peer_id = string_to_c(id.to_string()) };
            }
            FcnGossipEventType::NeighborDown
        }
        Ok(GossipEvent::Lagged) => FcnGossipEventType::Lagged,
        Err(_) => FcnGossipEventType::None,
    }
}

/// Receive a gossip event, blocking until one is available.
///
/// Same output parameters as `fcn_gossip_try_recv()`.
/// Returns `None` when the gossip topic has been shut down.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_gossip_recv(
    receiver: *mut FcnGossipReceiver,
    out_stream_id: *mut u8,
    out_data: *mut u8,
    out_data_cap: usize,
    out_data_len: *mut usize,
    out_peer_id: *mut *mut c_char,
) -> FcnGossipEventType {
    if receiver.is_null() {
        set_last_error("null receiver pointer");
        return FcnGossipEventType::Error;
    }
    // SAFETY: Caller guarantees receiver is valid.
    let receiver = unsafe { &mut *receiver };

    match block_on(receiver.inner.recv()) {
        Some(GossipEvent::Received {
            message,
            delivered_from,
        }) => {
            if !out_stream_id.is_null() {
                unsafe { *out_stream_id = message.stream_id as u8 };
            }
            if !out_data.is_null() && !out_data_len.is_null() {
                let copy_len = message.payload.len().min(out_data_cap);
                unsafe {
                    std::ptr::copy_nonoverlapping(message.payload.as_ptr(), out_data, copy_len);
                    *out_data_len = message.payload.len();
                }
            }
            if !out_peer_id.is_null() {
                unsafe { *out_peer_id = string_to_c(delivered_from.to_string()) };
            }
            FcnGossipEventType::Received
        }
        Some(GossipEvent::NeighborUp(id)) => {
            if !out_peer_id.is_null() {
                unsafe { *out_peer_id = string_to_c(id.to_string()) };
            }
            FcnGossipEventType::NeighborUp
        }
        Some(GossipEvent::NeighborDown(id)) => {
            if !out_peer_id.is_null() {
                unsafe { *out_peer_id = string_to_c(id.to_string()) };
            }
            FcnGossipEventType::NeighborDown
        }
        Some(GossipEvent::Lagged) => FcnGossipEventType::Lagged,
        None => FcnGossipEventType::None,
    }
}

/// Free a gossip sender handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_gossip_sender_free(sender: *mut FcnGossipSender) {
    if !sender.is_null() {
        // SAFETY: Caller guarantees sender was returned by fcn_gossip_subscribe.
        unsafe {
            let _ = Box::from_raw(sender);
        }
    }
}

/// Free a gossip receiver handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_gossip_receiver_free(receiver: *mut FcnGossipReceiver) {
    if !receiver.is_null() {
        // SAFETY: Caller guarantees receiver was returned by fcn_gossip_subscribe.
        unsafe {
            let _ = Box::from_raw(receiver);
        }
    }
}

/// Free a gossip handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_gossip_free(gossip: *mut FcnGossip) {
    if !gossip.is_null() {
        // SAFETY: Caller guarantees gossip was returned by fcn_gossip_new.
        unsafe {
            let _ = Box::from_raw(gossip);
        }
    }
}

// ---------------------------------------------------------------------------
// Blob FFI
// ---------------------------------------------------------------------------

/// Create a new in-memory blob store.
///
/// The caller must free with `fcn_blobs_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_blobs_new() -> *mut FcnBlobs {
    // GameBlobs::new() internally creates a MemStore that requires a
    // tokio runtime context, so we must run it inside block_on.
    let blobs = block_on(async { GameBlobs::new() });
    Box::into_raw(Box::new(FcnBlobs { inner: blobs }))
}

/// Import bytes into the blob store.
///
/// Returns the BLAKE3 hash as a hex string, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_blobs_import(
    blobs: *const FcnBlobs,
    data: *const u8,
    data_len: usize,
) -> *mut c_char {
    if blobs.is_null() {
        set_last_error("null blobs pointer");
        return std::ptr::null_mut();
    }
    if data.is_null() && data_len > 0 {
        set_last_error("null data pointer with non-zero length");
        return std::ptr::null_mut();
    }

    // SAFETY: Caller guarantees data is valid for data_len bytes.
    let bytes = if data_len > 0 {
        unsafe { std::slice::from_raw_parts(data, data_len) }.to_vec()
    } else {
        vec![]
    };

    // SAFETY: Caller guarantees blobs is valid.
    let blobs = unsafe { &*blobs };
    match block_on(blobs.inner.import_bytes(bytes)) {
        Ok(hash) => string_to_c(GameBlobs::hash_to_hex(hash)),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Check if a blob exists in the store.
///
/// `hash_hex` is the 64-char hex hash string.
/// Returns 1 if exists, 0 if not, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_blobs_has(blobs: *const FcnBlobs, hash_hex: *const c_char) -> i32 {
    if blobs.is_null() {
        set_last_error("null blobs pointer");
        return -1;
    }
    let hex = match cstr_to_str(hash_hex) {
        Some(s) => s,
        None => return -1,
    };
    let hash = match GameBlobs::hash_from_hex(hex) {
        Ok(h) => h,
        Err(e) => {
            set_last_error_from(e);
            return -1;
        }
    };

    // SAFETY: Caller guarantees blobs is valid.
    let blobs = unsafe { &*blobs };
    match block_on(blobs.inner.has(hash)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Read a blob from the store into a caller-provided buffer.
///
/// `hash_hex`: 64-char hex hash.
/// `out_data`/`out_data_cap`: caller's buffer.
/// `out_data_len`: set to actual blob size on success.
///
/// Returns 0 on success, 1 if blob not found, -1 on error.
/// If the blob is larger than `out_data_cap`, `out_data_len` is set
/// to the actual size and only `out_data_cap` bytes are copied.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_blobs_get(
    blobs: *const FcnBlobs,
    hash_hex: *const c_char,
    out_data: *mut u8,
    out_data_cap: usize,
    out_data_len: *mut usize,
) -> i32 {
    if blobs.is_null() {
        set_last_error("null blobs pointer");
        return -1;
    }
    let hex = match cstr_to_str(hash_hex) {
        Some(s) => s,
        None => return -1,
    };
    let hash = match GameBlobs::hash_from_hex(hex) {
        Ok(h) => h,
        Err(e) => {
            set_last_error_from(e);
            return -1;
        }
    };

    // SAFETY: Caller guarantees blobs is valid.
    let blobs = unsafe { &*blobs };
    match block_on(blobs.inner.get_bytes(hash)) {
        Ok(Some(data)) => {
            if !out_data_len.is_null() {
                // SAFETY: Caller guarantees out_data_len is valid.
                unsafe { *out_data_len = data.len() };
            }
            if !out_data.is_null() {
                let copy_len = data.len().min(out_data_cap);
                // SAFETY: Caller guarantees out_data has out_data_cap capacity.
                unsafe {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), out_data, copy_len);
                }
            }
            0
        }
        Ok(None) => 1, // not found
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Verify received data against an expected hash and import if valid.
///
/// Returns the hash as a hex string on success, or `NULL` on error
/// (hash mismatch, too large, etc.).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_blobs_verify_and_import(
    blobs: *const FcnBlobs,
    data: *const u8,
    data_len: usize,
    expected_hash_hex: *const c_char,
) -> *mut c_char {
    if blobs.is_null() {
        set_last_error("null blobs pointer");
        return std::ptr::null_mut();
    }
    let hex = match cstr_to_str(expected_hash_hex) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let expected = match GameBlobs::hash_from_hex(hex) {
        Ok(h) => h,
        Err(e) => {
            set_last_error_from(e);
            return std::ptr::null_mut();
        }
    };

    // SAFETY: Caller guarantees data is valid for data_len bytes.
    let bytes = if data_len > 0 && !data.is_null() {
        unsafe { std::slice::from_raw_parts(data, data_len) }.to_vec()
    } else {
        vec![]
    };

    // SAFETY: Caller guarantees blobs is valid.
    let blobs = unsafe { &*blobs };
    match block_on(blobs.inner.verify_and_import(bytes, expected)) {
        Ok(hash) => string_to_c(GameBlobs::hash_to_hex(hash)),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Free a blob store handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_blobs_free(blobs: *mut FcnBlobs) {
    if !blobs.is_null() {
        // SAFETY: Caller guarantees blobs was returned by fcn_blobs_new.
        unsafe {
            let _ = Box::from_raw(blobs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn runtime_initializes() {
        fcn_net_init();
        // Should not panic on second call
        fcn_net_init();
    }

    #[test]
    fn endpoint_lifecycle() {
        let ep = fcn_endpoint_new();
        assert!(!ep.is_null(), "endpoint creation failed");

        let id = fcn_endpoint_id(ep);
        assert!(!id.is_null(), "endpoint id failed");
        let id_str = unsafe { CStr::from_ptr(id) }.to_str().unwrap();
        assert!(!id_str.is_empty());
        crate::error::fcn_string_free(id);

        let count = fcn_endpoint_peer_count(ep);
        assert_eq!(count, 0);

        let ret = fcn_endpoint_shutdown(ep);
        assert_eq!(ret, 0);
    }

    #[test]
    fn endpoint_null_safety() {
        assert!(fcn_endpoint_id(std::ptr::null()).is_null());
        assert_eq!(fcn_endpoint_peer_count(std::ptr::null()), -1);
        assert_eq!(fcn_endpoint_shutdown(std::ptr::null_mut()), -1);
        fcn_endpoint_free(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn blobs_lifecycle() {
        let blobs = fcn_blobs_new();
        assert!(!blobs.is_null());

        // Import some data
        let data = b"hello blob world";
        let hash = fcn_blobs_import(blobs, data.as_ptr(), data.len());
        assert!(!hash.is_null());
        let hash_str = unsafe { CStr::from_ptr(hash) }.to_str().unwrap();
        assert_eq!(hash_str.len(), 64); // BLAKE3 hex = 64 chars

        // Check existence
        let exists = fcn_blobs_has(blobs, hash);
        assert_eq!(exists, 1);

        // Read back
        let mut buf = [0u8; 256];
        let mut len: usize = 0;
        let ret = fcn_blobs_get(blobs, hash, buf.as_mut_ptr(), buf.len(), &mut len);
        assert_eq!(ret, 0);
        assert_eq!(len, data.len());
        assert_eq!(&buf[..len], data);

        crate::error::fcn_string_free(hash);
        fcn_blobs_free(blobs);
    }

    #[test]
    fn blobs_verify_and_import_success() {
        let blobs = fcn_blobs_new();
        let data = b"verify this";

        // Compute expected hash first
        let hash_hex = GameBlobs::hash_to_hex(GameBlobs::hash_bytes(data));
        let hex_cstr = std::ffi::CString::new(hash_hex).unwrap();

        let result =
            fcn_blobs_verify_and_import(blobs, data.as_ptr(), data.len(), hex_cstr.as_ptr());
        assert!(!result.is_null());
        crate::error::fcn_string_free(result);

        // Should now exist
        assert_eq!(fcn_blobs_has(blobs, hex_cstr.as_ptr()), 1);

        fcn_blobs_free(blobs);
    }

    #[test]
    fn blobs_verify_and_import_mismatch() {
        let blobs = fcn_blobs_new();
        let data = b"actual data";
        let wrong_hash = GameBlobs::hash_to_hex(GameBlobs::hash_bytes(b"different data"));
        let hex_cstr = std::ffi::CString::new(wrong_hash).unwrap();

        let result =
            fcn_blobs_verify_and_import(blobs, data.as_ptr(), data.len(), hex_cstr.as_ptr());
        assert!(result.is_null(), "should fail on hash mismatch");

        let err = crate::error::fcn_last_error();
        assert!(!err.is_null());
        let err_str = unsafe { CStr::from_ptr(err) }.to_str().unwrap();
        assert!(err_str.contains("hash mismatch"), "error: {err_str}");

        fcn_blobs_free(blobs);
    }

    #[test]
    fn blobs_null_safety() {
        assert!(fcn_blobs_import(std::ptr::null(), std::ptr::null(), 0).is_null());
        assert_eq!(fcn_blobs_has(std::ptr::null(), std::ptr::null()), -1);
        fcn_blobs_free(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn blobs_get_not_found() {
        let blobs = fcn_blobs_new();
        let fake_hash = "a".repeat(64);
        let hex_cstr = std::ffi::CString::new(fake_hash).unwrap();
        let mut len: usize = 0;
        let ret = fcn_blobs_get(blobs, hex_cstr.as_ptr(), std::ptr::null_mut(), 0, &mut len);
        assert_eq!(ret, 1); // not found
        fcn_blobs_free(blobs);
    }

    #[test]
    fn gossip_sender_free_null() {
        fcn_gossip_sender_free(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn gossip_receiver_free_null() {
        fcn_gossip_receiver_free(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn gossip_free_null() {
        fcn_gossip_free(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn endpoint_addr_returns_json() {
        let ep = fcn_endpoint_new();
        assert!(!ep.is_null(), "endpoint creation failed");

        let addr = fcn_endpoint_addr(ep);
        assert!(!addr.is_null(), "endpoint addr failed");
        let addr_str = unsafe { CStr::from_ptr(addr) }.to_str().unwrap();
        // Should be valid JSON containing an "id" field
        assert!(addr_str.contains("\"id\""), "addr JSON: {addr_str}");
        crate::error::fcn_string_free(addr);

        let ret = fcn_endpoint_shutdown(ep);
        assert_eq!(ret, 0);
    }

    #[test]
    fn endpoint_addr_null_safety() {
        assert!(fcn_endpoint_addr(std::ptr::null()).is_null());
    }

    #[test]
    fn endpoint_accept_null_safety() {
        assert!(fcn_endpoint_accept(std::ptr::null()).is_null());
    }

    #[test]
    fn gossip_new_creates_instance() {
        let ep = fcn_endpoint_new();
        assert!(!ep.is_null(), "endpoint creation failed");

        let game_id = std::ffi::CString::new("deadbeefcafe1234").unwrap();
        let gossip = fcn_gossip_new(ep, game_id.as_ptr());
        assert!(
            !gossip.is_null(),
            "gossip creation failed — this proves the endpoint().clone() fix works"
        );

        fcn_gossip_free(gossip);
        let ret = fcn_endpoint_shutdown(ep);
        assert_eq!(ret, 0);
    }

    #[test]
    fn gossip_new_null_safety() {
        let game_id = std::ffi::CString::new("test").unwrap();
        assert!(fcn_gossip_new(std::ptr::null(), game_id.as_ptr()).is_null());
    }
}

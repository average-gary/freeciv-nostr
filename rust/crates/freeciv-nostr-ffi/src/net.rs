//! C FFI bindings for P2P networking (endpoint, gossip, blobs, transport).
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
use std::sync::{Mutex, OnceLock};

use tokio::runtime::Runtime;

use freeciv_nostr_net::blobs::GameBlobs;
use freeciv_nostr_net::endpoint::GameEndpoint;
use freeciv_nostr_net::gossip::{GameGossip, GameGossipReceiver, GameGossipSender, GossipEvent};
use freeciv_nostr_net::lobby::GameLobby;
use freeciv_nostr_net::message::FramedMessage;
use freeciv_nostr_net::protocol::StreamId;
use freeciv_nostr_net::transport::QuicTransport;

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

/// Opaque handle to a game lobby.
pub struct FcnLobby {
    inner: GameLobby,
}

/// Opaque handle to a QUIC transport instance.
///
/// Thread-safe: the inner `QuicTransport` is protected by a `Mutex`.
/// All `fcn_transport_*` FFI functions lock this mutex for the duration
/// of the call.
pub struct FcnTransport {
    inner: Mutex<QuicTransport>,
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

// ---------------------------------------------------------------------------
// Lobby FFI
// ---------------------------------------------------------------------------

/// Create a new game lobby.
///
/// `lobby_id`: unique game identifier (C string).
/// `lead_pk`: hex-encoded Nostr pubkey of the lobby creator (C string).
/// `max_players`: maximum number of players.
///
/// Returns an opaque handle, or `NULL` on error (check `fcn_last_error()`).
/// The caller must free with `fcn_lobby_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lobby_new(
    lobby_id: *const c_char,
    lead_pk: *const c_char,
    max_players: u8,
) -> *mut FcnLobby {
    let lid = match cstr_to_str(lobby_id) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let lead = match cstr_to_str(lead_pk) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    Box::into_raw(Box::new(FcnLobby {
        inner: GameLobby::new(lid, lead, max_players),
    }))
}

/// Accept a player into the lobby.
///
/// `pk`: hex-encoded Nostr pubkey (C string).
/// `addr`: JSON-serialised Iroh `EndpointAddr` (C string).
/// `accept_id`: hex-encoded Nostr event ID of the accept event (C string).
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lobby_accept_player(
    lobby: *mut FcnLobby,
    pk: *const c_char,
    addr: *const c_char,
    accept_id: *const c_char,
) -> i32 {
    if lobby.is_null() {
        set_last_error("null lobby pointer");
        return -1;
    }
    let pk_str = match cstr_to_str(pk) {
        Some(s) => s,
        None => return -1,
    };
    let addr_str = match cstr_to_str(addr) {
        Some(s) => s,
        None => return -1,
    };
    let accept_str = match cstr_to_str(accept_id) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees lobby is valid.
    let lobby = unsafe { &mut *lobby };
    match lobby.inner.accept_player(pk_str, addr_str, accept_str) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Transition the lobby to the Started state.
///
/// `lead_pk`: hex-encoded Nostr pubkey of the lead player (C string).
/// Only the lead may start the lobby.
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lobby_start(lobby: *mut FcnLobby, lead_pk: *const c_char) -> i32 {
    if lobby.is_null() {
        set_last_error("null lobby pointer");
        return -1;
    }
    let lead = match cstr_to_str(lead_pk) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees lobby is valid.
    let lobby = unsafe { &mut *lobby };
    match lobby.inner.start(lead) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Get the number of accepted players in the lobby.
///
/// Returns -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lobby_player_count(lobby: *const FcnLobby) -> i32 {
    if lobby.is_null() {
        set_last_error("null lobby pointer");
        return -1;
    }
    // SAFETY: Caller guarantees lobby is valid.
    let lobby = unsafe { &*lobby };
    lobby.inner.player_count() as i32
}

/// Connect our endpoint to every accepted lobby peer.
///
/// Skips `our_pk` (hex-encoded Nostr pubkey) when iterating through
/// accepted players. Returns the number of successful connections,
/// or -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lobby_connect_peers(
    lobby: *const FcnLobby,
    ep: *const FcnEndpoint,
    our_pk: *const c_char,
) -> i32 {
    if lobby.is_null() {
        set_last_error("null lobby pointer");
        return -1;
    }
    if ep.is_null() {
        set_last_error("null endpoint pointer");
        return -1;
    }
    let pk_str = match cstr_to_str(our_pk) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees lobby and ep are valid.
    let lobby = unsafe { &*lobby };
    let ep = unsafe { &*ep };
    match block_on(freeciv_nostr_net::lobby::connect_to_lobby_peers(
        &lobby.inner,
        &ep.inner,
        pk_str,
    )) {
        Ok(n) => n as i32,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Free a lobby handle.
///
/// After this call the handle must not be used.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lobby_free(lobby: *mut FcnLobby) {
    if !lobby.is_null() {
        // SAFETY: Caller guarantees lobby was returned by fcn_lobby_new.
        unsafe {
            let _ = Box::from_raw(lobby);
        }
    }
}

// ---------------------------------------------------------------------------
// Lockstep FFI
// ---------------------------------------------------------------------------

use freeciv_nostr_net::lockstep::{
    ActionCommitment, ActionReveal, LockstepConfig, LockstepProtocol, PhaseMode,
    StateHashSubmission, TurnAdvanceResult, TurnPhase,
};
use std::time::Duration;

/// Phase mode for C consumers.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FcnPhaseMode {
    /// All players act simultaneously (commit-reveal).
    Concurrent = 0,
    /// Players take turns one at a time.
    PlayersAlternate = 1,
    /// Teams take turns.
    TeamsAlternate = 2,
}

/// Turn phase for C consumers.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FcnTurnPhase {
    /// Waiting for commitments.
    Commit = 0,
    /// Waiting for reveals.
    Reveal = 1,
    /// Actions are being applied.
    Apply = 2,
    /// Waiting for state hash verification.
    Verify = 3,
    /// Turn is complete.
    Complete = 4,
}

/// Outcome tag for lockstep operations, for C consumers.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FcnTurnResult {
    /// Still waiting for other players.
    Waiting = 0,
    /// All players submitted; phase advances.
    Ready = 1,
    /// A reveal did not match its commitment.
    RevealMismatch = 2,
    /// Players disagree on the post-apply state.
    DesyncDetected = 3,
    /// The current phase has timed out.
    Timeout = 4,
    /// An error occurred (check `fcn_last_error()`).
    Error = 5,
}

/// Opaque handle to a lockstep protocol instance.
pub struct FcnLockstep {
    inner: LockstepProtocol,
}

impl From<TurnPhase> for FcnTurnPhase {
    fn from(p: TurnPhase) -> Self {
        match p {
            TurnPhase::Commit => FcnTurnPhase::Commit,
            TurnPhase::Reveal => FcnTurnPhase::Reveal,
            TurnPhase::Apply => FcnTurnPhase::Apply,
            TurnPhase::Verify => FcnTurnPhase::Verify,
            TurnPhase::Complete => FcnTurnPhase::Complete,
        }
    }
}

fn turn_advance_to_ffi(result: TurnAdvanceResult) -> FcnTurnResult {
    match result {
        TurnAdvanceResult::Waiting { .. } => FcnTurnResult::Waiting,
        TurnAdvanceResult::Ready => FcnTurnResult::Ready,
        TurnAdvanceResult::RevealMismatch { .. } => FcnTurnResult::RevealMismatch,
        TurnAdvanceResult::DesyncDetected { .. } => FcnTurnResult::DesyncDetected,
        TurnAdvanceResult::Timeout { .. } => FcnTurnResult::Timeout,
    }
}

/// Create a new lockstep protocol instance.
///
/// `phase_mode`: 0=Concurrent, 1=PlayersAlternate, 2=TeamsAlternate.
/// `timeout_ms`: per-phase timeout in milliseconds. 0 means no timeout.
/// `player_pubkeys_json`: JSON array of hex-encoded player pubkeys,
///   e.g. `["pk1","pk2"]`.
///
/// Returns an opaque handle, or `NULL` on error.
/// The caller must free with `fcn_lockstep_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_new(
    phase_mode: u8,
    timeout_ms: u64,
    player_pubkeys_json: *const c_char,
) -> *mut FcnLockstep {
    let mode = match phase_mode {
        0 => PhaseMode::Concurrent,
        1 => PhaseMode::PlayersAlternate,
        2 => PhaseMode::TeamsAlternate,
        other => {
            set_last_error(&format!("invalid phase_mode: {other}"));
            return std::ptr::null_mut();
        }
    };

    let json_str = match cstr_to_str(player_pubkeys_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let pubkeys: Vec<String> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("invalid player_pubkeys_json: {e}"));
            return std::ptr::null_mut();
        }
    };

    let timeout = if timeout_ms == 0 {
        None
    } else {
        Some(Duration::from_millis(timeout_ms))
    };

    let config = LockstepConfig {
        phase_mode: mode,
        turn_timeout: timeout,
        player_pubkeys: pubkeys,
    };

    Box::into_raw(Box::new(FcnLockstep {
        inner: LockstepProtocol::new(config),
    }))
}

/// Begin a new turn.
///
/// Returns 0 on success, -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_begin_turn(ls: *mut FcnLockstep, turn: u32) -> i32 {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return -1;
    }
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &mut *ls };
    ls.inner.begin_turn(turn);
    0
}

/// Submit a commitment for a player.
///
/// Returns a `FcnTurnResult` tag. On error returns `FcnTurnResult::Error`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_submit_commitment(
    ls: *mut FcnLockstep,
    player_pk: *const c_char,
    hash: *const c_char,
    turn: u32,
) -> FcnTurnResult {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return FcnTurnResult::Error;
    }
    let pk = match cstr_to_str(player_pk) {
        Some(s) => s,
        None => return FcnTurnResult::Error,
    };
    let h = match cstr_to_str(hash) {
        Some(s) => s,
        None => return FcnTurnResult::Error,
    };
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &mut *ls };
    match ls.inner.submit_commitment(ActionCommitment {
        hash: h.to_string(),
        turn,
        player_pubkey: pk.to_string(),
    }) {
        Ok(r) => turn_advance_to_ffi(r),
        Err(e) => {
            set_last_error_from(e);
            FcnTurnResult::Error
        }
    }
}

/// Submit a reveal for a player.
///
/// Returns a `FcnTurnResult` tag. On error returns `FcnTurnResult::Error`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_submit_reveal(
    ls: *mut FcnLockstep,
    player_pk: *const c_char,
    actions_json: *const c_char,
    turn: u32,
) -> FcnTurnResult {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return FcnTurnResult::Error;
    }
    let pk = match cstr_to_str(player_pk) {
        Some(s) => s,
        None => return FcnTurnResult::Error,
    };
    let json = match cstr_to_str(actions_json) {
        Some(s) => s,
        None => return FcnTurnResult::Error,
    };
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &mut *ls };
    match ls.inner.submit_reveal(ActionReveal {
        actions_json: json.to_string(),
        turn,
        player_pubkey: pk.to_string(),
    }) {
        Ok(r) => turn_advance_to_ffi(r),
        Err(e) => {
            set_last_error_from(e);
            FcnTurnResult::Error
        }
    }
}

/// Get ordered actions as a JSON array string.
///
/// Returns a JSON array of action reveal objects, e.g.
/// `[{"actions_json":"...","turn":1,"player_pubkey":"..."},...]`.
///
/// The caller must free the returned string with `fcn_string_free()`.
/// Returns `NULL` on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_ordered_actions(ls: *const FcnLockstep) -> *mut c_char {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &*ls };
    match ls.inner.ordered_actions() {
        Ok(actions) => {
            let json = match serde_json::to_string(&actions) {
                Ok(j) => j,
                Err(e) => {
                    set_last_error(&format!("failed to serialize actions: {e}"));
                    return std::ptr::null_mut();
                }
            };
            string_to_c(json)
        }
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Signal that actions have been applied to the game state.
///
/// Transitions the protocol from `Apply` to `Verify`.
/// Returns 0 on success, -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_actions_applied(ls: *mut FcnLockstep) -> i32 {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return -1;
    }
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &mut *ls };
    ls.inner.actions_applied();
    0
}

/// Submit a state hash for consensus verification.
///
/// Returns a `FcnTurnResult` tag. On error returns `FcnTurnResult::Error`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_submit_state_hash(
    ls: *mut FcnLockstep,
    player_pk: *const c_char,
    state_hash: *const c_char,
    turn: u32,
) -> FcnTurnResult {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return FcnTurnResult::Error;
    }
    let pk = match cstr_to_str(player_pk) {
        Some(s) => s,
        None => return FcnTurnResult::Error,
    };
    let hash = match cstr_to_str(state_hash) {
        Some(s) => s,
        None => return FcnTurnResult::Error,
    };
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &mut *ls };
    match ls.inner.submit_state_hash(StateHashSubmission {
        state_hash: hash.to_string(),
        turn,
        player_pubkey: pk.to_string(),
    }) {
        Ok(r) => turn_advance_to_ffi(r),
        Err(e) => {
            set_last_error_from(e);
            FcnTurnResult::Error
        }
    }
}

/// Check whether the current phase has timed out.
///
/// Returns `FcnTurnResult::Timeout` if timed out, `FcnTurnResult::Waiting`
/// if no timeout has occurred, or `FcnTurnResult::Error` on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_check_timeout(ls: *const FcnLockstep) -> FcnTurnResult {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return FcnTurnResult::Error;
    }
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &*ls };
    match ls.inner.check_timeout() {
        Some(result) => turn_advance_to_ffi(result),
        None => FcnTurnResult::Waiting,
    }
}

/// Get the current turn number.
///
/// Returns -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_current_turn(ls: *const FcnLockstep) -> i32 {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return -1;
    }
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &*ls };
    ls.inner.current_turn() as i32
}

/// Get the current turn phase.
///
/// Returns `FcnTurnPhase::Complete` on null pointer (and sets last error).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_current_phase(ls: *const FcnLockstep) -> FcnTurnPhase {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return FcnTurnPhase::Complete;
    }
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &*ls };
    ls.inner.current_phase().into()
}

/// Compute a SHA-256 commitment hash for the given actions JSON.
///
/// The caller must free the returned string with `fcn_string_free()`.
/// Returns `NULL` on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_compute_commitment(actions_json: *const c_char) -> *mut c_char {
    let json = match cstr_to_str(actions_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    string_to_c(LockstepProtocol::compute_commitment(json))
}

/// Get the consensus state hash after a completed turn.
///
/// Returns the hash string, or `NULL` if the turn is not complete
/// or the pointer is null.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_consensus_hash(ls: *const FcnLockstep) -> *mut c_char {
    if ls.is_null() {
        set_last_error("null lockstep pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees ls is valid.
    let ls = unsafe { &*ls };
    match ls.inner.consensus_state_hash() {
        Some(h) => string_to_c(h.to_string()),
        None => std::ptr::null_mut(),
    }
}

/// Free a lockstep protocol handle.
///
/// After this call the handle must not be used.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_lockstep_free(ls: *mut FcnLockstep) {
    if !ls.is_null() {
        // SAFETY: Caller guarantees ls was returned by fcn_lockstep_new.
        unsafe {
            let _ = Box::from_raw(ls);
        }
    }
}

// ---------------------------------------------------------------------------
// Transport FFI
// ---------------------------------------------------------------------------

/// C-compatible poll entry, matching `struct fc_transport_poll_entry`.
///
/// Must be layout-compatible with the C struct defined in `utility/transport.h`.
#[repr(C)]
pub struct FcnTransportPollEntry {
    /// The transport handle to monitor.
    pub handle: i32,
    /// Bitmask of requested events (`FC_TRANSPORT_READ`, etc.).
    pub requested_events: i32,
    /// Bitmask of returned events (output, set by poll).
    pub returned_events: i32,
}

/// Create a new QUIC transport instance.
///
/// Returns an opaque handle, or `NULL` on error (check `fcn_last_error()`).
/// The caller must free with `fcn_transport_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_new() -> *mut FcnTransport {
    Box::into_raw(Box::new(FcnTransport {
        inner: Mutex::new(QuicTransport::new()),
    }))
}

/// Set up a listener on the transport and return the listener handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_setup_listener(transport: *mut FcnTransport) -> i32 {
    if transport.is_null() {
        set_last_error("null transport pointer");
        return -1;
    }
    // SAFETY: Caller guarantees transport was returned by fcn_transport_new().
    let transport = unsafe { &*transport };
    match transport.inner.lock() {
        Ok(mut t) => t.setup_listener(),
        Err(e) => {
            set_last_error(&format!("transport mutex poisoned: {e}"));
            -1
        }
    }
}

/// Accept an incoming connection on the transport. Blocks until available.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_accept(transport: *mut FcnTransport) -> i32 {
    if transport.is_null() {
        set_last_error("null transport pointer");
        return -1;
    }
    // SAFETY: Caller guarantees transport was returned by fcn_transport_new().
    let transport = unsafe { &*transport };
    match transport.inner.lock() {
        Ok(mut t) => match block_on(t.accept()) {
            Ok(handle) => handle,
            Err(e) => {
                set_last_error_from(e);
                -1
            }
        },
        Err(e) => {
            set_last_error(&format!("transport mutex poisoned: {e}"));
            -1
        }
    }
}

/// Close a stream or listener handle on the transport.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_close_handle(transport: *mut FcnTransport, handle: i32) {
    if transport.is_null() {
        return;
    }
    // SAFETY: Caller guarantees transport was returned by fcn_transport_new().
    let transport = unsafe { &*transport };
    if let Ok(mut t) = transport.inner.lock() {
        t.close(handle);
    }
}

/// Read from a transport stream. Returns bytes read, 0 on EOF, -1 on error.
///
/// # Safety
///
/// `buf` must point to a buffer of at least `len` bytes.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_read(
    transport: *mut FcnTransport,
    handle: i32,
    buf: *mut u8,
    len: i32,
) -> i32 {
    if transport.is_null() {
        set_last_error("null transport pointer");
        return -1;
    }
    if buf.is_null() && len > 0 {
        set_last_error("null buffer with non-zero length");
        return -1;
    }
    if len < 0 {
        set_last_error("negative length");
        return -1;
    }
    // SAFETY: Caller guarantees transport is valid and buf points to len bytes.
    let transport = unsafe { &*transport };
    let slice = if len > 0 {
        unsafe { std::slice::from_raw_parts_mut(buf, len as usize) }
    } else {
        &mut []
    };
    match transport.inner.lock() {
        Ok(mut t) => match block_on(t.read(handle, slice)) {
            Ok(n) => n as i32,
            Err(e) => {
                set_last_error_from(e);
                -1
            }
        },
        Err(e) => {
            set_last_error(&format!("transport mutex poisoned: {e}"));
            -1
        }
    }
}

/// Write to a transport stream. Returns bytes written, -1 on error.
///
/// # Safety
///
/// `buf` must point to a buffer of at least `len` bytes.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_write(
    transport: *mut FcnTransport,
    handle: i32,
    buf: *const u8,
    len: i32,
) -> i32 {
    if transport.is_null() {
        set_last_error("null transport pointer");
        return -1;
    }
    if buf.is_null() && len > 0 {
        set_last_error("null buffer with non-zero length");
        return -1;
    }
    if len < 0 {
        set_last_error("negative length");
        return -1;
    }
    // SAFETY: Caller guarantees transport is valid and buf points to len bytes.
    let transport = unsafe { &*transport };
    let slice = if len > 0 {
        unsafe { std::slice::from_raw_parts(buf, len as usize) }
    } else {
        &[]
    };
    match transport.inner.lock() {
        Ok(mut t) => match block_on(t.write(handle, slice)) {
            Ok(n) => n as i32,
            Err(e) => {
                set_last_error_from(e);
                -1
            }
        },
        Err(e) => {
            set_last_error(&format!("transport mutex poisoned: {e}"));
            -1
        }
    }
}

/// Poll transport handles for readiness. Returns ready count, -1 on error.
///
/// # Safety
///
/// `entries_ptr` must point to `count` valid `FcnTransportPollEntry` structs.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_poll_handles(
    transport: *mut FcnTransport,
    entries_ptr: *mut FcnTransportPollEntry,
    count: i32,
    timeout_ms: i32,
) -> i32 {
    if transport.is_null() {
        set_last_error("null transport pointer");
        return -1;
    }
    if entries_ptr.is_null() && count > 0 {
        set_last_error("null entries with non-zero count");
        return -1;
    }
    if count < 0 {
        set_last_error("negative count");
        return -1;
    }
    // SAFETY: Caller guarantees transport is valid and entries_ptr points to count entries.
    let transport = unsafe { &*transport };
    let c_entries = if count > 0 {
        unsafe { std::slice::from_raw_parts_mut(entries_ptr, count as usize) }
    } else {
        &mut []
    };
    let mut rust_entries: Vec<freeciv_nostr_net::transport::PollEntry> = c_entries
        .iter()
        .map(|e| freeciv_nostr_net::transport::PollEntry {
            handle: e.handle,
            requested_events: e.requested_events as u32,
            returned_events: 0,
        })
        .collect();
    match transport.inner.lock() {
        Ok(t) => {
            let ready = t.poll(&mut rust_entries, timeout_ms);
            for (c, r) in c_entries.iter_mut().zip(rust_entries.iter()) {
                c.returned_events = r.returned_events as i32;
            }
            ready as i32
        }
        Err(e) => {
            set_last_error(&format!("transport mutex poisoned: {e}"));
            -1
        }
    }
}

/// Get the number of active streams on the transport. Returns -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_stream_count(transport: *const FcnTransport) -> i32 {
    if transport.is_null() {
        set_last_error("null transport pointer");
        return -1;
    }
    // SAFETY: Caller guarantees transport was returned by fcn_transport_new().
    let transport = unsafe { &*transport };
    match transport.inner.lock() {
        Ok(t) => t.stream_count() as i32,
        Err(e) => {
            set_last_error(&format!("transport mutex poisoned: {e}"));
            -1
        }
    }
}

/// Free a transport instance. After this call the pointer must not be used.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_transport_free(transport: *mut FcnTransport) {
    if !transport.is_null() {
        // SAFETY: Caller guarantees transport was returned by fcn_transport_new().
        unsafe {
            let _ = Box::from_raw(transport);
        }
    }
}

// ---------------------------------------------------------------------------
// Validation FFI
// ---------------------------------------------------------------------------

/// Validate a single action's payload schema.
///
/// `action_json` is the JSON-serialized `PlayerAction`.
/// Returns a JSON string describing the validation result, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_validate_action(action_json: *const c_char) -> *mut c_char {
    let json_str = match cstr_to_str(action_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let action: freeciv_nostr_core::actions::PlayerAction = match serde_json::from_str(json_str) {
        Ok(a) => a,
        Err(e) => {
            set_last_error(&format!("invalid action JSON: {e}"));
            return std::ptr::null_mut();
        }
    };
    let result = freeciv_nostr_net::validation::validate_schema(&action);
    match serde_json::to_string(&result) {
        Ok(s) => string_to_c(s),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Validate a batch of actions.
///
/// `player_pubkey_hex` is the player's pubkey.
/// `actions_json` is a JSON array of `PlayerAction` objects.
/// Returns a JSON string with the `BatchValidationResult`, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_validate_action_batch(
    player_pubkey_hex: *const c_char,
    turn: u32,
    actions_json: *const c_char,
) -> *mut c_char {
    let pk = match cstr_to_str(player_pubkey_hex) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let json_str = match cstr_to_str(actions_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let actions: Vec<freeciv_nostr_core::actions::PlayerAction> =
        match serde_json::from_str(json_str) {
            Ok(a) => a,
            Err(e) => {
                set_last_error(&format!("invalid actions JSON: {e}"));
                return std::ptr::null_mut();
            }
        };
    let result = freeciv_nostr_net::validation::validate_action_batch(pk, turn, &actions);
    match serde_json::to_string(&result) {
        Ok(s) => string_to_c(s),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Opaque handle to a consensus validator.
pub struct FcnConsensusValidator {
    inner: freeciv_nostr_net::validation::ConsensusValidator,
}

/// Create a new consensus validator.
///
/// `num_nodes` is the total number of nodes in the game.
/// The caller must free with `fcn_consensus_validator_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_consensus_validator_new(num_nodes: u32) -> *mut FcnConsensusValidator {
    Box::into_raw(Box::new(FcnConsensusValidator {
        inner: freeciv_nostr_net::validation::ConsensusValidator::new(num_nodes as usize),
    }))
}

/// Submit a validation vote from a node.
///
/// `result_json` is the JSON-serialized `ValidationResult`.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_consensus_validator_submit_vote(
    cv: *mut FcnConsensusValidator,
    player_pubkey: *const c_char,
    action_index: u32,
    node_pubkey: *const c_char,
    result_json: *const c_char,
) -> i32 {
    if cv.is_null() {
        set_last_error("null consensus validator pointer");
        return -1;
    }
    let player_pk = match cstr_to_str(player_pubkey) {
        Some(s) => s,
        None => return -1,
    };
    let node_pk = match cstr_to_str(node_pubkey) {
        Some(s) => s,
        None => return -1,
    };
    let result_str = match cstr_to_str(result_json) {
        Some(s) => s,
        None => return -1,
    };
    let result: freeciv_nostr_net::validation::ValidationResult =
        match serde_json::from_str(result_str) {
            Ok(r) => r,
            Err(e) => {
                set_last_error(&format!("invalid result JSON: {e}"));
                return -1;
            }
        };
    // SAFETY: Caller guarantees cv is valid.
    let cv = unsafe { &mut *cv };
    cv.inner
        .submit_vote(player_pk, action_index as usize, node_pk, result);
    0
}

/// Get the consensus decision for an action.
///
/// Returns a JSON string with the `ConsensusDecision`, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_consensus_validator_decide(
    cv: *const FcnConsensusValidator,
    player_pubkey: *const c_char,
    action_index: u32,
) -> *mut c_char {
    if cv.is_null() {
        set_last_error("null consensus validator pointer");
        return std::ptr::null_mut();
    }
    let player_pk = match cstr_to_str(player_pubkey) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    // SAFETY: Caller guarantees cv is valid.
    let cv = unsafe { &*cv };
    let decision = cv.inner.decide(player_pk, action_index as usize);
    match serde_json::to_string(&decision) {
        Ok(s) => string_to_c(s),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Free a consensus validator handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_consensus_validator_free(cv: *mut FcnConsensusValidator) {
    if !cv.is_null() {
        // SAFETY: Caller guarantees cv was returned by fcn_consensus_validator_new.
        unsafe {
            let _ = Box::from_raw(cv);
        }
    }
}

// ---------------------------------------------------------------------------
// Desync Detection FFI
// ---------------------------------------------------------------------------

/// Opaque handle to a desync detector.
pub struct FcnDesyncDetector {
    inner: freeciv_nostr_net::desync::DesyncDetector,
}

/// Create a new desync detector.
///
/// `player_pubkeys_json` is a JSON array of player public key hex strings.
/// The caller must free with `fcn_desync_detector_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_detector_new(
    checkpoint_interval: u32,
    max_checkpoints: u32,
    player_pubkeys_json: *const c_char,
) -> *mut FcnDesyncDetector {
    let json_str = match cstr_to_str(player_pubkeys_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let pubkeys: Vec<String> = match serde_json::from_str(json_str) {
        Ok(p) => p,
        Err(e) => {
            set_last_error(&format!("invalid player pubkeys JSON: {e}"));
            return std::ptr::null_mut();
        }
    };
    let config = freeciv_nostr_net::desync::DesyncConfig {
        checkpoint_interval,
        max_checkpoints: max_checkpoints as usize,
        player_pubkeys: pubkeys,
    };
    Box::into_raw(Box::new(FcnDesyncDetector {
        inner: freeciv_nostr_net::desync::DesyncDetector::new(config),
    }))
}

/// Record a state hash from a player for a given turn.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_record_hash(
    detector: *mut FcnDesyncDetector,
    turn: u32,
    player_pubkey: *const c_char,
    state_hash: *const c_char,
) -> i32 {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return -1;
    }
    let pk = match cstr_to_str(player_pubkey) {
        Some(s) => s,
        None => return -1,
    };
    let hash = match cstr_to_str(state_hash) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &mut *detector };
    detector.inner.record_hash(turn, pk, hash);
    0
}

/// Check desync status for a turn.
/// Returns a JSON string with the `DesyncStatus`, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_check_turn(
    detector: *const FcnDesyncDetector,
    turn: u32,
) -> *mut c_char {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &*detector };
    let status = detector.inner.check_turn(turn);
    match serde_json::to_string(&status) {
        Ok(s) => string_to_c(s),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Mark a turn as in-sync.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_mark_in_sync(detector: *mut FcnDesyncDetector, turn: u32) -> i32 {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return -1;
    }
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &mut *detector };
    detector.inner.mark_in_sync(turn);
    0
}

/// Get the last turn where all nodes were in sync.
/// Returns -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_last_sync_turn(detector: *const FcnDesyncDetector) -> i32 {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return -1;
    }
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &*detector };
    detector.inner.last_sync_turn() as i32
}

/// Store a recovery checkpoint.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_store_checkpoint(
    detector: *mut FcnDesyncDetector,
    turn: u32,
    state_hash: *const c_char,
    blob_hash: *const c_char,
    agreement_count: u32,
) -> i32 {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return -1;
    }
    let sh = match cstr_to_str(state_hash) {
        Some(s) => s,
        None => return -1,
    };
    let bh = match cstr_to_str(blob_hash) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &mut *detector };
    detector
        .inner
        .store_checkpoint(freeciv_nostr_net::desync::RecoveryCheckpoint {
            turn,
            state_hash: sh.to_string(),
            blob_hash: bh.to_string(),
            agreement_count: agreement_count as usize,
        });
    0
}

/// Check if a checkpoint should be created at this turn.
/// Returns 1 if yes, 0 if no, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_should_checkpoint(
    detector: *const FcnDesyncDetector,
    turn: u32,
) -> i32 {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return -1;
    }
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &*detector };
    if detector.inner.should_checkpoint(turn) {
        1
    } else {
        0
    }
}

/// Determine the recovery strategy for a desync at the given turn.
/// Returns a JSON string with the `RecoveryStrategy`, or `NULL` on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_determine_recovery(
    detector: *const FcnDesyncDetector,
    turn: u32,
) -> *mut c_char {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &*detector };
    let status = detector.inner.check_turn(turn);
    let strategy = detector.inner.determine_recovery(&status);
    match serde_json::to_string(&strategy) {
        Ok(s) => string_to_c(s),
        Err(e) => {
            set_last_error_from(e);
            std::ptr::null_mut()
        }
    }
}

/// Find the divergence turn using binary search.
/// Returns the turn number, or -1 if not found or on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_find_divergence(
    detector: *const FcnDesyncDetector,
    start: u32,
    end: u32,
) -> i32 {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return -1;
    }
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &*detector };
    match detector.inner.find_divergence_turn(start, end) {
        Some(turn) => turn as i32,
        None => -1,
    }
}

/// Get the number of stored checkpoints.
/// Returns -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_checkpoint_count(detector: *const FcnDesyncDetector) -> i32 {
    if detector.is_null() {
        set_last_error("null detector pointer");
        return -1;
    }
    // SAFETY: Caller guarantees detector is valid.
    let detector = unsafe { &*detector };
    detector.inner.checkpoint_count() as i32
}

/// Free a desync detector handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_desync_detector_free(detector: *mut FcnDesyncDetector) {
    if !detector.is_null() {
        // SAFETY: Caller guarantees detector was returned by fcn_desync_detector_new.
        unsafe {
            let _ = Box::from_raw(detector);
        }
    }
}

// ---------------------------------------------------------------------------
// Relay / Connection Monitor FFI
// ---------------------------------------------------------------------------

use freeciv_nostr_net::relay::{ConnectionMonitor, ConnectionQuality, RelayConfig};

/// Opaque handle to a connection monitor.
pub struct FcnConnectionMonitor {
    inner: ConnectionMonitor,
}

/// Return the default relay configuration as a JSON string.
///
/// The caller must free the returned string with `fcn_string_free()`.
/// Returns `NULL` on serialization error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_relay_config_default() -> *mut c_char {
    let config = RelayConfig::default();
    match serde_json::to_string(&config) {
        Ok(s) => string_to_c(s),
        Err(e) => {
            set_last_error(&format!("failed to serialize relay config: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// Create a new connection monitor.
///
/// The caller must free with `fcn_connection_monitor_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_connection_monitor_new() -> *mut FcnConnectionMonitor {
    Box::into_raw(Box::new(FcnConnectionMonitor {
        inner: ConnectionMonitor::new(),
    }))
}

/// Update connection quality for a peer.
///
/// `quality_json` is a JSON-serialized `ConnectionQuality`.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_connection_monitor_update(
    monitor: *mut FcnConnectionMonitor,
    quality_json: *const c_char,
) -> i32 {
    if monitor.is_null() {
        set_last_error("null monitor pointer");
        return -1;
    }
    let json_str = match cstr_to_str(quality_json) {
        Some(s) => s,
        None => return -1,
    };
    let quality: ConnectionQuality = match serde_json::from_str(json_str) {
        Ok(q) => q,
        Err(e) => {
            set_last_error(&format!("invalid quality JSON: {e}"));
            return -1;
        }
    };
    // SAFETY: Caller guarantees monitor is valid.
    let monitor = unsafe { &mut *monitor };
    monitor.inner.update(quality);
    0
}

/// Get connection quality for a specific peer.
///
/// Returns a JSON string with the `ConnectionQuality`, or `NULL` if not found
/// or on error.
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_connection_monitor_get(
    monitor: *const FcnConnectionMonitor,
    peer_id: *const c_char,
) -> *mut c_char {
    if monitor.is_null() {
        set_last_error("null monitor pointer");
        return std::ptr::null_mut();
    }
    let pid = match cstr_to_str(peer_id) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    // SAFETY: Caller guarantees monitor is valid.
    let monitor = unsafe { &*monitor };
    match monitor.inner.get(pid) {
        Some(quality) => match serde_json::to_string(quality) {
            Ok(s) => string_to_c(s),
            Err(e) => {
                set_last_error(&format!("failed to serialize quality: {e}"));
                std::ptr::null_mut()
            }
        },
        None => std::ptr::null_mut(),
    }
}

/// Get the number of direct connections.
///
/// Returns -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_connection_monitor_direct_count(monitor: *const FcnConnectionMonitor) -> i32 {
    if monitor.is_null() {
        set_last_error("null monitor pointer");
        return -1;
    }
    // SAFETY: Caller guarantees monitor is valid.
    let monitor = unsafe { &*monitor };
    monitor.inner.direct_count() as i32
}

/// Get the number of relayed connections.
///
/// Returns -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_connection_monitor_relayed_count(
    monitor: *const FcnConnectionMonitor,
) -> i32 {
    if monitor.is_null() {
        set_last_error("null monitor pointer");
        return -1;
    }
    // SAFETY: Caller guarantees monitor is valid.
    let monitor = unsafe { &*monitor };
    monitor.inner.relayed_count() as i32
}

/// Get the number of active connections.
///
/// Returns -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_connection_monitor_active_count(monitor: *const FcnConnectionMonitor) -> i32 {
    if monitor.is_null() {
        set_last_error("null monitor pointer");
        return -1;
    }
    // SAFETY: Caller guarantees monitor is valid.
    let monitor = unsafe { &*monitor };
    monitor.inner.active_count() as i32
}

/// Free a connection monitor handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_connection_monitor_free(monitor: *mut FcnConnectionMonitor) {
    if !monitor.is_null() {
        // SAFETY: Caller guarantees monitor was returned by fcn_connection_monitor_new.
        unsafe {
            let _ = Box::from_raw(monitor);
        }
    }
}

// ---------------------------------------------------------------------------
// Game Node FFI
// ---------------------------------------------------------------------------

use freeciv_nostr_net::node::{GameNode, NodeConfig, NodeState};

/// Opaque handle to a game node.
pub struct FcnGameNode {
    inner: GameNode,
}

/// Helper struct for deserializing node configuration from JSON.
#[derive(serde::Deserialize)]
struct NodeConfigJson {
    player_pubkey: String,
    is_lead: bool,
    game_event_id: Option<String>,
    phase_mode: u8,
    turn_timeout_secs: u32,
    checkpoint_interval: u32,
    #[serde(default)]
    relay_config: Option<RelayConfig>,
}

/// Create a new game node.
///
/// `config_json` is a JSON object with fields:
///   - `player_pubkey`: hex string
///   - `is_lead`: boolean
///   - `game_event_id`: optional hex string
///   - `phase_mode`: 0=Concurrent, 1=PlayersAlternate, 2=TeamsAlternate
///   - `turn_timeout_secs`: u32 (0 = no timeout)
///   - `checkpoint_interval`: u32 (0 = disabled)
///   - `relay_config`: optional RelayConfig JSON
///
/// Returns an opaque handle, or `NULL` on error.
/// The caller must free with `fcn_node_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_new(config_json: *const c_char) -> *mut FcnGameNode {
    let json_str = match cstr_to_str(config_json) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let cfg: NodeConfigJson = match serde_json::from_str(json_str) {
        Ok(c) => c,
        Err(e) => {
            set_last_error(&format!("invalid node config JSON: {e}"));
            return std::ptr::null_mut();
        }
    };
    let phase_mode = match cfg.phase_mode {
        0 => PhaseMode::Concurrent,
        1 => PhaseMode::PlayersAlternate,
        2 => PhaseMode::TeamsAlternate,
        other => {
            set_last_error(&format!("invalid phase_mode: {other}"));
            return std::ptr::null_mut();
        }
    };
    let config = NodeConfig {
        player_pubkey: cfg.player_pubkey,
        is_lead: cfg.is_lead,
        game_event_id: cfg.game_event_id,
        phase_mode,
        turn_timeout_secs: cfg.turn_timeout_secs,
        checkpoint_interval: cfg.checkpoint_interval,
        relay_config: cfg.relay_config.unwrap_or_default(),
    };
    Box::into_raw(Box::new(FcnGameNode {
        inner: GameNode::new(config),
    }))
}

/// Get the current node state as an integer.
///
/// Returns: 0=Initializing, 1=InLobby, 2=Connecting, 3=Playing,
/// 4=Finished, 5=Error. Returns -1 on null pointer.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_state(node: *const FcnGameNode) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &*node };
    match node.inner.state() {
        NodeState::Initializing => 0,
        NodeState::InLobby => 1,
        NodeState::Connecting => 2,
        NodeState::Playing => 3,
        NodeState::Finished => 4,
        NodeState::Error => 5,
    }
}

/// Create a lobby (lead player).
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_create_lobby(
    node: *mut FcnGameNode,
    lobby_id: *const c_char,
    max_players: u8,
) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    let lid = match cstr_to_str(lobby_id) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &mut *node };
    match node.inner.create_lobby(lid, max_players) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Join an existing lobby (non-lead player).
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_join_lobby(
    node: *mut FcnGameNode,
    lobby_id: *const c_char,
    lead_pk: *const c_char,
    max_players: u8,
) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    let lid = match cstr_to_str(lobby_id) {
        Some(s) => s,
        None => return -1,
    };
    let lead = match cstr_to_str(lead_pk) {
        Some(s) => s,
        None => return -1,
    };
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &mut *node };
    match node.inner.join_lobby(lid, lead, max_players) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Start the game.
///
/// `player_pubkeys_json` is a JSON array of hex pubkey strings.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_start_game(
    node: *mut FcnGameNode,
    player_pubkeys_json: *const c_char,
) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    let json_str = match cstr_to_str(player_pubkeys_json) {
        Some(s) => s,
        None => return -1,
    };
    let pubkeys: Vec<String> = match serde_json::from_str(json_str) {
        Ok(p) => p,
        Err(e) => {
            set_last_error(&format!("invalid player pubkeys JSON: {e}"));
            return -1;
        }
    };
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &mut *node };
    match node.inner.start_game(pubkeys) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Mark connections as established, transition to Playing.
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_connections_ready(node: *mut FcnGameNode) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &mut *node };
    match node.inner.connections_ready() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// Begin a new turn.
///
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_begin_turn(node: *mut FcnGameNode, turn: u32) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &mut *node };
    match node.inner.begin_turn(turn) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error_from(e);
            -1
        }
    }
}

/// End the game.
///
/// Returns 0 on success, -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_end_game(node: *mut FcnGameNode) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &mut *node };
    node.inner.end_game();
    0
}

/// Get the current turn number.
///
/// Returns -1 on error (null pointer).
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_current_turn(node: *const FcnGameNode) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &*node };
    node.inner.current_turn() as i32
}

/// Get the player's public key as a hex string.
///
/// The caller must free the returned string with `fcn_string_free()`.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_player_pubkey(node: *const FcnGameNode) -> *mut c_char {
    if node.is_null() {
        set_last_error("null node pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &*node };
    string_to_c(node.inner.player_pubkey().to_string())
}

/// Check if this node is the game lead.
///
/// Returns 1 if lead, 0 if not, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_is_lead(node: *const FcnGameNode) -> i32 {
    if node.is_null() {
        set_last_error("null node pointer");
        return -1;
    }
    // SAFETY: Caller guarantees node is valid.
    let node = unsafe { &*node };
    if node.inner.is_lead() { 1 } else { 0 }
}

/// Free a game node handle.
#[unsafe(no_mangle)]
pub extern "C" fn fcn_node_free(node: *mut FcnGameNode) {
    if !node.is_null() {
        // SAFETY: Caller guarantees node was returned by fcn_node_new.
        unsafe {
            let _ = Box::from_raw(node);
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

    // -- Lobby FFI ---------------------------------------------------------

    #[test]
    fn lobby_lifecycle() {
        let lid = std::ffi::CString::new("lobby-1").unwrap();
        let lead = std::ffi::CString::new("lead_pk").unwrap();
        let lobby = fcn_lobby_new(lid.as_ptr(), lead.as_ptr(), 4);
        assert!(!lobby.is_null());

        assert_eq!(fcn_lobby_player_count(lobby), 0);

        let pk = std::ffi::CString::new("pk1").unwrap();
        let addr = std::ffi::CString::new("{}").unwrap();
        let eid = std::ffi::CString::new("evt1").unwrap();
        assert_eq!(
            fcn_lobby_accept_player(lobby, pk.as_ptr(), addr.as_ptr(), eid.as_ptr()),
            0
        );
        assert_eq!(fcn_lobby_player_count(lobby), 1);

        assert_eq!(fcn_lobby_start(lobby, lead.as_ptr()), 0);

        fcn_lobby_free(lobby);
    }

    #[test]
    fn lobby_new_null_safety() {
        assert!(fcn_lobby_new(std::ptr::null(), std::ptr::null(), 4).is_null());
    }

    #[test]
    fn lobby_accept_null_safety() {
        assert_eq!(
            fcn_lobby_accept_player(
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null()
            ),
            -1
        );
    }

    #[test]
    fn lobby_start_null_safety() {
        assert_eq!(fcn_lobby_start(std::ptr::null_mut(), std::ptr::null()), -1);
    }

    #[test]
    fn lobby_player_count_null_safety() {
        assert_eq!(fcn_lobby_player_count(std::ptr::null()), -1);
    }

    #[test]
    fn lobby_connect_peers_null_safety() {
        assert_eq!(
            fcn_lobby_connect_peers(std::ptr::null(), std::ptr::null(), std::ptr::null()),
            -1
        );
    }

    #[test]
    fn lobby_free_null() {
        fcn_lobby_free(std::ptr::null_mut()); // no-op
    }

    // -- Lockstep FFI ------------------------------------------------------

    /// Helper: create a 2-player concurrent lockstep via FFI.
    fn make_lockstep() -> *mut FcnLockstep {
        let pks = std::ffi::CString::new(r#"["alice","bob"]"#).unwrap();
        let ls = fcn_lockstep_new(0, 0, pks.as_ptr());
        assert!(!ls.is_null(), "lockstep creation failed");
        ls
    }

    #[test]
    fn lockstep_full_lifecycle_ffi() {
        let ls = make_lockstep();
        assert_eq!(fcn_lockstep_begin_turn(ls, 1), 0);
        assert_eq!(fcn_lockstep_current_turn(ls), 1);
        assert_eq!(fcn_lockstep_current_phase(ls), FcnTurnPhase::Commit);

        // Compute commitments.
        let actions_a = std::ffi::CString::new(r#"["move"]"#).unwrap();
        let actions_b = std::ffi::CString::new(r#"["build"]"#).unwrap();
        let hash_a_ptr = fcn_lockstep_compute_commitment(actions_a.as_ptr());
        let hash_b_ptr = fcn_lockstep_compute_commitment(actions_b.as_ptr());
        assert!(!hash_a_ptr.is_null());
        assert!(!hash_b_ptr.is_null());

        let alice = std::ffi::CString::new("alice").unwrap();
        let bob = std::ffi::CString::new("bob").unwrap();

        // Commit phase.
        let r = fcn_lockstep_submit_commitment(ls, alice.as_ptr(), hash_a_ptr, 1);
        assert_eq!(r, FcnTurnResult::Waiting);
        let r = fcn_lockstep_submit_commitment(ls, bob.as_ptr(), hash_b_ptr, 1);
        assert_eq!(r, FcnTurnResult::Ready);
        assert_eq!(fcn_lockstep_current_phase(ls), FcnTurnPhase::Reveal);

        // Reveal phase.
        let r = fcn_lockstep_submit_reveal(ls, alice.as_ptr(), actions_a.as_ptr(), 1);
        assert_eq!(r, FcnTurnResult::Waiting);
        let r = fcn_lockstep_submit_reveal(ls, bob.as_ptr(), actions_b.as_ptr(), 1);
        assert_eq!(r, FcnTurnResult::Ready);
        assert_eq!(fcn_lockstep_current_phase(ls), FcnTurnPhase::Apply);

        // Ordered actions.
        let actions_ptr = fcn_lockstep_ordered_actions(ls);
        assert!(!actions_ptr.is_null());
        let actions_str = unsafe { CStr::from_ptr(actions_ptr) }.to_str().unwrap();
        assert!(actions_str.contains("alice"));
        assert!(actions_str.contains("bob"));
        crate::error::fcn_string_free(actions_ptr);

        // Apply + Verify.
        assert_eq!(fcn_lockstep_actions_applied(ls), 0);
        assert_eq!(fcn_lockstep_current_phase(ls), FcnTurnPhase::Verify);

        let hash = std::ffi::CString::new("state_hash_1").unwrap();
        let r = fcn_lockstep_submit_state_hash(ls, alice.as_ptr(), hash.as_ptr(), 1);
        assert_eq!(r, FcnTurnResult::Waiting);
        let r = fcn_lockstep_submit_state_hash(ls, bob.as_ptr(), hash.as_ptr(), 1);
        assert_eq!(r, FcnTurnResult::Ready);
        assert_eq!(fcn_lockstep_current_phase(ls), FcnTurnPhase::Complete);

        // Consensus hash.
        let ch = fcn_lockstep_consensus_hash(ls);
        assert!(!ch.is_null());
        let ch_str = unsafe { CStr::from_ptr(ch) }.to_str().unwrap();
        assert_eq!(ch_str, "state_hash_1");
        crate::error::fcn_string_free(ch);

        crate::error::fcn_string_free(hash_a_ptr);
        crate::error::fcn_string_free(hash_b_ptr);
        fcn_lockstep_free(ls);
    }

    #[test]
    fn lockstep_null_safety_new() {
        // Null player pubkeys JSON.
        assert!(fcn_lockstep_new(0, 0, std::ptr::null()).is_null());
    }

    #[test]
    fn lockstep_null_safety_begin_turn() {
        assert_eq!(fcn_lockstep_begin_turn(std::ptr::null_mut(), 1), -1);
    }

    #[test]
    fn lockstep_null_safety_submit_commitment() {
        assert_eq!(
            fcn_lockstep_submit_commitment(
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                0
            ),
            FcnTurnResult::Error
        );
    }

    #[test]
    fn lockstep_null_safety_submit_reveal() {
        assert_eq!(
            fcn_lockstep_submit_reveal(std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), 0),
            FcnTurnResult::Error
        );
    }

    #[test]
    fn lockstep_null_safety_ordered_actions() {
        assert!(fcn_lockstep_ordered_actions(std::ptr::null()).is_null());
    }

    #[test]
    fn lockstep_null_safety_actions_applied() {
        assert_eq!(fcn_lockstep_actions_applied(std::ptr::null_mut()), -1);
    }

    #[test]
    fn lockstep_null_safety_submit_state_hash() {
        assert_eq!(
            fcn_lockstep_submit_state_hash(
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                0
            ),
            FcnTurnResult::Error
        );
    }

    #[test]
    fn lockstep_null_safety_check_timeout() {
        assert_eq!(
            fcn_lockstep_check_timeout(std::ptr::null()),
            FcnTurnResult::Error
        );
    }

    #[test]
    fn lockstep_null_safety_current_turn() {
        assert_eq!(fcn_lockstep_current_turn(std::ptr::null()), -1);
    }

    #[test]
    fn lockstep_null_safety_current_phase() {
        // Returns Complete on null (with error set).
        assert_eq!(
            fcn_lockstep_current_phase(std::ptr::null()),
            FcnTurnPhase::Complete
        );
    }

    #[test]
    fn lockstep_null_safety_compute_commitment() {
        assert!(fcn_lockstep_compute_commitment(std::ptr::null()).is_null());
    }

    #[test]
    fn lockstep_null_safety_consensus_hash() {
        assert!(fcn_lockstep_consensus_hash(std::ptr::null()).is_null());
    }

    #[test]
    fn lockstep_free_null() {
        fcn_lockstep_free(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn lockstep_invalid_phase_mode() {
        let pks = std::ffi::CString::new(r#"["alice"]"#).unwrap();
        assert!(fcn_lockstep_new(99, 0, pks.as_ptr()).is_null());
    }

    #[test]
    fn lockstep_invalid_json() {
        let bad_json = std::ffi::CString::new("not json").unwrap();
        assert!(fcn_lockstep_new(0, 0, bad_json.as_ptr()).is_null());
    }

    #[test]
    fn lockstep_reveal_mismatch_ffi() {
        let ls = make_lockstep();
        fcn_lockstep_begin_turn(ls, 1);

        let actions_a = std::ffi::CString::new(r#"["move"]"#).unwrap();
        let actions_b = std::ffi::CString::new(r#"["build"]"#).unwrap();
        let hash_a = fcn_lockstep_compute_commitment(actions_a.as_ptr());
        let hash_b = fcn_lockstep_compute_commitment(actions_b.as_ptr());

        let alice = std::ffi::CString::new("alice").unwrap();
        let bob = std::ffi::CString::new("bob").unwrap();

        fcn_lockstep_submit_commitment(ls, alice.as_ptr(), hash_a, 1);
        fcn_lockstep_submit_commitment(ls, bob.as_ptr(), hash_b, 1);

        // Alice reveals different actions than committed.
        let cheat = std::ffi::CString::new(r#"["CHEAT"]"#).unwrap();
        let r = fcn_lockstep_submit_reveal(ls, alice.as_ptr(), cheat.as_ptr(), 1);
        assert_eq!(r, FcnTurnResult::RevealMismatch);

        crate::error::fcn_string_free(hash_a);
        crate::error::fcn_string_free(hash_b);
        fcn_lockstep_free(ls);
    }

    #[test]
    fn lockstep_desync_ffi() {
        let ls = make_lockstep();
        fcn_lockstep_begin_turn(ls, 1);

        let actions_a = std::ffi::CString::new("a").unwrap();
        let actions_b = std::ffi::CString::new("b").unwrap();
        let hash_a = fcn_lockstep_compute_commitment(actions_a.as_ptr());
        let hash_b = fcn_lockstep_compute_commitment(actions_b.as_ptr());

        let alice = std::ffi::CString::new("alice").unwrap();
        let bob = std::ffi::CString::new("bob").unwrap();

        fcn_lockstep_submit_commitment(ls, alice.as_ptr(), hash_a, 1);
        fcn_lockstep_submit_commitment(ls, bob.as_ptr(), hash_b, 1);
        fcn_lockstep_submit_reveal(ls, alice.as_ptr(), actions_a.as_ptr(), 1);
        fcn_lockstep_submit_reveal(ls, bob.as_ptr(), actions_b.as_ptr(), 1);
        fcn_lockstep_actions_applied(ls);

        let h1 = std::ffi::CString::new("hash_alice").unwrap();
        let h2 = std::ffi::CString::new("hash_bob").unwrap();
        fcn_lockstep_submit_state_hash(ls, alice.as_ptr(), h1.as_ptr(), 1);
        let r = fcn_lockstep_submit_state_hash(ls, bob.as_ptr(), h2.as_ptr(), 1);
        assert_eq!(r, FcnTurnResult::DesyncDetected);

        crate::error::fcn_string_free(hash_a);
        crate::error::fcn_string_free(hash_b);
        fcn_lockstep_free(ls);
    }

    #[test]
    fn lockstep_wrong_phase_ffi() {
        let ls = make_lockstep();
        fcn_lockstep_begin_turn(ls, 1);

        // Try to reveal during Commit phase.
        let alice = std::ffi::CString::new("alice").unwrap();
        let actions = std::ffi::CString::new("x").unwrap();
        let r = fcn_lockstep_submit_reveal(ls, alice.as_ptr(), actions.as_ptr(), 1);
        assert_eq!(r, FcnTurnResult::Error);

        fcn_lockstep_free(ls);
    }

    // -- Transport FFI tests -----------------------------------------------

    #[test]
    fn transport_lifecycle() {
        let t = fcn_transport_new();
        assert!(!t.is_null(), "transport creation failed");
        assert_eq!(fcn_transport_stream_count(t), 0);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_setup_listener() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        let lh = fcn_transport_setup_listener(t);
        assert!(lh >= 0, "listener handle should be non-negative");
        assert_eq!(fcn_transport_stream_count(t), 0);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_close_unknown_handle() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        fcn_transport_close_handle(t, 999);
        assert_eq!(fcn_transport_stream_count(t), 0);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_poll_empty() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        assert_eq!(fcn_transport_poll_handles(t, std::ptr::null_mut(), 0, 0), 0);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_poll_unknown_handle() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        let mut entry = FcnTransportPollEntry {
            handle: 999,
            requested_events: 0x01,
            returned_events: 0,
        };
        assert_eq!(fcn_transport_poll_handles(t, &mut entry, 1, 0), 0);
        assert_eq!(entry.returned_events, 0);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_poll_listener_no_pending() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        let lh = fcn_transport_setup_listener(t);
        let mut entry = FcnTransportPollEntry {
            handle: lh,
            requested_events: 0x01,
            returned_events: 0,
        };
        assert_eq!(fcn_transport_poll_handles(t, &mut entry, 1, 0), 0);
        assert_eq!(entry.returned_events, 0);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_read_unknown_handle() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        let mut buf = [0u8; 64];
        assert_eq!(fcn_transport_read(t, 999, buf.as_mut_ptr(), 64), -1);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_write_unknown_handle() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        assert_eq!(fcn_transport_write(t, 999, b"hello".as_ptr(), 5), -1);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_null_safety() {
        assert_eq!(fcn_transport_setup_listener(std::ptr::null_mut()), -1);
        assert_eq!(fcn_transport_accept(std::ptr::null_mut()), -1);
        fcn_transport_close_handle(std::ptr::null_mut(), 0);
        assert_eq!(
            fcn_transport_read(std::ptr::null_mut(), 0, std::ptr::null_mut(), 0),
            -1
        );
        assert_eq!(
            fcn_transport_write(std::ptr::null_mut(), 0, std::ptr::null(), 0),
            -1
        );
        assert_eq!(
            fcn_transport_poll_handles(std::ptr::null_mut(), std::ptr::null_mut(), 0, 0),
            -1
        );
        assert_eq!(fcn_transport_stream_count(std::ptr::null()), -1);
        fcn_transport_free(std::ptr::null_mut());
    }

    #[test]
    fn transport_read_null_buf_with_len() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        assert_eq!(fcn_transport_read(t, 0, std::ptr::null_mut(), 10), -1);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_write_null_buf_with_len() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        assert_eq!(fcn_transport_write(t, 0, std::ptr::null(), 10), -1);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_read_negative_len() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        let mut buf = [0u8; 8];
        assert_eq!(fcn_transport_read(t, 0, buf.as_mut_ptr(), -1), -1);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_write_negative_len() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        assert_eq!(fcn_transport_write(t, 0, [0u8; 8].as_ptr(), -1), -1);
        fcn_transport_free(t);
    }

    #[test]
    fn transport_poll_negative_count() {
        let t = fcn_transport_new();
        assert!(!t.is_null());
        assert_eq!(
            fcn_transport_poll_handles(t, std::ptr::null_mut(), -1, 0),
            -1
        );
        fcn_transport_free(t);
    }

    // -- Relay / Connection Monitor FFI ------------------------------------

    #[test]
    fn relay_config_default_returns_json() {
        let ptr = fcn_relay_config_default();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert!(s.contains("use_public_relays"));
        assert!(s.contains("true"));
        crate::error::fcn_string_free(ptr);
    }

    #[test]
    fn connection_monitor_lifecycle() {
        let m = fcn_connection_monitor_new();
        assert!(!m.is_null());

        assert_eq!(fcn_connection_monitor_active_count(m), 0);
        assert_eq!(fcn_connection_monitor_direct_count(m), 0);
        assert_eq!(fcn_connection_monitor_relayed_count(m), 0);

        // Update with a direct peer.
        let quality_json = std::ffi::CString::new(
            r#"{"conn_type":"Direct","rtt":null,"is_active":true,"peer_id":"peer1"}"#,
        )
        .unwrap();
        assert_eq!(fcn_connection_monitor_update(m, quality_json.as_ptr()), 0);
        assert_eq!(fcn_connection_monitor_active_count(m), 1);
        assert_eq!(fcn_connection_monitor_direct_count(m), 1);
        assert_eq!(fcn_connection_monitor_relayed_count(m), 0);

        // Update with a relayed peer.
        let quality_json2 = std::ffi::CString::new(
            r#"{"conn_type":"Relayed","rtt":null,"is_active":true,"peer_id":"peer2"}"#,
        )
        .unwrap();
        assert_eq!(fcn_connection_monitor_update(m, quality_json2.as_ptr()), 0);
        assert_eq!(fcn_connection_monitor_active_count(m), 2);
        assert_eq!(fcn_connection_monitor_direct_count(m), 1);
        assert_eq!(fcn_connection_monitor_relayed_count(m), 1);

        // Get peer1 quality.
        let peer_id = std::ffi::CString::new("peer1").unwrap();
        let q = fcn_connection_monitor_get(m, peer_id.as_ptr());
        assert!(!q.is_null());
        let q_str = unsafe { CStr::from_ptr(q) }.to_str().unwrap();
        assert!(q_str.contains("Direct"));
        crate::error::fcn_string_free(q);

        // Get nonexistent peer.
        let missing = std::ffi::CString::new("ghost").unwrap();
        let q = fcn_connection_monitor_get(m, missing.as_ptr());
        assert!(q.is_null());

        fcn_connection_monitor_free(m);
    }

    #[test]
    fn connection_monitor_null_safety() {
        assert_eq!(
            fcn_connection_monitor_update(std::ptr::null_mut(), std::ptr::null()),
            -1
        );
        assert!(fcn_connection_monitor_get(std::ptr::null(), std::ptr::null()).is_null());
        assert_eq!(fcn_connection_monitor_direct_count(std::ptr::null()), -1);
        assert_eq!(fcn_connection_monitor_relayed_count(std::ptr::null()), -1);
        assert_eq!(fcn_connection_monitor_active_count(std::ptr::null()), -1);
        fcn_connection_monitor_free(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn connection_monitor_invalid_json() {
        let m = fcn_connection_monitor_new();
        let bad_json = std::ffi::CString::new("not json").unwrap();
        assert_eq!(fcn_connection_monitor_update(m, bad_json.as_ptr()), -1);
        fcn_connection_monitor_free(m);
    }

    // -- Game Node FFI -----------------------------------------------------

    fn make_node_config_json(is_lead: bool) -> std::ffi::CString {
        std::ffi::CString::new(format!(
            r#"{{"player_pubkey":"player_abc","is_lead":{},"game_event_id":null,"phase_mode":0,"turn_timeout_secs":0,"checkpoint_interval":5}}"#,
            is_lead
        ))
        .unwrap()
    }

    #[test]
    fn node_full_lifecycle_ffi() {
        let cfg = make_node_config_json(true);
        let node = fcn_node_new(cfg.as_ptr());
        assert!(!node.is_null());

        assert_eq!(fcn_node_state(node), 0); // Initializing
        assert_eq!(fcn_node_is_lead(node), 1);
        assert_eq!(fcn_node_current_turn(node), 0);

        let pk = fcn_node_player_pubkey(node);
        assert!(!pk.is_null());
        let pk_str = unsafe { CStr::from_ptr(pk) }.to_str().unwrap();
        assert_eq!(pk_str, "player_abc");
        crate::error::fcn_string_free(pk);

        // Create lobby.
        let lobby_id = std::ffi::CString::new("lobby_1").unwrap();
        assert_eq!(fcn_node_create_lobby(node, lobby_id.as_ptr(), 4), 0);
        assert_eq!(fcn_node_state(node), 1); // InLobby

        // Start game.
        let players = std::ffi::CString::new(r#"["alice","bob"]"#).unwrap();
        assert_eq!(fcn_node_start_game(node, players.as_ptr()), 0);
        assert_eq!(fcn_node_state(node), 2); // Connecting

        // Connections ready.
        assert_eq!(fcn_node_connections_ready(node), 0);
        assert_eq!(fcn_node_state(node), 3); // Playing

        // Begin turn.
        assert_eq!(fcn_node_begin_turn(node, 1), 0);
        assert_eq!(fcn_node_current_turn(node), 1);

        // End game.
        assert_eq!(fcn_node_end_game(node), 0);
        assert_eq!(fcn_node_state(node), 4); // Finished

        fcn_node_free(node);
    }

    #[test]
    fn node_join_lobby_ffi() {
        let cfg = make_node_config_json(false);
        let node = fcn_node_new(cfg.as_ptr());
        assert!(!node.is_null());

        assert_eq!(fcn_node_is_lead(node), 0);

        let lobby_id = std::ffi::CString::new("lobby_1").unwrap();
        let lead_pk = std::ffi::CString::new("lead_pk").unwrap();
        assert_eq!(
            fcn_node_join_lobby(node, lobby_id.as_ptr(), lead_pk.as_ptr(), 4),
            0
        );
        assert_eq!(fcn_node_state(node), 1); // InLobby

        fcn_node_free(node);
    }

    #[test]
    fn node_wrong_state_transitions_ffi() {
        let cfg = make_node_config_json(true);
        let node = fcn_node_new(cfg.as_ptr());
        assert!(!node.is_null());

        // Can't start game from Initializing.
        let players = std::ffi::CString::new(r#"["alice"]"#).unwrap();
        assert_eq!(fcn_node_start_game(node, players.as_ptr()), -1);

        // Can't connections_ready from Initializing.
        assert_eq!(fcn_node_connections_ready(node), -1);

        // Can't begin_turn from Initializing.
        assert_eq!(fcn_node_begin_turn(node, 1), -1);

        fcn_node_free(node);
    }

    #[test]
    fn node_null_safety() {
        assert!(fcn_node_new(std::ptr::null()).is_null());
        assert_eq!(fcn_node_state(std::ptr::null()), -1);
        assert_eq!(
            fcn_node_create_lobby(std::ptr::null_mut(), std::ptr::null(), 4),
            -1
        );
        assert_eq!(
            fcn_node_join_lobby(std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), 4),
            -1
        );
        assert_eq!(
            fcn_node_start_game(std::ptr::null_mut(), std::ptr::null()),
            -1
        );
        assert_eq!(fcn_node_connections_ready(std::ptr::null_mut()), -1);
        assert_eq!(fcn_node_begin_turn(std::ptr::null_mut(), 1), -1);
        assert_eq!(fcn_node_end_game(std::ptr::null_mut()), -1);
        assert_eq!(fcn_node_current_turn(std::ptr::null()), -1);
        assert!(fcn_node_player_pubkey(std::ptr::null()).is_null());
        assert_eq!(fcn_node_is_lead(std::ptr::null()), -1);
        fcn_node_free(std::ptr::null_mut()); // no-op
    }

    #[test]
    fn node_invalid_config_json() {
        let bad = std::ffi::CString::new("not json").unwrap();
        assert!(fcn_node_new(bad.as_ptr()).is_null());
    }

    #[test]
    fn node_invalid_phase_mode() {
        let cfg = std::ffi::CString::new(
            r#"{"player_pubkey":"pk","is_lead":true,"game_event_id":null,"phase_mode":99,"turn_timeout_secs":0,"checkpoint_interval":0}"#,
        )
        .unwrap();
        assert!(fcn_node_new(cfg.as_ptr()).is_null());
    }

    #[test]
    fn node_invalid_player_pubkeys_json() {
        let cfg = make_node_config_json(true);
        let node = fcn_node_new(cfg.as_ptr());
        assert!(!node.is_null());

        let lobby_id = std::ffi::CString::new("lobby_1").unwrap();
        fcn_node_create_lobby(node, lobby_id.as_ptr(), 4);

        let bad = std::ffi::CString::new("not json").unwrap();
        assert_eq!(fcn_node_start_game(node, bad.as_ptr()), -1);

        fcn_node_free(node);
    }
}

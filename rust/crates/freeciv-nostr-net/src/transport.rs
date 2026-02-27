//! QUIC stream transport adapter for the `fc_transport_ops` vtable.
//!
//! Maps Iroh QUIC streams to the handle-based transport API expected by
//! Freeciv's networking layer. Each transport handle corresponds to one
//! bidirectional QUIC stream.
//!
//! # Architecture
//!
//! The C transport layer uses integer handles (`fc_transport_handle`) to
//! identify connections. For the TCP backend, these are raw file descriptors.
//! This module maintains a handle table mapping integers to QUIC stream
//! pairs (`SendStream`, `RecvStream`), presenting the same interface.
//!
//! # Listener emulation
//!
//! QUIC doesn't have a traditional listen/accept model. Instead, we
//! emulate it with an internal channel: when the endpoint accepts a new
//! bidirectional stream, it pushes the stream pair into the channel.
//! The transport's `accept()` pops from this channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};

use iroh::endpoint::{RecvStream, SendStream};
use tokio::sync::mpsc;

use crate::error::NetError;

/// A handle identifying a QUIC stream in the transport layer.
/// Maps to `fc_transport_handle` on the C side.
pub type TransportHandle = i32;

/// Invalid handle constant (matches C side `FC_TRANSPORT_INVALID = -1`).
pub const INVALID_HANDLE: TransportHandle = -1;

/// Transport event: data available to read.
pub const EVENT_READ: u32 = 0x01;

/// Transport event: ready to write.
pub const EVENT_WRITE: u32 = 0x02;

/// Transport event: error condition.
pub const EVENT_ERROR: u32 = 0x04;

/// A poll entry for the transport layer.
///
/// Mirrors `struct fc_transport_poll_entry` on the C side. Callers fill
/// in `handle` and `requested_events`; after poll, `returned_events`
/// indicates which events fired.
#[derive(Debug, Clone)]
pub struct PollEntry {
    /// The transport handle to monitor.
    pub handle: TransportHandle,
    /// Bitmask of events to watch for (e.g. `EVENT_READ | EVENT_WRITE`).
    pub requested_events: u32,
    /// Bitmask of events that fired (output, set by `poll()`).
    pub returned_events: u32,
}

/// State for a single bidirectional QUIC stream.
struct StreamState {
    /// Send half of the QUIC stream.
    send: SendStream,
    /// Receive half of the QUIC stream.
    recv: RecvStream,
    /// Data read from QUIC but not yet consumed by the C caller.
    read_buf: Vec<u8>,
    /// Read offset into `read_buf` (bytes already consumed).
    read_pos: usize,
    /// Whether the stream is ready for writing. QUIC streams are
    /// generally always write-ready unless back-pressured.
    write_ready: bool,
}

/// QUIC transport state mapping integer handles to QUIC streams.
///
/// This is the core data structure behind the Iroh transport backend.
/// It is NOT thread-safe on its own; the FFI layer wraps it in a `Mutex`.
pub struct QuicTransport {
    /// Active streams keyed by handle.
    streams: HashMap<TransportHandle, StreamState>,
    /// Monotonically increasing handle allocator. Starts at 1 because
    /// 0 is sometimes used as a sentinel in C code.
    next_handle: AtomicI32,
    /// Receiving end of the accept channel (listener emulation).
    accept_rx: Option<mpsc::Receiver<(SendStream, RecvStream)>>,
    /// Sending end of the accept channel (fed by the endpoint accept loop).
    accept_tx: Option<mpsc::Sender<(SendStream, RecvStream)>>,
    /// The handle returned by `setup_listener()`, if active.
    listen_handle: Option<TransportHandle>,
}

impl QuicTransport {
    /// Create a new, empty transport with no active streams.
    pub fn new() -> Self {
        Self {
            streams: HashMap::new(),
            next_handle: AtomicI32::new(1),
            accept_rx: None,
            accept_tx: None,
            listen_handle: None,
        }
    }

    /// Allocate the next unused handle.
    fn alloc_handle(&self) -> TransportHandle {
        self.next_handle.fetch_add(1, Ordering::Relaxed)
    }

    /// Register an already-established QUIC stream pair and return its handle.
    ///
    /// The caller is responsible for ensuring the streams are open and
    /// ready for I/O.
    pub fn register_stream(&mut self, send: SendStream, recv: RecvStream) -> TransportHandle {
        let handle = self.alloc_handle();
        self.streams.insert(
            handle,
            StreamState {
                send,
                recv,
                read_buf: Vec::with_capacity(4096),
                read_pos: 0,
                write_ready: true,
            },
        );
        handle
    }

    /// Set up a listener that accepts connections via an internal channel.
    ///
    /// Returns the listener handle. Feed accepted stream pairs into
    /// the sender obtained from [`accept_sender()`].
    pub fn setup_listener(&mut self) -> TransportHandle {
        let (tx, rx) = mpsc::channel(16);
        let handle = self.alloc_handle();
        self.accept_tx = Some(tx);
        self.accept_rx = Some(rx);
        self.listen_handle = Some(handle);
        handle
    }

    /// Get a clone of the accept channel sender.
    ///
    /// The endpoint accept loop uses this to push newly accepted stream
    /// pairs into the transport.
    pub fn accept_sender(&self) -> Option<mpsc::Sender<(SendStream, RecvStream)>> {
        self.accept_tx.clone()
    }

    /// Try to accept a connection from the listener channel.
    ///
    /// This is async because it waits on the internal mpsc channel.
    /// Returns the handle for the newly accepted stream.
    pub async fn accept(&mut self) -> Result<TransportHandle, NetError> {
        let rx = self.accept_rx.as_mut().ok_or(NetError::EndpointClosed)?;
        let (send, recv) = rx.recv().await.ok_or(NetError::EndpointClosed)?;
        Ok(self.register_stream(send, recv))
    }

    /// Try to accept without blocking (non-async version for poll contexts).
    ///
    /// Returns `Ok(Some(handle))` if a stream was available, `Ok(None)` if
    /// no stream is pending, or `Err` if the listener is not set up.
    pub fn try_accept(&mut self) -> Result<Option<TransportHandle>, NetError> {
        let rx = self.accept_rx.as_mut().ok_or(NetError::EndpointClosed)?;
        match rx.try_recv() {
            Ok((send, recv)) => Ok(Some(self.register_stream(send, recv))),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(NetError::EndpointClosed),
        }
    }

    /// Close a stream by handle, dropping both send and recv halves.
    pub fn close(&mut self, handle: TransportHandle) {
        self.streams.remove(&handle);
    }

    /// Read from a stream into the provided buffer.
    ///
    /// First drains any internally buffered data, then reads from the
    /// QUIC stream. Returns the number of bytes read, 0 on stream
    /// finish (EOF), or an error.
    pub async fn read(
        &mut self,
        handle: TransportHandle,
        buf: &mut [u8],
    ) -> Result<usize, NetError> {
        let state = self
            .streams
            .get_mut(&handle)
            .ok_or(NetError::PeerNotFound(handle.to_string()))?;

        // First, drain any buffered data from a previous over-read.
        if state.read_pos < state.read_buf.len() {
            let available = &state.read_buf[state.read_pos..];
            let n = available.len().min(buf.len());
            buf[..n].copy_from_slice(&available[..n]);
            state.read_pos += n;
            if state.read_pos >= state.read_buf.len() {
                state.read_buf.clear();
                state.read_pos = 0;
            }
            return Ok(n);
        }

        // Read directly from the QUIC stream.
        match state.recv.read(buf).await {
            Ok(Some(n)) => Ok(n),
            Ok(None) => Ok(0), // Stream finished (EOF)
            Err(e) => Err(NetError::Stream(e.to_string())),
        }
    }

    /// Write data to a stream.
    ///
    /// Writes all bytes (QUIC streams don't do partial writes like TCP).
    /// Returns the number of bytes written (always `data.len()` on success).
    pub async fn write(&mut self, handle: TransportHandle, data: &[u8]) -> Result<usize, NetError> {
        let state = self
            .streams
            .get_mut(&handle)
            .ok_or(NetError::PeerNotFound(handle.to_string()))?;

        state
            .send
            .write_all(data)
            .await
            .map_err(|e| NetError::Stream(e.to_string()))?;
        Ok(data.len())
    }

    /// Poll handles for readiness, setting `returned_events` on each entry.
    ///
    /// This is a synchronous, non-blocking check of locally known state:
    /// - Read readiness: buffered data exists in `read_buf`.
    /// - Write readiness: always true for QUIC (no kernel send buffer).
    /// - Listener readiness: accept channel has pending connections.
    ///
    /// Returns the number of handles with at least one event.
    ///
    /// Note: `_timeout_ms` is accepted for API compatibility but currently
    /// ignored — this implementation is always non-blocking. A future
    /// version could use `tokio::time::timeout` to implement blocking poll.
    pub fn poll(&self, entries: &mut [PollEntry], _timeout_ms: i32) -> usize {
        let mut ready = 0;
        for entry in entries.iter_mut() {
            entry.returned_events = 0;

            // Check listener handle.
            if Some(entry.handle) == self.listen_handle {
                if entry.requested_events & EVENT_READ != 0
                    && let Some(ref rx) = self.accept_rx
                {
                    // mpsc::Receiver::is_empty() tells us if accept would block.
                    if !rx.is_empty() {
                        entry.returned_events |= EVENT_READ;
                        ready += 1;
                    }
                }
                continue;
            }

            // Check stream handles.
            if let Some(state) = self.streams.get(&entry.handle) {
                if entry.requested_events & EVENT_READ != 0 && state.read_pos < state.read_buf.len()
                {
                    entry.returned_events |= EVENT_READ;
                }
                if entry.requested_events & EVENT_WRITE != 0 && state.write_ready {
                    entry.returned_events |= EVENT_WRITE;
                }
                if entry.returned_events != 0 {
                    ready += 1;
                }
            }
        }
        ready
    }

    /// Get the number of active streams (excluding the listener).
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    /// Check whether a handle (stream or listener) exists.
    pub fn has_handle(&self, handle: TransportHandle) -> bool {
        self.streams.contains_key(&handle) || self.listen_handle == Some(handle)
    }
}

impl Default for QuicTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_transport_is_empty() {
        let t = QuicTransport::new();
        assert_eq!(t.stream_count(), 0);
        assert!(!t.has_handle(1));
        assert!(!t.has_handle(INVALID_HANDLE));
    }

    #[test]
    fn default_matches_new() {
        let t = QuicTransport::default();
        assert_eq!(t.stream_count(), 0);
    }

    #[test]
    fn setup_listener_returns_valid_handle() {
        let mut t = QuicTransport::new();
        let h = t.setup_listener();
        assert_ne!(h, INVALID_HANDLE);
        assert!(t.has_handle(h));
        assert!(t.accept_sender().is_some());
    }

    #[test]
    fn close_unknown_handle_is_noop() {
        let mut t = QuicTransport::new();
        t.close(42); // should not panic
        assert_eq!(t.stream_count(), 0);
    }

    #[test]
    fn poll_empty_returns_zero() {
        let t = QuicTransport::new();
        let mut entries: Vec<PollEntry> = vec![];
        assert_eq!(t.poll(&mut entries, 0), 0);
    }

    #[test]
    fn poll_unknown_handle_returns_zero() {
        let t = QuicTransport::new();
        let mut entries = vec![PollEntry {
            handle: 999,
            requested_events: EVENT_READ | EVENT_WRITE,
            returned_events: 0,
        }];
        assert_eq!(t.poll(&mut entries, 0), 0);
        assert_eq!(entries[0].returned_events, 0);
    }

    #[test]
    fn poll_listener_no_pending() {
        let mut t = QuicTransport::new();
        let lh = t.setup_listener();
        let mut entries = vec![PollEntry {
            handle: lh,
            requested_events: EVENT_READ,
            returned_events: 0,
        }];
        // Nothing in the accept channel yet.
        assert_eq!(t.poll(&mut entries, 0), 0);
        assert_eq!(entries[0].returned_events, 0);
    }

    #[test]
    fn handle_allocation_is_monotonic() {
        let t = QuicTransport::new();
        let h1 = t.alloc_handle();
        let h2 = t.alloc_handle();
        let h3 = t.alloc_handle();
        assert!(h1 < h2);
        assert!(h2 < h3);
        assert!(h1 >= 1, "handles should start at 1");
    }

    #[test]
    fn try_accept_without_listener_errors() {
        let mut t = QuicTransport::new();
        assert!(t.try_accept().is_err());
    }

    #[test]
    fn try_accept_empty_returns_none() {
        let mut t = QuicTransport::new();
        let _lh = t.setup_listener();
        let result = t.try_accept().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn event_constants_match_c_side() {
        // These must match the C enum fc_transport_event values.
        assert_eq!(EVENT_READ, 0x01);
        assert_eq!(EVENT_WRITE, 0x02);
        assert_eq!(EVENT_ERROR, 0x04);
    }

    #[test]
    fn invalid_handle_is_negative_one() {
        assert_eq!(INVALID_HANDLE, -1);
    }

    #[tokio::test]
    async fn accept_with_sender() {
        let mut t = QuicTransport::new();
        let _lh = t.setup_listener();
        let sender = t.accept_sender().unwrap();

        // We can't easily create real SendStream/RecvStream without an Iroh
        // endpoint, but we verify the channel mechanics work by checking that
        // try_accept returns None when nothing is sent.
        assert!(t.try_accept().unwrap().is_none());

        // Verify sender is usable (it's cloned from the transport).
        assert!(!sender.is_closed());
    }

    #[tokio::test]
    async fn read_unknown_handle_errors() {
        let mut t = QuicTransport::new();
        let mut buf = [0u8; 64];
        let result = t.read(42, &mut buf).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn write_unknown_handle_errors() {
        let mut t = QuicTransport::new();
        let result = t.write(42, b"hello").await;
        assert!(result.is_err());
    }
}

/***********************************************************************
 Freeciv - Copyright (C) 1996 - A Kjeldberg, L Gregersen, P Unold
   This program is free software; you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation; either version 2, or (at your option)
   any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU General Public License for more details.
***********************************************************************/

/***********************************************************************
  Iroh QUIC transport backend for the fc_transport_ops vtable.

  Bridges Freeciv's synchronous, handle-based transport API to the
  Iroh QUIC implementation in Rust. Each function delegates to a
  corresponding fcn_transport_*() FFI function.

  All code is guarded by FREECIV_NOSTR; when the flag is not defined,
  this translation unit compiles to nothing.
***********************************************************************/

#ifdef HAVE_CONFIG_H
#include <fc_config.h>
#endif

#ifdef FREECIV_NOSTR

#include <stdio.h>   /* snprintf */
#include <stddef.h>  /* size_t */

/* utility */
#include "log.h"
#include "transport.h"

#include "iroh_transport.h"

/* ------------------------------------------------------------------ */
/* FFI declarations — implemented in the Rust static library           */
/* (freeciv-nostr-ffi crate).                                          */
/* ------------------------------------------------------------------ */

extern void *fcn_transport_new(void);
extern int   fcn_transport_setup_listener(void *transport);
extern int   fcn_transport_accept(void *transport);
extern void  fcn_transport_close_handle(void *transport, int handle);
extern int   fcn_transport_read(void *transport, int handle,
                                char *buf, int len);
extern int   fcn_transport_write(void *transport, int handle,
                                 const char *buf, int len);
extern int   fcn_transport_poll_handles(void *transport,
                                        struct fc_transport_poll_entry *entries,
                                        int count, int timeout_ms);
extern int   fcn_transport_stream_count(void *transport);
extern void  fcn_transport_free(void *transport);

/* ------------------------------------------------------------------ */
/* Global state                                                        */
/* ------------------------------------------------------------------ */

/* Lazily initialized on first use (listen or connect). */
static void *g_transport = NULL;

/**********************************************************************//**
  Ensure the global Rust transport is initialized.
  Returns the transport pointer, or NULL on failure.
**************************************************************************/
static void *ensure_transport(void)
{
  if (g_transport == NULL) {
    g_transport = fcn_transport_new();
    if (g_transport == NULL) {
      log_error("iroh_transport: failed to create Rust transport");
    }
  }
  return g_transport;
}

/* ------------------------------------------------------------------ */
/* fc_transport_ops callbacks                                          */
/* ------------------------------------------------------------------ */

/**********************************************************************//**
  Iroh: Create a listener.

  For the Iroh backend, bind_addr and port are ignored — the QUIC
  endpoint listens on its own address determined at endpoint creation.
  The backlog parameter is also unused (the internal channel has a
  fixed capacity).
**************************************************************************/
static int iroh_listen_at(fc_transport_handle *out,
                          const char *bind_addr, int port,
                          int backlog)
{
  void *t;
  int handle;

  (void)bind_addr;
  (void)port;
  (void)backlog;

  t = ensure_transport();
  if (t == NULL) {
    return -1;
  }

  handle = fcn_transport_setup_listener(t);
  if (handle < 0) {
    log_error("iroh_transport: setup_listener failed");
    return -1;
  }

  *out = handle;
  log_verbose("iroh_transport: listener handle = %d", handle);
  return 0;
}

/**********************************************************************//**
  Iroh: Accept an incoming connection on a listener.

  Blocks until a peer stream is available. The listener handle
  (listen_h) is currently unused — there is only one global listener
  channel.
**************************************************************************/
static int iroh_accept_conn(fc_transport_handle listen_h,
                            fc_transport_handle *out,
                            char *dst_host, int dst_host_len)
{
  int handle;

  (void)listen_h;

  if (g_transport == NULL) {
    return -1;
  }

  handle = fcn_transport_accept(g_transport);
  if (handle < 0) {
    return -1;
  }

  *out = handle;

  if (dst_host != NULL && dst_host_len > 0) {
    snprintf(dst_host, dst_host_len, "quic-peer");
  }

  log_verbose("iroh_transport: accepted stream handle = %d", handle);
  return 0;
}

/**********************************************************************//**
  Iroh: Connect to a remote endpoint.

  For the P2P model, connections are established through the lobby
  mechanism rather than direct host:port connect. This function logs
  an error and returns -1. A future version may support direct
  connect via EndpointAddr.
**************************************************************************/
static int iroh_connect_to(fc_transport_handle *out,
                           const char *host, int port)
{
  (void)out;
  (void)host;
  (void)port;

  log_error("iroh_transport: connect_to not supported; "
            "use lobby-based connection instead");
  return -1;
}

/**********************************************************************//**
  Iroh: Close a stream handle.
**************************************************************************/
static void iroh_close(fc_transport_handle h)
{
  if (g_transport != NULL) {
    fcn_transport_close_handle(g_transport, h);
    log_verbose("iroh_transport: closed handle %d", h);
  }
}

/**********************************************************************//**
  Iroh: Read from a stream.

  Returns bytes read, 0 on EOF, -1 on error.
**************************************************************************/
static int iroh_read(fc_transport_handle h, void *buf, size_t len)
{
  if (g_transport == NULL) {
    return -1;
  }
  return fcn_transport_read(g_transport, h, (char *)buf, (int)len);
}

/**********************************************************************//**
  Iroh: Write to a stream.

  Returns bytes written, -1 on error.
**************************************************************************/
static int iroh_write(fc_transport_handle h, const void *buf, size_t len)
{
  if (g_transport == NULL) {
    return -1;
  }
  return fcn_transport_write(g_transport, h, (const char *)buf, (int)len);
}

/**********************************************************************//**
  Iroh: Poll a set of handles for readiness.

  Delegates to the Rust transport's poll implementation. The C
  fc_transport_poll_entry struct is layout-compatible with the FFI
  FcnTransportPollEntry struct (both are three consecutive ints).
**************************************************************************/
static int iroh_poll(struct fc_transport_poll_set *set, int timeout_ms)
{
  if (g_transport == NULL || set == NULL) {
    return -1;
  }
  return fcn_transport_poll_handles(g_transport,
                                    set->entries,
                                    set->count,
                                    timeout_ms);
}

/**********************************************************************//**
  Iroh: Set non-blocking mode.

  QUIC streams are inherently asynchronous; this is a no-op.
**************************************************************************/
static void iroh_set_nonblock(fc_transport_handle h)
{
  (void)h;
  /* QUIC streams are always non-blocking by nature. */
}

/* ------------------------------------------------------------------ */
/* Operations vtable                                                   */
/* ------------------------------------------------------------------ */

static const struct fc_transport_ops iroh_ops = {
  .name         = "iroh-quic",
  .listen_at    = iroh_listen_at,
  .accept_conn  = iroh_accept_conn,
  .connect_to   = iroh_connect_to,
  .close        = iroh_close,
  .read         = iroh_read,
  .write        = iroh_write,
  .poll         = iroh_poll,
  .set_nonblock = iroh_set_nonblock,
};

/* ------------------------------------------------------------------ */
/* Public API                                                          */
/* ------------------------------------------------------------------ */

/**********************************************************************//**
  Get a pointer to the Iroh transport operations vtable.
**************************************************************************/
const struct fc_transport_ops *fc_iroh_transport_ops(void)
{
  return &iroh_ops;
}

/**********************************************************************//**
  Initialize the Iroh QUIC transport backend.

  Creates the Rust-side QuicTransport and registers the Iroh ops as
  the active transport backend.
**************************************************************************/
void fc_iroh_transport_init(void)
{
  if (ensure_transport() == NULL) {
    log_error("iroh_transport: init failed — could not create transport");
    return;
  }

  fc_transport_set_backend(&iroh_ops);
  log_normal("iroh_transport: backend initialized");
}

#endif /* FREECIV_NOSTR */

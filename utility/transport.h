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

#ifndef FC__TRANSPORT_H
#define FC__TRANSPORT_H

#ifdef __cplusplus
extern "C" {
#endif

/***********************************************************************
  Transport abstraction layer for Freeciv networking.

  This module decouples the game engine from specific transport
  mechanisms (TCP sockets, QUIC streams, etc.) by providing a
  vtable-based abstraction.

  The default backend wraps POSIX TCP sockets (via netintf.c).
  Alternative backends (e.g., Iroh QUIC) implement the same
  interface and are registered at startup.

  Design constraints:
  - fc_transport_handle is typedef'd to int for minimal disruption.
    The TCP backend uses it directly as a file descriptor.
    Non-fd backends use a handle-table to map integers to their
    internal connection objects.
  - The poll mechanism wraps select() semantics but uses an opaque
    poll set to allow non-fd-based backends.
***********************************************************************/

#include <stddef.h>      /* size_t */

/* utility */
#include "support.h"     /* bool, fc_timeval */

/**********************************************************************//**
  Opaque transport handle. For the TCP backend this is a raw file
  descriptor. Non-socket backends maintain a handle table mapping
  these integers to internal connection objects.

  A value of -1 indicates an invalid / uninitialized handle.
**************************************************************************/
typedef int fc_transport_handle;

#define FC_TRANSPORT_INVALID (-1)

/**********************************************************************//**
  Maximum number of handles that can be monitored in a single poll call.
  Must be >= MAX_NUM_CONNECTIONS (1024, from common/fc_types.h) plus a
  margin for listen sockets. We cannot include fc_types.h here (it would
  create a circular dependency with utility/), so the value is hardcoded.
  A static_assert in transport.c verifies this stays in sync.
**************************************************************************/
#define FC_TRANSPORT_POLL_MAX 1032

/**********************************************************************//**
  Events that can be monitored / reported by the poll mechanism.
**************************************************************************/
enum fc_transport_event {
  FC_TRANSPORT_READ  = 0x01,
  FC_TRANSPORT_WRITE = 0x02,
  FC_TRANSPORT_ERROR = 0x04
};

/**********************************************************************//**
  A single entry in a poll set. Callers fill in `handle` and
  `requested_events`; after poll() returns, `returned_events` indicates
  which events fired.
**************************************************************************/
struct fc_transport_poll_entry {
  fc_transport_handle handle;
  int requested_events;  /* bitmask of fc_transport_event */
  int returned_events;   /* bitmask of fc_transport_event (output) */
};

/**********************************************************************//**
  Poll set: an array of entries plus a count.
**************************************************************************/
struct fc_transport_poll_set {
  struct fc_transport_poll_entry entries[FC_TRANSPORT_POLL_MAX];
  int count;
};

/**********************************************************************//**
  Transport operations vtable. Each backend provides an implementation
  of these functions. All functions must be safe to call from the
  main thread (Freeciv is single-threaded for networking).

  Return values follow POSIX conventions:
  - read/write return bytes transferred, 0 on EOF, -1 on error
  - listen/accept/connect return 0 on success, -1 on error
  - poll returns number of ready handles, -1 on error
**************************************************************************/
struct fc_transport_ops {

  /* Human-readable name for this backend (e.g., "tcp", "iroh-quic") */
  const char *name;

  /*--- Connection lifecycle ---*/

  /* Create a listening endpoint bound to `bind_addr:port`.
   * Stores the handle in *out. bind_addr may be NULL for INADDR_ANY.
   * Returns 0 on success, -1 on error. */
  int (*listen_at)(fc_transport_handle *out,
                   const char *bind_addr, int port,
                   int backlog);

  /* Accept an incoming connection on a listening handle.
   * Stores the new connection handle in *out.
   * Optionally fills dst_host (up to dst_host_len) with peer info.
   * Returns 0 on success, -1 on error.
   * Callers should use poll() to check readiness before calling. */
  int (*accept_conn)(fc_transport_handle listen_h,
                     fc_transport_handle *out,
                     char *dst_host, int dst_host_len);

  /* Open a connection to a remote endpoint.
   * Stores the handle in *out.
   * Returns 0 on success, -1 on error. */
  int (*connect_to)(fc_transport_handle *out,
                    const char *host, int port);

  /* Close a handle (connection or listener). */
  void (*close)(fc_transport_handle h);

  /*--- Data transfer ---*/

  /* Read up to `len` bytes into `buf`.
   * Returns bytes read, 0 on EOF, -1 on error. */
  int (*read)(fc_transport_handle h, void *buf, size_t len);

  /* Write up to `len` bytes from `buf`.
   * Returns bytes written, -1 on error. */
  int (*write)(fc_transport_handle h, const void *buf, size_t len);

  /*--- Readiness / polling ---*/

  /* Wait for events on a set of handles.
   * timeout_ms < 0 means block indefinitely, 0 means non-blocking poll.
   * Returns number of handles with events, 0 on timeout, -1 on error. */
  int (*poll)(struct fc_transport_poll_set *set, int timeout_ms);

  /*--- Configuration ---*/

  /* Set a handle to non-blocking mode.
   * Optional: may be NULL for inherently async backends (e.g., QUIC)
   * where all I/O is non-blocking by design. */
  void (*set_nonblock)(fc_transport_handle h);
};

/**********************************************************************//**
  Global transport API. These functions dispatch through the currently
  registered backend.
**************************************************************************/

/* Initialize the transport layer. Must be called once at startup,
 * before any other transport functions. Installs the TCP backend
 * by default. */
void fc_transport_init(void);

/* Shut down the transport layer and release resources. */
void fc_transport_done(void);

/* Register an alternative transport backend. This replaces the
 * current backend. Should only be called during startup, before
 * any connections are established. */
void fc_transport_set_backend(const struct fc_transport_ops *ops);

/* Get a pointer to the current transport operations vtable. */
const struct fc_transport_ops *fc_transport_get_ops(void);

/* Get the name of the current backend. */
const char *fc_transport_backend_name(void);

/*--- Convenience wrappers that dispatch through the current backend ---*/

int fc_transport_listen(fc_transport_handle *out,
                        const char *bind_addr, int port,
                        int backlog);
int fc_transport_accept(fc_transport_handle listen_h,
                        fc_transport_handle *out,
                        char *dst_host, int dst_host_len);
int fc_transport_connect(fc_transport_handle *out,
                         const char *host, int port);
void fc_transport_close(fc_transport_handle h);
int fc_transport_read(fc_transport_handle h, void *buf, size_t len);
int fc_transport_write(fc_transport_handle h, const void *buf, size_t len);
int fc_transport_poll(struct fc_transport_poll_set *set, int timeout_ms);
void fc_transport_set_nonblock(fc_transport_handle h);

#ifdef __cplusplus
}
#endif

#endif /* FC__TRANSPORT_H */

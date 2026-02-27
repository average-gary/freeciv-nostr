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

#ifdef FREECIV_NOSTR
#ifndef FC_IROH_TRANSPORT_H
#define FC_IROH_TRANSPORT_H

#ifdef __cplusplus
extern "C" {
#endif

#include "transport.h"

/***********************************************************************
  Iroh QUIC transport backend for the fc_transport_ops vtable.

  This backend replaces TCP sockets with Iroh QUIC streams for P2P
  networking. It bridges to Rust FFI functions that manage the actual
  QUIC endpoint and stream state.

  Usage:
    1. Call fc_iroh_transport_init() during startup (after fc_transport_init()).
       This registers the Iroh backend as the active transport.
    2. All subsequent fc_transport_*() calls are routed through Iroh.

  The backend delegates to the following Rust FFI functions (linked from
  the freeciv-nostr-ffi static library):
    fcn_transport_new()
    fcn_transport_setup_listener()
    fcn_transport_accept()
    fcn_transport_close_handle()
    fcn_transport_read()
    fcn_transport_write()
    fcn_transport_poll_handles()
    fcn_transport_stream_count()
    fcn_transport_free()
***********************************************************************/

/**********************************************************************//**
  Initialize the Iroh QUIC transport backend.

  Creates the Rust-side transport state and registers the Iroh ops
  table as the active transport backend via fc_transport_set_backend().

  Must be called after fc_transport_init() and before any connections
  are established.
**************************************************************************/
void fc_iroh_transport_init(void);

/**********************************************************************//**
  Get a pointer to the Iroh transport operations vtable.

  Returns a static pointer; the caller must NOT free it.
  Can be used to check the backend without switching to it.
**************************************************************************/
const struct fc_transport_ops *fc_iroh_transport_ops(void);

#ifdef __cplusplus
}
#endif

#endif /* FC_IROH_TRANSPORT_H */
#endif /* FREECIV_NOSTR */

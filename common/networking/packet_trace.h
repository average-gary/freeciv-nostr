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

#ifndef FC__PACKET_TRACE_H
#define FC__PACKET_TRACE_H

#ifdef __cplusplus
extern "C" {
#endif

/* common */
#include "packets.h"

/* Packet trace recording for testing and debugging.
 *
 * Captures all packets sent/received to a binary trace file for
 * later analysis. Controlled via FREECIV_PACKET_TRACE_DIR env var.
 * Zero-cost when tracing is not active.
 */

/* Binary trace file header (written once at start of file) */
#define PACKET_TRACE_MAGIC  0x46435054  /* "FCPT" */
#define PACKET_TRACE_VERSION 1

/* Direction constants for trace records */
#define PACKET_TRACE_DIR_SEND 0
#define PACKET_TRACE_DIR_RECV 1

/* Initialize packet tracing. If trace_dir is NULL, checks the
 * FREECIV_PACKET_TRACE_DIR environment variable. If neither is set,
 * tracing remains inactive (zero-cost). */
void packet_trace_init(const char *trace_dir);

/* Finalize and close trace files, print summary stats */
void packet_trace_done(void);

/* Record a packet being sent */
void packet_trace_record_send(enum packet_type type,
                               const unsigned char *data,
                               int len,
                               int connection_id);

/* Record a packet being received */
void packet_trace_record_recv(enum packet_type type,
                               const unsigned char *data,
                               int len,
                               int connection_id);

/* Check if tracing is active */
bool packet_trace_is_active(void);

/* Get count of packets traced */
int packet_trace_get_count(void);

#ifdef __cplusplus
}
#endif

#endif /* FC__PACKET_TRACE_H */

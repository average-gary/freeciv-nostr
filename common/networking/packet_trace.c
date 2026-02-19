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

#ifdef HAVE_CONFIG_H
#include <fc_config.h>
#endif

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef HAVE_SYS_TIME_H
#include <sys/time.h>
#endif

/* utility */
#include "log.h"
#include "mem.h"
#include "support.h"

#include "packet_trace.h"

/*
 * Binary trace file format:
 *
 * File header:
 *   uint32  magic    (0x46435054 = "FCPT")
 *   uint32  version  (1)
 *
 * Per-packet record:
 *   uint16  packet_type
 *   uint32  data_len
 *   uint32  connection_id
 *   uint8   direction (0 = send, 1 = recv)
 *   uint64  timestamp_usec (microseconds since epoch)
 *   <data_len bytes of raw packet data>
 */

/* Per-packet record header size: 2 + 4 + 4 + 1 + 8 = 19 bytes */
#define TRACE_RECORD_HEADER_SIZE 19

/* Trace state - file-scoped globals */
static FILE *trace_file = NULL;
static bool trace_active = FALSE;
static int trace_packet_count = 0;
static long trace_total_bytes = 0;

/* Per-type counters for summary stats */
static int trace_type_count[PACKET_LAST];
static long trace_type_bytes[PACKET_LAST];

/**********************************************************************//**
  Get current time in microseconds since epoch.
  Falls back to seconds precision if gettimeofday is not available.
**************************************************************************/
static uint64_t get_timestamp_usec(void)
{
#ifdef HAVE_GETTIMEOFDAY
  struct timeval tv;

  gettimeofday(&tv, NULL);
  return (uint64_t)tv.tv_sec * 1000000ULL + (uint64_t)tv.tv_usec;
#else
  return (uint64_t)time(NULL) * 1000000ULL;
#endif
}

/**********************************************************************//**
  Disable tracing due to a write error. Logs the error and closes
  the trace file so no further writes are attempted.
**************************************************************************/
static void trace_disable_on_error(void)
{
  log_error("packet_trace: write failed, disabling trace");
  trace_active = FALSE;
  if (trace_file != NULL) {
    fclose(trace_file);
    trace_file = NULL;
  }
}

/**********************************************************************//**
  Write a uint16 to the trace file in little-endian byte order.
  Returns TRUE on success, FALSE on write failure.
**************************************************************************/
static bool write_uint16(FILE *fp, uint16_t val)
{
  unsigned char buf[2];

  buf[0] = (unsigned char)(val & 0xFF);
  buf[1] = (unsigned char)((val >> 8) & 0xFF);
  if (fwrite(buf, 1, 2, fp) != 2) {
    return FALSE;
  }
  return TRUE;
}

/**********************************************************************//**
  Write a uint32 to the trace file in little-endian byte order.
  Returns TRUE on success, FALSE on write failure.
**************************************************************************/
static bool write_uint32(FILE *fp, uint32_t val)
{
  unsigned char buf[4];

  buf[0] = (unsigned char)(val & 0xFF);
  buf[1] = (unsigned char)((val >> 8) & 0xFF);
  buf[2] = (unsigned char)((val >> 16) & 0xFF);
  buf[3] = (unsigned char)((val >> 24) & 0xFF);
  if (fwrite(buf, 1, 4, fp) != 4) {
    return FALSE;
  }
  return TRUE;
}

/**********************************************************************//**
  Write a uint64 to the trace file in little-endian byte order.
  Returns TRUE on success, FALSE on write failure.
**************************************************************************/
static bool write_uint64(FILE *fp, uint64_t val)
{
  unsigned char buf[8];
  int i;

  for (i = 0; i < 8; i++) {
    buf[i] = (unsigned char)((val >> (i * 8)) & 0xFF);
  }
  if (fwrite(buf, 1, 8, fp) != 8) {
    return FALSE;
  }
  return TRUE;
}

/**********************************************************************//**
  Write the trace file header.
**************************************************************************/
static void write_file_header(FILE *fp)
{
  write_uint32(fp, PACKET_TRACE_MAGIC);
  write_uint32(fp, PACKET_TRACE_VERSION);
}

/**********************************************************************//**
  Initialize packet tracing. If trace_dir is NULL, checks the
  FREECIV_PACKET_TRACE_DIR environment variable. Tracing remains
  inactive if neither is set (zero-cost).
**************************************************************************/
void packet_trace_init(const char *trace_dir)
{
  const char *dir = trace_dir;
  char filepath[1024];

  if (trace_active) {
    /* Already initialized */
    return;
  }

  if (dir == NULL) {
    dir = getenv("FREECIV_PACKET_TRACE_DIR");
  }

  if (dir == NULL || dir[0] == '\0') {
    /* No trace directory specified, tracing stays inactive */
    return;
  }

  fc_snprintf(filepath, sizeof(filepath), "%s/packet_trace.bin", dir);

  trace_file = fc_fopen(filepath, "wb");
  if (trace_file == NULL) {
    log_error("packet_trace: failed to open trace file '%s'", filepath);
    return;
  }

  /* Initialize counters */
  trace_packet_count = 0;
  trace_total_bytes = 0;
  memset(trace_type_count, 0, sizeof(trace_type_count));
  memset(trace_type_bytes, 0, sizeof(trace_type_bytes));

  /* Write file header */
  write_file_header(trace_file);

  trace_active = TRUE;

  log_normal("packet_trace: tracing enabled, writing to '%s'", filepath);
}

/**********************************************************************//**
  Finalize and close trace files. Print summary statistics including
  total packets, bytes, and per-type breakdown.
**************************************************************************/
void packet_trace_done(void)
{
  int i;
  int types_seen = 0;

  if (!trace_active) {
    return;
  }

  if (trace_file != NULL) {
    fflush(trace_file);
    fclose(trace_file);
    trace_file = NULL;
  }

  trace_active = FALSE;

  /* Print summary */
  log_normal("packet_trace: === Trace Summary ===");
  log_normal("packet_trace: total packets: %d", trace_packet_count);
  log_normal("packet_trace: total data bytes: %ld", trace_total_bytes);

  for (i = 0; i < PACKET_LAST; i++) {
    if (trace_type_count[i] > 0) {
      types_seen++;
      log_verbose("packet_trace:   type %3d (%-30s): %6d packets, %8ld bytes",
                  i, packet_name(i),
                  trace_type_count[i], trace_type_bytes[i]);
    }
  }

  log_normal("packet_trace: packet types seen: %d / %d",
             types_seen, PACKET_LAST);
}

/**********************************************************************//**
  Record a single packet (internal helper for both send and recv).
  Writes the binary record header followed by raw packet data.
  Note: single-threaded, no mutex needed for the typical server case.
**************************************************************************/
static void packet_trace_record(enum packet_type type,
                                const unsigned char *data,
                                int len,
                                int connection_id,
                                int direction)
{
  uint64_t timestamp;

  if (!trace_active || trace_file == NULL) {
    return;
  }

  if (type < 0 || type >= PACKET_LAST) {
    return;
  }

  timestamp = get_timestamp_usec();

  /* Write record header, disabling tracing on any write failure */
  if (!write_uint16(trace_file, (uint16_t)type)
      || !write_uint32(trace_file, (uint32_t)len)
      || !write_uint32(trace_file, (uint32_t)connection_id)) {
    trace_disable_on_error();
    return;
  }
  if (fputc((unsigned char)direction, trace_file) == EOF) {
    trace_disable_on_error();
    return;
  }
  if (!write_uint64(trace_file, timestamp)) {
    trace_disable_on_error();
    return;
  }

  /* Write raw packet data */
  if (data != NULL && len > 0) {
    if (fwrite(data, 1, len, trace_file) != (size_t)len) {
      trace_disable_on_error();
      return;
    }
  }

  /* Update counters */
  trace_packet_count++;
  trace_total_bytes += len;
  trace_type_count[type]++;
  trace_type_bytes[type] += len;

  /* Periodic flush every 1024 packets to limit data loss on crash */
  if ((trace_packet_count & 0x3FF) == 0) {
    fflush(trace_file);
  }
}

/**********************************************************************//**
  Record a packet being sent.
**************************************************************************/
void packet_trace_record_send(enum packet_type type,
                               const unsigned char *data,
                               int len,
                               int connection_id)
{
  packet_trace_record(type, data, len, connection_id,
                      PACKET_TRACE_DIR_SEND);
}

/**********************************************************************//**
  Record a packet being received.
**************************************************************************/
void packet_trace_record_recv(enum packet_type type,
                               const unsigned char *data,
                               int len,
                               int connection_id)
{
  packet_trace_record(type, data, len, connection_id,
                      PACKET_TRACE_DIR_RECV);
}

/**********************************************************************//**
  Check if tracing is currently active.
**************************************************************************/
bool packet_trace_is_active(void)
{
  return trace_active;
}

/**********************************************************************//**
  Get total count of packets traced so far.
**************************************************************************/
int packet_trace_get_count(void)
{
  return trace_packet_count;
}

#!/usr/bin/env python3
"""
Packet trace statistics analyzer for Freeciv.

Reads a binary packet trace file produced by the packet_trace facility
and prints statistics about packet types, directions, and coverage.

Usage:
    python3 packet_stats.py <trace_file> [packets.def]

The trace file uses the following binary format:

File header:
    uint32  magic    (0x46435054 = "FCPT")
    uint32  version  (1)

Per-packet record:
    uint16  packet_type
    uint32  data_len
    uint32  connection_id
    uint8   direction (0 = send, 1 = recv)
    uint64  timestamp_usec
    <data_len bytes of raw data>
"""

import struct
import sys
import os
import re
from collections import defaultdict

TRACE_MAGIC = 0x46435054  # "FCPT"
TRACE_VERSION = 1

# Record header field order (matching C write order in packet_trace.c):
#   uint16  packet_type      (H)
#   uint32  data_len         (I)
#   uint32  connection_id    (I)
#   uint8   direction        (B)
#   uint64  timestamp_usec   (Q)
# Total: 2 + 4 + 4 + 1 + 8 = 19 bytes
RECORD_HEADER_FORMAT = '<HIIBQ'
RECORD_HEADER_SIZE = 19


def parse_packets_def(filepath):
    """Parse packets.def to extract packet type names and numbers.

    Looks for lines matching: PACKET_<NAME> = <number>;
    Returns a dict mapping packet number -> packet name.
    """
    packet_names = {}

    if not os.path.exists(filepath):
        print(f"Warning: packets.def not found at {filepath}", file=sys.stderr)
        return packet_names

    with open(filepath, 'r') as f:
        for line in f:
            line = line.strip()
            # Match: PACKET_FOO = 123; ...
            m = re.match(r'(PACKET_\w+)\s*=\s*(\d+)\s*;', line)
            if m:
                name = m.group(1)
                num = int(m.group(2))
                packet_names[num] = name

    return packet_names


def read_uint16_le(data, offset):
    """Read a little-endian uint16 from bytes."""
    return struct.unpack_from('<H', data, offset)[0]


def read_uint32_le(data, offset):
    """Read a little-endian uint32 from bytes."""
    return struct.unpack_from('<I', data, offset)[0]


def read_uint64_le(data, offset):
    """Read a little-endian uint64 from bytes."""
    return struct.unpack_from('<Q', data, offset)[0]


def read_uint8(data, offset):
    """Read a uint8 from bytes."""
    return data[offset]


def analyze_trace(trace_path, packet_names=None):
    """Read and analyze a binary trace file."""
    if packet_names is None:
        packet_names = {}

    if not os.path.exists(trace_path):
        print(f"Error: trace file not found: {trace_path}", file=sys.stderr)
        sys.exit(1)

    file_size = os.path.getsize(trace_path)
    if file_size < 8:
        print(f"Error: trace file too small ({file_size} bytes)", file=sys.stderr)
        sys.exit(1)

    with open(trace_path, 'rb') as f:
        data = f.read()

    # Verify file header
    magic = read_uint32_le(data, 0)
    version = read_uint32_le(data, 4)

    if magic != TRACE_MAGIC:
        print(f"Error: invalid magic number 0x{magic:08X} (expected 0x{TRACE_MAGIC:08X})",
              file=sys.stderr)
        sys.exit(1)

    if version != TRACE_VERSION:
        print(f"Warning: trace version {version} (expected {TRACE_VERSION})",
              file=sys.stderr)

    # Parse records
    offset = 8  # Skip file header
    total_packets = 0
    total_data_bytes = 0
    send_count = 0
    recv_count = 0

    type_counts = defaultdict(int)
    type_bytes = defaultdict(int)
    type_send = defaultdict(int)
    type_recv = defaultdict(int)
    conn_counts = defaultdict(int)

    first_timestamp = None
    last_timestamp = None

    while offset + RECORD_HEADER_SIZE <= len(data):
        pkt_type = read_uint16_le(data, offset)
        data_len = read_uint32_le(data, offset + 2)
        conn_id = read_uint32_le(data, offset + 6)
        direction = read_uint8(data, offset + 10)
        timestamp = read_uint64_le(data, offset + 11)

        offset += RECORD_HEADER_SIZE

        # Validate we have enough data
        if offset + data_len > len(data):
            print(f"Warning: truncated record at offset {offset - RECORD_HEADER_SIZE}, "
                  f"expected {data_len} bytes but only {len(data) - offset} remain",
                  file=sys.stderr)
            break

        offset += data_len

        total_packets += 1
        total_data_bytes += data_len

        if direction == 0:
            send_count += 1
            type_send[pkt_type] += 1
        else:
            recv_count += 1
            type_recv[pkt_type] += 1

        type_counts[pkt_type] += 1
        type_bytes[pkt_type] += data_len
        conn_counts[conn_id] += 1

        if first_timestamp is None:
            first_timestamp = timestamp
        last_timestamp = timestamp

    # Print results
    print("=" * 78)
    print("FREECIV PACKET TRACE ANALYSIS")
    print("=" * 78)
    print(f"Trace file: {trace_path}")
    print(f"File size:  {file_size:,} bytes")
    print()

    print(f"Total packets:     {total_packets:>10,}")
    print(f"Total data bytes:  {total_data_bytes:>10,}")
    print(f"  Sent:            {send_count:>10,}")
    print(f"  Received:        {recv_count:>10,}")
    print()

    if first_timestamp is not None and last_timestamp is not None:
        duration_sec = (last_timestamp - first_timestamp) / 1_000_000.0
        print(f"Trace duration:    {duration_sec:>10.2f} seconds")
        if duration_sec > 0:
            print(f"Packets/second:    {total_packets / duration_sec:>10.1f}")
        print()

    print(f"Unique connections: {len(conn_counts)}")
    for conn_id in sorted(conn_counts.keys()):
        print(f"  Connection {conn_id}: {conn_counts[conn_id]:,} packets")
    print()

    # Per-type breakdown
    print("-" * 78)
    print(f"{'Type':>5}  {'Name':<40} {'Count':>8} {'Bytes':>10} {'Avg':>6} {'Send':>6} {'Recv':>6}")
    print("-" * 78)

    for pkt_type in sorted(type_counts.keys()):
        name = packet_names.get(pkt_type, f"UNKNOWN_{pkt_type}")
        count = type_counts[pkt_type]
        bytes_total = type_bytes[pkt_type]
        avg = bytes_total // count if count > 0 else 0
        s = type_send.get(pkt_type, 0)
        r = type_recv.get(pkt_type, 0)
        print(f"{pkt_type:>5}  {name:<40} {count:>8,} {bytes_total:>10,} {avg:>6} {s:>6} {r:>6}")

    print("-" * 78)
    print(f"{'':>5}  {'TOTAL':<40} {total_packets:>8,} {total_data_bytes:>10,}")
    print()

    # Coverage analysis
    if packet_names:
        all_defined = set(packet_names.keys())
        seen = set(type_counts.keys())
        not_seen = all_defined - seen
        unknown_seen = seen - all_defined

        print("=" * 78)
        print("PACKET TYPE COVERAGE")
        print("=" * 78)
        print(f"Defined packet types:  {len(all_defined)}")
        print(f"Types seen in trace:   {len(seen & all_defined)}")
        print(f"Types NOT seen:        {len(not_seen)}")
        if unknown_seen:
            print(f"Unknown types seen:    {len(unknown_seen)}")
        print(f"Coverage:              {100.0 * len(seen & all_defined) / len(all_defined):.1f}%")
        print()

        if not_seen:
            print("Packet types NOT seen in trace:")
            for pkt_type in sorted(not_seen):
                print(f"  {pkt_type:>5}  {packet_names[pkt_type]}")
            print()

    print("=" * 78)


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <trace_file> [packets.def]", file=sys.stderr)
        sys.exit(1)

    trace_path = sys.argv[1]
    packets_def_path = sys.argv[2] if len(sys.argv) > 2 else None

    packet_names = {}
    if packets_def_path:
        packet_names = parse_packets_def(packets_def_path)
        print(f"Loaded {len(packet_names)} packet type definitions from {packets_def_path}")

    analyze_trace(trace_path, packet_names)


if __name__ == '__main__':
    main()

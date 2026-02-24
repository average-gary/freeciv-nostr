#!/usr/bin/env python3
"""
Determinism verification tool for Freeciv.

Compares two binary packet trace files produced by the packet_trace facility
and reports whether the game engine produced identical packet sequences.

Timestamps and connection IDs are ignored during comparison since they are
expected to differ between runs. The comparison focuses on:
  - Packet type sequence
  - Packet direction sequence
  - Packet payload data (byte-for-byte)

Usage:
    python3 compare_traces.py <trace_a> <trace_b> [packets.def]

Exit codes:
    0  Traces are deterministically identical
    1  Traces differ (non-determinism detected)
    2  Error (bad file, wrong args, etc.)
"""

import struct
import sys
import os
import re
from collections import defaultdict

TRACE_MAGIC = 0x46435054  # "FCPT"
TRACE_VERSION = 1

# Record header layout (on-disk, packed with no padding):
#   uint16 type, uint32 data_len, uint32 conn_id, uint8 dir, uint64 timestamp
#   Total: 2 + 4 + 4 + 1 + 8 = 19 bytes
#
# NOTE: RECORD_HEADER_FORMAT is for documentation only. Actual parsing uses
# individual struct.unpack_from() calls at fixed offsets to avoid Python
# struct padding (struct.calcsize('<HIIBQ') == 24 due to alignment).
RECORD_HEADER_FORMAT = '<HIIBQ'  # documentation only
RECORD_HEADER_SIZE = 19  # actual on-disk size (no padding)


def parse_packets_def(filepath):
    """Parse packets.def to extract packet type names and numbers."""
    packet_names = {}
    if not os.path.exists(filepath):
        return packet_names
    with open(filepath, 'r') as f:
        for line in f:
            m = re.match(r'(PACKET_\w+)\s*=\s*(\d+)\s*;', line.strip())
            if m:
                packet_names[int(m.group(2))] = m.group(1)
    return packet_names


class TraceRecord:
    """A single packet trace record (excluding timestamp and connection_id)."""
    __slots__ = ('packet_type', 'direction', 'data_len', 'data')

    def __init__(self, packet_type, direction, data_len, data):
        self.packet_type = packet_type
        self.direction = direction
        self.data_len = data_len
        self.data = data

    def __eq__(self, other):
        return (self.packet_type == other.packet_type
                and self.direction == other.direction
                and self.data_len == other.data_len
                and self.data == other.data)


def read_trace(filepath):
    """Read a trace file and return a list of TraceRecords.

    Strips timestamps and connection IDs since those are expected to vary.
    """
    if not os.path.exists(filepath):
        print(f"Error: trace file not found: {filepath}", file=sys.stderr)
        sys.exit(2)

    file_size = os.path.getsize(filepath)
    if file_size < 8:
        print(f"Error: trace file too small ({file_size} bytes): {filepath}",
              file=sys.stderr)
        sys.exit(2)

    with open(filepath, 'rb') as f:
        data = f.read()

    # Verify header
    magic = struct.unpack_from('<I', data, 0)[0]
    version = struct.unpack_from('<I', data, 4)[0]

    if magic != TRACE_MAGIC:
        print(f"Error: invalid magic 0x{magic:08X} in {filepath}", file=sys.stderr)
        sys.exit(2)

    if version != TRACE_VERSION:
        print(f"Warning: trace version {version} (expected {TRACE_VERSION}) in {filepath}",
              file=sys.stderr)

    records = []
    offset = 8

    while offset + RECORD_HEADER_SIZE <= len(data):
        pkt_type = struct.unpack_from('<H', data, offset)[0]
        data_len = struct.unpack_from('<I', data, offset + 2)[0]
        # conn_id at offset+6 (skipped for comparison)
        direction = data[offset + 10]
        # timestamp at offset+11 (skipped for comparison)

        offset += RECORD_HEADER_SIZE

        if offset + data_len > len(data):
            print(f"Warning: truncated record at offset {offset - RECORD_HEADER_SIZE} "
                  f"in {filepath}", file=sys.stderr)
            break

        pkt_data = data[offset:offset + data_len]
        offset += data_len

        records.append(TraceRecord(pkt_type, direction, data_len, pkt_data))

    return records


class TraceDiff:
    """Holds information about a single difference between two traces."""

    def __init__(self, index, kind, detail_a=None, detail_b=None):
        self.index = index
        self.kind = kind
        self.detail_a = detail_a
        self.detail_b = detail_b

    def __str__(self):
        if self.kind == 'type_mismatch':
            return (f"  Packet #{self.index}: type differs: "
                    f"A={self.detail_a} vs B={self.detail_b}")
        elif self.kind == 'direction_mismatch':
            dirs = {0: 'send', 1: 'recv'}
            return (f"  Packet #{self.index}: direction differs: "
                    f"A={dirs.get(self.detail_a, self.detail_a)} vs "
                    f"B={dirs.get(self.detail_b, self.detail_b)}")
        elif self.kind == 'length_mismatch':
            return (f"  Packet #{self.index}: data length differs: "
                    f"A={self.detail_a} vs B={self.detail_b}")
        elif self.kind == 'data_mismatch':
            return (f"  Packet #{self.index}: payload data differs "
                    f"(first diff at byte {self.detail_a})")
        elif self.kind == 'count_mismatch':
            return (f"  Trace length differs: A={self.detail_a} packets vs "
                    f"B={self.detail_b} packets")
        else:
            return f"  Packet #{self.index}: {self.kind}"


def find_first_data_diff(data_a, data_b):
    """Find the byte offset of the first difference between two byte sequences."""
    min_len = min(len(data_a), len(data_b))
    for i in range(min_len):
        if data_a[i] != data_b[i]:
            return i
    if len(data_a) != len(data_b):
        return min_len
    return -1


def compare_records(records_a, records_b, max_diffs=50):
    """Compare two lists of TraceRecords, returning a list of TraceDiffs.

    Compares packet-by-packet, ignoring timestamps and connection IDs.
    Stops after max_diffs differences to avoid flooding output.
    """
    diffs = []

    if len(records_a) != len(records_b):
        diffs.append(TraceDiff(-1, 'count_mismatch',
                               len(records_a), len(records_b)))

    min_len = min(len(records_a), len(records_b))

    for i in range(min_len):
        if len(diffs) >= max_diffs:
            break

        a = records_a[i]
        b = records_b[i]

        if a.packet_type != b.packet_type:
            diffs.append(TraceDiff(i, 'type_mismatch',
                                   a.packet_type, b.packet_type))
            continue  # No point comparing data if type differs

        if a.direction != b.direction:
            diffs.append(TraceDiff(i, 'direction_mismatch',
                                   a.direction, b.direction))

        if a.data_len != b.data_len:
            diffs.append(TraceDiff(i, 'length_mismatch',
                                   a.data_len, b.data_len))
        elif a.data != b.data:
            diff_offset = find_first_data_diff(a.data, b.data)
            diffs.append(TraceDiff(i, 'data_mismatch', diff_offset))

    return diffs


def compute_summary(records, packet_names):
    """Compute a quick summary of a trace for the report."""
    type_counts = defaultdict(int)
    send_count = 0
    recv_count = 0

    for r in records:
        type_counts[r.packet_type] += 1
        if r.direction == 0:
            send_count += 1
        else:
            recv_count += 1

    return {
        'total': len(records),
        'send': send_count,
        'recv': recv_count,
        'unique_types': len(type_counts),
        'type_counts': dict(type_counts),
    }


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <trace_a> <trace_b> [packets.def]",
              file=sys.stderr)
        sys.exit(2)

    trace_a_path = sys.argv[1]
    trace_b_path = sys.argv[2]
    packets_def_path = sys.argv[3] if len(sys.argv) > 3 else None

    packet_names = {}
    if packets_def_path:
        packet_names = parse_packets_def(packets_def_path)

    print("=" * 78)
    print("FREECIV DETERMINISM TRACE COMPARISON")
    print("=" * 78)
    print(f"Trace A: {trace_a_path}")
    print(f"Trace B: {trace_b_path}")
    print()

    # Read both traces
    print("Reading traces...")
    records_a = read_trace(trace_a_path)
    records_b = read_trace(trace_b_path)

    # Summaries
    summary_a = compute_summary(records_a, packet_names)
    summary_b = compute_summary(records_b, packet_names)

    print(f"Trace A: {summary_a['total']:,} packets "
          f"({summary_a['send']:,} sent, {summary_a['recv']:,} recv, "
          f"{summary_a['unique_types']} types)")
    print(f"Trace B: {summary_b['total']:,} packets "
          f"({summary_b['send']:,} sent, {summary_b['recv']:,} recv, "
          f"{summary_b['unique_types']} types)")
    print()

    # Compare
    print("Comparing packet sequences (ignoring timestamps, connection IDs)...")
    diffs = compare_records(records_a, records_b)

    if not diffs:
        print()
        print("RESULT: DETERMINISTIC")
        print(f"  All {len(records_a):,} packets match byte-for-byte.")
        print("=" * 78)
        return 0
    else:
        print()
        print(f"RESULT: NON-DETERMINISTIC ({len(diffs)} difference(s) found)")
        print()

        for diff in diffs:
            line = str(diff)
            # Enrich with packet type name if available
            if diff.kind == 'type_mismatch' and packet_names:
                name_a = packet_names.get(diff.detail_a, f"type_{diff.detail_a}")
                name_b = packet_names.get(diff.detail_b, f"type_{diff.detail_b}")
                line += f"  ({name_a} vs {name_b})"
            elif diff.kind in ('data_mismatch', 'length_mismatch', 'direction_mismatch'):
                if diff.index >= 0:
                    rec = records_a[diff.index]
                    name = packet_names.get(rec.packet_type,
                                            f"type_{rec.packet_type}")
                    line += f"  [packet type: {name}]"
            print(line)

        # Per-type divergence summary
        print()
        print("-" * 78)
        print("Divergence by packet type:")
        type_diffs = defaultdict(int)
        for diff in diffs:
            if diff.index >= 0 and diff.index < len(records_a):
                type_diffs[records_a[diff.index].packet_type] += 1
        for pkt_type in sorted(type_diffs.keys()):
            name = packet_names.get(pkt_type, f"type_{pkt_type}")
            print(f"  {pkt_type:>5}  {name:<40} {type_diffs[pkt_type]:>4} diff(s)")

        print("=" * 78)
        return 1


if __name__ == '__main__':
    sys.exit(main())

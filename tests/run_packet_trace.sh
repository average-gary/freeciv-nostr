#!/bin/bash
# Run a Freeciv autogame with packet tracing enabled.
#
# Captures all packets to a binary trace file, then analyzes
# the results using packet_stats.py.
#
# Usage: ./tests/run_packet_trace.sh <server-binary> [trace-dir]
#
# Example:
#   ./tests/run_packet_trace.sh ./build/freeciv-server ./packet_traces

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

SERVER="$1"
TRACE_DIR="${2:-./packet_traces}"

if [ -z "$SERVER" ]; then
    echo "Usage: $0 <server-binary> [trace-dir]"
    echo ""
    echo "Arguments:"
    echo "  server-binary   Path to the freeciv-server executable"
    echo "  trace-dir       Directory for trace output (default: ./packet_traces)"
    exit 1
fi

if [ ! -x "$SERVER" ]; then
    echo "Error: server binary not found or not executable: $SERVER"
    exit 1
fi

mkdir -p "$TRACE_DIR"

# Set env var to enable tracing (checked by packet_trace_init)
export FREECIV_PACKET_TRACE_DIR="$TRACE_DIR"

echo "============================================"
echo "Freeciv Packet Trace Runner"
echo "============================================"
echo "Server:      $SERVER"
echo "Trace dir:   $TRACE_DIR"
echo "Autogame:    scripts/test-autogame.serv"
echo ""

echo "Starting autogame with packet tracing..."
"$SERVER" --Announce none -e -F --read "$PROJECT_DIR/scripts/test-autogame.serv" || {
    echo "Warning: server exited with non-zero status (may be normal for autogame)"
}

echo ""
echo "Autogame complete."

TRACE_FILE="$TRACE_DIR/packet_trace.bin"
PACKETS_DEF="$PROJECT_DIR/common/networking/packets.def"

if [ ! -f "$TRACE_FILE" ]; then
    echo "Error: trace file not found: $TRACE_FILE"
    echo "Tracing may not have been compiled in or no packets were sent."
    exit 1
fi

echo "Analyzing packet trace..."
echo ""

python3 "$SCRIPT_DIR/packet_stats.py" "$TRACE_FILE" "$PACKETS_DEF"

echo ""
echo "Trace file available at: $TRACE_FILE"
echo "Done."

#!/bin/bash
# Run two identical autogame sessions and compare packet traces
# to verify deterministic game engine behavior.
#
# Usage: ./tests/run_determinism_check.sh <server-binary> [trace-dir]
#
# Example:
#   ./tests/run_determinism_check.sh ./build/freeciv-server ./det_traces
#
# Exit codes:
#   0  Game engine is deterministic (traces match)
#   1  Non-determinism detected (traces differ)
#   2  Error (server failed to run, traces missing, etc.)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

SERVER="$1"
TRACE_DIR="${2:-./det_traces}"

if [ -z "${SERVER:-}" ]; then
    echo "Usage: $0 <server-binary> [trace-dir]"
    echo ""
    echo "Runs two identical autogames with fixed seeds and compares"
    echo "packet traces to verify determinism."
    exit 2
fi

if [ ! -x "$SERVER" ]; then
    echo "Error: server binary not found or not executable: $SERVER"
    exit 2
fi

TRACE_DIR_A="$TRACE_DIR/run_a"
TRACE_DIR_B="$TRACE_DIR/run_b"

echo "============================================"
echo "Freeciv Determinism Verification"
echo "============================================"
echo "Server:      $SERVER"
echo "Trace dir:   $TRACE_DIR"
echo "Autogame:    scripts/test-autogame.serv"
echo ""

# Clean previous traces
rm -rf "$TRACE_DIR_A" "$TRACE_DIR_B"
mkdir -p "$TRACE_DIR_A" "$TRACE_DIR_B"

# ---- Run A ----
echo "=== Run A ==="
export FREECIV_PACKET_TRACE_DIR="$TRACE_DIR_A"
"$SERVER" --Announce none -e -F --read "$PROJECT_DIR/scripts/test-autogame.serv" || {
    echo "Warning: server exited with non-zero status on Run A (may be normal for autogame)"
}

if [ ! -f "$TRACE_DIR_A/packet_trace.bin" ]; then
    echo "Error: Run A did not produce a trace file"
    exit 2
fi
echo "Run A complete: $(wc -c < "$TRACE_DIR_A/packet_trace.bin") bytes"
echo ""

# ---- Run B ----
echo "=== Run B ==="
export FREECIV_PACKET_TRACE_DIR="$TRACE_DIR_B"
"$SERVER" --Announce none -e -F --read "$PROJECT_DIR/scripts/test-autogame.serv" || {
    echo "Warning: server exited with non-zero status on Run B (may be normal for autogame)"
}

if [ ! -f "$TRACE_DIR_B/packet_trace.bin" ]; then
    echo "Error: Run B did not produce a trace file"
    exit 2
fi
echo "Run B complete: $(wc -c < "$TRACE_DIR_B/packet_trace.bin") bytes"
echo ""

# ---- Compare ----
echo "=== Comparing Traces ==="
PACKETS_DEF="$PROJECT_DIR/common/networking/packets.def"

COMPARE_ARGS=("$TRACE_DIR_A/packet_trace.bin" "$TRACE_DIR_B/packet_trace.bin")
if [ -f "$PACKETS_DEF" ]; then
    COMPARE_ARGS+=("$PACKETS_DEF")
fi

python3 "$SCRIPT_DIR/compare_traces.py" "${COMPARE_ARGS[@]}"
exit $?

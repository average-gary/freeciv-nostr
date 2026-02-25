#!/bin/bash
# Run a Freeciv server with packet tracing and a minimal dummy client
# that connects to generate network traffic.
#
# Usage: ./tests/run_packet_trace_with_client.sh <server-binary> [trace-dir]

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

SERVER="$1"
TRACE_DIR="${2:-./packet_traces}"
PORT=5556

if [ -z "$SERVER" ]; then
    echo "Usage: $0 <server-binary> [trace-dir]"
    exit 1
fi

if [ ! -x "$SERVER" ]; then
    echo "Error: server binary not found or not executable: $SERVER"
    exit 1
fi

mkdir -p "$TRACE_DIR"
export FREECIV_PACKET_TRACE_DIR="$TRACE_DIR"

# Remove old trace
rm -f "$TRACE_DIR/packet_trace.bin"

echo "============================================"
echo "Freeciv Packet Trace (with dummy client)"
echo "============================================"
echo "Server:      $SERVER"
echo "Trace dir:   $TRACE_DIR"
echo "Port:        $PORT"
echo ""

# Create a server script that pre-configures but does NOT auto-start
# We'll start the game via a client-side command after connecting
cat > /tmp/fc_trace_test.serv <<'SERVEOF'
set aifill 2
set endt 10
set timeout -1
set minp 0
set gameseed 42
set mapseed 42
set size 1
SERVEOF

# Start server in background (no -e so it stays up; no auto-start)
echo "Starting server..."
FREECIV_DATA="$PROJECT_DIR/data" "$SERVER" \
    --Announce none -p "$PORT" -F \
    --read /tmp/fc_trace_test.serv \
    > /tmp/fc_trace_server.log 2>&1 &
SERVER_PID=$!

# Wait for server to start listening
echo "Waiting for server to start..."
for i in $(seq 1 20); do
    if python3 -c "
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
try:
    s.connect(('127.0.0.1', $PORT))
    s.close()
    sys.exit(0)
except:
    sys.exit(1)
" 2>/dev/null; then
        echo "  Server is listening (attempt $i)"
        break
    fi
    sleep 0.5
done

# Connect a dummy client that sends a join request, receives the reply,
# then sends /start to begin the game, and keeps receiving until done
echo "Connecting dummy client..."
python3 -c "
import socket, time, struct, sys

sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
try:
    sock.connect(('127.0.0.1', $PORT))
    sock.settimeout(10.0)

    # Build PACKET_SERVER_JOIN_REQ (type 4)
    # Fields: username, capability, version_label, major, minor, patch
    username = b'tracer\x00'
    capability = b'+Freeciv.Devel-3.4-2025.Sep.12\x00'
    version_label = b'trace-client\x00'
    major = struct.pack('>I', 3)
    minor = struct.pack('>I', 3)
    patch = struct.pack('>I', 90)

    payload = username + capability + version_label + major + minor + patch
    pkt_type = 4  # PACKET_SERVER_JOIN_REQ
    total_len = 2 + 1 + len(payload)
    header = struct.pack('>HB', total_len, pkt_type)

    sock.sendall(header + payload)
    print(f'  Sent JOIN_REQ ({total_len} bytes)')

    # Read responses until timeout or connection close
    # The server will send a join reply + connection info + settings etc.
    total_received = 0
    recv_count = 0
    start_time = time.time()
    
    # After join, send a chat message '/start' to kick off the game
    # PACKET_CHAT_MSG_REQ = 26
    # Fields: message (string)
    time.sleep(1)  # Let join handshake complete
    
    chat_msg = b'/start\x00'
    chat_len = 2 + 1 + len(chat_msg)
    chat_header = struct.pack('>HB', chat_len, 26)
    sock.sendall(chat_header + chat_msg)
    print('  Sent /start command')
    
    # Now keep reading until the game ends or we timeout
    sock.settimeout(2.0)
    try:
        while time.time() - start_time < 60:
            data = sock.recv(65536)
            if not data:
                print('  Server closed connection')
                break
            total_received += len(data)
            recv_count += 1
    except socket.timeout:
        pass

    elapsed = time.time() - start_time
    print(f'  Received {total_received} bytes in {recv_count} recv calls ({elapsed:.1f}s)')
except ConnectionRefusedError:
    print('  ERROR: Could not connect to server')
except Exception as e:
    print(f'  ERROR: {e}')
finally:
    sock.close()
" || true

echo "Waiting for server to finish..."

# Give the server a moment, then kill it
sleep 2
if kill -0 $SERVER_PID 2>/dev/null; then
    echo "Sending SIGTERM to server..."
    kill $SERVER_PID 2>/dev/null || true
    sleep 1
fi

wait $SERVER_PID 2>/dev/null || true

echo ""
echo "Server finished."

TRACE_FILE="$TRACE_DIR/packet_trace.bin"
PACKETS_DEF="$PROJECT_DIR/common/networking/packets.def"

if [ ! -f "$TRACE_FILE" ]; then
    echo "Error: trace file not found"
    exit 1
fi

TRACE_SIZE=$(wc -c < "$TRACE_FILE" | tr -d ' ')
echo "Trace file size: $TRACE_SIZE bytes"

if [ "$TRACE_SIZE" -le 8 ]; then
    echo "Warning: trace file contains only header (no packets captured)"
    echo "Server log (last 30 lines):"
    tail -30 /tmp/fc_trace_server.log
    exit 1
fi

echo ""
echo "Analyzing packet trace..."
python3 "$SCRIPT_DIR/packet_stats.py" "$TRACE_FILE" "$PACKETS_DEF"

echo ""
echo "Trace file: $TRACE_FILE"
echo "Server log: /tmp/fc_trace_server.log"
echo "Done."

#!/bin/zsh
set -euo pipefail

BIN="$HOME/NetworkAudioBridge/nab-live"
STATUS_FILE="$HOME/.nab/status.json"
SOCKET="$HOME/Library/Caches/KenichiNAB/nab-tap.sock"
LOCK="$SOCKET.lock"
RUNTIME_LOCK_DIR="$HOME/.nab/locks"
TOKEN_HEALTH_URL="https://livekit.kenichi-kawabata.com/healthz"

clear 2>/dev/null || true
echo "NAB Live Status"
echo "==============="
echo ""

sender_pids() {
  pgrep -f "$BIN" 2>/dev/null || true
}

read_status() {
  python3 - "$STATUS_FILE" "$1" <<'PY'
import json
import sys
import time
from pathlib import Path

path = Path(sys.argv[1])
label = sys.argv[2]
data = json.loads(path.read_text())
age = max(0.0, time.time() - (data.get("updated_at_unix_ms", 0) / 1000))
print(json.dumps({
    "label": label,
    "age": age,
    "connection": data.get("connection"),
    "source": data.get("source"),
    "source_kind": data.get("source_kind", "unknown"),
    "room": data.get("room"),
    "identity": data.get("identity"),
    "track": data.get("track"),
    "livekit_url": data.get("livekit_url"),
    "listen_url": data.get("listen_url"),
    "profile": data.get("profile"),
    "bitrate_bps": data.get("bitrate_bps", 0),
    "red_enabled": data.get("red_enabled"),
    "dtx_enabled": data.get("dtx_enabled"),
    "audio_state": data.get("audio_state", "unknown"),
    "last_audio_frame_age_ms": data.get("last_audio_frame_age_ms", -1),
    "captured_frames": data.get("captured_frames", 0),
    "sent_frames": data.get("sent_frames", 0),
    "tap_packets": data.get("tap_packets", 0),
    "tap_sequence_gaps": data.get("tap_sequence_gaps", 0),
    "plugin_reported_drops": data.get("plugin_reported_drops", 0),
    "overflow_frames": data.get("overflow_frames", 0),
    "underruns": data.get("underruns", 0),
    "input_errors": data.get("input_errors", 0),
    "livekit_errors": data.get("livekit_errors", 0),
    "reconnects": data.get("reconnects", 0),
    "peak_left_milli": data.get("peak_left_milli", 0),
    "peak_right_milli": data.get("peak_right_milli", 0),
    "rms_left_milli": data.get("rms_left_milli", 0),
    "rms_right_milli": data.get("rms_right_milli", 0),
}))
PY
}

current_source_kind() {
  if [[ -f "$STATUS_FILE" ]] && command -v python3 >/dev/null 2>&1; then
    python3 - "$STATUS_FILE" <<'PY' 2>/dev/null || true
import json
import sys
from pathlib import Path
print(json.loads(Path(sys.argv[1]).read_text()).get("source_kind", "unknown"))
PY
  fi
}

pids="$(sender_pids)"
if [[ -z "$pids" ]]; then
  echo "NAB Live Sender is NOT running."
  echo ""
  echo "Open NAB Live Sender.command after REAPER has NAB Tap on Master FX or Monitor FX."
  echo ""
  read -r "reply?Press Enter to close." || true
  exit 0
fi

echo "Sender PID(s): ${(j:, :)${(f)pids}}"
ps -p ${(j:,:)${(f)pids}} -o pid=,etime=,command= 2>/dev/null || true
echo ""
source_kind="$(current_source_kind)"
if [[ "$source_kind" == "test-tone" ]]; then
  echo "Socket     : n/a (internal test tone bypasses NAB Tap/VST3)"
  echo "Lock       : n/a (internal test tone bypasses NAB Tap/VST3)"
else
  if [[ -S "$SOCKET" ]]; then
    echo "Socket     : ready ($SOCKET)"
  else
    echo "Socket     : missing ($SOCKET)"
  fi
  if [[ -f "$LOCK" ]]; then
    if grep -q "pid=" "$LOCK" 2>/dev/null; then
      lock_pid="$(grep '^pid=' "$LOCK" 2>/dev/null | head -1 | cut -d= -f2)"
      if [[ -n "$lock_pid" ]] && ps -p "$lock_pid" >/dev/null 2>&1; then
        echo "Lock       : valid ($LOCK pid=$lock_pid)"
      else
        echo "Lock       : stale? ($LOCK pid=${lock_pid:-unknown})"
      fi
    else
      echo "Lock       : present but unreadable ($LOCK)"
    fi
  else
    echo "Lock       : missing ($LOCK)"
  fi
fi
runtime_locks=("$RUNTIME_LOCK_DIR"/nab-live-*.lock(N))
if (( ${#runtime_locks[@]} > 0 )); then
  for runtime_lock in "${runtime_locks[@]}"; do
    runtime_pid="$(grep '^pid=' "$runtime_lock" 2>/dev/null | head -1 | cut -d= -f2)"
    if [[ -n "$runtime_pid" ]] && ps -p "$runtime_pid" >/dev/null 2>&1; then
      echo "RuntimeLock: valid ($runtime_lock pid=$runtime_pid)"
    else
      echo "RuntimeLock: stale? ($runtime_lock pid=${runtime_pid:-unknown})"
    fi
  done
else
  echo "RuntimeLock: missing ($RUNTIME_LOCK_DIR)"
fi
if command -v curl >/dev/null 2>&1; then
  token_code="$(curl -s -o /dev/null -w "%{http_code}" "$TOKEN_HEALTH_URL" 2>/dev/null || true)"
  echo "Token API  : ${token_code:-unknown} ($TOKEN_HEALTH_URL)"
else
  echo "Token API  : unknown (curl missing)"
fi
echo ""

if [[ ! -f "$STATUS_FILE" ]]; then
  echo "Sender is running, but no status file exists yet:"
  echo "  $STATUS_FILE"
  echo ""
  read -r "reply?Press Enter to close." || true
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 not found. Raw status:"
  cat "$STATUS_FILE"
  echo ""
  read -r "reply?Press Enter to close." || true
  exit 0
fi

first="$(read_status first)"
sleep 2
second="$(read_status second)"

python3 - "$first" "$second" <<'PY'
import json
import sys

a = json.loads(sys.argv[1])
b = json.loads(sys.argv[2])

sent_delta = b["sent_frames"] - a["sent_frames"]
tap_delta = b["tap_packets"] - a["tap_packets"]
captured_delta = b["captured_frames"] - a["captured_frames"]
problem_total = (
    b["overflow_frames"]
    + b["underruns"]
    + b["input_errors"]
    + b["livekit_errors"]
    + b["plugin_reported_drops"]
)
problem_delta = (
    (b["overflow_frames"] - a["overflow_frames"])
    + (b["underruns"] - a["underruns"])
    + (b["input_errors"] - a["input_errors"])
    + (b["livekit_errors"] - a["livekit_errors"])
    + (b["plugin_reported_drops"] - a["plugin_reported_drops"])
)
tap_gap_delta = b["tap_sequence_gaps"] - a["tap_sequence_gaps"]

print(f"State      : {b['connection']}  (updated {b['age']:.1f}s ago)")
print(f"Source     : {b['source']}  [{b['source_kind']}]")
print(f"Room       : {b['room']}")
print(f"Identity   : {b['identity']}")
print(f"Track      : {b['track']}")
print(f"LiveKit    : {b['livekit_url']}")
print(f"Listen     : {b['listen_url']}")
print(f"Profile    : {b['profile']} / {int(b['bitrate_bps'] / 1000)} kbps / RED={b['red_enabled']} / DTX={b['dtx_enabled']}")
print(f"AudioState : {b['audio_state']} / lastAudioAge={b['last_audio_frame_age_ms']} ms")
print("")
print("2 second movement check")
print(f"  VST3 packets: +{tap_delta}" if b["source_kind"] == "plugin" else f"  VST3 packets: n/a ({b['source_kind']})")
print(f"  Captured    : +{captured_delta} frames")
print(f"  Sent        : +{sent_delta} frames")
print(f"  Peak now    : L {b['peak_left_milli']}/1000  R {b['peak_right_milli']}/1000")
print(f"  RMS now     : L {b['rms_left_milli']}/1000  R {b['rms_right_milli']}/1000")
print("  RTP bytes   : not exposed by current LiveKit Rust status path")
print("  Subscribers : not exposed by local status; verify on VPS/listen page")
print("")
print(f"Problems total: overflow={b['overflow_frames']} underrun={b['underruns']} inputErr={b['input_errors']} livekitErr={b['livekit_errors']} reconnect={b['reconnects']} tapGaps={b['tap_sequence_gaps']} pluginDrops={b['plugin_reported_drops']}")
print(f"Problems +2s : {problem_delta}  tapGaps +2s={tap_gap_delta}")
print("")

if b["connection"] != "Connected":
    print("Result     : NOT READY. Sender is not connected to LiveKit.")
elif sent_delta > 0 and (tap_delta > 0 or b["source_kind"] != "plugin"):
    print("Result     : AUDIO IS MOVING NOW.")
elif tap_delta > 0 and sent_delta == 0:
    print("Result     : TAP IS MOVING, BUT LIVEKIT SEND IS NOT MOVING.")
else:
    print("Result     : CONNECTED, BUT NO AUDIO MOVEMENT IN THE LAST 2 SECONDS.")

if problem_delta:
    print("Warning    : Problems increased during this 2 second check. Restart cleanly if this repeats.")
elif problem_total:
    print("Note       : Old problems are recorded above, but they did not increase during this check.")
elif tap_gap_delta:
    print("Note       : Tap sequence jumped, usually from restarting REAPER/NAB Tap or a synthetic test tone.")
PY

echo ""
read -r "reply?Press Enter to close." || true

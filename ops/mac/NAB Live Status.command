#!/bin/zsh
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH:-}"
path=(/usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin $path)
hash -r

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
    "track_sid": data.get("track_sid", ""),
    "vst3_connected": data.get("vst3_connected", "unknown"),
    "livekit_url": data.get("livekit_url"),
    "listen_url": data.get("listen_url"),
    "log_path": data.get("log_path"),
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
    "frames_dropped_total": data.get("frames_dropped_total", data.get("overflow_frames", 0) + data.get("plugin_reported_drops", 0)),
    "rtp_packets_sent": data.get("rtp_packets_sent", 0),
    "rtp_bytes_sent": data.get("rtp_bytes_sent", 0),
    "rtp_header_bytes_sent": data.get("rtp_header_bytes_sent", 0),
    "rtp_retransmitted_packets_sent": data.get("rtp_retransmitted_packets_sent", 0),
    "rtp_retransmitted_bytes_sent": data.get("rtp_retransmitted_bytes_sent", 0),
    "rtp_stats_errors": data.get("rtp_stats_errors", 0),
    "last_rtp_stats_age_ms": data.get("last_rtp_stats_age_ms", -1),
    "subscriber_count": data.get("subscriber_count", 0),
    "listener_identities": data.get("listener_identities", []),
    "overflow_frames": data.get("overflow_frames", 0),
    "underruns": data.get("underruns", 0),
    "input_errors": data.get("input_errors", 0),
    "livekit_errors": data.get("livekit_errors", 0),
    "reconnects": data.get("reconnects", 0),
    "peak_left_milli": data.get("peak_left_milli", 0),
    "peak_right_milli": data.get("peak_right_milli", 0),
    "rms_left_milli": data.get("rms_left_milli", 0),
    "rms_right_milli": data.get("rms_right_milli", 0),
    "last_error": data.get("last_error", ""),
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

print_token_health() {
  if ! command -v curl >/dev/null 2>&1; then
    echo "Token API  : unknown (curl missing)"
    return
  fi

  local body
  body="$(curl -fsS --max-time 5 "$TOKEN_HEALTH_URL" 2>/dev/null || true)"
  if [[ -z "$body" ]]; then
    echo "Token API  : failed ($TOKEN_HEALTH_URL)"
    return
  fi

  python3 - "$TOKEN_HEALTH_URL" "$body" <<'PY' 2>/dev/null || echo "Token API  : 200 ($TOKEN_HEALTH_URL)"
import json
import sys

url = sys.argv[1]
data = json.loads(sys.argv[2])
print(f"Token API  : {'200' if data.get('ok') else 'not ok'} ({url})")
last = data.get("lastProof")
if last:
    age = last.get("ageSeconds", "unknown")
    device = last.get("device") or "browser"
    proof = last.get("proof") or "unknown"
    player = last.get("player") or "unknown"
    track = last.get("track") or "unknown"
    packets = last.get("packets", 0)
    level = last.get("level", "unknown")
    loss = last.get("loss", "unknown")
    try:
        stale = int(age) > 30
    except Exception:
        stale = True
    if stale:
        print(f"LastListenerProof: stale / {device} / {proof} / {player} / age={age}s")
    else:
        print(f"ListenerProof: {device} / {proof} / {player} / age={age}s")
    print(f"  track={track} packets={packets} level={level} loss={loss}")
else:
    print("ListenerProof: none yet")
PY
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
print_token_health
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

def delta(key):
    return max(0, int(b.get(key, 0)) - int(a.get(key, 0)))

sent_delta = delta("sent_frames")
tap_delta = delta("tap_packets")
captured_delta = delta("captured_frames")
rtp_packet_delta = delta("rtp_packets_sent")
rtp_byte_delta = delta("rtp_bytes_sent")
problem_total = (
    b["overflow_frames"]
    + b["underruns"]
    + b["input_errors"]
    + b["livekit_errors"]
    + b["plugin_reported_drops"]
    + b["rtp_stats_errors"]
)
problem_delta = (
    delta("overflow_frames")
    + delta("underruns")
    + delta("input_errors")
    + delta("livekit_errors")
    + delta("plugin_reported_drops")
    + delta("rtp_stats_errors")
)
tap_gap_delta = delta("tap_sequence_gaps")
listeners = b.get("listener_identities") or []
listeners_text = ", ".join(listeners) if listeners else "none"

print(f"State      : {b['connection']}  (updated {b['age']:.1f}s ago)")
print(f"Source     : {b['source']}  [{b['source_kind']}]")
print(f"Room       : {b['room']}")
print(f"Identity   : {b['identity']}")
print(f"Track      : {b['track']}  sid={b['track_sid'] or 'unknown'}")
print(f"VST3       : {b['vst3_connected']}")
print(f"LiveKit    : {b['livekit_url']}")
print(f"Listen     : {b['listen_url']}")
print(f"Log        : {b['log_path'] or 'unknown'}")
print(f"Profile    : {b['profile']} / {int(b['bitrate_bps'] / 1000)} kbps / RED={b['red_enabled']} / FEC={b.get('opus_fec_mode', 'auto')} / DTX={b['dtx_enabled']}")
print(f"AudioState : {b['audio_state']} / lastAudioAge={b['last_audio_frame_age_ms']} ms")
print(f"RTP Stats  : age={b['last_rtp_stats_age_ms']} ms / errors={b['rtp_stats_errors']}")
print(f"Listeners  : {b['subscriber_count']}  [{listeners_text}]")
print(f"LastError  : {b['last_error'] or 'none'}")
print("")
print("2 second movement check")
print(f"  VST3 packets: +{tap_delta}" if b["source_kind"] == "plugin" else f"  VST3 packets: n/a ({b['source_kind']})")
print(f"  Captured    : +{captured_delta} frames")
print(f"  Sent        : +{sent_delta} frames")
print(f"  Dropped     : total={b['frames_dropped_total']}  pluginDrops={b['plugin_reported_drops']}  overflow={b['overflow_frames']}")
print(f"  RTP packets : +{rtp_packet_delta}  total={b['rtp_packets_sent']}")
print(f"  RTP bytes   : +{rtp_byte_delta}  total={b['rtp_bytes_sent']}")
print(f"  RTP retrans : packets={b['rtp_retransmitted_packets_sent']} bytes={b['rtp_retransmitted_bytes_sent']}")
print(f"  Peak now    : L {b['peak_left_milli']}/1000  R {b['peak_right_milli']}/1000")
print(f"  RMS now     : L {b['rms_left_milli']}/1000  R {b['rms_right_milli']}/1000")
print("")
print(f"Problems total: overflow={b['overflow_frames']} underrun={b['underruns']} inputErr={b['input_errors']} livekitErr={b['livekit_errors']} reconnect={b['reconnects']} tapGaps={b['tap_sequence_gaps']} pluginDrops={b['plugin_reported_drops']} rtpStatsErr={b['rtp_stats_errors']}")
print(f"Problems +2s : {problem_delta}  tapGaps +2s={tap_gap_delta}")
print("")

audio_moving = sent_delta > 0 and (tap_delta > 0 or b["source_kind"] != "plugin")
rtp_moving = rtp_packet_delta > 0 and rtp_byte_delta > 0
has_listener = int(b.get("subscriber_count", 0)) > 0
active_audio = b["audio_state"] == "active" and max(
    b["peak_left_milli"],
    b["peak_right_milli"],
    b["rms_left_milli"],
    b["rms_right_milli"],
) > 1

if b["connection"] != "Connected":
    print("Result     : NOT READY. Sender is not connected to LiveKit.")
elif audio_moving and rtp_moving and has_listener and active_audio:
    print("Result     : LIVE AUDIO IS MOVING TO A LISTENER NOW.")
elif audio_moving and rtp_moving and has_listener:
    print("Result     : SIGNAL PATH IS MOVING TO A LISTENER, BUT CURRENT AUDIO IS SILENCE.")
elif audio_moving and rtp_moving and active_audio:
    print("Result     : AUDIO IS REACHING LIVEKIT, BUT NO LISTENER IS CONNECTED.")
elif audio_moving and rtp_moving:
    print("Result     : SIGNAL PATH IS REACHING LIVEKIT, BUT CURRENT AUDIO IS SILENCE AND NO LISTENER IS CONNECTED.")
elif audio_moving and not rtp_moving:
    print("Result     : AUDIO ENTERS SENDER, BUT LIVEKIT RTP IS NOT MOVING.")
elif tap_delta > 0 and sent_delta == 0:
    print("Result     : TAP IS MOVING, BUT LIVEKIT SEND IS NOT MOVING.")
else:
    print("Result     : CONNECTED, BUT NO AUDIO MOVEMENT IN THE LAST 2 SECONDS.")

if problem_delta:
    print("Warning    : Problems increased during this 2 second check. Restart cleanly if this repeats.")
elif problem_total:
    print("Note       : Old problems are recorded above, but they did not increase during this check.")
elif tap_gap_delta:
    print("Note       : Tap sequence jumped, usually from restarting REAPER/NAB Tap or reopening the sender.")
PY

echo ""
read -r "reply?Press Enter to close." || true

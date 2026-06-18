#!/bin/zsh
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH:-}"
path=(/usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin $path)
hash -r

BIN="$HOME/NetworkAudioBridge/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"
PLUGIN="$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
SOCKET="$HOME/Library/Caches/KenichiNAB/nab-tap.sock"
STATUS_FILE="$HOME/.nab/status.json"
RUNTIME_LOCK="$HOME/.nab/locks/nab-live-reaper-master-nab-live-mac-mini.lock"
PROFILE="${NAB_LIVE_PROFILE:-stable-music}"
BITRATE="${NAB_LIVE_BITRATE:-}"
LIVEKIT_BUFFER_MS="${NAB_LIVE_BUFFER_MS:-}"
RED_DISABLED="${NAB_LIVE_DISABLE_RED:-}"
DTX_ENABLED="${NAB_LIVE_ENABLE_DTX:-}"

print_header() {
  clear 2>/dev/null || true
  echo "NAB Live Sender"
  echo "==============="
  echo ""
  echo "Route:"
  echo "  REAPER Master -> NAB Tap VST3 -> nab-live -> LiveKit/VPS -> browser"
  echo ""
  echo "Listen:"
  echo "  https://livekit.kenichi-kawabata.com/"
  echo ""
}

print_status() {
  if [[ ! -f "$STATUS_FILE" ]]; then
    echo "No status file yet:"
    echo "  $STATUS_FILE"
    return
  fi

  if command -v python3 >/dev/null 2>&1; then
    python3 - "$STATUS_FILE" <<'PY'
import json
import sys
import time
from pathlib import Path

path = Path(sys.argv[1])
data = json.loads(path.read_text())
age = max(0.0, time.time() - (data.get("updated_at_unix_ms", 0) / 1000))
print(f"State    : {data.get('connection')}  (updated {age:.1f}s ago)")
print(f"Source   : {data.get('source')}")
print(f"Room     : {data.get('room')}")
print(f"Profile  : {data.get('profile')} / {int(data.get('bitrate_bps', 0) / 1000)} kbps / RED={data.get('red_enabled')} / DTX={data.get('dtx_enabled')}")
print(f"Frames   : captured={data.get('captured_frames')} sent={data.get('sent_frames')} tapPackets={data.get('tap_packets')}")
print(f"VST3     : {data.get('vst3_connected', 'unknown')}  dropped={data.get('frames_dropped_total', 0)}")
print(f"RTP      : packets={data.get('rtp_packets_sent', 0)} bytes={data.get('rtp_bytes_sent', 0)} statsErr={data.get('rtp_stats_errors', 0)}")
listeners = data.get("listener_identities") or []
print(f"Listeners: {data.get('subscriber_count', 0)} [{', '.join(listeners) if listeners else 'none'}]")
print(f"Problems : overflow={data.get('overflow_frames')} underrun={data.get('underruns')} inputErr={data.get('input_errors')} livekitErr={data.get('livekit_errors')} reconnect={data.get('reconnects')}")
print(f"LastError: {data.get('last_error') or 'none'}")
print(f"Peak     : L {data.get('peak_left_milli')}/1000  R {data.get('peak_right_milli')}/1000")
PY
  else
    cat "$STATUS_FILE"
  fi
}

sender_pids() {
  pgrep -f "$BIN" 2>/dev/null || true
}

stop_senders() {
  local pids_text
  local pids
  pids_text="$(sender_pids)"
  if [[ -n "$pids_text" ]]; then
    pids=("${(@f)pids_text}")
    echo "Stopping old NAB Live Sender: ${pids[*]}"
    kill "${pids[@]}" 2>/dev/null || true
    sleep 1
  fi
  pids_text="$(sender_pids)"
  if [[ -n "$pids_text" ]]; then
    pids=("${(@f)pids_text}")
    echo "Force stopping old NAB Live Sender: ${pids[*]}"
    kill -9 "${pids[@]}" 2>/dev/null || true
    sleep 1
  fi
  rm -f "$SOCKET" "$SOCKET.lock" "$RUNTIME_LOCK"
}

print_header

if [[ ! -x "$BIN" ]]; then
  echo "Missing sender:"
  echo "  $BIN"
  read -r "reply?Press Enter to close." || true
  exit 1
fi

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing LiveKit env file:"
  echo "  $ENV_FILE"
  read -r "reply?Press Enter to close." || true
  exit 1
fi

if [[ ! -d "$PLUGIN" ]]; then
  echo "WARNING: NAB Tap VST3 is not installed at:"
  echo "  $PLUGIN"
  echo ""
  echo "Run NAB Tap Installer.command first."
  read -r "reply?Press Enter to close." || true
  exit 1
fi

running_text="$(sender_pids)"
if [[ -n "$running_text" ]]; then
  running=("${(@f)running_text}")
  echo "NAB Live Sender is already running."
  echo "Existing PID(s): ${running[*]}"
  echo ""
  print_status
  echo ""
  echo "No second sender was started. This prevents broken receiver sockets."
  echo ""
  echo "To restart cleanly, type RESTART and press Enter."
  echo "To leave it running, just press Enter."
  read -r "reply?> " || reply=""
  if [[ "$reply" != "RESTART" ]]; then
    exit 0
  fi
  echo ""
  stop_senders
  echo ""
fi

mkdir -p "$(dirname "$SOCKET")" "$(dirname "$STATUS_FILE")" "$(dirname "$RUNTIME_LOCK")"

args=(
  --source plugin
  --plugin-socket "$SOCKET"
  --status-file "$STATUS_FILE"
  --env-file "$ENV_FILE"
  --room "reaper-master"
  --identity "nab-live-mac-mini"
  --profile "$PROFILE"
  --no-tui
)

if [[ -n "$BITRATE" ]]; then
  args+=(--bitrate "$BITRATE")
fi

if [[ -n "$LIVEKIT_BUFFER_MS" ]]; then
  args+=(--livekit-buffer-ms "$LIVEKIT_BUFFER_MS")
fi

if [[ -n "$RED_DISABLED" ]]; then
  args+=(--disable-red)
fi

if [[ -n "$DTX_ENABLED" ]]; then
  args+=(--enable-dtx)
fi

echo "Starting one NAB Live Sender..."
echo "Profile: $PROFILE"
echo "Default: stable-music = 160 kbps / RED on / DTX off"
echo "Socket : $SOCKET"
echo "Status : $STATUS_FILE"
echo ""
echo "Keep this window open while listening."
echo ""

"$BIN" "${args[@]}"

status=$?
echo ""
echo "NAB Live Sender stopped with status $status"
read -r "reply?Press Enter to close." || true
exit "$status"

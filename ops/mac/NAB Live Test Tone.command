#!/bin/zsh
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH:-}"
path=(/usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin $path)
hash -r

BIN="$HOME/NetworkAudioBridge/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"
STATUS_FILE="$HOME/.nab/status.json"
RUNTIME_LOCK="$HOME/.nab/locks/nab-live-reaper-master-nab-live-mac-mini.lock"
SOCKET="$HOME/Library/Caches/KenichiNAB/nab-tap.sock"
DURATION="${NAB_TEST_TONE_SECONDS:-600}"
FREQUENCY="${NAB_TEST_TONE_HZ:-1000}"
LEVEL="${NAB_TEST_TONE_DBFS:--18}"
PROFILE="${NAB_LIVE_PROFILE:-stable-music}"

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

clear 2>/dev/null || true
echo "NAB Live Test Tone"
echo "=================="
echo ""
echo "Route:"
echo "  Internal 1 kHz test tone -> nab-live -> LiveKit/VPS -> browser/iPhone"
echo ""
echo "This bypasses REAPER and NAB Tap. Use it to check iPhone/browser listening."
echo ""
echo "Listen:"
echo "  https://livekit.kenichi-kawabata.com/"
echo ""

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

running_text="$(sender_pids)"
if [[ -n "$running_text" ]]; then
  running=("${(@f)running_text}")
  echo "NAB Live is already running."
  echo "Existing PID(s): ${running[*]}"
  echo ""
  echo "No test tone was started. This prevents duplicate LiveKit identity."
  echo ""
  echo "To stop the old sender and start test tone, type TEST and press Enter."
  echo "To leave it running, just press Enter."
  read -r "reply?> " || reply=""
  if [[ "$reply" != "TEST" ]]; then
    exit 0
  fi
  echo ""
  stop_senders
  echo ""
fi

mkdir -p "$(dirname "$STATUS_FILE")" "$(dirname "$RUNTIME_LOCK")" "$(dirname "$SOCKET")"

echo "Starting test tone..."
echo "Duration: ${DURATION}s"
echo "Tone    : ${FREQUENCY} Hz / ${LEVEL} dBFS / stereo"
echo "Profile : $PROFILE"
echo "Status  : $STATUS_FILE"
echo ""
echo "Keep this window open while checking iPhone/browser audio."
echo ""

"$BIN" \
  --test-tone \
  --test-tone-duration "$DURATION" \
  --test-tone-hz "$FREQUENCY" \
  --test-tone-dbfs="$LEVEL" \
  --status-file "$STATUS_FILE" \
  --env-file "$ENV_FILE" \
  --room "reaper-master" \
  --identity "nab-live-mac-mini" \
  --profile "$PROFILE" \
  --no-tui

status=$?
echo ""
echo "NAB Live test tone stopped with status $status"
read -r "reply?Press Enter to close." || true
exit "$status"

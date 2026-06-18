#!/bin/zsh
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH:-}"
path=(/usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin $path)
hash -r

APP_DIR="$HOME/NetworkAudioBridge"
BIN="$APP_DIR/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"
STATUS_FILE="$HOME/.nab/status.json"
STATUS_CMD="$HOME/Desktop/NAB Live Status.command"
SOCKET="$HOME/Library/Caches/KenichiNAB/nab-tap.sock"
RUNTIME_LOCK="$HOME/.nab/locks/nab-live-reaper-master-nab-live-mac-mini.lock"
REAPER_DIR="$HOME/Music/ReaClassical_26"
REAPER_BIN="$REAPER_DIR/REAPER.app/Contents/MacOS/REAPER"
REAPER_KB="$REAPER_DIR/reaper-kb.ini"
SCRIPT_SRC="$APP_DIR/tools/reaper_nab_tap_selftest.lua"
REAPER_SCRIPT_DIR="$REAPER_DIR/Scripts/Kenichi/NAB Live"
REAPER_SCRIPT="$REAPER_SCRIPT_DIR/NAB Live REAPER Selftest.lua"
ACTION_ID="RS5b0fda7e0cdabbeef000000000000000000000001"
ACTION_NAME="NAB Live REAPER Selftest"
TONE_FILE="/tmp/nab_tap_selftest_tone.wav"
RESULT_FILE="/tmp/nab_tap_reaper_selftest_result.txt"
DURATION="${NAB_REAPER_SELFTEST_SECONDS:-20}"
VOLUME="${NAB_REAPER_SELFTEST_VOLUME:-0.2}"
STARTED_SENDER=0

sender_pids() {
  /usr/bin/pgrep -f "$BIN" 2>/dev/null || true
}

stop_started_sender() {
  if (( STARTED_SENDER != 1 )); then
    return
  fi
  local pids_text
  local pids
  pids_text="$(sender_pids)"
  if [[ -n "$pids_text" ]]; then
    pids=("${(@f)pids_text}")
    echo ""
    echo "Stopping the sender started by this selftest: ${pids[*]}"
    /bin/kill "${pids[@]}" 2>/dev/null || true
    /bin/sleep 1
  fi
  pids_text="$(sender_pids)"
  if [[ -n "$pids_text" ]]; then
    pids=("${(@f)pids_text}")
    echo "Force stopping leftover sender: ${pids[*]}"
    /bin/kill -9 "${pids[@]}" 2>/dev/null || true
    /bin/sleep 1
  fi
  /bin/rm -f "$SOCKET" "$SOCKET.lock" "$RUNTIME_LOCK"
}

finish() {
  local exit_status="${1:-0}"
  stop_started_sender
  read -r "reply?Press Enter to close." || true
  exit "$exit_status"
}

trap 'finish 130' INT TERM

make_tone_if_missing() {
  if [[ -f "$TONE_FILE" ]]; then
    return
  fi
  /usr/bin/python3 - "$TONE_FILE" <<'PY'
import math
import struct
import sys
import wave

path = sys.argv[1]
rate = 96000
seconds = 2
freq = 1000.0
amp = 0.25
with wave.open(path, "wb") as wav:
    wav.setnchannels(2)
    wav.setsampwidth(2)
    wav.setframerate(rate)
    for n in range(rate * seconds):
        value = int(max(-1.0, min(1.0, math.sin(2 * math.pi * freq * n / rate) * amp)) * 32767)
        frame = struct.pack("<hh", value, value)
        wav.writeframesraw(frame)
PY
}

ensure_reaper_action() {
  /bin/mkdir -p "$REAPER_SCRIPT_DIR"
  /usr/bin/install -m 644 "$SCRIPT_SRC" "$REAPER_SCRIPT"

  /usr/bin/python3 - "$REAPER_KB" "$ACTION_ID" <<'PY'
from pathlib import Path
import sys

kb = Path(sys.argv[1])
action_id = sys.argv[2]
line = f'SCR 4 0 {action_id} "Custom: NAB Live REAPER Selftest.lua" "Kenichi/NAB Live/NAB Live REAPER Selftest.lua"\\n'
text = kb.read_text()
if action_id in text:
    print("action_registered=already")
else:
    backup = kb.with_suffix(".ini.nab-live-selftest-bak")
    if not backup.exists():
        backup.write_text(text)
    kb.write_text(line + text)
    print("action_registered=added")
PY
}

start_sender_if_needed() {
  if [[ -n "$(sender_pids)" ]]; then
    echo "NAB Live Sender is already running. Using the current sender."
    return
  fi

  /bin/mkdir -p "$(/usr/bin/dirname "$SOCKET")" "$(/usr/bin/dirname "$STATUS_FILE")" "$(/usr/bin/dirname "$RUNTIME_LOCK")"
  /bin/rm -f "$SOCKET" "$SOCKET.lock" "$RUNTIME_LOCK"
  echo "Starting temporary NAB Live Sender for REAPER/NAB Tap selftest..."
  "$BIN" \
    --source plugin \
    --plugin-socket "$SOCKET" \
    --status-file "$STATUS_FILE" \
    --env-file "$ENV_FILE" \
    --room "reaper-master" \
    --identity "nab-live-mac-mini" \
    --profile "stable-music" \
    --no-tui \
    > "$HOME/.nab/reaper-selftest-sender.log" 2>&1 &
  STARTED_SENDER=1
  /bin/sleep 3
}

trigger_reaper_action() {
  /usr/bin/osascript <<OSA
tell application "REAPER" to activate
delay 0.5
tell application "System Events"
  tell process "REAPER"
    if not (exists window "Actions") then
      click menu item "Show action list..." of menu "ReaClassical" of menu bar 1
      delay 0.8
    end if
    tell window "Actions"
      set value of text field "Search filter" to "$ACTION_NAME"
      delay 0.5
      click button "Run"
    end tell
  end tell
end tell
OSA
}

wait_for_reaper_menu() {
  local deadline=$(( SECONDS + 20 ))
  while (( SECONDS < deadline )); do
    if /usr/bin/osascript <<'OSA' >/dev/null 2>&1
tell application "System Events"
  if not (exists process "REAPER") then error "REAPER not running"
  tell process "REAPER"
    if not (exists menu bar 1) then error "menu missing"
    if not (exists menu bar item "ReaClassical" of menu bar 1) then error "ReaClassical menu missing"
  end tell
end tell
OSA
    then
      return 0
    fi
    /bin/sleep 1
  done
  return 1
}

start_reaper_if_needed() {
  if /usr/bin/pgrep -f "/REAPER.app/Contents/MacOS/REAPER" >/dev/null 2>&1; then
    if wait_for_reaper_menu; then
      return
    fi
    echo "REAPER is running, but the ReaClassical menu is not ready."
    echo "Please close startup dialogs or restart REAPER, then run this command again."
    finish 1
  fi

  echo "REAPER is not running. Starting ReaClassical..."
  (
    cd "$REAPER_DIR"
    "$REAPER_BIN" "$REAPER_DIR/ProjectTemplates/ReaClassical.RPP" > /tmp/nab_reaper_selftest_start.log 2>&1 &
  )
  if ! wait_for_reaper_menu; then
    echo "REAPER did not become ready within 20 seconds."
    echo "Start REAPER manually, then run this command again."
    finish 1
  fi
}

clear 2>/dev/null || true
echo "NAB Live REAPER Selftest"
echo "========================"
echo ""
echo "Route:"
echo "  REAPER test tone -> NAB Tap VST3 -> nab-live -> LiveKit/VPS -> listen page"
echo ""
echo "This is a diagnostic command. It creates a temporary REAPER test track"
echo "and deletes it after the test. REAPER may still show the project as modified."
echo ""

for path in "$BIN" "$ENV_FILE" "$STATUS_CMD" "$REAPER_BIN" "$REAPER_KB" "$SCRIPT_SRC"; do
  if [[ ! -e "$path" ]]; then
    echo "Missing:"
    echo "  $path"
    finish 1
  fi
done

make_tone_if_missing
ensure_reaper_action
start_reaper_if_needed
start_sender_if_needed

echo "$DURATION" > /tmp/nab_tap_selftest_seconds.txt
echo "$VOLUME" > /tmp/nab_tap_selftest_volume.txt
/bin/rm -f "$RESULT_FILE"

echo ""
echo "Running REAPER selftest action..."
trigger_reaper_action

deadline=$(( SECONDS + DURATION + 12 ))
while (( SECONDS < deadline )); do
  if [[ -f "$RESULT_FILE" ]] && /usr/bin/grep -q "transport_stopped=1" "$RESULT_FILE"; then
    echo ""
    echo "REAPER selftest result:"
    /bin/cat "$RESULT_FILE"
    echo ""
    if [[ -x "$STATUS_CMD" ]]; then
      printf "\n" | "$STATUS_CMD" || true
    fi
    finish 0
  fi
  /bin/sleep 1
done

echo ""
echo "TIMEOUT: REAPER selftest did not finish."
if [[ -f "$RESULT_FILE" ]]; then
  echo ""
  /bin/cat "$RESULT_FILE"
fi
finish 2

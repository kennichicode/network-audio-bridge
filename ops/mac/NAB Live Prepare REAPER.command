#!/bin/zsh
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH:-}"
path=(/usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin $path)
hash -r

APP_DIR="$HOME/NetworkAudioBridge"
REAPER_DIR="$HOME/Music/ReaClassical_26"
REAPER_BIN="$REAPER_DIR/REAPER.app/Contents/MacOS/REAPER"
REAPER_KB="$REAPER_DIR/reaper-kb.ini"
SCRIPT_SRC="$APP_DIR/tools/reaper_reload_nab_tap.lua"
REAPER_SCRIPT_DIR="$REAPER_DIR/Scripts/Kenichi/NAB Live"
REAPER_SCRIPT="$REAPER_SCRIPT_DIR/NAB Live Prepare REAPER.lua"
ACTION_ID="RS5b0fda7e0cdabbeef000000000000000000000002"
ACTION_NAME="NAB Live Prepare REAPER"
LOG_FILE="/tmp/nab_tap_reload.log"
VST3="$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
ACTION_REG_RESULT=""

finish() {
  local exit_status="${1:-0}"
  read -r "reply?Press Enter to close." || true
  exit "$exit_status"
}

focus_reaper_main_window() {
  /usr/bin/osascript <<'OSA' >/dev/null 2>&1 || true
tell application "REAPER" to activate
delay 0.2
tell application "System Events"
  if not (exists process "REAPER") then return
  tell process "REAPER"
    repeat with w in windows
      if name of w contains "REAPER" then
        try
          perform action "AXRaise" of w
        end try
        exit repeat
      end if
    end repeat
  end tell
end tell
OSA
}

wait_for_reaper_menu() {
  local deadline=$(( SECONDS + 20 ))
  while (( SECONDS < deadline )); do
    focus_reaper_main_window
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
    echo "Close startup dialogs or restart REAPER, then run this command again."
    finish 1
  fi

  echo "REAPER is not running. Starting ReaClassical..."
  (
    cd "$REAPER_DIR"
    "$REAPER_BIN" "$REAPER_DIR/ProjectTemplates/ReaClassical.RPP" > /tmp/nab_reaper_prepare_start.log 2>&1 &
  )
  if ! wait_for_reaper_menu; then
    echo "REAPER did not become ready within 20 seconds."
    finish 1
  fi
}

ensure_prepare_action() {
  /bin/mkdir -p "$REAPER_SCRIPT_DIR"
  /usr/bin/install -m 644 "$SCRIPT_SRC" "$REAPER_SCRIPT"

  ACTION_REG_RESULT="$(/usr/bin/python3 - "$REAPER_KB" "$ACTION_ID" <<'PY'
from pathlib import Path
import sys

kb = Path(sys.argv[1])
action_id = sys.argv[2]
line = f'SCR 4 0 {action_id} "Custom: NAB Live Prepare REAPER.lua" "Kenichi/NAB Live/NAB Live Prepare REAPER.lua"\n'
text = kb.read_text()
if action_id in text:
    print("action_registered=already")
else:
    backup = kb.with_suffix(".ini.nab-live-prepare-bak")
    if not backup.exists():
        backup.write_text(text)
    kb.write_text(line + text)
    print("action_registered=added")
PY
)"
  echo "$ACTION_REG_RESULT"
}

reaper_project_modified() {
  /usr/bin/osascript <<'OSA' 2>/dev/null | /usr/bin/grep -q "modified"
tell application "System Events"
  if not (exists process "REAPER") then return "not_running"
  tell process "REAPER"
    set out to ""
    repeat with w in windows
      set out to out & name of w & linefeed
    end repeat
    return out
  end tell
end tell
OSA
}

restart_reaper_if_action_was_added() {
  if [[ "$ACTION_REG_RESULT" != *"action_registered=added"* ]]; then
    return
  fi
  if ! /usr/bin/pgrep -f "/REAPER.app/Contents/MacOS/REAPER" >/dev/null 2>&1; then
    return
  fi

  if reaper_project_modified; then
    echo ""
    echo "The Prepare action was registered, but REAPER must be restarted once to load it."
    echo "REAPER currently looks modified, so this command will not close it automatically."
    echo "Save or close REAPER, then run this command again."
    finish 1
  fi

  echo "Restarting REAPER once so it can load the newly registered Prepare action..."
  "$REAPER_BIN" -nonewinst -closeall:nosave:exit >/tmp/nab_reaper_prepare_restart.log 2>&1 || true
  local deadline=$(( SECONDS + 15 ))
  while (( SECONDS < deadline )); do
    if ! /usr/bin/pgrep -f "/REAPER.app/Contents/MacOS/REAPER" >/dev/null 2>&1; then
      break
    fi
    /bin/sleep 1
  done
}

trigger_prepare_action() {
  focus_reaper_main_window
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

close_action_window() {
  /usr/bin/osascript <<'OSA' >/dev/null 2>&1 || true
tell application "System Events"
  if not (exists process "REAPER") then return
  tell process "REAPER"
    if exists window "Actions" then
      try
        click button "Close" of window "Actions"
      end try
    end if
  end tell
end tell
OSA
}

verify_reaper_loaded_tap() {
  local pid
  pid="$(/usr/bin/pgrep -f "/REAPER.app/Contents/MacOS/REAPER" | /usr/bin/head -1 || true)"
  if [[ -z "$pid" ]]; then
    echo "REAPER is not running after prepare."
    return 1
  fi
  if /usr/sbin/lsof -p "$pid" 2>/dev/null | /usr/bin/grep -q "NAB Tap.vst3"; then
    return 0
  fi
  if /usr/bin/osascript <<'OSA' 2>/dev/null | /usr/bin/grep -qi "NAB Tap"
tell application "System Events"
  if not (exists process "REAPER") then return ""
  tell process "REAPER"
    set out to ""
    repeat with w in windows
      set out to out & name of w & linefeed
    end repeat
    return out
  end tell
end tell
OSA
  then
    return 0
  fi
  if [[ -f "$LOG_FILE" ]] \
    && /usr/bin/grep -q "added_fx_name=.*NAB Tap" "$LOG_FILE" \
    && /usr/bin/grep -q "added_fx_offline=false" "$LOG_FILE"; then
    return 0
  fi
  return 1
}

run_prepare_action_once() {
  /bin/rm -f "$LOG_FILE"
  echo ""
  echo "Loading NAB Tap on REAPER Master FX..."
  trigger_prepare_action
  /bin/sleep 2
  close_action_window
}

run_prepare_action_with_retry() {
  run_prepare_action_once
  if [[ -f "$LOG_FILE" ]]; then
    return
  fi

  echo "Prepare action did not run. REAPER may not have loaded the newly registered action yet."
  if reaper_project_modified; then
    echo "REAPER currently looks modified, so this command will not restart it automatically."
    return
  fi

  echo "Restarting REAPER once, then trying Prepare again..."
  "$REAPER_BIN" -nonewinst -closeall:nosave:exit >/tmp/nab_reaper_prepare_retry_restart.log 2>&1 || true
  local deadline=$(( SECONDS + 15 ))
  while (( SECONDS < deadline )); do
    if ! /usr/bin/pgrep -f "/REAPER.app/Contents/MacOS/REAPER" >/dev/null 2>&1; then
      break
    fi
    /bin/sleep 1
  done
  start_reaper_if_needed
  run_prepare_action_once
}

clear 2>/dev/null || true
echo "NAB Live Prepare REAPER"
echo "======================="
echo ""
echo "This prepares the normal route:"
echo "  REAPER Master FX -> NAB Tap VST3 -> NAB Live Sender"
echo ""
echo "It does not start streaming and does not play audio."
echo ""

for path in "$REAPER_BIN" "$REAPER_KB" "$SCRIPT_SRC" "$VST3"; do
  if [[ ! -e "$path" ]]; then
    echo "Missing:"
    echo "  $path"
    finish 1
  fi
done

start_reaper_if_needed
ensure_prepare_action
restart_reaper_if_action_was_added
start_reaper_if_needed

run_prepare_action_with_retry

echo ""
echo "Prepare log:"
if [[ -f "$LOG_FILE" ]]; then
  /bin/cat "$LOG_FILE"
else
  echo "  $LOG_FILE was not created."
fi

echo ""
if verify_reaper_loaded_tap; then
  echo "READY: REAPER has loaded NAB Tap.vst3."
  echo "Next: open NAB Live Sender.command and listen at https://livekit.kenichi-kawabata.com/"
  finish 0
else
  echo "NOT READY: REAPER has not loaded NAB Tap.vst3."
  echo "Open REAPER's Master FX and confirm VST3: NAB Tap (Kenichi Kawabata)."
  finish 2
fi

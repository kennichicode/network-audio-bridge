#!/bin/zsh
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH:-}"
path=(/usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin $path)
hash -r

BIN="$HOME/NetworkAudioBridge/nab-live"
VST3="$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
RME_IP="${NAB_RME_IP:-192.168.1.20}"
LISTEN_URL="https://livekit.kenichi-kawabata.com/"
STATUS_CMD="$HOME/Desktop/NAB Live Status.command"
SENDER_CMD="$HOME/Desktop/NAB Live Sender.command"
PREPARE_LOG="/tmp/nab_tap_reload.log"

ok_count=0
ng_count=0

ok() {
  ok_count=$((ok_count + 1))
  echo "  [OK] $1"
}

ng() {
  ng_count=$((ng_count + 1))
  echo "  [NG] $1"
}

check_file() {
  if [[ -e "$1" ]]; then
    ok "$2: $1"
  else
    ng "$2 missing: $1"
  fi
}

reaper_pid() {
  pgrep -f "/REAPER.app/Contents/MacOS/REAPER" 2>/dev/null | head -1 || true
}

recent_prepare_log_confirms_tap() {
  if [[ ! -f "$PREPARE_LOG" ]]; then
    return 1
  fi
  local now mtime age
  now="$(date +%s)"
  mtime="$(stat -f %m "$PREPARE_LOG" 2>/dev/null || echo 0)"
  age=$(( now - mtime ))
  if (( age > 900 )); then
    return 1
  fi
  grep -q "added_fx_name=.*NAB Tap" "$PREPARE_LOG" \
    && grep -q "added_fx_offline=false" "$PREPARE_LOG"
}

reaper_windows_confirm_tap() {
  osascript <<'OSA' 2>/dev/null | grep -qi "NAB Tap"
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
}

reaper_has_nab_tap() {
  local pid="$1"
  if lsof -p "$pid" 2>/dev/null | grep -q "NAB Tap.vst3"; then
    return 0
  fi
  if reaper_windows_confirm_tap; then
    return 0
  fi
  recent_prepare_log_confirms_tap
}

clear 2>/dev/null || true
echo "NAB Live RME / REAPER Preflight"
echo "==============================="
echo ""
echo "This does not start streaming and does not change REAPER."
echo "It only checks the RME / AVB / REAPER / NAB Tap readiness."
echo ""

echo "Runtime"
check_file "$BIN" "nab-live"
check_file "$VST3" "NAB Tap VST3"
check_file "$SENDER_CMD" "Sender command"
check_file "$STATUS_CMD" "Status command"
echo ""

echo "Network"
en0_ip="$(ifconfig en0 2>/dev/null | awk '/inet / {print $2; exit}' || true)"
en1_ip="$(ifconfig en1 2>/dev/null | awk '/inet / {print $2; exit}' || true)"
[[ -n "$en0_ip" ]] && ok "en0 IP: $en0_ip" || ng "en0 IP missing"
[[ -n "$en1_ip" ]] && ok "en1 IP: $en1_ip" || ng "en1 IP missing"
if ping -c 2 -W 1000 "$RME_IP" >/dev/null 2>&1; then
  ok "RME ping: $RME_IP"
else
  ng "RME ping failed: $RME_IP"
fi
if curl -fsS --max-time 3 "http://$RME_IP/" >/dev/null 2>&1; then
  ok "RME web UI: http://$RME_IP/"
else
  ng "RME web UI failed: http://$RME_IP/"
fi
echo ""

echo "AVB audio device"
audio_report="$(system_profiler SPAudioDataType 2>/dev/null || true)"
if print -r -- "$audio_report" | grep -q "Transport: AVB"; then
  ok "AVB transport is visible in macOS audio devices"
else
  ng "AVB transport is not visible in macOS audio devices"
fi
if print -r -- "$audio_report" | grep -q "Input Channels: 16"; then
  ok "AVB input channels include 16ch"
else
  ng "16ch AVB input was not found"
fi
if print -r -- "$audio_report" | grep -q "Current SampleRate: 96000"; then
  ok "96 kHz audio device is visible"
else
  ng "96 kHz audio device was not found"
fi
if print -r -- "$audio_report" | grep -q "Default Input Device: Yes"; then
  ok "Default input device is set"
else
  ng "No default input device reported"
fi
echo ""

echo "REAPER"
pid="$(reaper_pid)"
if [[ -n "$pid" ]]; then
  ok "REAPER running: pid=$pid"
  if reaper_has_nab_tap "$pid"; then
    ok "REAPER has NAB Tap on the Master FX path"
  else
    ng "REAPER has not loaded NAB Tap.vst3; run NAB Live Prepare REAPER.command"
  fi
else
  ng "REAPER is not running"
fi
echo ""

echo "Sender state"
sender_pids="$(pgrep -f "$BIN" 2>/dev/null || true)"
if [[ -n "$sender_pids" ]]; then
  ok "nab-live sender is running: ${(j:, :)${(f)sender_pids}}"
else
  ok "nab-live sender is stopped; start it with NAB Live Sender.command when ready"
fi
echo ""

if (( ng_count == 0 )); then
  echo "READY: RME / AVB / REAPER / NAB Tap preflight passed."
  echo ""
  echo "Next real 12Mic check:"
  echo "  1. Open NAB Live Sender.command."
  echo "  2. Open $LISTEN_URL on iPhone or browser."
  echo "  3. Play or speak into the 12Mic input routed through REAPER."
  echo "  4. Open NAB Live Status.command and confirm audio movement + ListenerProof."
else
  echo "NOT READY: $ng_count preflight item(s) failed."
  echo "Fix the [NG] item(s) before relying on 12Mic remote listening."
fi
echo ""
read -r "reply?Press Enter to close." || true
exit "$(( ng_count == 0 ? 0 : 1 ))"

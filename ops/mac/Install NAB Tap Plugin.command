#!/bin/zsh
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH:-}"
path=(/usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin $path)
hash -r

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_DIR="$HOME/NetworkAudioBridge"
BIN="$APP_DIR/nab-tap-whip-sender"
OLD_BIN="$APP_DIR/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"
VST3_SRC="$APP_DIR/plugins/nab-tap/NAB Tap.vst3"
VST3_DST="$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
TOOLS_DIR="$HOME/Desktop/NAB Live Tools"
SENDER="$HOME/Desktop/NAB Live Sender.command"
STATUS="$HOME/Desktop/NAB Live Status.command"
IPHONE_CHECK="$TOOLS_DIR/NAB Live iPhone Check.command"
RME_PREFLIGHT="$TOOLS_DIR/NAB Live RME Preflight.command"
PREPARE_REAPER="$TOOLS_DIR/NAB Live Prepare REAPER.command"
TOKEN_HEALTH_URL="https://livekit.kenichi-kawabata.com/healthz"

ok_line() {
  echo "  [OK] $1"
}

ng_line() {
  echo "  [NG] $1"
}

check_install() {
  echo "Current install check"
  echo "---------------------"
  if [[ -x "$BIN" ]]; then
    ok_line "StageDAW WHIP sender binary: $BIN"
  else
    ng_line "StageDAW WHIP sender binary missing: $BIN"
  fi
  if [[ -x "$OLD_BIN" ]]; then
    ok_line "legacy nab-live binary is present but not used for normal sending: $OLD_BIN"
  fi
  if [[ -d "$VST3_SRC" ]]; then
    ok_line "bundled NAB Tap: $VST3_SRC"
  else
    ng_line "bundled NAB Tap missing: $VST3_SRC"
  fi
  if [[ -d "$VST3_DST" ]]; then
    ok_line "installed NAB Tap VST3: $VST3_DST"
  else
    ng_line "installed NAB Tap VST3 missing: $VST3_DST"
  fi
  if [[ -x "$SENDER" ]]; then
    ok_line "Sender command: $SENDER"
  else
    ng_line "Sender command missing: $SENDER"
  fi
  if [[ -x "$STATUS" ]]; then
    ok_line "Status command: $STATUS"
  else
    ng_line "Status command missing: $STATUS"
  fi
  if [[ -x "$IPHONE_CHECK" ]]; then
    ok_line "iPhone check command: $IPHONE_CHECK"
  else
    ng_line "iPhone check command missing: $IPHONE_CHECK"
  fi
  if [[ -x "$RME_PREFLIGHT" ]]; then
    ok_line "RME preflight command: $RME_PREFLIGHT"
  else
    ng_line "RME preflight command missing: $RME_PREFLIGHT"
  fi
  if [[ -x "$PREPARE_REAPER" ]]; then
    ok_line "Prepare REAPER command: $PREPARE_REAPER"
  else
    ng_line "Prepare REAPER command missing: $PREPARE_REAPER"
  fi
  if [[ -f "$ENV_FILE" ]]; then
    ok_line "LiveKit env file: $ENV_FILE"
  else
    ng_line "LiveKit env file missing: $ENV_FILE"
  fi
  if command -v curl >/dev/null 2>&1; then
    token_code="$(curl -s -o /dev/null -w "%{http_code}" "$TOKEN_HEALTH_URL" 2>/dev/null || true)"
    if [[ "$token_code" == "200" ]]; then
      ok_line "Token API: 200 $TOKEN_HEALTH_URL"
    else
      ng_line "Token API: ${token_code:-unknown} $TOKEN_HEALTH_URL"
    fi
  else
    ng_line "curl missing; token API not checked"
  fi
  echo ""
}

clear 2>/dev/null || true
echo "NAB Tap Installer"
echo "================="
echo ""
echo "You do not need to run this every time."
echo ""
echo "Run it only when:"
echo "  - NAB Tap is missing in REAPER"
echo "  - the plugin was updated"
echo "  - Codex asks you to reinstall the plugin"
echo ""
echo "This does not start streaming. It reinstalls the VST3 and checks the NAB Live runtime."
echo ""
check_install
read -r "reply?Type INSTALL to reinstall VST3, CHECK to re-check, or press Enter to close: " || reply=""
if [[ "$reply" == "CHECK" ]]; then
  echo ""
  check_install
  read -r "reply?Press Enter to close." || true
  exit 0
fi
if [[ "$reply" != "INSTALL" ]]; then
  echo "Cancelled."
  read -r "reply?Press Enter to close." || true
  exit 0
fi

if [[ -x "$SCRIPT_DIR/install-nab-tap-plugin.sh" ]]; then
  "$SCRIPT_DIR/install-nab-tap-plugin.sh" "$APP_DIR/plugins/nab-tap"
elif [[ -x "$APP_DIR/install-nab-tap-plugin.sh" ]]; then
  "$APP_DIR/install-nab-tap-plugin.sh" "$APP_DIR/plugins/nab-tap"
else
  echo "install-nab-tap-plugin.sh was not found."
  read -r "reply?Press Enter to close." || true
  exit 1
fi

echo ""
check_install
echo "Done. Open NAB Live Sender.command only when you want to start streaming."
read -r "reply?Press Enter to close." || true

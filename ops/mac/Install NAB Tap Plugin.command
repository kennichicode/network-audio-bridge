#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_DIR="$HOME/NetworkAudioBridge"
BIN="$APP_DIR/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"
VST3_SRC="$APP_DIR/plugins/nab-tap/NAB Tap.vst3"
VST3_DST="$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
SENDER="$HOME/Desktop/NAB Live Sender.command"
STATUS="$HOME/Desktop/NAB Live Status.command"
TEST_TONE="$HOME/Desktop/NAB Live Test Tone.command"
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
    ok_line "nab-live binary: $BIN"
  else
    ng_line "nab-live binary missing: $BIN"
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
  if [[ -x "$TEST_TONE" ]]; then
    ok_line "Test tone command: $TEST_TONE"
  else
    ng_line "Test tone command missing: $TEST_TONE"
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
read -r "reply?Type INSTALL to reinstall VST3, CHECK to re-check, or press Enter to close: "
if [[ "$reply" == "CHECK" ]]; then
  echo ""
  check_install
  read -r "reply?Press Enter to close."
  exit 0
fi
if [[ "$reply" != "INSTALL" ]]; then
  echo "Cancelled."
  read -r "reply?Press Enter to close."
  exit 0
fi

if [[ -x "$SCRIPT_DIR/install-nab-tap-plugin.sh" ]]; then
  "$SCRIPT_DIR/install-nab-tap-plugin.sh" "$APP_DIR/plugins/nab-tap"
elif [[ -x "$APP_DIR/install-nab-tap-plugin.sh" ]]; then
  "$APP_DIR/install-nab-tap-plugin.sh" "$APP_DIR/plugins/nab-tap"
else
  echo "install-nab-tap-plugin.sh was not found."
  read -r "reply?Press Enter to close."
  exit 1
fi

echo ""
check_install
echo "Done. Open NAB Live Sender.command only when you want to start streaming."
read -r "reply?Press Enter to close."

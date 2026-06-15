#!/bin/zsh
set -euo pipefail

BIN="$HOME/NetworkAudioBridge/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"

clear
echo "NAB Live Wizard"
echo "==============="
echo ""
echo "Use Up/Down and Enter. Choose NAB Tap for REAPER Master."
echo ""

if [[ ! -x "$BIN" ]]; then
  echo "Missing sender: $BIN"
  read -r "reply?Press Enter to close."
  exit 1
fi

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing LiveKit env file: $ENV_FILE"
  read -r "reply?Press Enter to close."
  exit 1
fi

"$BIN" \
  --env-file "$ENV_FILE" \
  --room "reaper-master" \
  --identity "nab-live-mac-mini"

status=$?
echo ""
echo "nab-live ended with status $status"
read -r "reply?Press Enter to close."
exit "$status"

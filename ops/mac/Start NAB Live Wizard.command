#!/bin/zsh
set -euo pipefail

BIN="$HOME/NetworkAudioBridge/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"

clear
echo "NAB Live Advanced Wizard"
echo "========================"
echo ""
echo "Normal concert use: close this and open NAB Live Sender.command instead."
echo ""
echo "Use this only when you want to choose a different source, such as CoreAudio."
echo "If you continue, choose 'REAPER Master Plugin - NAB Tap' for the normal route."
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
  --identity "nab-live-mac-mini" \
  --status-file "$HOME/.nab/status.json"

status=$?
echo ""
echo "nab-live ended with status $status"
read -r "reply?Press Enter to close."
exit "$status"

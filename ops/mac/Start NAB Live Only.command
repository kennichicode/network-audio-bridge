#!/bin/zsh
set -euo pipefail

BIN="$HOME/NetworkAudioBridge/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"
PLUGIN="$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"

clear
echo "NAB Live Sender"
echo "==============="
echo ""
echo "Route:"
echo "  REAPER Master -> NAB Tap VST3 -> nab-live -> LiveKit/VPS -> browser"
echo ""
echo "What to do:"
echo "  1. Keep this window open."
echo "  2. In REAPER, insert: VST3: NAB Tap (Kenichi Kawabata)"
echo "     on Master FX or Monitor FX."
echo "  3. Listen at: https://livekit.kenichi-kawabata.com/"
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

if [[ ! -d "$PLUGIN" ]]; then
  echo "Warning: NAB Tap VST3 is not installed at:"
  echo "  $PLUGIN"
  echo "Run Install NAB Tap Plugin.command first."
  echo ""
fi

"$BIN" \
  --source plugin \
  --env-file "$ENV_FILE" \
  --room "reaper-master" \
  --identity "nab-live-mac-mini" \
  --bitrate 256000 \
  --livekit-buffer-ms 1000

status=$?
echo ""
echo "nab-live ended with status $status"
read -r "reply?Press Enter to close."
exit "$status"

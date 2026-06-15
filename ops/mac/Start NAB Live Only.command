#!/bin/zsh
set -euo pipefail

BIN="$HOME/NetworkAudioBridge/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"
PLUGIN="$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
SOCKET="$HOME/Library/Caches/KenichiNAB/nab-tap.sock"
PROFILE="${NAB_LIVE_PROFILE:-concert}"
BITRATE="${NAB_LIVE_BITRATE:-}"
LIVEKIT_BUFFER_MS="${NAB_LIVE_BUFFER_MS:-}"
RED_DISABLED="${NAB_LIVE_DISABLE_RED:-}"
DTX_ENABLED="${NAB_LIVE_ENABLE_DTX:-}"

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
echo "Settings:"
echo "  profile: $PROFILE"
echo "  socket : $SOCKET"
echo "  bitrate override: ${BITRATE:-profile default}"
echo "  LiveKit buffer : ${LIVEKIT_BUFFER_MS:-profile default}"
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
  echo "Run NAB Tap Installer.command first."
  echo ""
fi

args=(
  --source plugin \
  --plugin-socket "$SOCKET" \
  --env-file "$ENV_FILE" \
  --room "reaper-master" \
  --identity "nab-live-mac-mini" \
  --profile "$PROFILE"
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

"$BIN" "${args[@]}"

status=$?
echo ""
echo "nab-live ended with status $status"
read -r "reply?Press Enter to close."
exit "$status"

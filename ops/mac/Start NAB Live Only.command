#!/bin/zsh
set -euo pipefail

BIN="$HOME/NetworkAudioBridge/nab-live"
ENV_FILE="$HOME/.config/kenichi-vps/livekit.env"
PLUGIN="$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
SOCKET="$HOME/Library/Caches/KenichiNAB/nab-tap.sock"
STATUS_FILE="$HOME/.nab/status.json"
PROFILE="${NAB_LIVE_PROFILE:-concert}"
BITRATE="${NAB_LIVE_BITRATE:-}"
LIVEKIT_BUFFER_MS="${NAB_LIVE_BUFFER_MS:-}"
RED_DISABLED="${NAB_LIVE_DISABLE_RED:-}"
DTX_ENABLED="${NAB_LIVE_ENABLE_DTX:-}"

clear
echo "NAB Live Sender - usual button"
echo "=============================="
echo ""
echo "This is the normal command. Use this one for concerts and remote checks."
echo ""
echo "Route"
echo "  REAPER Master -> NAB Tap VST3 -> nab-live -> LiveKit/VPS -> browser"
echo ""
echo "Order"
echo "  1. Open REAPER."
echo "  2. Make sure Master FX or Monitor FX has:"
echo "       VST3: NAB Tap (Kenichi Kawabata)"
echo "  3. Open this command and keep this window open."
echo "  4. Listen at:"
echo "       https://livekit.kenichi-kawabata.com/"
echo ""
echo "Notes"
echo "  - NAB Tap Installer is only for first install or after an update."
echo "  - Wizard is advanced; do not use it for the normal REAPER monitor."
echo ""
echo "Settings:"
echo "  profile: $PROFILE"
echo "  socket : $SOCKET"
echo "  status : $STATUS_FILE"
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
  --status-file "$STATUS_FILE" \
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

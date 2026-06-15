#!/bin/zsh
set -euo pipefail

open -a "/Applications/REAPER.app" || true

exec "$HOME/NetworkAudioBridge/nab-live" \
  --input "BlackHole 2ch" \
  --input-sample-rate 96000 \
  --input-channels 2 \
  --left-channel 1 \
  --right-channel 2 \
  --room "reaper-master" \
  --identity "nab-live-mac-mini"

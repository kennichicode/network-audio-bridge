#!/bin/zsh
set -euo pipefail

exec "$HOME/NetworkAudioBridge/nab-live" \
  --source plugin \
  --room "reaper-master" \
  --identity "nab-live-mac-mini"

#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

if [[ -x "$SCRIPT_DIR/install-nab-tap-plugin.sh" ]]; then
  exec "$SCRIPT_DIR/install-nab-tap-plugin.sh"
fi

if [[ -x "$HOME/NetworkAudioBridge/install-nab-tap-plugin.sh" ]]; then
  exec "$HOME/NetworkAudioBridge/install-nab-tap-plugin.sh"
fi

echo "install-nab-tap-plugin.sh was not found."
read -r "reply?Press Enter to close."

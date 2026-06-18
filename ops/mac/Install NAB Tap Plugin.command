#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

clear
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
echo "This does not start streaming. It only copies the VST3 plugin."
echo ""
read -r "reply?Type INSTALL to reinstall, or press Enter to cancel: "
if [[ "$reply" != "INSTALL" ]]; then
  echo "Cancelled."
  read -r "reply?Press Enter to close."
  exit 0
fi

if [[ -x "$SCRIPT_DIR/install-nab-tap-plugin.sh" ]]; then
  exec "$SCRIPT_DIR/install-nab-tap-plugin.sh"
fi

if [[ -x "$HOME/NetworkAudioBridge/install-nab-tap-plugin.sh" ]]; then
  exec "$HOME/NetworkAudioBridge/install-nab-tap-plugin.sh"
fi

echo "install-nab-tap-plugin.sh was not found."
read -r "reply?Press Enter to close."

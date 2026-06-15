#!/bin/zsh
set -euo pipefail

SRC_ROOT="${1:-$HOME/NetworkAudioBridge/plugins/nab-tap}"
VST3_SRC="$SRC_ROOT/NAB Tap.vst3"

if [[ ! -d "$VST3_SRC" ]]; then
  echo "Missing VST3 bundle: $VST3_SRC" >&2
  exit 1
fi

mkdir -p "$HOME/Library/Audio/Plug-Ins/VST3"

rm -rf "$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
ditto "$VST3_SRC" "$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"

codesign --force --deep -s - "$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3" >/dev/null

echo "Installed NAB Tap:"
echo "  $HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
echo ""
echo "Restart REAPER or rescan plugins, then insert NAB Tap on Master FX or Monitor FX."

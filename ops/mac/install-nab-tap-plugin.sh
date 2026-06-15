#!/bin/zsh
set -euo pipefail

SRC_ROOT="${1:-$HOME/NetworkAudioBridge/plugins/nab-tap}"
VST3_SRC="$SRC_ROOT/NAB Tap.vst3"
AU_SRC="$SRC_ROOT/NAB Tap.component"

if [[ ! -d "$VST3_SRC" ]]; then
  echo "Missing VST3 bundle: $VST3_SRC" >&2
  exit 1
fi

if [[ ! -d "$AU_SRC" ]]; then
  echo "Missing AU bundle: $AU_SRC" >&2
  exit 1
fi

mkdir -p "$HOME/Library/Audio/Plug-Ins/VST3"
mkdir -p "$HOME/Library/Audio/Plug-Ins/Components"

ditto "$VST3_SRC" "$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
ditto "$AU_SRC" "$HOME/Library/Audio/Plug-Ins/Components/NAB Tap.component"

codesign --force --deep -s - "$HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3" >/dev/null
codesign --force --deep -s - "$HOME/Library/Audio/Plug-Ins/Components/NAB Tap.component" >/dev/null

echo "Installed NAB Tap:"
echo "  $HOME/Library/Audio/Plug-Ins/VST3/NAB Tap.vst3"
echo "  $HOME/Library/Audio/Plug-Ins/Components/NAB Tap.component"
echo ""
echo "Restart REAPER or rescan plugins, then insert NAB Tap on Master FX or Monitor FX."

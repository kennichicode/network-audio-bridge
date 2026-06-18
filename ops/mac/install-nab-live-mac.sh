#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
APP_DIR="${NAB_APP_DIR:-$HOME/NetworkAudioBridge}"
DESKTOP_DIR="$HOME/Desktop"
TOOLS_DIR="$DESKTOP_DIR/NAB Live Tools"
VST3_BUILD="$REPO_ROOT/plugins/nab-tap/build/NABTap_artefacts/Release/VST3/NAB Tap.vst3"
WHIP_BUILD="${NAB_WHIP_BUILD:-$REPO_ROOT/../StageDAW-Recorder/build/nab-tap-whip-sender_artefacts/Release/nab-tap-whip-sender}"

need_file() {
  if [[ ! -f "$1" ]]; then
    echo "Missing file: $1" >&2
    exit 1
  fi
}

need_dir() {
  if [[ ! -d "$1" ]]; then
    echo "Missing directory: $1" >&2
    exit 1
  fi
}

need_file "$REPO_ROOT/target/release/nab-live"
need_file "$WHIP_BUILD"
need_dir "$VST3_BUILD"

mkdir -p "$APP_DIR/plugins/nab-tap" "$APP_DIR/tools" "$DESKTOP_DIR" "$TOOLS_DIR" "$HOME/Applications"

install -m 755 "$REPO_ROOT/target/release/nab-live" "$APP_DIR/nab-live"
install -m 755 "$WHIP_BUILD" "$APP_DIR/nab-tap-whip-sender"

if [[ -f "$REPO_ROOT/target/release/nab" ]]; then
  install -m 755 "$REPO_ROOT/target/release/nab" "$APP_DIR/nab"
fi

if [[ -f "$REPO_ROOT/target/release/nab-recv" ]]; then
  install -m 755 "$REPO_ROOT/target/release/nab-recv" "$APP_DIR/nab-recv"
fi

rm -rf "$APP_DIR/plugins/nab-tap/NAB Tap.vst3"
ditto "$VST3_BUILD" "$APP_DIR/plugins/nab-tap/NAB Tap.vst3"
install -m 644 "$REPO_ROOT/scripts/reaper_reload_nab_tap.lua" "$APP_DIR/tools/reaper_reload_nab_tap.lua"

for file in \
  "install-nab-tap-plugin.sh" \
  "Install NAB Tap Plugin.command" \
  "Start NAB Live Only.command" \
  "Start NAB Live Wizard.command" \
  "NAB Live iPhone Check.command" \
  "NAB Live RME Preflight.command" \
  "NAB Live Prepare REAPER.command" \
  "NAB Live Status.command"; do
  install -m 755 "$REPO_ROOT/ops/mac/$file" "$APP_DIR/$file"
done

install -m 755 "$APP_DIR/Start NAB Live Only.command" "$DESKTOP_DIR/NAB Live Sender.command"
install -m 755 "$APP_DIR/NAB Live Status.command" "$DESKTOP_DIR/NAB Live Status.command"
install -m 755 "$APP_DIR/Install NAB Tap Plugin.command" "$DESKTOP_DIR/NAB Tap Installer.command"
install -m 755 "$APP_DIR/NAB Live iPhone Check.command" "$TOOLS_DIR/NAB Live iPhone Check.command"
install -m 755 "$APP_DIR/NAB Live RME Preflight.command" "$TOOLS_DIR/NAB Live RME Preflight.command"
install -m 755 "$APP_DIR/NAB Live Prepare REAPER.command" "$TOOLS_DIR/NAB Live Prepare REAPER.command"
rm -f "$DESKTOP_DIR/NAB Live Wizard.command"
rm -f \
  "$DESKTOP_DIR/NAB Live Test Tone.command" \
  "$DESKTOP_DIR/NAB Live iPhone Check.command" \
  "$DESKTOP_DIR/NAB Live RME Preflight.command" \
  "$DESKTOP_DIR/NAB Live Prepare REAPER.command" \
  "$DESKTOP_DIR/NAB Live REAPER Selftest.command" \
  "$TOOLS_DIR/NAB Live Test Tone.command" \
  "$TOOLS_DIR/NAB Live REAPER Selftest.command" \
  "$APP_DIR/NAB Live Test Tone.command" \
  "$APP_DIR/NAB Live REAPER Selftest.command" \
  "$APP_DIR/tools/reaper_nab_tap_selftest.lua"

install -m 755 "$APP_DIR/Start NAB Live Only.command" "$HOME/Applications/NAB Live Sender.command"
install -m 755 "$APP_DIR/Start NAB Live Wizard.command" "$HOME/Applications/NAB Live Advanced Wizard.command"
install -m 755 "$APP_DIR/NAB Live Status.command" "$HOME/Applications/NAB Live Status.command"
install -m 755 "$APP_DIR/NAB Live iPhone Check.command" "$HOME/Applications/NAB Live iPhone Check.command"
install -m 755 "$APP_DIR/NAB Live RME Preflight.command" "$HOME/Applications/NAB Live RME Preflight.command"
install -m 755 "$APP_DIR/NAB Live Prepare REAPER.command" "$HOME/Applications/NAB Live Prepare REAPER.command"
install -m 755 "$APP_DIR/Install NAB Tap Plugin.command" "$HOME/Applications/NAB Tap Installer.command"
rm -f \
  "$HOME/Applications/NAB Live Test Tone.command" \
  "$HOME/Applications/NAB Live REAPER Selftest.command"

"$APP_DIR/install-nab-tap-plugin.sh" "$APP_DIR/plugins/nab-tap"

echo ""
echo "Installed NAB Live runtime:"
echo "  $APP_DIR/nab-tap-whip-sender"
echo "  $APP_DIR/nab-live (legacy fallback, not normal sender)"
echo "  $APP_DIR/plugins/nab-tap/NAB Tap.vst3"
echo ""
echo "Launchers:"
echo "  $DESKTOP_DIR/NAB Live Sender.command"
echo "  $DESKTOP_DIR/NAB Live Status.command"
echo "  $DESKTOP_DIR/NAB Tap Installer.command"
echo "  $TOOLS_DIR/"

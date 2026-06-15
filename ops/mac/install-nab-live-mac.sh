#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
APP_DIR="${NAB_APP_DIR:-$HOME/NetworkAudioBridge}"
DESKTOP_DIR="$HOME/Desktop"
VST3_BUILD="$REPO_ROOT/plugins/nab-tap/build/NABTap_artefacts/Release/VST3/NAB Tap.vst3"

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
need_dir "$VST3_BUILD"

mkdir -p "$APP_DIR/plugins/nab-tap" "$DESKTOP_DIR" "$HOME/Applications"

install -m 755 "$REPO_ROOT/target/release/nab-live" "$APP_DIR/nab-live"

if [[ -f "$REPO_ROOT/target/release/nab" ]]; then
  install -m 755 "$REPO_ROOT/target/release/nab" "$APP_DIR/nab"
fi

if [[ -f "$REPO_ROOT/target/release/nab-recv" ]]; then
  install -m 755 "$REPO_ROOT/target/release/nab-recv" "$APP_DIR/nab-recv"
fi

rm -rf "$APP_DIR/plugins/nab-tap/NAB Tap.vst3"
ditto "$VST3_BUILD" "$APP_DIR/plugins/nab-tap/NAB Tap.vst3"

for file in \
  "install-nab-tap-plugin.sh" \
  "Install NAB Tap Plugin.command" \
  "Start NAB Live Only.command" \
  "Start NAB Live Wizard.command"; do
  install -m 755 "$REPO_ROOT/ops/mac/$file" "$APP_DIR/$file"
done

install -m 755 "$APP_DIR/Start NAB Live Only.command" "$DESKTOP_DIR/NAB Live Sender.command"
install -m 755 "$APP_DIR/Start NAB Live Wizard.command" "$DESKTOP_DIR/NAB Live Wizard.command"
install -m 755 "$APP_DIR/Install NAB Tap Plugin.command" "$DESKTOP_DIR/NAB Tap Installer.command"

install -m 755 "$APP_DIR/Start NAB Live Only.command" "$HOME/Applications/NAB Live Sender.command"
install -m 755 "$APP_DIR/Start NAB Live Wizard.command" "$HOME/Applications/NAB Live Wizard.command"
install -m 755 "$APP_DIR/Install NAB Tap Plugin.command" "$HOME/Applications/NAB Tap Installer.command"

"$APP_DIR/install-nab-tap-plugin.sh" "$APP_DIR/plugins/nab-tap"

echo ""
echo "Installed NAB Live runtime:"
echo "  $APP_DIR/nab-live"
echo "  $APP_DIR/plugins/nab-tap/NAB Tap.vst3"
echo ""
echo "Launchers:"
echo "  $DESKTOP_DIR/NAB Live Sender.command"
echo "  $DESKTOP_DIR/NAB Live Wizard.command"
echo "  $DESKTOP_DIR/NAB Tap Installer.command"

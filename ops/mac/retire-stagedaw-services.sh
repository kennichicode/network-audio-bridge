#!/bin/zsh
set -euo pipefail

if [[ "${CONFIRM_STAGE_DAW_RETIRE:-}" != "yes" ]]; then
  echo "This disables StageDAW LaunchAgents but keeps AVB helpers."
  echo "Run only after Reaper -> BlackHole -> NAB Live -> VPS browser has passed the stability test."
  echo "To execute:"
  echo "  CONFIRM_STAGE_DAW_RETIRE=yes $0"
  exit 2
fi

uid="$(id -u)"
stamp="$(date +%Y%m%d_%H%M%S)"
disabled_dir="$HOME/Library/LaunchAgents.disabled-stage/reaper-migration-$stamp"
mkdir -p "$disabled_dir"

labels=(
  com.kenichi.stagedaw-recorder-core
  com.kenichi.stagedaw-control
  com.kenichi.stagedaw-livemonitor
  com.kenichi.stagedaw-pluginhost
  com.kenichi.stagedaw-heartbeat
  com.kenichi.stagedaw-local-dashboard
  com.kenichi.stagedaw-tunnel
)

for label in "${labels[@]}"; do
  plist="$HOME/Library/LaunchAgents/$label.plist"
  echo "Retiring $label"
  launchctl bootout "gui/$uid" "$plist" 2>/dev/null || true
  launchctl disable "gui/$uid/$label" 2>/dev/null || true
  if [[ -f "$plist" ]]; then
    mv "$plist" "$disabled_dir/"
  fi
done

echo "StageDAW LaunchAgents moved to:"
echo "$disabled_dir"
echo "AVB helpers were not changed."

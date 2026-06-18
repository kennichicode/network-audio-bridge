#!/bin/zsh
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH:-}"
path=(/usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin $path)
hash -r

BIN="$HOME/NetworkAudioBridge/nab-live"
STATUS_CMD="$HOME/Desktop/NAB Live Status.command"
HEALTH_URL="https://livekit.kenichi-kawabata.com/healthz"
LISTEN_URL="https://livekit.kenichi-kawabata.com/"
RUNTIME_LOCK="$HOME/.nab/locks/nab-live-reaper-master-nab-live-mac-mini.lock"
SOCKET="$HOME/Library/Caches/KenichiNAB/nab-tap.sock"
TARGET_DEVICE="${NAB_PROOF_DEVICE:-iPhone}"
TIMEOUT_SECONDS="${NAB_PROOF_TIMEOUT_SECONDS:-300}"
MAX_PROOF_AGE_SECONDS="${NAB_PROOF_MAX_AGE_SECONDS:-30}"

sender_pids() {
  pgrep -f "$BIN" 2>/dev/null || true
}

finish() {
  local exit_status="${1:-0}"
  read -r "reply?Press Enter to close." || true
  exit "$exit_status"
}

trap 'finish 130' INT TERM

print_health() {
  python3 - "$HEALTH_URL" "$TARGET_DEVICE" "$MAX_PROOF_AGE_SECONDS" <<'PY'
import json
import sys
import urllib.request

url = sys.argv[1]
target_device = sys.argv[2]
max_age = int(sys.argv[3])

try:
    with urllib.request.urlopen(url, timeout=5) as response:
        data = json.loads(response.read().decode("utf-8"))
except Exception as exc:
    print(f"health_error={exc}")
    sys.exit(2)

proof = data.get("lastProof") or {}
if not proof:
    print("proof=none")
    sys.exit(1)

age = int(proof.get("ageSeconds", 999999))
device = proof.get("device", "unknown")
state = proof.get("proof", "unknown")
player = proof.get("player", "unknown")
track = proof.get("track", "unknown")
packets = proof.get("packets", 0)
level = proof.get("level", "unknown")
loss = proof.get("loss", "unknown")

print(f"device={device} proof={state} player={player} age={age}s packets={packets} level={level} loss={loss}")
print(f"track={track}")

if device == target_device and state in ("音あり", "Audio OK") and age <= max_age:
    sys.exit(0)
sys.exit(1)
PY
}

clear 2>/dev/null || true
echo "NAB Live iPhone Check"
echo "====================="
echo ""
echo "Listen page:"
echo "  $LISTEN_URL"
echo ""
echo "Target proof:"
echo "  $TARGET_DEVICE / 音あり"
echo ""

if [[ ! -x "$BIN" ]]; then
  echo "Missing sender:"
  echo "  $BIN"
  read -r "reply?Press Enter to close." || true
  exit 1
fi

if [[ -z "$(sender_pids)" ]]; then
  echo "No NAB Live sender is running."
  echo ""
  echo "For recording safety, this command will NOT start a test tone."
  echo "Open NAB Live Sender.command and use the real REAPER/12Mic audio."
  finish 2
else
  echo "NAB Live sender is already running. Using the current sender."
fi

echo ""
echo "On iPhone:"
echo "  1. Open $LISTEN_URL"
echo "  2. Enter the passcode"
echo "  3. Tap Listen"
echo "  4. If needed, tap Unlock Audio"
echo ""
echo "Waiting for $TARGET_DEVICE / 音あり proof..."
echo ""

deadline=$(( SECONDS + TIMEOUT_SECONDS ))
while (( SECONDS < deadline )); do
  if output="$(print_health 2>&1)"; then
    echo "$output"
    echo ""
    echo "PASS: $TARGET_DEVICE is receiving and playing audio."
    echo ""
    if [[ -x "$STATUS_CMD" ]]; then
      printf "\n" | "$STATUS_CMD" || true
    fi
    finish 0
  fi

  echo "$output"
  sleep 5
done

echo ""
echo "TIMEOUT: Did not see $TARGET_DEVICE / 音あり within ${TIMEOUT_SECONDS}s."
echo ""
echo "Open NAB Live Status.command and check ListenerProof."
finish 2

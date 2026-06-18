#!/bin/zsh
set -euo pipefail

BIN="$HOME/NetworkAudioBridge/nab-live"
TEST_TONE="$HOME/Desktop/NAB Live Test Tone.command"
STATUS_CMD="$HOME/Desktop/NAB Live Status.command"
HEALTH_URL="https://livekit.kenichi-kawabata.com/healthz"
LISTEN_URL="https://livekit.kenichi-kawabata.com/"
RUNTIME_LOCK="$HOME/.nab/locks/nab-live-reaper-master-nab-live-mac-mini.lock"
SOCKET="$HOME/Library/Caches/KenichiNAB/nab-tap.sock"
TARGET_DEVICE="${NAB_PROOF_DEVICE:-iPhone}"
TIMEOUT_SECONDS="${NAB_PROOF_TIMEOUT_SECONDS:-300}"
MAX_PROOF_AGE_SECONDS="${NAB_PROOF_MAX_AGE_SECONDS:-30}"
STARTED_TEST_TONE=0

sender_pids() {
  pgrep -f "$BIN" 2>/dev/null || true
}

stop_started_test_tone() {
  if (( STARTED_TEST_TONE != 1 )); then
    return
  fi

  local pids_text
  local pids
  pids_text="$(sender_pids)"
  if [[ -n "$pids_text" ]]; then
    pids=("${(@f)pids_text}")
    echo ""
    echo "Stopping the test tone started by this check: ${pids[*]}"
    kill "${pids[@]}" 2>/dev/null || true
    sleep 1
  fi

  pids_text="$(sender_pids)"
  if [[ -n "$pids_text" ]]; then
    pids=("${(@f)pids_text}")
    echo "Force stopping leftover test tone: ${pids[*]}"
    kill -9 "${pids[@]}" 2>/dev/null || true
    sleep 1
  fi

  rm -f "$SOCKET" "$SOCKET.lock" "$RUNTIME_LOCK"
}

finish() {
  local exit_status="${1:-0}"
  stop_started_test_tone
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

if device == target_device and state == "Audio OK" and age <= max_age:
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
echo "  $TARGET_DEVICE / Audio OK"
echo ""

if [[ ! -x "$BIN" ]]; then
  echo "Missing sender:"
  echo "  $BIN"
  read -r "reply?Press Enter to close." || true
  exit 1
fi

if [[ ! -x "$TEST_TONE" ]]; then
  echo "Missing test tone command:"
  echo "  $TEST_TONE"
  read -r "reply?Press Enter to close." || true
  exit 1
fi

if [[ -z "$(sender_pids)" ]]; then
  echo "No NAB Live sender is running."
  echo "Starting test tone so the iPhone can listen now..."
  NAB_TEST_TONE_SECONDS="$(( TIMEOUT_SECONDS + 120 ))" nohup "$TEST_TONE" > "$HOME/.nab/iphone-check-test-tone.log" 2>&1 < /dev/null &
  STARTED_TEST_TONE=1
  sleep 8
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
echo "Waiting for $TARGET_DEVICE / Audio OK proof..."
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
echo "TIMEOUT: Did not see $TARGET_DEVICE / Audio OK within ${TIMEOUT_SECONDS}s."
echo ""
echo "Open NAB Live Status.command and check ListenerProof."
finish 2

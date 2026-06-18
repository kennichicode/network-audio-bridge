#!/bin/zsh
set -euo pipefail

STATUS_FILE="$HOME/.nab/status.json"

clear
echo "NAB Live Status"
echo "==============="
echo ""

if ! pgrep -f "$HOME/NetworkAudioBridge/nab-live" >/dev/null 2>&1; then
  echo "NAB Live Sender is not running."
  echo ""
  echo "Normal order:"
  echo "  1. Open REAPER."
  echo "  2. Confirm NAB Tap is on Master FX or Monitor FX."
  echo "  3. Open NAB Live Sender.command."
  echo "  4. Listen at https://livekit.kenichi-kawabata.com/"
  echo ""
  read -r "reply?Press Enter to close."
  exit 0
fi

if [[ ! -f "$STATUS_FILE" ]]; then
  echo "NAB Live Sender is running, but no status file exists yet:"
  echo "  $STATUS_FILE"
  echo ""
  echo "Restart NAB Live Sender.command once after the latest update."
  read -r "reply?Press Enter to close."
  exit 1
fi

if command -v python3 >/dev/null 2>&1; then
  python3 - "$STATUS_FILE" <<'PY'
import json
import sys
import time
from pathlib import Path

path = Path(sys.argv[1])
data = json.loads(path.read_text())
age = max(0.0, time.time() - (data.get("updated_at_unix_ms", 0) / 1000))

print(f"State      : {data.get('connection')}  (updated {age:.1f}s ago)")
print(f"Source     : {data.get('source')}")
print(f"Input      : {data.get('input_rate_hz')} Hz")
print(f"Output     : {data.get('output_rate_hz')} Hz WebRTC audio")
print(f"Send to    : {data.get('livekit_url')}")
print(f"Room       : {data.get('room')}")
print(f"Identity   : {data.get('identity')}")
print(f"Listen     : {data.get('listen_url')}")
print("")
print(f"Captured   : {data.get('captured_frames')} frames")
print(f"Sent       : {data.get('sent_frames')} frames")
print(f"Buffer     : {data.get('buffer_ms')} ms")
print(f"Tap packets: {data.get('tap_packets')}")
print(f"Peak       : L {data.get('peak_left_milli')}/1000  R {data.get('peak_right_milli')}/1000")
print("")
print(f"Problems   : overflow={data.get('overflow_frames')} underrun={data.get('underruns')} inputErr={data.get('input_errors')} livekitErr={data.get('livekit_errors')} reconnect={data.get('reconnects')}")
print("")

if data.get("connection") == "Connected" and data.get("sent_frames", 0) > 0:
    print("Result     : sending to LiveKit now.")
else:
    print("Result     : not confirmed yet. Keep NAB Live Sender open and check REAPER/NAB Tap.")
PY
else
  echo "python3 not found. Raw status:"
  cat "$STATUS_FILE"
fi

echo ""
read -r "reply?Press Enter to close."

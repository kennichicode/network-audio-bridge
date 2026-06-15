# NAB Tap

NAB Tap is a minimal REAPER master-bus tap for `nab-live`.

It is a pass-through JUCE audio plugin. The plugin does not encode Opus, connect to
LiveKit, or perform network reconnect logic. In the audio callback it only copies
stereo samples into a preallocated local ring buffer. A background thread forwards
small PCM packets to `nab-live` over `/tmp/nab-tap.sock`.

Run `nab-live --source plugin` first, then insert `NAB Tap` on REAPER Master FX or
Monitor FX.

Build:

```bash
cmake -S plugins/nab-tap -B plugins/nab-tap/build -G Ninja
cmake --build plugins/nab-tap/build --config Release
```

The default JUCE path is:

```text
/Users/kenichikawabata/Documents/Claude/StageDAW-Recorder/JUCE
```

Override it with:

```bash
cmake -S plugins/nab-tap -B plugins/nab-tap/build -DNAB_JUCE_PATH=/path/to/JUCE
```

# AudioZero 0.0.1 — Mac VST3 compatibility test

1. Install Xcode and CMake; run `bash scripts/build-mac.sh`.
2. Launch AudioZero.app (locally compiled, unsigned).
3. Click **Load VST3** and select the plugin bundle from `/Library/Audio/Plug-Ins/VST3` or `~/Library/Audio/Plug-Ins/VST3`.
4. Open the plugin editor; confirm the UI renders and responds.
5. For an instrument such as Kick Me, connect a MIDI controller. Enable it as the system's default MIDI input if needed; the host listens for enabled MIDI inputs.
6. For effects such as GrainBrain or The Blucher, note: this spike has no audio-file playback or audio input routing UI yet. Full effect processing validation is a follow-up.
7. Save state, adjust parameters, restore state, and verify recall.

## Record results

| Plugin | Loads | Editor | MIDI/audio | State recall | Notes |
| --- | --- | --- | --- | --- | --- |
| Kick Me | | | | | |
| GrainBrain | | | | | |
| The Blucher | | | | | |
| Third-party VST3 | | | | | |

## Known limitations

- Proof of concept only: no tracks, timeline, plugin scanner database, or offline rendering.
- The host currently uses a single plugin instance.
- The state file is a single test file and can be overwritten between plugins.
- Third-party plugin crashes can crash the host; sandboxing is planned.
- No macOS signing/notarization yet.
- MIDI input enabling and audio input/clip playback need dedicated UI in the next iteration.

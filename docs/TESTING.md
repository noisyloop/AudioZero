# AudioZero manual test checklist

Automated tests run with `ctest` on every platform:

- **CoreTests:** the Operator synth, built-in effects, engine, mute/solo, audio clips, looping, save/load, undo, MIDI import and WAV export.
- **OperatorVST3Hosting:** loads the Operator VST3 through the AudioZero host.

The steps below cover what needs a person, real audio hardware or your own plugins.

## Arrangement and Operator

1. Launch AudioZero. The demo song opens. Press **Space** and confirm you hear audio and the playhead moves.
2. Double-click an empty lane on the "Pad" track. A clip appears and the Piano Roll opens. Click to add notes.
3. Drag a clip to another bar and onto another track. Then press **Cmd/Ctrl+Z**.
4. Click the "Chords" track. In the Devices tab, change the Operator preset and algorithm while it plays.
5. Press **A-K** on the computer keyboard to play the selected track.

## Your plugins (Kick Me, GrainBrain, The Blucher, third-party)

1. Drag the `.vst3` bundle from `/Library/Audio/Plug-Ins/VST3`, `~/Library/Audio/Plug-Ins/VST3` or `C:\Program Files\Common Files\VST3` onto a track.
2. Instrument: draw notes in a clip and press play. Effect: it should process the track's audio, for example on the Perc Loop track.
3. Click **Open Editor** on the device card. Confirm the UI renders and responds.
4. Save the project, reopen it (**File > Open**) and confirm the plugin settings came back.

| Plugin | Loads | Editor | MIDI/audio | State recall | Notes |
| --- | --- | --- | --- | --- | --- |
| Kick Me | | | | | |
| GrainBrain | | | | | |
| The Blucher | | | | | |
| Third-party VST3 | | | | | |

## Known limitations

- Plugins run in the same process, so a plugin crash takes down the app.
- There's no audio input recording yet.
- The macOS and Windows builds are unsigned.

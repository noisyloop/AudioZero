# Game controller (PS4 DualShock 4)

A game controller plays AudioZero like a keyboard: press a button, hear a note. It is read locally
through the Linux input system (evdev, via the `gilrs` crate). No driver, MIDI port, root, cloud
service or external program is involved.

Code: `crates/audiozero-controller` (device reading, mapping, profile, diagnostic tool) and
`crates/audiozero-app/src/gui/controller.rs` (the window and playing the notes).

## Setup (Linux)

- **Build:** needs the libudev headers, as well as the ALSA ones: `libudev-dev` (Debian/Ubuntu) or
  `systemd-libs` (Arch, normally already installed).
- **Connect:** plug the DualShock 4 in with a USB **data** cable (some cables only charge). The
  kernel's `hid-sony` / `hid-playstation` driver does the rest. Bluetooth should work the same way
  once paired (`bluetoothctl`, hold Share + PS until the light bar flashes), but it has not been
  tried.
- **Permissions:** normal desktop logins can read the controller with no setup. If
  `audiozero-controller list` says it can't read input devices, check `ls -l /dev/input/event*`:
  the devices should be readable by your user, or by a group you're in (usually `input`). Add
  yourself with `sudo usermod -aG input $USER`, then log out and in. Don't run AudioZero as root.

## Playing

1. Open AudioZero and plug the controller in (before or after; it is noticed either way).
2. Select or arm a track that has an instrument. Notes go to the same track the computer keyboard
   (A-K) plays. **Audio > Controller...** shows the status.
3. Press buttons.

| Controller | Does |
|---|---|
| Cross, Circle, Square, Triangle, D-pad left / up / right / down | The song's scale from its root: C D E F G A B C in C major. It follows the song's key and scale. Hold several for chords. |
| L1 / R1 | Octave down / up (held notes are released) |
| L2 | Velocity of the next notes (not pulled = the Velocity setting) |
| Options | Play / stop |
| Share | Record (arm a track first) |
| Left stick X / Y, right stick X / Y, R2 | Pitch bend, expression, filter cutoff, resonance, modulation |

On a drum rack the same buttons play pads from the kick up.

**Not yet:** the stick and R2 controls are dead-zoned, smoothed and shown in the monitor as MIDI
(pitch bend, CC 11 / 74 / 71 / 1), but the engine has no live control for them yet, so they don't
change the sound. Drum and chord pad modes, remapping buttons in the window (the profile file can
already do it), and other controllers' extras (touchpad, gyro, lights, rumble) are later stages.

Settings (on / off, octave, velocity, dead zone, smoothing, channel, note buttons) are saved in
`controller.json` next to AudioZero's other settings. Notes still held are released when the
controller is unplugged, switched off, or the target track changes. The "MIDI channel" only labels
the monitor's messages; notes go straight to the track.

## Diagnostic tool

No window, no sound:

```
cargo run -p audiozero-controller -- list      # controllers the system sees
cargo run -p audiozero-controller -- monitor   # press things; see events and the MIDI they make
cargo run -p audiozero-controller -- test      # scripted performance, no controller needed
```

Options: `--key A --scale minor --channel 2 --octave 3`. `test` checks note on / off, velocity,
octave change, pitch bend, control changes and that nothing is left hanging after a disconnect. It
tests the mapping, not a physical controller.

## Troubleshooting

- **"No controllers found":** try another cable, then `ls /dev/input/by-id/`. The DS4 should be
  listed as a joystick.
- **Buttons show in the window but nothing sounds:** no track with an instrument is selected or
  armed.
- **Wrong buttons (a clone pad):** run `monitor` and compare. Other pads map onto the same names.

# Keyestra Demo Assets

This directory contains the reproducible MIDI performance and screenshots used
by the project README.

## Generate sample performance data

Start Keyestra with a virtual MIDI port named `Keyestra MIDI`, open the
Monitor, then run:

```powershell
cargo run --example send_demo_midi
```

Pass a different output-port substring when needed:

```powershell
cargo run --example send_demo_midi -- "another MIDI port"
```

The example sends a deterministic 20-chord phrase through the normal MIDI
path. It includes:

- gradual dynamic growth and release;
- small velocity differences between voices in each chord;
- varied note-on spacing, chord duration, and release velocity;
- sustain-pedal transitions;
- a final chord centered around `mf`, rather than a uniform maximum velocity.

Because the sequence is deterministic, screenshots remain comparable between
UI revisions without looking mechanically uniform.

## README screenshots

The current screenshots are:

- `screenshots/keyestra-monitor.png` — the primary performance dashboard;
- `screenshots/keyestra-recorder.png` — the expanded rolling Recorder and clip
  editor;
- `screenshots/keyestra-mobile-live-performance.png` — phone-sized velocity,
  chord, and metronome controls;
- `screenshots/keyestra-mobile-rhythm-practice.png` — a completed rhythm
  practice round;
- `screenshots/keyestra-mobile-clip-editor.png` — phone-sized clip selection,
  preview, and save controls;
- `screenshots/keyestra-mobile-remote.png` — the composed **Perform · Practice ·
  Capture** image used near the top of the main README.

To refresh them:

1. Start from a fresh Monitor instance when exact event counts matter.
2. Run `cargo run --example send_demo_midi`.
3. Capture the desktop Monitor after the final chord.
4. For the Recorder image, expand **编辑片段**, select the **30 秒** view, and
   include the overview, detail timeline, boundary controls, preview, and save
   actions.
5. Replace the images in `demo/screenshots/` without changing their filenames.

### Mobile README showcase

Use a `430 × 932` browser viewport for the three phone screenshots:

1. **Live performance:** place the Metronome at the top of the viewport and
   include current feedback, recent velocity, and the current chord.
2. **Rhythm practice:** choose four rounds, start practice, wait through the
   count-in, and send the demo phrase. Capture the completed feedback together
   with the metronome.
3. **Clip editor:** expand **编辑片段**, choose **30 秒**, and frame the overview,
   detailed selection, boundary controls, preview, and save button.

`mobile-showcase.html` composes the three phone captures into the README hero.
Open it at a `2880 × 1440` viewport to render the 2× asset, then replace
`screenshots/keyestra-mobile-remote.png` after refreshing any source image.

These are documentation fixtures, not prerecorded MIDI files. Running the
example adds events to the active Monitor's in-memory rolling buffer but does
not save a `.mid` file.

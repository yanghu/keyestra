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
  editor.

To refresh them:

1. Start from a fresh Monitor instance when exact event counts matter.
2. Run `cargo run --example send_demo_midi`.
3. Capture the desktop Monitor after the final chord.
4. For the Recorder image, expand **编辑片段**, select the **30 秒** view, and
   include the overview, detail timeline, boundary controls, preview, and save
   actions.
5. Replace the images in `demo/screenshots/` without changing their filenames.

These are documentation fixtures, not prerecorded MIDI files. Running the
example adds events to the active Monitor's in-memory rolling buffer but does
not save a `.mid` file.

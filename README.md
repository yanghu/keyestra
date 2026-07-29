# Keyestra

**Digital Piano Companion**

Keyestra is an open-source companion for digital pianos. It shapes MIDI
velocity, preserves the rest of the performance stream, reconnects devices,
monitors playing dynamics and chords, provides a metronome, and keeps a rolling
MIDI recorder with precise clip preview and export.

It reads MIDI from a hardware input such as a Roland FP-10 and forwards it to an
existing virtual MIDI output such as loopMIDI on Windows or IAC Driver on macOS.

```text
Roland FP-10 -> keyestra -> Keyestra MIDI -> Reaper / DAW / standalone piano
```

The tool does not create virtual MIDI ports. On Windows, create a loopMIDI port first, for example `Keyestra MIDI`.

## Build

```bash
cargo build --release
```

The executable will be at:

```text
target/release/keyestra.exe
```

## List MIDI Ports

```bash
cargo run -- --list
```

## Run

Using names:

```bash
cargo run -- --in "Roland FP-10" --out "Keyestra MIDI" --curve examples/curve.toml
```

Using indexes from `--list`:

```bash
cargo run -- --in 0 --out 0 --curve examples/curve.toml
```

Monitor mode:

```bash
cargo run -- --in "Roland FP-10" --out "Keyestra MIDI" --curve examples/curve.toml --monitor
```

Bypass mode:

```bash
cargo run -- --in "Roland FP-10" --out "Keyestra MIDI" --bypass
```

## Windows Tray Mode

Build the three executables:

```powershell
cargo build --release --bin keyestra --bin keyestra-tray --bin keyestra-monitor
```

`target\release` is Cargo build output, not the permanent installation
directory. For local development only, the tray can be started from there:

```powershell
.\target\release\keyestra-tray.exe
```

The tray app starts `keyestra.exe` in the background with these defaults:

```text
input:  Roland Digital Piano
output: Keyestra MIDI
curve:  examples/curve.toml
```

Right-click the tray icon for:

- Status: Running/Stopped/Waiting
- Start mapper
- Stop mapper
- Restart mapper
- Select the forum curve or mid-control curve; the tray remembers the last selected curve
- Install startup
- Uninstall startup
- Exit

If the MIDI input or output port is not available at login, the tray app stays
open and shows `Status: Waiting`. It checks port availability without launching
the mapper, then starts mapping automatically as soon as the piano and loopMIDI
port are available. If either port disappears while mapping, the tray returns
to standby and reconnects when it comes back.

Unexpected mapper crashes use an exponential retry delay from 5 seconds up to
60 seconds. A mapper that stays up for 30 seconds is considered stable and
resets the delay.
Choosing `Stop mapper` disables retry until you choose `Start mapper` or `Restart mapper`.

## Windows Deployment

Deploy the current build for the current Windows user:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\deploy-windows.ps1
```

The deployment script runs formatting and tests, builds all three release
binaries, and publishes an immutable copy under:

```text
%LOCALAPPDATA%\keyestra\releases\<version>-<build>\
```

It also writes `%LOCALAPPDATA%\keyestra\current.json` and updates Windows
Startup to the newly deployed tray. Deployment does not stop the currently
running mapper, tray, or monitor, so it does not clear the recorder's unsaved
in-memory buffer. The new version becomes active at the next login or after an
explicit activation.

Committed and uncommitted builds are both deployable. Their identities make the
source state explicit:

```text
clean commit:         0.2.0-d137d292
uncommitted preview:  0.2.0-d137d292-dirty-20260728T231500Z
Git ID unavailable:   0.2.0-snapshot-20260728T231500Z
```

The same build identifier is embedded in the binaries, used in the immutable
release directory, written to `current.json`, returned by `/version`, and shown
in the monitor footer. Timestamped preview identities let you deploy and test
multiple uncommitted iterations without overwriting an earlier snapshot.

To deploy and immediately activate a release:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\deploy-windows.ps1 -Activate
```

Activation stops both Keyestra and legacy FP10 mapper, tray, and monitor
processes, then starts the deployed Keyestra tray. It discards any unsaved
rolling recorder buffer, so use it only when that interruption is acceptable.

The three executables remain siblings in every deployed release, and the two
built-in curves are packaged in its `examples` directory. Built-in curve
choices saved by an older tray are migrated to the active deployed release;
an explicit custom curve path remains unchanged.

To roll Startup back to an earlier installed release without interrupting the
currently running version:

```powershell
& "$env:LOCALAPPDATA\keyestra\releases\<old-release>\keyestra-tray.exe" --install-startup
```

Exit the current tray and start that older deployed tray only if the rollback
must take effect immediately. Deployment does not delete prior releases.

Remove current-user Startup by invoking any deployed tray:

```powershell
& "$env:LOCALAPPDATA\keyestra\releases\<release>\keyestra-tray.exe" --uninstall-startup
```

The startup entry is a small script at:

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\keyestra-tray.vbs
```

Logs go to:

```text
%APPDATA%\keyestra\tray.log
```

`tray.log` is capped at 5 MB and retains one rotated backup at `tray.log.1`.

### Migrating from fp10-map

Keyestra uses `Keyestra MIDI` as its default virtual MIDI port. During the
transition it automatically falls back to the legacy `FP10 Mapped` port when
the new port is unavailable.

On first launch, Keyestra copies legacy tray settings and saved `.mid`
recordings from `%APPDATA%\fp10-map` into `%APPDATA%\keyestra`. The old files
are retained as a backup. Installing Keyestra Startup writes
`keyestra-tray.vbs` and removes the legacy `fp10-map-tray.vbs` only after the
new Startup entry has been written.

## Config

Piecewise mode:

```toml
[input]
name = "Roland FP-10"

[output]
name = "Keyestra MIDI"

[mapping]
mode = "piecewise"
points = [
  [0, 0],
  [1, 0],
  [18, 30],
  [32, 57],
  [50, 84],
  [108, 127],
  [127, 127],
]
```

Each point is `[input_velocity, output_velocity]`. The tool builds a 128-value lookup table at startup using linear interpolation between points.

The default `examples/curve.toml` is the FP-10 curve shared on the Pianoteq forum.
`examples/curve-mid-control.toml` is a conservative variant that makes the middle range a little heavier so velocities around `50-80` have more finger-control room.
The tray app can switch between these two curves and restarts the mapper automatically.

Table mode requires exactly 128 values. Index is input velocity, value is output velocity. Velocity `0` is always kept as `0` to avoid stuck notes.

## Monitor Dynamic Bands

The monitor translates played MIDI velocities into practical dynamic working ranges. Each marking has a suggested center, and neighboring ranges overlap so boundary values can display as transitions such as `p/mp` instead of flickering between hard labels.

| Dynamic | Center | Working range |
| --- | ---: | ---: |
| `pp` | 20 | 12-30 |
| `p` | 40 | 28-50 |
| `mp` | 60 | 48-70 |
| `mf` | 80 | 68-90 |
| `f` | 100 | 88-110 |
| `ff` | 120 | 108-127 |

Velocity `0` is treated as note-off-style passthrough and is not shown as a played dynamic.

## Reaper Notes

In Reaper, enable only the virtual mapped input such as `Keyestra MIDI`. Do not enable the original `Roland FP-10` input at the same time, or notes may double-trigger.

If recorded MIDI still has the original velocity, the track is probably listening to the raw FP-10 input instead of the mapped virtual input.

## MIDI Recorder

The monitor server continuously keeps the latest 60 minutes of mapped MIDI in
memory. This includes Note On, Note Off, sustain and other CC messages, pitch
bend, program changes, channel pressure, and SysEx. MIDI clock, active sensing,
and other system real-time messages are intentionally ignored because they do
not describe the piano performance and would make the rolling buffer noisy.

Open the monitor page from the tray menu on a computer or phone to:

- Mark the beginning and end of a take, then save it.
- Save the latest 5 or 15 minutes without having started a take first.
- Expand **编辑片段** to inspect the activity timeline, select an exact range
  with touch-friendly handles, and fine-adjust either boundary.
- Use the full-buffer overview to pan or resize the detailed viewport. On the
  detail timeline, drag to pan, pinch (or Ctrl/Cmd-wheel) to zoom, fit the
  current selection, or return to a live-following view. Starting Follow from
  the full-buffer view uses a two-minute window; otherwise it preserves the
  current zoom level.
- Preview that prepared range through the computer's MIDI sound source and save
  the same range as a standard MIDI file.
- Download any of the 20 most recently saved `.mid` files.

The bottom of the monitor page shows the package version and Git build ID so
desktop and phone clients can confirm which monitor binary is serving the UI.

Recordings are written atomically to:

```text
%APPDATA%\keyestra\recordings
```

The files use a fixed high-resolution timing scale and can be imported into
Reaper or another DAW without quantizing the performance. At the end of each
export, the recorder writes sustain-off and all-notes-off messages on every
channel used by the take to prevent stuck notes during playback.

The rolling buffer currently lives in memory, so restarting the monitor clears
unsaved material. Saved `.mid` files remain on disk.

Preview runs on the computer and defaults to the `Keyestra MIDI` MIDI output.
Rolling capture pauses during preview so playback returning through that
virtual port cannot contaminate the buffer, then resumes shortly after cleanup.
The live Monitor display can still show those messages. Preview is unavailable
while a Take is actively recording.

The detailed boundary and API design is in
[`docs/RECORDER_PREVIEW_DESIGN.md`](docs/RECORDER_PREVIEW_DESIGN.md). A
crash-recoverable rolling journal remains a future enhancement.

## Current Scope

Implemented:

- List MIDI inputs and outputs.
- Select input/output by index or name substring.
- Forward all MIDI messages.
- Map Note On velocity values from `1..=127`.
- Preserve Note On velocity `0`, Note Off, sustain pedal, CC, pitch bend, and all other messages.
- Support curve and table config.
- Support monitor and bypass modes.
- Keep a 60-minute rolling MIDI recording buffer with take and recent-range export.
- Stop cleanly with Ctrl+C.

Future useful additions:

- Device reconnect loop.
- CSV velocity logging for practice analysis.
- Histogram/monitor-only mode.
- Simple GUI curve editor.

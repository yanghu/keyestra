# fp10-map

`fp10-map` is a small cross-platform MIDI velocity mapping forwarder.

It reads MIDI from a hardware input such as a Roland FP-10, maps Note On velocity values, and forwards the result to an existing virtual MIDI output such as loopMIDI on Windows or IAC Driver on macOS.

```text
Roland FP-10 -> fp10-map -> FP10 Mapped -> Reaper / DAW / standalone piano
```

The tool does not create virtual MIDI ports. On Windows, create a loopMIDI port first, for example `FP10 Mapped`.

## Build

```bash
cargo build --release
```

The executable will be at:

```text
target/release/fp10-map.exe
```

## List MIDI Ports

```bash
cargo run -- --list
```

## Run

Using names:

```bash
cargo run -- --in "Roland FP-10" --out "FP10 Mapped" --curve examples/curve.toml
```

Using indexes from `--list`:

```bash
cargo run -- --in 0 --out 0 --curve examples/curve.toml
```

Monitor mode:

```bash
cargo run -- --in "Roland FP-10" --out "FP10 Mapped" --curve examples/curve.toml --monitor
```

Bypass mode:

```bash
cargo run -- --in "Roland FP-10" --out "FP10 Mapped" --bypass
```

## Windows Tray Mode

Build both executables:

```bash
cargo build --release --bin fp10-map --bin fp10-map-tray
```

Start the tray app:

```powershell
.\target\release\fp10-map-tray.exe
```

The tray app starts `fp10-map.exe` in the background with these defaults:

```text
input:  Roland Digital Piano
output: FP10 Mapped
curve:  examples/curve.toml
```

Right-click the tray icon for:

- Status: Running/Stopped/Waiting
- Start mapper
- Stop mapper
- Restart mapper
- Install startup
- Uninstall startup
- Exit

If the MIDI input or output port is not available at login, the tray app stays open and shows `Status: Waiting`.
It retries every few seconds until the piano and loopMIDI port are available.
Choosing `Stop mapper` disables retry until you choose `Start mapper` or `Restart mapper`.

Install current-user Windows startup from the command line:

```powershell
.\target\release\fp10-map-tray.exe --install-startup
```

Remove startup:

```powershell
.\target\release\fp10-map-tray.exe --uninstall-startup
```

The startup entry is a small script at:

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\fp10-map-tray.vbs
```

Logs go to:

```text
%APPDATA%\fp10-map\tray.log
```

## Config

Piecewise mode:

```toml
[input]
name = "Roland FP-10"

[output]
name = "FP10 Mapped"

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

Table mode requires exactly 128 values. Index is input velocity, value is output velocity. Velocity `0` is always kept as `0` to avoid stuck notes.

## Reaper Notes

In Reaper, enable only the virtual mapped input such as `FP10 Mapped`. Do not enable the original `Roland FP-10` input at the same time, or notes may double-trigger.

If recorded MIDI still has the original velocity, the track is probably listening to the raw FP-10 input instead of the mapped virtual input.

## Current Scope

Implemented:

- List MIDI inputs and outputs.
- Select input/output by index or name substring.
- Forward all MIDI messages.
- Map Note On velocity values from `1..=127`.
- Preserve Note On velocity `0`, Note Off, sustain pedal, CC, pitch bend, and all other messages.
- Support curve and table config.
- Support monitor and bypass modes.
- Stop cleanly with Ctrl+C.

Future useful additions:

- Device reconnect loop.
- CSV velocity logging for practice analysis.
- Histogram/monitor-only mode.
- Tray app or simple GUI curve editor.

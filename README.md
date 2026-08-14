<p align="center">
  <img src="assets/icons/keyestra-app.svg" width="112" height="112" alt="Keyestra icon">
</p>

<h1 align="center">Keyestra</h1>

<p align="center">
  English · <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <strong>Digital Piano Companion</strong><br>
  Shape your piano's response, understand your playing, and capture performances
  from the computer or your phone.
</p>

Keyestra sits between a digital piano and the virtual piano instrument or DAW
you already use. It reshapes Note On velocity while preserving the rest of the
MIDI performance, then turns the mapped stream into live feedback, rhythm
practice, and recoverable recordings.

![Keyestra mobile remote showing live velocity feedback, rhythm practice, and clip export](demo/screenshots/keyestra-mobile-remote.png)

## Why Keyestra?

Digital pianos and software instruments do not always agree about how touch
should translate into dynamics. Keyestra gives you a configurable velocity
curve without sacrificing sustain, pitch bend, controllers, or other MIDI
messages.

![Signal flow from a digital piano through Keyestra to virtual piano instruments, a DAW, and the Monitor](docs/keyestra-signal-flow.svg)

Keyestra includes both the MIDI mapper and the browser-based performance
companion. The existing virtual port carries the mapped performance to virtual
piano instruments or DAWs and back into Keyestra's Monitor, rhythm practice,
and rolling recorder.

Keyestra does not create virtual MIDI ports. On Windows, create a loopMIDI port
named `Keyestra MIDI`. On macOS, use an IAC Driver port or another existing
virtual MIDI port.

## Highlights

- **Expressive velocity shaping** — use a piecewise curve or an exact 128-value
  table to match your piano to your preferred instrument.
- **MIDI-safe routing** — Note Off, sustain, CC, pitch bend, program changes,
  channel pressure, SysEx, and unknown messages pass through unchanged.
- **Phone remote** — view dynamics and chords, control the metronome, practice
  rhythm, and manage recordings without keeping the computer in front of you.
- **Rolling recorder** — recover the latest performance, mark Takes, select a
  precise clip, preview it, and export a standard MIDI file.
- **One-click CFX rendering** — turn a saved MIDI performance into a compact
  MP3 or lossless WAV through your REAPER and Garritan CFX templates, with the
  templates controlling normalization, true-peak limits, and render quality.
- **Resilient Windows Tray** — wait for missing devices, reconnect
  automatically, supervise the mapper, and remember the selected curve.

## Requirements

The complete Tray and deployment workflow is currently Windows-focused. To try
Keyestra from this repository, you need:

- a digital piano or MIDI keyboard;
- a virtual MIDI port such as loopMIDI;
- a software instrument or DAW that can receive the mapped MIDI port; and
- a Rust toolchain with `cargo`.

For phone access, connect the phone and computer to the same trusted local
network.

## Quick start from source

### 1. Create the virtual port

Create a virtual MIDI port named:

```text
Keyestra MIDI
```

### 2. Confirm your MIDI port names

```powershell
cargo run -- --list
```

Port selectors accept either a numeric index or a case-insensitive name
substring.

### 3. Build and start the Windows Tray

```powershell
cargo build --release `
  --bin keyestra `
  --bin keyestra-tray `
  --bin keyestra-monitor

.\target\release\keyestra-tray.exe --in "Roland FP-10"
```

Replace `Roland FP-10` with a distinctive part of your piano's MIDI input name.
The Tray uses `Keyestra MIDI` as the mapped output and Monitor input.

This runs directly from Cargo output for evaluation. For a permanent
current-user installation and Windows Startup integration, follow
[`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md).

### 4. Connect your instrument or DAW

In Pianoteq, Reaper, or another instrument, enable the mapped `Keyestra MIDI`
input. Disable the raw piano input for that track, or each note may arrive
twice.

The Monitor is available on the computer at:

```text
http://localhost:8770
```

## Use your phone as the remote

Connect the phone to the same local network as the computer and open:

```text
http://<computer-ip>:8770
```

Use `ipconfig` on Windows to find the computer's IPv4 address. No phone app is
required. If Windows asks for network permission, allow Keyestra only on
trusted private networks.

The phone layout is organized around three jobs:

- **Perform** — see the current chord, per-note velocity, dynamic range,
  performance history, and metronome.
- **Practice** — choose the tempo, notes per beat, and number of rounds, then
  get immediate early/late feedback and round summaries.
- **Capture** — mark a Take, save a recent range, or select and preview an exact
  section before exporting it.

The separate **Piano** tab can control a local Pianoteq instance. Start
Pianoteq with its JSON-RPC server bound to localhost:

```powershell
& "C:\Program Files\Modartt\Pianoteq 9\Pianoteq 9.exe" --serve 127.0.0.1:8081
```

Keyestra groups presets by instrument, prioritizes licensed instruments, and
stores favorite preset buttons in the phone browser. The Piano tab also offers
touch-friendly controls for volume, reverb amount, room size, reverb on/off,
and dynamics. These controls change the live sound but do not save a new
preset. Pianoteq's RPC server stays private to the PC; only the existing
Keyestra Monitor port is exposed to the local network. Use `keyestra-monitor
--pianoteq-rpc <host:port>` only if Pianoteq uses a different local address.

Free KIViR and Bells/Carillons instruments are shown with a **Free** badge once
they have been downloaded from the Modartt user area and installed in
Pianoteq. They are grouped with usable instruments rather than unlicensed demo
models.

### Compare pianos hosted in REAPER

The **Piano** tab can also switch among preloaded REAPER instrument tracks,
including sampled pianos such as Garritan CFX and Pianoteq models hosted as
plug-ins. The selector uses REAPER's built-in web interface locally, confirms
the resulting mute state, and includes a persistent one-tap A/B toggle. Direct
Pianoteq RPC control remains available in the same tab.

See [REAPER Piano Compare setup](docs/REAPER_PIANO_COMPARE.md) for the REAPER
bootstrap ReaScript, one-time plug-in confirmation boundary, project layout,
web-interface setup, configurable piano list, and repeatable LUFS-I loudness
calibration. It also includes a fixed-MIDI MAESTRO benchmark project for
repeatable listening comparisons that do not depend on live performance.

## Daily use from the Tray

Right-click the Tray icon to:

- inspect mapper and Monitor status;
- start, stop, or restart mapping;
- switch between the built-in velocity curves;
- open the Monitor;
- manage Windows Startup; or
- exit Keyestra.

If a required MIDI port is missing, the Tray remains available in **Waiting**
state and starts mapping when the devices return.

The default Tray settings are:

```text
input:   Roland Digital Piano
output:  Keyestra MIDI
curve:   examples/curve.toml
monitor: http://localhost:8770
```

## Rhythm practice

On a phone-width Monitor, **节奏练习** appears above the regular metronome. Set
the BPM, choose 2, 3, or 4 notes per beat, select the number of four-beat
rounds, and start when ready.

The first bar is an unscored count-in. During the exercise, the Monitor shows:

- whether recent attacks are early or late;
- median timing offset and timing spread;
- hit rate and missed or extra attacks; and
- a summary for each completed round.

Pausing preserves completed work. Resuming begins with another one-bar
count-in.

## Recording and clip export

![Keyestra rolling MIDI recorder and clip editor](demo/screenshots/keyestra-recorder.png)

The Recorder continuously keeps the latest 60 minutes of mapped MIDI in memory.
You can:

- mark and save a Take;
- save the latest 5 or 15 minutes immediately;
- inspect 30-second, 2-minute, 5-minute, or complete-buffer views;
- pan and zoom the timeline;
- drag or nudge the selection boundaries;
- preview the selected range through a MIDI output; and
- save the selection as a standard `.mid` file.

Saved recordings are stored in:

```text
%APPDATA%\keyestra\recordings
```

> The rolling buffer is memory-backed. Restarting the Monitor permanently
> clears unsaved material. Saved `.mid` files remain on disk.

Preview temporarily pauses rolling capture so playback returning through the
virtual port cannot be recorded into the buffer.

### Render saved MIDI with REAPER and Garritan CFX

On Windows, each saved MIDI row can render an MP3 for everyday listening or a
lossless WAV through local REAPER project templates. Keyestra automatically
looks for:

```text
%APPDATA%\REAPER\ProjectTemplates\Keyestra CFX Render.rpp
%APPDATA%\REAPER\ProjectTemplates\Keyestra CFX Render MP3.rpp
```

The template must contain a track named `CFX Render` with the CFX Concert Grand
VSTi. Each template's saved REAPER render settings determine the output format,
quality, normalization, peak ceiling, and render tail. A practical MP3 template
uses 320 kbps, -18 LUFS-I normalization, and a -1 dB true-peak ceiling. Keep the
WAV template for lossless exports and later editing.

Click **Generate MP3** or **Generate WAV** beside a saved MIDI. Rendering runs
in a single-file background queue so the Monitor and rolling recorder remain
available. The finished file is stored beside the MIDI as `*_natural.mp3` or
`*_natural.wav`; the source MIDI and reusable templates are not modified.

Keyestra uses the normal REAPER installation at
`C:\Program Files\REAPER (x64)\reaper.exe`. Custom REAPER or template locations
can be supplied with the Monitor options documented in [`docs/CLI.md`](docs/CLI.md).

## Velocity curves

The default curve is [`examples/curve.toml`](examples/curve.toml):

```toml
[input]
name = "Roland Digital Piano"

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

Each point is `[input_velocity, output_velocity]`. Keyestra interpolates the
points into its velocity lookup table at startup. Velocity `0` is always kept
at `0`, because Note On with velocity `0` is commonly used as Note Off.

[`examples/curve-mid-control.toml`](examples/curve-mid-control.toml) keeps more
finger-control room in the middle velocities. The Tray can switch between both
built-in curves. Exact table mode is demonstrated in
[`examples/table.toml`](examples/table.toml).

## Advanced CLI use

The mapper and Monitor can run without the Tray. See
[`docs/CLI.md`](docs/CLI.md) for port discovery, direct routing, bypass mode,
terminal monitoring, and Monitor host/port options.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| Tray shows **Waiting** | Confirm that the piano and `Keyestra MIDI` ports exist and that the configured name matches. |
| Notes play twice | Disable the raw piano input in the instrument or DAW; listen only to `Keyestra MIDI`. |
| Phone cannot open the Monitor | Confirm both devices are on the same network, use the computer's IPv4 address, and allow access on private networks. |
| Metronome is silent | Open **Settings** in the Monitor and choose the intended audio output. |
| An unsaved performance disappeared | Restarting the Monitor clears the memory-backed rolling buffer; saved MIDI files remain in the recordings directory. |

## Documentation

- [简体中文说明](README.zh-CN.md)
- [Windows deployment and rollback](docs/DEPLOYMENT.md)
- [Command-line usage](docs/CLI.md)
- [Recorder and clip-editor design](docs/RECORDER_PREVIEW_DESIGN.md)
- [Demo data and screenshot workflow](demo/README.md)
- [Contributing and validation](CONTRIBUTING.md)

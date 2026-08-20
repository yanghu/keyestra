# Command-line usage

The `keyestra` binary routes MIDI and applies the selected velocity mapping.
The `keyestra-monitor` binary provides the browser dashboard and rolling
recorder. The Windows Tray normally supervises both.

## List MIDI ports

```powershell
cargo run -- --list
```

Port selectors accept either a numeric index or a case-insensitive name
substring.

## Run the mapper

```powershell
cargo run -- `
  --in "Clavinova-1" `
  --out "Keyestra MIDI" `
  --curve examples/curve.toml
```

Print mapped Note On and passthrough activity:

```powershell
cargo run -- --in "Clavinova-1" --out "Keyestra MIDI" `
  --curve examples/curve.toml --monitor
```

Route all MIDI without velocity mapping:

```powershell
cargo run -- --in "Clavinova-1" --out "Keyestra MIDI" --bypass
```

## Run the web Monitor separately

```powershell
cargo run --bin keyestra-monitor -- `
  --in "Keyestra Output" `
  --piano-out "Clavinova-1" `
  --port 8770
```

The Monitor listens on `0.0.0.0` by default so another device on the same local
network can connect. To restrict it to the current computer:

```powershell
cargo run --bin keyestra-monitor -- `
  --in "Keyestra Output" `
  --piano-out "Clavinova-1" `
  --host 127.0.0.1 `
  --port 8770
```

## Override REAPER/CFX rendering paths

The Monitor normally discovers REAPER plus the `Keyestra CFX Render.rpp` WAV
template and `Keyestra CFX Render MP3.rpp` MP3 template in their standard
Windows locations. Override any path when using a portable REAPER installation
or differently named templates:

```powershell
cargo run --bin keyestra-monitor -- `
  --in "Keyestra Output" `
  --reaper "D:\Audio\REAPER\reaper.exe" `
  --cfx-template "D:\Audio\Templates\Keyestra CFX Render.rpp" `
  --cfx-mp3-template "D:\Audio\Templates\Keyestra CFX Render MP3.rpp"
```

The template must contain a track named `CFX Render` and the CFX Concert Grand
VSTi. Keyestra copies the selected template's settings into a per-recording
render project; it never edits either source template. MP3 is the API default;
the Monitor UI offers both MP3 and WAV for every saved MIDI.

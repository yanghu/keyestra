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
  --in "Roland FP-10" `
  --out "Keyestra MIDI" `
  --curve examples/curve.toml
```

Print mapped Note On and passthrough activity:

```powershell
cargo run -- --in "Roland FP-10" --out "Keyestra MIDI" `
  --curve examples/curve.toml --monitor
```

Route all MIDI without velocity mapping:

```powershell
cargo run -- --in "Roland FP-10" --out "Keyestra MIDI" --bypass
```

## Run the web Monitor separately

```powershell
cargo run --bin keyestra-monitor -- `
  --in "Keyestra MIDI" `
  --port 8770
```

The Monitor listens on `0.0.0.0` by default so another device on the same local
network can connect. To restrict it to the current computer:

```powershell
cargo run --bin keyestra-monitor -- `
  --in "Keyestra MIDI" `
  --host 127.0.0.1 `
  --port 8770
```


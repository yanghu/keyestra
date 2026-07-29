# Contributing to Keyestra

Keyestra is a Rust application with three binaries:

- `keyestra` — MIDI routing and velocity mapping;
- `keyestra-tray` — the Windows Tray supervisor; and
- `keyestra-monitor` — the local web Monitor and rolling recorder.

## Validation

For normal code changes, run:

```powershell
cargo fmt --check
cargo test
```

For changes affecting Tray behavior, the Monitor server, binary identity, or
packaging, also run:

```powershell
cargo build --release `
  --bin keyestra `
  --bin keyestra-tray `
  --bin keyestra-monitor
```

Hardware behavior cannot be fully validated without MIDI devices and a virtual
MIDI port. When those are unavailable, verify formatting, compilation, and
tests, and state that live MIDI behavior was not manually tested.

## Demo and screenshot assets

The deterministic MIDI demo and screenshot workflow are documented in
[`demo/README.md`](demo/README.md).

Run the sample performance with:

```powershell
cargo run --example send_demo_midi
```

## Icons

Regenerate icon assets with:

```powershell
python .\scripts\generate-icons.py
```

Brand assets live in [`assets/icons`](assets/icons), including SVG masters,
transparent PNGs, the runtime Tray RGBA payload, and the multi-resolution
Windows `.ico`.

## Deployment

See [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) for immutable Windows releases,
activation, release identity, Startup, and rollback.


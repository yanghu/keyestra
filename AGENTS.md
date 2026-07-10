# AGENTS.md

This is the agent-facing maintenance note. Put user instructions in `README.md`;
keep this file short and focused on things an agent might otherwise miss.

## Validation Notes

For normal code changes, run:

```powershell
cargo fmt --check
cargo test
```

For changes touching tray behavior, monitor server behavior, binary names, or
release packaging assumptions, also run:

```powershell
cargo build --release --bin fp10-map --bin fp10-map-tray --bin fp10-monitor-server
```

Hardware behavior cannot be fully validated without MIDI devices and a virtual
MIDI port. If no MIDI hardware is available, verify compile/tests and clearly
say that live MIDI behavior was not manually tested.

## MIDI Invariants

- Preserve MIDI messages other than Note On velocity mapping. Note Off, sustain
  pedal, CC, pitch bend, active sensing, and unknown messages should pass through.
- Treat Note On with velocity `0` as passthrough. It is commonly used as Note Off
  and should not be remapped.
- Keep velocity values in `0..=127`. Velocity table mode must contain exactly
  128 values; index `0` is always forced to `0`.
- Port selection accepts either a numeric index or a case-insensitive name substring.

## Local Assumptions

- The app does not create virtual MIDI ports. Assume the user creates loopMIDI/IAC
  ports outside this project.
- Avoid changing default port names casually; they match the README and likely
  the user's local loopMIDI setup.
- `examples/curve.toml` is the default curve path used by examples and tray fallback.
- The tray app assumes sibling release binaries named `fp10-map.exe` and
  `fp10-monitor-server.exe`.
- Startup install writes `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\fp10-map-tray.vbs`.

## Coding Notes

- Add or update unit tests in `src/mapping.rs` when changing mapping behavior.
- Keep user-facing commands and defaults in sync between `README.md` and code.
- Do not remove `Cargo.lock`; this is an application, not a library-only crate.

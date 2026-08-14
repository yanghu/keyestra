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
cargo build --release --bin keyestra --bin keyestra-tray --bin keyestra-monitor
```

Hardware behavior cannot be fully validated without MIDI devices and a virtual
MIDI port. If no MIDI hardware is available, verify compile/tests and clearly
say that live MIDI behavior was not manually tested.

## Build/Deploy Notes

`target\release` is Cargo output, not an installation directory. Normal local
deployment uses `scripts\deploy-windows.ps1` to publish immutable releases under
`%LOCALAPPDATA%\keyestra\releases`. Startup must point to one of those deployed
versions, never to `target\release`.

Deployment and activation are separate operations. A normal deployment updates
Startup but does not stop the live Keyestra or legacy FP10 processes or clear
the recorder buffer.
Only use `-Activate` when the user explicitly requests immediate activation and
accepts losing the unsaved in-memory recorder buffer.

Legacy installations may still be running from `target\release` and lock Cargo
output. Do not stop them merely to validate a change; ask the user to exit or
explicitly activate a deployed release first. Once a deployed release is
running, future release builds must succeed without stopping it.

## Release Identity

`build.rs` derives the monitor build ID from the current Git commit. The monitor
page footer and `GET /version` combine that ID with the package version from
`Cargo.toml`.

Any artifact described to the user as a new release, deployed locally, or used
by Startup must be built from the intended committed source:

- Clean committed releases use the eight-character Git hash as their build ID.
- Local preview deployments may contain uncommitted changes. Their build ID must
  include the commit hash, `dirty`, and a UTC timestamp so separate previews
  never share an immutable release directory.
- If a Git hash is unavailable, use a timestamped `snapshot` identifier.
- Do not present a dirty, snapshot, or unknown-source build as a clean committed
  release; state its source status explicitly.
- Build all three release binaries after choosing the identity. The embedded
  build ID, release directory, `current.json`, `/version`, and monitor footer
  must agree.
- Before handoff, run the built or deployed monitor on an available port and
  verify that `GET /version` reports the package version and expected build ID.
  For a clean build this is the short Git hash; previews include their dirty,
  snapshot, or unknown-source suffix. Also confirm the served page contains the
  version footer.
- A release-profile build used only for validation is not deployed and must be
  described as validation output.

After every build, release, or deployment requested by the user, include these
items in the final response:

- the absolute path of the relevant tray binary;
- the package version, build ID, and source state;
- the exact footer text the user should see, for example
  `Keyestra v0.2.0 · 9a9460c0`;
- whether `/version` was verified from that exact binary;
- whether the running process and Windows Startup already point to that build,
  or whether it is only built/staged and still needs activation.

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
- `Keyestra MIDI` is the default virtual port. The runtime accepts the legacy
  `FP10 Mapped` name as a migration fallback.
- Avoid changing default port names casually; they match the README and likely
  the user's local loopMIDI setup.
- `examples/curve.toml` is the default curve path used by examples and tray fallback.
- The tray app assumes sibling release binaries named `keyestra.exe` and
  `keyestra-monitor.exe`.
- Deployed releases include `examples\curve.toml` and
  `examples\curve-mid-control.toml` beside the sibling binaries, plus
  `examples\reaper-pianos.toml` as the REAPER Piano Compare config template and
  the bootstrap, calibration, and MAESTRO benchmark scripts under
  `scripts\reaper` for automated setup.
- Startup install writes `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\keyestra-tray.vbs`.
- Legacy settings and recordings are copied from `%APPDATA%\fp10-map` on first
  use; source files are retained.

## Coding Notes

- Add or update unit tests in `src/mapping.rs` when changing mapping behavior.
- Keep user-facing commands and defaults in sync between `README.md` and code.
- Do not remove `Cargo.lock`; this is an application, not a library-only crate.

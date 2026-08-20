# Setup and Release Packaging Plan

Status: proposed future work; not required for current source-based development.

## Goal

Make Keyestra feel like one Windows application even if it continues to use
separate mapper and Monitor processes internally.

For a user, the intended experience is:

1. download and install or extract Keyestra;
2. launch **Keyestra**;
3. choose the piano input and mapped MIDI output;
4. leave Keyestra in the system tray; and
5. reopen **Settings** from the Tray whenever the setup changes.

Users should not need Cargo, command-line arguments, or direct TOML editing.
Those interfaces should remain available for development and advanced use.

## Decisions

### Keep the multi-process architecture

Do not merge the Tray, mapper, and Monitor solely to create one physical
executable. The current process boundaries let the Tray supervise and restart
the mapper and Monitor independently.

Instead, provide one public launcher named `Keyestra.exe`. Treat the mapper and
Monitor executables as private implementation files. A future installer can
still be delivered as one downloadable setup executable.

### Make the Tray the application entry point

`Keyestra.exe` is the only executable users should be instructed to run. It:

- loads and validates saved settings;
- opens first-run setup when no settings exist;
- starts and supervises the mapper and Monitor;
- exposes **Settings** in its menu;
- exposes Monitor, restart, Startup, and exit actions; and
- reports missing devices without exiting.

### Keep the main curve outside application settings

Application settings describe devices and runtime behavior. The supported Tray
mapping is the bundled main CLP `examples/curve.toml`; it is not a user
setting and the UI must not offer alternative curves. The mapper's explicit
`--curve` option remains available only for development and diagnostics.

Explicit command-line arguments should continue to override saved settings:

```text
CLI arguments > saved user settings > built-in defaults
```

## First-run and Settings experience

### First run

Open setup automatically when the user settings file does not exist. The
screen should contain:

- **Piano input** — available MIDI inputs, with a refresh action;
- **Mapped output** — available MIDI outputs;
- **Monitor input** — defaults to `Keyestra Output` and is hidden under
  advanced settings unless changed;
- **Velocity mapping** — read-only confirmation that the main CLP curve is active;
- **Monitor port** — defaults to `8770`;
- **Phone access** — allow the Monitor on the trusted local network; and
- **Start with Windows** — optional and off unless the user selects it.

Setup must be usable when the piano or virtual MIDI port is disconnected.
Unavailable saved devices should be shown as unavailable, not silently
replaced.

Saving valid settings starts or restarts the affected processes. Canceling the
first-run screen should leave the Tray running in an unconfigured state, with
**Open Settings** still available.

### Later changes

Add **Settings…** to the Tray menu. It opens the same screen used for first
run. Changing a value should clearly state what will restart:

- input or output changes restart the mapper;
- Monitor input, port, or phone-access changes restart the Monitor; and
- Startup changes only update Windows login behavior.

Protect the rolling recorder when possible. If a Monitor restart will clear
unsaved material, show a warning and require confirmation before applying that
change.

### Proposed implementation

Reuse the project's browser UI rather than introducing a second native UI
toolkit. The Monitor currently opens its MIDI input before starting its HTTP
server, so this requires separating web-server startup from MIDI connection.

The intended behavior is:

1. the local web UI can start without a valid MIDI device;
2. MIDI connection runs as a reconnectable service;
3. the Tray opens a local settings route such as
   `http://127.0.0.1:8770/settings`; and
4. settings modification is allowed only from the local computer.

Do not expose settings writes to unauthenticated devices on the LAN. Use a
loopback-only administration endpoint or an unguessable, short-lived setup
token supplied by the Tray. The normal performance Monitor may continue to be
available to the phone.

## Persisted settings

Extend the existing file:

```text
%APPDATA%\keyestra\tray-settings.toml
```

A possible user-readable shape is:

```toml
version = 1

[midi]
input = "Clavinova-1"
output = "Keyestra MIDI"
monitor_input = "Keyestra Output"

[monitor]
port = 8770
phone_access = true
```

Persist device names rather than port indexes because indexes can change after
devices reconnect. Continue accepting indexes through the CLI for temporary
and diagnostic use.

Settings writes should be atomic. Invalid or newer unsupported settings must
produce an actionable Tray error and must not overwrite the last valid file.

The legacy settings file may contain a curve choice or custom path. Migration
should ignore those retired fields and use the bundled main CLP curve.

## Release folder layout

The proposed portable or installed layout is:

```text
Keyestra\
├── Keyestra.exe
├── internal\
│   ├── keyestra-mapper.exe
│   └── keyestra-monitor.exe
├── curves\
│   ├── default.toml
│   └── top-linear.toml
├── README.txt
└── LICENSE
```

Only `Keyestra.exe` is a public entry point.

The existing Cargo targets do not have to be renamed immediately. Packaging
can stage:

```text
keyestra-tray.exe    -> Keyestra.exe
keyestra.exe         -> internal\keyestra-mapper.exe
keyestra-monitor.exe -> internal\keyestra-monitor.exe
```

During migration, the Tray should resolve the packaged `internal` paths first
and retain sibling-binary fallbacks for development builds from
`target\release`.

Helper processes should continue to launch without console windows. Error
messages should be surfaced through Tray status and logs rather than requiring
users to run the helpers directly.

## Windows Startup and the current VBS

Running Keyestra manually requires only `Keyestra.exe`.

The current `keyestra-tray.vbs` is an implementation detail for optional
**Start with Windows** behavior. Windows runs files placed in the current
user's Startup folder. The VBS file points to the Tray executable in its
immutable release directory and invokes it with a hidden window:

```text
Startup folder
    -> keyestra-tray.vbs
        -> <installed release>\keyestra-tray.exe
```

This avoids copying the application into the Startup folder and makes it easy
for deployment to repoint the next login to a new immutable release. The Tray
is already built as a Windows GUI application, so the VBS is not needed to
hide a console during ordinary manual launch.

For an early release, it is acceptable to keep generating the VBS behind the
**Start with Windows** checkbox. Users should never need to find, edit, or run
it themselves.

A future installer may replace it with a Start menu/Startup shortcut, a
current-user `Run` registration, or the appropriate packaged-app startup
mechanism. That is a packaging choice, not a reason to change Keyestra's
runtime architecture.

Startup must always target an installed immutable release, never Cargo's
`target\release` directory or an executable left in Downloads.

## Release delivery

The first public artifact can be a portable ZIP with one obvious launcher:

```text
keyestra-v<version>-windows-x64.zip
```

When distribution becomes important, wrap the same folder in a per-user
installer:

```text
Keyestra-Setup-v<version>.exe
```

The installer should:

- require no administrator privileges;
- install under `%LOCALAPPDATA%\keyestra`;
- create one **Keyestra** Start menu entry;
- optionally enable **Start with Windows**;
- preserve settings and saved recordings during upgrades; and
- remove application files cleanly without deleting recordings unless the
  user explicitly requests it.

GitHub Actions should build from a clean version tag, validate all three
binaries, assemble the folder, generate checksums, and attach the ZIP or
installer to a GitHub Release. Cargo remains a requirement only for building
from source.

## Implementation stages

### Stage 1: persistent configuration

- Expand `TraySettings` to cover MIDI and Monitor settings.
- Define migration that ignores the retired curve-only settings.
- Apply CLI-over-settings precedence.
- Add validation and atomic writes.
- Add tests for defaults, migration, precedence, and invalid files.

### Stage 2: setup UI

- Allow the Monitor web server to start without a MIDI connection.
- Add local-only settings APIs and the setup page.
- Add **Settings…** to the Tray menu.
- Open setup on first run.
- Restart only affected components after confirmation.

### Stage 3: application bundle

- Teach the Tray to find helpers under `internal`.
- Retain development-path fallbacks.
- Add a packaging script that stages the release layout.
- Present only `Keyestra.exe` as the user entry point.
- Test paths containing spaces and non-ASCII characters.

### Stage 4: automated release

- Add pull-request and branch CI.
- Add a version-tag release workflow.
- Verify build identity, `/version`, and the Monitor footer from the exact
  staged binaries.
- Publish the ZIP and checksums.
- Add a per-user installer only when release usage justifies it.

## Acceptance criteria

- A fresh Windows user can configure and start Keyestra without a terminal or
  editing TOML.
- The setup screen still opens when required MIDI devices are absent.
- The Tray can reopen settings later.
- Saved settings survive logout, restart, and application upgrade.
- A missing configured device leaves Keyestra in **Waiting** state.
- Users launch only `Keyestra.exe`; helpers remain internal.
- Manual launch does not depend on VBS.
- Optional Start with Windows launches the installed release silently.
- Updating Startup never points to `target\release`.
- Applying Monitor changes warns before discarding unsaved recorder data.
- Uninstalling the application does not silently delete saved recordings.

## Non-goals for the first release

- Combining all services into one runtime process.
- A Microsoft Store listing.
- Automatic in-app updates.
- Hiding advanced TOML and CLI interfaces from developers.
- Supporting every Windows installer technology from the outset.

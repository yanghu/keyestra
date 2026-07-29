# Windows Release Deployment Handoff

## Goal

Separate Cargo build output from the binaries used by the live Windows tray
installation.

The desired workflow must allow a new release to be built and staged while the
previous release is still running. Windows may lock a running `.exe`, but that
must never block writes to `target\release`.

This handoff is intentionally limited to release deployment and Startup
management. Do not redesign MIDI mapping, recorder behavior, or the monitor UI
as part of this work.

## Confirmed State at Handoff

- Repository: `C:\Users\hueyh\OneDrive\Documents\midi curve`
- Current commit: `9a9460c` (`Show monitor build version in dashboard`)
- `target\release` was successfully rebuilt at that commit on July 28, 2026.
- The monitor build identifies itself as `v0.1.0 · 9a9460c0`.
- `target\manual-release` has been deleted.
- No FP10 processes were running when the standard release was rebuilt and
  verified. The user subsequently started all three binaries from
  `target\release` at approximately 10:09 PM on July 28, 2026, so that directory
  is locked again at the time this handoff was written.
- The current Startup script is:

  ```text
  %APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\fp10-map-tray.vbs
  ```

- At handoff time, that script points directly to:

  ```text
  C:\Users\hueyh\OneDrive\Documents\midi curve\target\release\fp10-map-tray.exe
  ```

- The tray locates `fp10-map.exe` and `fp10-monitor-server.exe` as siblings of
  its own executable. Preserve that invariant.
- The tray's `Install startup` action currently writes `env::current_exe()` into
  the Startup VBS. See `install_startup()` in
  `src/bin/fp10-map-tray.rs`.
- User data currently lives under `%APPDATA%\fp10-map`:
  settings, logs, and recordings must not be moved or deleted by deployment.

## Why the Current Layout Is Wrong

`target\release` is Cargo output, not an installation directory. Running
`target\release\fp10-map-tray.exe` causes Windows to lock that executable and
the sibling mapper/monitor executables. A later `cargo build --release` then
cannot replace the locked files.

Stopping the live app before every build works around the symptom, but it:

- interrupts MIDI forwarding and the monitor;
- clears the recorder's in-memory rolling buffer;
- makes automated release validation fragile;
- conflates build artifacts with deployed artifacts.

Do not solve this by creating another Cargo target directory such as
`target\manual-release`. That only creates another ambiguous build location.

## Recommended Deployment Layout

Keep Cargo output unchanged:

```text
<workspace>\target\release\
  fp10-map.exe
  fp10-map-tray.exe
  fp10-monitor-server.exe
```

Deploy immutable, versioned copies outside the repository:

```text
%LOCALAPPDATA%\fp10-map\
  releases\
    0.1.0-9a9460c0\
      fp10-map.exe
      fp10-map-tray.exe
      fp10-monitor-server.exe
      examples\
        curve.toml
        curve-mid-control.toml
  current.json
```

Use `%LOCALAPPDATA%` for installed binaries and `%APPDATA%` for the existing
roaming user data.

Each release directory is immutable after publication. A new release is copied
to a new directory, so the old running executables never block the build or
deployment.

`current.json` is recommended metadata for diagnostics and future tooling. A
minimal shape is:

```json
{
  "version": "0.1.0",
  "build": "9a9460c0",
  "path": "C:\\Users\\...\\AppData\\Local\\fp10-map\\releases\\0.1.0-9a9460c0"
}
```

Do not make the first implementation depend on a directory symlink or junction.
Those add privilege, OneDrive, and atomic-switching complications.

## Recommended First Implementation

Add a repository script:

```text
scripts\deploy-windows.ps1
```

The script should:

1. Resolve the workspace, Cargo output, deployment root, and Startup paths to
   absolute paths.
2. Refuse a dirty Git worktree by default. An explicit development override is
   acceptable, but deployed builds must then retain the `-dirty` suffix.
3. Run:

   ```powershell
   cargo fmt --check
   cargo test
   cargo build --release --bin fp10-map --bin fp10-map-tray --bin fp10-monitor-server
   ```

4. Derive the package version from `Cargo.toml` and the build ID from
   `git rev-parse --short=8 HEAD`.
5. Create a uniquely named staging directory under:

   ```text
   %LOCALAPPDATA%\fp10-map\releases
   ```

6. Copy the three release executables and the two built-in curve files into the
   staging directory. Preserve sibling executable names exactly.
7. Verify all required files exist and are non-empty.
8. Rename the staging directory to the final immutable version directory.
   Staging and final paths must be on the same volume.
9. Atomically write `current.json` using a temporary sibling followed by rename.
10. Update Startup by invoking the newly deployed tray:

    ```powershell
    & "$releaseDir\fp10-map-tray.exe" --install-startup
    ```

    Because `install_startup()` uses `env::current_exe()`, this makes the VBS
    point to that exact deployed version without duplicating VBS escaping logic
    in the script.
11. Print the deployed tray path, Startup script path, version, build ID, and
    whether activation is pending.

Deployment must not stop or replace the currently running version by default.
It should be safe to deploy while practicing or while an unsaved recorder
buffer exists.

## Activation Semantics

Treat deployment and activation as different operations:

- **Deploy:** build, copy to a new immutable directory, and update Startup for
  the next login. This must not interrupt the current process.
- **Activate now:** exit the old tray and start the newly deployed tray. This
  clears the monitor's in-memory recorder buffer and must be an explicit user
  action.

An optional `-Activate` flag may be added, but it must:

1. Warn clearly that unsaved rolling-buffer MIDI will be lost.
2. Stop only `fp10-map`, `fp10-map-tray`, and `fp10-monitor-server`.
3. Avoid killing unrelated processes or using broad path/glob deletion.
4. Start the deployed tray, not `target\release`.
5. Verify that the tray and monitor remain running.

Do not require administrator privileges for normal deployment or Startup
installation. All proposed locations are per-user. A previously elevated tray
may require the user to exit it manually before immediate activation.

## Startup and Rollback

The Startup VBS may point directly to the selected immutable version directory.
Every successful deployment updates it to the new version.

Rollback should be simple:

```powershell
& "$oldReleaseDir\fp10-map-tray.exe" --install-startup
```

Then exit the current tray and start the old deployed tray if immediate rollback
is desired.

Do not delete the previous deployed version during deployment. Retain at least
the newest two successful versions.

Cleanup should be a separate, conservative operation:

- never delete the directory referenced by Startup;
- never delete the directory referenced by `current.json`;
- never delete a directory containing a running FP10 executable;
- if the running executable path cannot be inspected due to permissions, skip
  cleanup rather than guessing.

## Curve and Settings Caveat

`curve_path()` first looks for:

```text
<tray directory>\examples\<curve file>
```

and then falls back to a workspace-relative path. A deployed package therefore
must include the `examples` directory.

`tray-settings.toml` currently stores an absolute curve path. Existing settings
may still point into the workspace. The implementation must test this migration
case.

Preferred behavior:

- built-in Forum and Mid-control choices resolve to the deployed `examples`
  files;
- a genuinely custom curve keeps its explicit absolute path;
- deployment does not overwrite a custom user curve.

If necessary, update settings serialization to store a built-in choice
identifier separately from custom paths. Keep README and tests synchronized.

## Files Likely to Change

- New: `scripts/deploy-windows.ps1`
- `README.md`
  - document build vs deploy vs activate;
  - stop instructing users to run `target\release` as the permanent app;
  - document installed binary and rollback locations.
- `AGENTS.md`
  - remove the assumption that release binaries are normally run from
    `target\release`;
  - update build/deploy/restart instructions for versioned deployment.
- Possibly `src/bin/fp10-map-tray.rs`
  - curve setting migration;
  - optional deployed-version/status display;
  - only change Startup behavior if the deploy script cannot safely reuse
    `--install-startup`.
- Tests for any Rust behavior changed.

Do not remove the sibling-binary lookup unless replacing it with an equally
explicit packaged layout.

## Validation Requirements

Run the normal project validation:

```powershell
cargo fmt --check
cargo test
cargo build --release --bin fp10-map --bin fp10-map-tray --bin fp10-monitor-server
```

Then validate the deployment workflow:

1. Deploy version A.
2. Start version A from `%LOCALAPPDATA%`.
3. Confirm Startup points to version A, not the repository.
4. While version A is still running, make/build version B.
5. Confirm `cargo build --release` succeeds without stopping version A.
6. Deploy version B while version A remains running.
7. Confirm version A continues running and version B files are complete.
8. Confirm Startup now points to version B.
9. Explicitly activate version B.
10. Open the monitor from desktop and phone.
11. Confirm the footer reports version B's build ID.
12. Confirm Overview, `全部`, `回到最新`, drag/pan, zoom controls, desktop
    Ctrl/Cmd-wheel, and mobile pinch are present.
13. Roll Startup back to version A and verify rollback.
14. Confirm recordings, settings, and logs under `%APPDATA%\fp10-map` remain
    untouched.

Hardware behavior cannot be fully validated without the real MIDI path. If
hardware is unavailable, report that live MIDI reconnect and recorder behavior
were not manually tested.

## Acceptance Criteria

- Running an installed release never locks anything under `target`.
- A new release can be built and deployed while the previous version runs.
- Startup always points to an immutable deployed release, never a Cargo target
  directory.
- Deployment is per-user and normally requires no administrator permission.
- Deploying does not silently restart the app or clear recorder memory.
- Immediate activation is explicit.
- The monitor footer identifies the exact deployed build.
- At least one prior release remains available for rollback.
- Temporary/staging directories are cleaned safely after failure.
- README and `AGENTS.md` describe the new workflow accurately.

## Out of Scope for the First Pass

- Automatic background updates.
- Downloading releases from GitHub.
- A system-wide installer.
- Windows Service installation.
- Self-updating a currently running executable in place.
- Moving or redesigning recorder persistence.

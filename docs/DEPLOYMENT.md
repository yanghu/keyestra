# Windows Deployment

This document covers local packaging, activation, rollback, and release
identity for maintainers and advanced users. For a first look at Keyestra, use
the source-build quick start in the main README.

## Deploy an immutable release

`target\release` is Cargo output, not an installation directory. Publish a
current-user release with:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass `
  -File .\scripts\deploy-windows.ps1
```

The script:

1. runs formatting checks and tests;
2. builds `keyestra`, `keyestra-tray`, and `keyestra-monitor`;
3. publishes them under:

   ```text
   %LOCALAPPDATA%\keyestra\releases\<version>-<build>\
   ```

4. writes `%LOCALAPPDATA%\keyestra\current.json`; and
5. updates:

   ```text
   %APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\keyestra-tray.vbs
   ```

The Startup command records the complete MIDI route explicitly:
`Clavinova-1` → `Keyestra MIDI`, with the Monitor reading `Keyestra Output`.
It does not rely on device defaults compiled into the executable.

Normal deployment does not stop the running release. The new version starts
at the next login or after the current Tray exits, preserving any unsaved
in-memory recorder buffer.

## Activate immediately

To deploy and switch to the new build immediately:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass `
  -File .\scripts\deploy-windows.ps1 -Activate
```

> **Activation stops running Keyestra processes and permanently discards the
> unsaved in-memory recorder buffer.**

## Release identity

The deployment script makes the source state explicit:

```text
clean commit:         0.2.0-d137d292
uncommitted preview:  0.2.0-d137d292-dirty-20260728T231500Z
Git ID unavailable:   0.2.0-snapshot-20260728T231500Z
```

The same identity is embedded in the binaries, stored in `current.json`,
returned by `GET /version`, shown in the Monitor footer, and used for the
immutable release directory.

## Startup rollback and removal

Point Startup to an earlier installed release without stopping the current
one:

```powershell
& "$env:LOCALAPPDATA\keyestra\releases\<old-release>\keyestra-tray.exe" `
  --install-startup
```

Remove the current-user Startup entry:

```powershell
& "$env:LOCALAPPDATA\keyestra\releases\<release>\keyestra-tray.exe" `
  --uninstall-startup
```

Startup is stored at:

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\keyestra-tray.vbs
```

## Logs

Tray logs are stored at `%APPDATA%\keyestra\tray.log`. The active log is capped
at 5 MB, with one rotated backup at `tray.log.1`.

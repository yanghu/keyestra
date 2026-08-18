# REAPER Piano Compare

Keyestra can use REAPER as an invisible host for several ready-to-play piano
instruments. The phone talks only to Keyestra; Keyestra talks to REAPER's
built-in web interface over the piano PC's loopback connection.

For daily playing, keep the four-track Compare project as a reference and save
a separate `Keyestra Piano Live.rpp`. Running the registered Keyestra bootstrap
while that Live project is open invokes
[`keyestra-piano-live-simplify.lua`](../scripts/reaper/keyestra-piano-live-simplify.lua):
it keeps `Garritan CFX`, renames the SK-EX instance to `Pianoteq Live`, removes
the other Pianoteq comparison tracks, and saves the result. The Live config then
uses that single Pianoteq instance for instrument host presets and independent
reverb scenes.

## Recommended two-stage setup

The repeatable host setup is automated with
[`scripts/reaper/keyestra-piano-compare-bootstrap.lua`](../scripts/reaper/keyestra-piano-compare-bootstrap.lua).
The script uses REAPER's ReaScript API rather than mouse/GUI automation. It:

- creates or updates four identity-tagged piano tracks;
- finds `Keyestra MIDI` (with the legacy `FP10 Mapped` fallback);
- arms each track, enables input monitoring, routes it to the master output,
  and disables anticipative FX for low-latency live playing;
- inserts an installed CFX or Pianoteq plug-in by host-visible FX name;
- leaves only Garritan CFX unmuted initially;
- writes the Keyestra piano config if one does not already exist; and
- saves both a canonical project and a REAPER project template.

To run it, open an empty project tab, then use **Actions > Show action list >
New action > Load ReaScript**, select the Lua file, and run it. It saves:

```text
%APPDATA%\REAPER\Projects\Keyestra\Keyestra Piano Compare.rpp
%APPDATA%\REAPER\ProjectTemplates\Keyestra Piano Compare.RPP
```

The bootstrap is idempotent for a project it has tagged. Rerunning it updates
the same logical tracks via persistent `KEYESTRA_PIANO_ID` track metadata. It
will not modify a nonempty, untagged project. If an expected plug-in is not in
REAPER's scanned FX list, the track and routing are still created and the
missing plug-in is reported; install/rescan it and rerun the script.

### One-time third-party plug-in confirmation

REAPER can load host-visible FX and presets, but it cannot reliably express
arbitrary state that a vendor exposes only inside its own UI. After bootstrap,
open each plug-in once when needed to confirm:

- license/authorization;
- the CFX sample-library path;
- the desired CFX mic/perspective/mix;
- the exact Pianoteq model and factory/user preset; and
- any vendor-specific setting not exposed as a REAPER parameter or preset.

Then save `Keyestra Piano Compare.rpp` again. From that point onward, the
canonical project contains the complete plug-in state and normal practice does
not require repeating setup. If you later save exact states in REAPER's FX
preset dropdown, you may put those exact preset names in the bootstrap script's
`preset` fields so reruns can restore them automatically.

This boundary is deliberate: do not automate third-party plug-in GUIs with
screen coordinates. Prefer REAPER FX presets, track templates, or the saved
canonical project whenever the host can preserve the state.

## 1. Manual/final project checks

Create one track (or one group of tracks) per logical piano. Load the plugin,
preset, routing, and effects in advance. Give every controlled track a unique
name, for example:

- `Garritan CFX`
- `Pianoteq SK-EX`
- `Pianoteq Steinway D`
- `Pianoteq Bösendorfer 280VC`

Enable the mapped `Keyestra MIDI` input on all of these tracks and disable the
keyboard's raw MIDI input there. Keep the tracks armed/monitored so switching
does not load a plugin or preset. Keyestra switches only the configured track
mute states; unrelated project tracks are not changed.

In **Preferences > Audio > MIDI Devices**, keep `Keyestra MIDI` enabled as an
input and disable the physical piano input (for example `Clavinova-1`). Some
Windows/Yamaha MIDI drivers reserve the paired output while REAPER holds the
physical input open; Keyestra needs that output briefly to send Local Control
CC 122 when switching between the piano body and REAPER VSTs.

The bootstrap supplies these four tracks automatically. If you extend or build
the project manually, follow the same rules. Before saving, leave exactly one
configured piano unmuted.

## Calibrate piano loudness

Use
[`scripts/reaper/keyestra-piano-compare-calibrate.lua`](../scripts/reaper/keyestra-piano-compare-calibrate.lua)
after the plug-in sounds, mic mixes, and presets are final. Load it once from
**Actions > Show action list > New action > Load ReaScript**, then run it with
the canonical Piano Compare project active.

The calibrator uses REAPER's native dry-run renderer, so it needs no SWS,
ffmpeg, or other external tool. It temporarily adds the same roughly
30-second MIDI sequence to every tagged piano, covering velocities 40, 55, 70,
85, 100, and 115. It measures LUFS-I and True Peak through the master, then
deletes the temporary items and restores track mute, solo, selection, cursor,
and render settings.

The quietest measured piano becomes the reference. The script only proposes
cuts to the other piano groups, preserving headroom and preserving the internal
balance of a piano that owns multiple tagged tracks. A report is shown before
anything is changed. Choose **Yes** to multiply the existing track faders by
the proposed constant corrections, or **No** to keep a measurement-only run.
Applied changes are one REAPER undo step, and the script asks before saving the
project. The bootstrap does not reset these faders when rerun.

Before calibrating, release all notes and the sustain pedal. Do not change a
plug-in's output control, preset, mic balance, master FX, or the REAPER master
fader afterward without recalibrating. Finish with a blind A/B check; a final
subjective adjustment of about 0.5 dB can still be appropriate because equal
LUFS does not guarantee identical perceived loudness for different spectra and
decay envelopes.

## Create the fixed-MIDI benchmark project

Keep `Keyestra Piano Compare.rpp` as the canonical live-playing project. Save
it after plug-in setup and calibration. For repeatable listening tests, open
that clean saved project and run
[`scripts/reaper/keyestra-piano-benchmark-bootstrap.lua`](../scripts/reaper/keyestra-piano-benchmark-bootstrap.lua).
When prompted, select `maestro-v3.0.0.csv` from an extracted MAESTRO v3.0.0
MIDI-only download. The script saves a separate project at:

```text
%APPDATA%\REAPER\Projects\Keyestra\Keyestra Piano Benchmark.rpp
```

The benchmark reuses the canonical project's four loaded instruments and
fader calibration. It adds one silent-master MIDI source track, routes that
track to every tagged piano, and creates these fixed regions:

- **A — Chopin, Nocturne Op. 9 No. 2:** about 35 seconds for lyrical p–mf,
  voicing, and treble sweetness;
- **B — Debussy, Des pas sur la neige:** about 29 seconds for pp response,
  resonance, and decay;
- **C — Chopin, Nocturne Op. 48 No. 1:** about 30 seconds from the loud section
  for bass, projection, and high-register harshness; and
- **D — Scriabin, Etude Op. 8 No. 10:** 25 seconds for speed, attack, and
  clarity under dense playing.

The excerpts retain MAESTRO's original event timing, key velocities, and
continuous pedal data. Each region initializes controller state, ends with a
pedal/all-notes-off reset, and includes a release-tail area. The selected MIDI,
metadata, license, and attribution are copied to
`%APPDATA%\REAPER\Media\Keyestra\MAESTRO v3.0.0`, so the saved project does not
depend on the Downloads folder.

Region A becomes the initial loop selection. Turn on REAPER **Repeat** and press
Play. Switch the phone's Piano A/B control during the four-second release tail,
so the newly selected (previously muted) instrument receives the next pass from
its beginning with the correct note and pedal state. To test a different
passage, double-click its region before playback. Rerunning the script updates
its tagged source track and regions without rebuilding the third-party
plug-ins.

## 2. Enable REAPER web control

In REAPER, open **Options > Preferences > Control/OSC/web**, add a **Web browser
interface**, and use port `8080`. Keep this interface on the local piano PC;
the phone does not need direct access to the REAPER port. A username and
password are optional and can be placed in the Keyestra config below.

You can confirm the REAPER interface locally at:

```text
http://127.0.0.1:8080
```

## 3. Configure Keyestra pianos

The bootstrap creates this file if it is missing. For manual setup, copy
[`examples/reaper-pianos.toml`](../examples/reaper-pianos.toml) to:

```text
%APPDATA%\keyestra\reaper-pianos.toml
```

Edit each `name` for the phone and each `tracks` entry to exactly match a
unique REAPER track name. A logical piano can own multiple tracks:

```toml
[[piano]]
id = "cfx-player"
name = "CFX Player"
tracks = ["CFX Player Close", "CFX Player Room"]
```

All listed tracks activate together. A REAPER track cannot belong to two
logical pianos. Piano IDs must be unique and should remain stable because the
phone and saved configuration refer to them by ID.

## Control a Pianoteq VST from the phone (prototype)

REAPER must remain the only application that opens the ASIO driver. Keyestra
uses REAPER's native OSC input for Pianoteq plug-in parameters; it does not
start Pianoteq standalone and does not require Pianoteq's JSON-RPC server.

Copy [`scripts/reaper/Keyestra.ReaperOSC`](../scripts/reaper/Keyestra.ReaperOSC)
to:

```text
%APPDATA%\REAPER\OSC\Keyestra.ReaperOSC
```

Then open **Options > Preferences > Control/OSC/web**, add an **OSC (Open Sound
Control)** surface, and set:

- mode/config: `Keyestra`;
- device IP: `127.0.0.1`;
- device port: `9001` (Keyestra briefly listens here after a preset change);
- local listen port: `9000`;
- local IP: `127.0.0.1`.

Add a `[pianoteq_vst]` section to `reaper-pianos.toml`. `fx` is one-based and
normally `1` when Pianoteq is the first plug-in on the track:

```toml
[pianoteq_vst]
piano_id = "sk-ex"
track = "Pianoteq SK-EX"
fx = 1
osc_address = "127.0.0.1:9000"
# Optional; this is the default and must match the OSC surface's device port.
osc_feedback_address = "127.0.0.1:9001"
# Optional normalized override applied after every named instrument preset.
dynamics_after_preset = 0.45
```

The phone exposes Dynamics, Reverb on/off, Reverb Duration, Reverb Mix, and
Room Dimensions. It sends absolute normalized values when a slider is released.
After a named instrument preset is selected, Keyestra briefly listens for OSC
feedback and asks REAPER for the two parameter banks containing these controls.
The sliders are updated only when a complete fresh snapshot arrives; Keyestra
does not continuously poll plug-in parameters.

`dynamics_after_preset` is the initial value for a reversible practice overlay.
The separate **Practice Dynamics** slider saves the user's latest value in the
server-side `%APPDATA%\keyestra\user-settings.json`; it is shared by every
phone. Keyestra loads the complete REAPER host preset first and then applies
the saved Dynamics value. It does not rewrite REAPER's encoded user-preset
file, and selecting the preset directly in REAPER still uses its stored value.

### Add named sound slots

Choose a complete sound inside the Pianoteq plug-in, then use the preset menu
in REAPER's FX header to save it under a unique name. Add the same host preset
to the config:

```toml
[[pianoteq_vst.preset]]
id = "sk-ex-player"
name = "SK-EX Player"
reaper_preset = "SK-EX Player"
```

Repeat this for the ten or twenty sounds worth keeping on the phone. A sound
slot stores the complete host-visible plug-in state; the quick controls can
then make temporary Dynamics and room adjustments on top.

For an alternate config location, start the monitor with:

```powershell
keyestra-monitor --reaper-config "C:\path\to\reaper-pianos.toml"
```

## 4. Daily use

Open the prepared REAPER project, then open Keyestra's **Piano** tab on the
phone. Tap a piano name to switch. Keyestra first mutes the other configured
pianos, unmutes the chosen piano, and reads REAPER's state back before marking
the switch successful.

For repeated comparison, select Piano A and Piano B and use the large toggle.
The pair is stored in that phone browser. Direct Pianoteq preset and sound
controls remain available below REAPER Piano Compare.

If REAPER is closed, its web interface is disabled, a configured track is
missing/duplicated, or authentication fails, the selector shows that it is
unavailable instead of silently accepting a tap.

For the cleanest switch, release held notes and the sustain pedal before
changing pianos. Track mute switching is immediate, but a plugin's internal
voice and pedal state can continue evolving while its track is muted.

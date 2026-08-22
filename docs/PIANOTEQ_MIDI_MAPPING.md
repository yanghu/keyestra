# Pianoteq MIDI Mapping contract

Keyestra uses a private application-control route for native Pianoteq preset
operations:

The private route uses a Windows MIDI Services **MIDI Loopback 2.0** pair:

```text
Keyestra writes:  Pianoteq Control In
                         |
             MIDI Loopback 2.0 pair
                         |
REAPER reads:    Pianoteq Control Out
                         |
             Pianoteq track only -> Pianoteq VST
```

The `In`/`Out` names are from the application's perspective defined by MIDI
Loopback 2.0: Keyestra opens `Pianoteq Control In` as its output destination,
and the REAPER Pianoteq track listens to `Pianoteq Control Out`. Garritan and
other instrument tracks must not listen to the control pair. Performance notes,
pedals, and other playing data continue to use the performance MIDI route.

Create a Pianoteq MIDI Mapping named `Keyestra` with these exact entries. The
Program Change numbers are sent exactly as listed, on channel 1; the Pianoteq
mappings accept any channel.

| Program Change | Action | Pianoteq preset |
| ---: | --- | --- |
| 1 | Load preset | `My Presets/My Kawai SK-EX Player` |
| 2 | Load preset | `My Presets/My Bösendorfer VC Player` |
| 3 | Load preset | `Shigeru Kawai SK-EX Warm` |
| 4 | Load preset | `Shigeru Kawai SK-EX Prelude` |
| 5 | Load preset | `Bösendorfer VC Warm` |
| 6 | Load preset | `Bösendorfer VC Classical Recording` |
| 7 | Load preset | `Bösendorfer VC New Age` |
| 8 | Load preset | `Shigeru Kawai SK-EX Classical Recording` |
| 9 | Load preset | `Shigeru Kawai SK-EX Binaural` |
| 10 | Load preset | `Bösendorfer VC Binaural` |
| 11 | Load preset | `Erard Player` |
| 12 | Load preset | `CP-80 Recording` |
| 13 | Load preset | `Pleyel Player` |
| 14 | Load preset | `A. Walter 415 Player` |
| 15 | Load preset | `C. Graf 415 Player` |
| 41 | Reload current preset | — |

The application-facing preset IDs and this transport mapping live together in
`src/pianoteq_midi.rs`. Add future presets there only after adding the matching
entry to Pianoteq's `Keyestra` mapping. Do not put Program Change numbers in UI
or business logic.

Preset selection and reload use this MIDI route. Pianoteq Volume, Dynamics,
Reverb, and other exposed continuous parameters remain on the REAPER VST/OSC
parameter route. Standalone Pianoteq continues to use RPC.

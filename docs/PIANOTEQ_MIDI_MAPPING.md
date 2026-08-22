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

Create a Pianoteq MIDI Mapping named `Keyestra` with these exact entries. All
messages use MIDI channel 16 and value 127. The dedicated CC range avoids the
VST3 host Program Change path that can compete with Pianoteq's native mapping.

| MIDI channel | Control Change | Value | Action | Pianoteq preset |
| ---: | ---: | ---: | --- | --- |
| 16 | 102 | 127 | Load preset | `My Presets/My Kawai SK-EX Player` |
| 16 | 103 | 127 | Load preset | `My Presets/My Bösendorfer VC Player` |
| 16 | 104 | 127 | Load preset | `Shigeru Kawai SK-EX Warm` |
| 16 | 105 | 127 | Load preset | `Shigeru Kawai SK-EX Prelude` |
| 16 | 106 | 127 | Load preset | `Bösendorfer VC Warm` |
| 16 | 107 | 127 | Load preset | `Bösendorfer VC Classical Recording` |
| 16 | 108 | 127 | Load preset | `Bösendorfer VC New Age` |
| 16 | 109 | 127 | Load preset | `Shigeru Kawai SK-EX Classical Recording` |
| 16 | 110 | 127 | Load preset | `Shigeru Kawai SK-EX Binaural` |
| 16 | 111 | 127 | Load preset | `Bösendorfer VC Binaural` |
| 16 | 112 | 127 | Load preset | `Erard Player` |
| 16 | 113 | 127 | Load preset | `CP-80 Recording` |
| 16 | 114 | 127 | Load preset | `Pleyel Player` |
| 16 | 115 | 127 | Load preset | `A. Walter 415 Player` |
| 16 | 116 | 127 | Load preset | `C. Graf 415 Player` |
| 16 | 117 | 127 | Reload current preset | — |

The application-facing preset IDs and this transport mapping live together in
`src/pianoteq_midi.rs`. Add future presets there only after adding the matching
entry to Pianoteq's `Keyestra` mapping. Do not put transport CC numbers in UI or
business logic.

Preset selection and reload first finish any required REAPER track activation,
then use this MIDI route as the final external operation. Nothing reads or
writes REAPER, OSC, or VST state after the control CC. Selecting a preset
does not automatically restore or write Reverb parameters, so the Pianoteq
native preset can finish loading without a competing host operation. Explicit
manual Pianoteq Volume, Dynamics, Reverb, and other exposed continuous-parameter
operations remain on the REAPER VST/OSC route. Standalone Pianoteq continues to
use RPC.

## REAPER VST3 transport

Do not retain the old Program Change entries in Pianoteq's `Keyestra` mapping.
Program Change works in Standalone but can enter the VST3 host program path in
REAPER and cause the native preset to flash and return to a host program such as
`Player`. The dedicated channel-16 CC messages above are the only supported
Keyestra preset transport in REAPER mode.

Keyestra's monitor log records the complete message for every send. For example,
CC109 logs `channel=16 control_change=109 value=127` and
`wire_bytes=[BF,6D,7F]`.

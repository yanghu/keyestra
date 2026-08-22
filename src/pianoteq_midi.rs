use anyhow::{anyhow, Context, Result};
use midir::{MidiOutput, MidiOutputPort};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Windows MIDI Services MIDI Loopback 2.0 exposes the write side of a pair as
// "In" and the receiving side as "Out". Keyestra opens Control In for output;
// REAPER listens to Control Out.
pub const DEFAULT_CONTROL_WRITE_PORT: &str = "Pianoteq Control In";
pub const CONTROL_MIDI_CHANNEL: u8 = 16;
pub const CONTROL_VALUE: u8 = 127;
pub const RELOAD_CURRENT_PRESET_CONTROL_CHANGE: u8 = 117;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PianoteqPresetMapping {
    pub id: &'static str,
    pub name: &'static str,
    pub pianoteq_preset: &'static str,
    pub control_change: u8,
}

// External contract with Pianoteq's manually configured "Keyestra" MIDI
// Mapping. Application code uses `id`; only this module knows the dedicated
// Control Change numbers. Extend this table when another mapping is added in
// Pianoteq.
pub const PRESET_MAPPINGS: &[PianoteqPresetMapping] = &[
    PianoteqPresetMapping {
        id: "sk-ex-player",
        name: "PT | SK-EX | My Player",
        pianoteq_preset: "My Presets/My Kawai SK-EX Player",
        control_change: 102,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-player",
        name: "PT | Bösendorfer VC | My Player",
        pianoteq_preset: "My Presets/My Bösendorfer VC Player",
        control_change: 103,
    },
    PianoteqPresetMapping {
        id: "sk-ex-warm",
        name: "PT | SK-EX | Warm",
        pianoteq_preset: "Shigeru Kawai SK-EX Warm",
        control_change: 104,
    },
    PianoteqPresetMapping {
        id: "sk-ex-prelude",
        name: "PT | SK-EX | Prelude",
        pianoteq_preset: "Shigeru Kawai SK-EX Prelude",
        control_change: 105,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-warm",
        name: "PT | Bösendorfer VC | Warm",
        pianoteq_preset: "Bösendorfer VC Warm",
        control_change: 106,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-classical-recording",
        name: "PT | Bösendorfer VC | Classical Recording",
        pianoteq_preset: "Bösendorfer VC Classical Recording",
        control_change: 107,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-new-age",
        name: "PT | Bösendorfer VC | New Age",
        pianoteq_preset: "Bösendorfer VC New Age",
        control_change: 108,
    },
    PianoteqPresetMapping {
        id: "sk-ex-classical-recording",
        name: "PT | SK-EX | Classical Recording",
        pianoteq_preset: "Shigeru Kawai SK-EX Classical Recording",
        control_change: 109,
    },
    PianoteqPresetMapping {
        id: "sk-ex-binaural",
        name: "PT | SK-EX | Binaural",
        pianoteq_preset: "Shigeru Kawai SK-EX Binaural",
        control_change: 110,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-binaural",
        name: "PT | Bösendorfer VC | Binaural",
        pianoteq_preset: "Bösendorfer VC Binaural",
        control_change: 111,
    },
    PianoteqPresetMapping {
        id: "erard-player",
        name: "PT | Erard | Player",
        pianoteq_preset: "Erard Player",
        control_change: 112,
    },
    PianoteqPresetMapping {
        id: "cp-80-recording",
        name: "PT | CP-80 | Recording",
        pianoteq_preset: "CP-80 Recording",
        control_change: 113,
    },
    PianoteqPresetMapping {
        id: "pleyel-player",
        name: "PT | Pleyel | Player",
        pianoteq_preset: "Pleyel Player",
        control_change: 114,
    },
    PianoteqPresetMapping {
        id: "a-walter-415-player",
        name: "PT | A. Walter 415 | Player",
        pianoteq_preset: "A. Walter 415 Player",
        control_change: 115,
    },
    PianoteqPresetMapping {
        id: "c-graf-415-player",
        name: "PT | C. Graf 415 | Player",
        pianoteq_preset: "C. Graf 415 Player",
        control_change: 116,
    },
];

pub fn default_control_output() -> String {
    DEFAULT_CONTROL_WRITE_PORT.to_string()
}

pub fn preset(id: &str) -> Option<&'static PianoteqPresetMapping> {
    PRESET_MAPPINGS.iter().find(|preset| preset.id == id)
}

pub fn ensure_control_output(output_name: &str) -> Result<()> {
    let midi_out = MidiOutput::new("keyestra-pianoteq-control-probe")?;
    resolve_output(&midi_out, output_name).map(|_| ())
}

pub fn send_control_change(output_name: &str, control_change: u8) -> Result<()> {
    let message = control_change_message(control_change)?;
    let midi_out = MidiOutput::new("keyestra-pianoteq-control")?;
    let (port, resolved_name) = resolve_output(&midi_out, output_name)?;
    let mut connection = midi_out
        .connect(&port, "keyestra-pianoteq-control")
        .with_context(|| {
            format!("Failed to connect Pianoteq MIDI control output {resolved_name}")
        })?;
    connection.send(&message).with_context(|| {
        format!("Failed to send Pianoteq CC {control_change} to {resolved_name}")
    })?;
    let time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis();
    println!(
        "Pianoteq MIDI control: time_ms={time_ms} channel={CONTROL_MIDI_CHANNEL} control_change={control_change} value={CONTROL_VALUE} wire_bytes=[{:02X},{:02X},{:02X}] destination={resolved_name}",
        message[0], message[1], message[2]
    );
    Ok(())
}

fn control_change_message(control_change: u8) -> Result<[u8; 3]> {
    if control_change > 127 {
        return Err(anyhow!("Pianoteq Control Change must be in 0..=127"));
    }
    Ok([
        0xB0 | (CONTROL_MIDI_CHANNEL - 1),
        control_change,
        CONTROL_VALUE,
    ])
}

fn resolve_output(midi_out: &MidiOutput, output_name: &str) -> Result<(MidiOutputPort, String)> {
    let matches = midi_out
        .ports()
        .into_iter()
        .filter_map(|port| midi_out.port_name(&port).ok().map(|name| (port, name)))
        .filter(|(_, name)| name.to_lowercase().contains(&output_name.to_lowercase()))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [(port, name)] => Ok((port.clone(), name.clone())),
        [] => {
            return Err(anyhow!(
                "Pianoteq MIDI control output {output_name:?} was not found"
            ))
        }
        _ => {
            return Err(anyhow!(
                "Pianoteq MIDI control output {output_name:?} is ambiguous"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn static_mapping_matches_the_pianoteq_contract() {
        assert_eq!(PRESET_MAPPINGS.len(), 15);
        assert_eq!(preset("sk-ex-player").unwrap().control_change, 102);
        assert_eq!(
            preset("sk-ex-player").unwrap().pianoteq_preset,
            "My Presets/My Kawai SK-EX Player"
        );
        assert_eq!(preset("c-graf-415-player").unwrap().control_change, 116);
        assert_eq!(RELOAD_CURRENT_PRESET_CONTROL_CHANGE, 117);
        assert_eq!(CONTROL_MIDI_CHANNEL, 16);
        assert_eq!(CONTROL_VALUE, 127);
        assert_eq!(DEFAULT_CONTROL_WRITE_PORT, "Pianoteq Control In");

        let ids = PRESET_MAPPINGS
            .iter()
            .map(|preset| preset.id)
            .collect::<HashSet<_>>();
        let controls = PRESET_MAPPINGS
            .iter()
            .map(|preset| preset.control_change)
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), PRESET_MAPPINGS.len());
        assert_eq!(controls.len(), PRESET_MAPPINGS.len());
        assert_eq!(controls, (102..=116).collect::<HashSet<_>>());
        assert!(!controls.contains(&RELOAD_CURRENT_PRESET_CONTROL_CHANGE));
    }

    #[test]
    fn pianoteq_controls_use_channel_16_cc_messages() {
        assert_eq!(control_change_message(102).unwrap(), [0xBF, 102, 127]);
        assert_eq!(control_change_message(117).unwrap(), [0xBF, 117, 127]);
        assert!(control_change_message(128).is_err());
    }
}

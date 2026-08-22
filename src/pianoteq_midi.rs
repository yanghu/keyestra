use anyhow::{anyhow, Context, Result};
use midir::{MidiOutput, MidiOutputPort};

// Windows MIDI Services MIDI Loopback 2.0 exposes the write side of a pair as
// "In" and the receiving side as "Out". Keyestra opens Control In for output;
// REAPER listens to Control Out.
pub const DEFAULT_CONTROL_WRITE_PORT: &str = "Pianoteq Control In";
pub const RELOAD_CURRENT_PRESET_PROGRAM_CHANGE: u8 = 41;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PianoteqPresetMapping {
    pub id: &'static str,
    pub name: &'static str,
    pub pianoteq_preset: &'static str,
    pub program_change: u8,
}

// External contract with Pianoteq's manually configured "Keyestra" MIDI
// Mapping. Application code uses `id`; only this module knows Program Change
// numbers. Extend this table when another mapping is added in Pianoteq.
pub const PRESET_MAPPINGS: &[PianoteqPresetMapping] = &[
    PianoteqPresetMapping {
        id: "sk-ex-player",
        name: "PT | SK-EX | My Player",
        pianoteq_preset: "My Presets/My Kawai SK-EX Player",
        program_change: 1,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-player",
        name: "PT | Bösendorfer VC | My Player",
        pianoteq_preset: "My Presets/My Bösendorfer VC Player",
        program_change: 2,
    },
    PianoteqPresetMapping {
        id: "sk-ex-warm",
        name: "PT | SK-EX | Warm",
        pianoteq_preset: "Shigeru Kawai SK-EX Warm",
        program_change: 3,
    },
    PianoteqPresetMapping {
        id: "sk-ex-prelude",
        name: "PT | SK-EX | Prelude",
        pianoteq_preset: "Shigeru Kawai SK-EX Prelude",
        program_change: 4,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-warm",
        name: "PT | Bösendorfer VC | Warm",
        pianoteq_preset: "Bösendorfer VC Warm",
        program_change: 5,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-classical-recording",
        name: "PT | Bösendorfer VC | Classical Recording",
        pianoteq_preset: "Bösendorfer VC Classical Recording",
        program_change: 6,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-new-age",
        name: "PT | Bösendorfer VC | New Age",
        pianoteq_preset: "Bösendorfer VC New Age",
        program_change: 7,
    },
    PianoteqPresetMapping {
        id: "sk-ex-classical-recording",
        name: "PT | SK-EX | Classical Recording",
        pianoteq_preset: "Shigeru Kawai SK-EX Classical Recording",
        program_change: 8,
    },
    PianoteqPresetMapping {
        id: "sk-ex-binaural",
        name: "PT | SK-EX | Binaural",
        pianoteq_preset: "Shigeru Kawai SK-EX Binaural",
        program_change: 9,
    },
    PianoteqPresetMapping {
        id: "bosendorfer-vc-binaural",
        name: "PT | Bösendorfer VC | Binaural",
        pianoteq_preset: "Bösendorfer VC Binaural",
        program_change: 10,
    },
    PianoteqPresetMapping {
        id: "erard-player",
        name: "PT | Erard | Player",
        pianoteq_preset: "Erard Player",
        program_change: 11,
    },
    PianoteqPresetMapping {
        id: "cp-80-recording",
        name: "PT | CP-80 | Recording",
        pianoteq_preset: "CP-80 Recording",
        program_change: 12,
    },
    PianoteqPresetMapping {
        id: "pleyel-player",
        name: "PT | Pleyel | Player",
        pianoteq_preset: "Pleyel Player",
        program_change: 13,
    },
    PianoteqPresetMapping {
        id: "a-walter-415-player",
        name: "PT | A. Walter 415 | Player",
        pianoteq_preset: "A. Walter 415 Player",
        program_change: 14,
    },
    PianoteqPresetMapping {
        id: "c-graf-415-player",
        name: "PT | C. Graf 415 | Player",
        pianoteq_preset: "C. Graf 415 Player",
        program_change: 15,
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

pub fn send_program_change(output_name: &str, program_change: u8) -> Result<()> {
    let midi_out = MidiOutput::new("keyestra-pianoteq-control")?;
    let (port, resolved_name) = resolve_output(&midi_out, output_name)?;
    let mut connection = midi_out
        .connect(&port, "keyestra-pianoteq-control")
        .with_context(|| {
            format!("Failed to connect Pianoteq MIDI control output {resolved_name}")
        })?;
    // Channel 1. The Pianoteq mappings are configured for any channel, and the
    // attached mapping numbers are used exactly as listed.
    connection.send(&[0xC0, program_change]).with_context(|| {
        format!("Failed to send Program Change {program_change} to {resolved_name}")
    })?;
    println!("Pianoteq MIDI control: sent Program Change {program_change} to {resolved_name}");
    Ok(())
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
        assert_eq!(preset("sk-ex-player").unwrap().program_change, 1);
        assert_eq!(
            preset("sk-ex-player").unwrap().pianoteq_preset,
            "My Presets/My Kawai SK-EX Player"
        );
        assert_eq!(preset("c-graf-415-player").unwrap().program_change, 15);
        assert_eq!(RELOAD_CURRENT_PRESET_PROGRAM_CHANGE, 41);
        assert_eq!(DEFAULT_CONTROL_WRITE_PORT, "Pianoteq Control In");

        let ids = PRESET_MAPPINGS
            .iter()
            .map(|preset| preset.id)
            .collect::<HashSet<_>>();
        let programs = PRESET_MAPPINGS
            .iter()
            .map(|preset| preset.program_change)
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), PRESET_MAPPINGS.len());
        assert_eq!(programs.len(), PRESET_MAPPINGS.len());
        assert!(!programs.contains(&RELOAD_CURRENT_PRESET_PROGRAM_CHANGE));
    }
}

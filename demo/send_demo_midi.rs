//! Send a short, expressive MIDI progression for Keyestra demos and screenshots.
//!
//! Usage:
//!   cargo run --example send_demo_midi
//!   cargo run --example send_demo_midi -- "output name substring"

use std::env;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use midir::MidiOutput;

const VOICING_OFFSETS: [[i8; 4]; 5] = [
    [-3, 2, 0, 4],
    [1, -2, 3, -1],
    [-1, 4, -3, 2],
    [3, 0, -2, 1],
    [-2, 1, 4, -3],
];

fn main() -> Result<()> {
    let selector = env::args()
        .nth(1)
        .unwrap_or_else(|| "Keyestra MIDI".to_string());
    let midi_out = MidiOutput::new("keyestra-demo-output")?;
    let ports = midi_out.ports();

    let Some(port) = ports.iter().find(|port| {
        midi_out
            .port_name(port)
            .is_ok_and(|name| name.to_lowercase().contains(&selector.to_lowercase()))
    }) else {
        let available = ports
            .iter()
            .filter_map(|port| midi_out.port_name(port).ok())
            .collect::<Vec<_>>();
        bail!(
            "No MIDI output matches {selector:?}. Available outputs: {}",
            available.join(", ")
        );
    };

    let output_name = midi_out.port_name(port)?;
    let mut connection = midi_out
        .connect(port, "keyestra-demo-output")
        .context("Failed to open the demo MIDI output")?;

    println!("Sending Keyestra demo performance to {output_name}...");
    connection.send(&[0xB0, 64, 96])?;

    let progression: &[(&[u8], u8)] = &[
        (&[48, 55, 64], 34),
        (&[50, 57, 65], 40),
        (&[52, 59, 67], 48),
        (&[53, 60, 69], 56),
        (&[55, 62, 71], 65),
        (&[57, 64, 72], 74),
        (&[59, 65, 74], 83),
        (&[60, 67, 76], 92),
        (&[62, 69, 77], 101),
        (&[60, 67, 76], 94),
        (&[57, 64, 72], 85),
        (&[55, 62, 71], 77),
        (&[53, 60, 69], 68),
        (&[52, 59, 67], 59),
        (&[50, 57, 65], 51),
        (&[48, 55, 64], 44),
        (&[48, 60, 67, 72], 55),
        (&[53, 60, 65, 69], 69),
        (&[55, 62, 67, 71], 79),
        (&[48, 55, 64, 72], 88),
    ];

    for (index, &(notes, base_velocity)) in progression.iter().enumerate() {
        if index == 8 {
            connection.send(&[0xB0, 64, 0])?;
        } else if index == 12 {
            connection.send(&[0xB0, 64, 92])?;
        }

        let offsets = VOICING_OFFSETS[index % VOICING_OFFSETS.len()];
        for (voice, &note) in notes.iter().enumerate() {
            let velocity = (base_velocity as i16 + offsets[voice] as i16).clamp(1, 127) as u8;
            connection.send(&[0x90, note, velocity])?;
            let note_gap_ms = 5 + ((index + voice * 2) % 5) as u64;
            thread::sleep(Duration::from_millis(note_gap_ms));
        }
        let chord_length_ms = 155 + ((index * 29) % 85) as u64;
        thread::sleep(Duration::from_millis(chord_length_ms));
        for (voice, &note) in notes.iter().enumerate() {
            let release_velocity = 38 + ((index * 3 + voice * 5) % 18) as u8;
            connection.send(&[0x80, note, release_velocity])?;
        }
        let chord_gap_ms = 58 + ((index * 17) % 42) as u64;
        thread::sleep(Duration::from_millis(chord_gap_ms));
    }

    connection.send(&[0xB0, 64, 0])?;
    connection.send(&[0xB0, 123, 0])?;
    println!("Demo complete: {} chords sent.", progression.len());
    Ok(())
}

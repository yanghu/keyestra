use std::str::FromStr;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use midir::{Ignore, MidiInput, MidiInputPort, MidiOutput, MidiOutputConnection, MidiOutputPort};

use crate::mapping::VelocityMapper;
use crate::monitor;

const DEFAULT_OUTPUT_NAME: &str = "Keyestra MIDI";
const LEGACY_OUTPUT_NAME: &str = "FP10 Mapped";

#[derive(Debug, Clone)]
pub enum PortSelector {
    Index(usize),
    Name(String),
}

impl PortSelector {
    pub fn parse(value: &str) -> Self {
        match usize::from_str(value) {
            Ok(index) => Self::Index(index),
            Err(_) => Self::Name(value.to_string()),
        }
    }

    pub fn name(value: &str) -> Self {
        Self::Name(value.to_string())
    }
}

pub fn list_ports() -> Result<()> {
    let midi_in = MidiInput::new("keyestra-list-inputs")?;
    let midi_out = MidiOutput::new("keyestra-list-outputs")?;

    println!("MIDI Inputs:");
    for (index, port) in midi_in.ports().iter().enumerate() {
        println!("  [{}] {}", index, midi_in.port_name(port)?);
    }

    println!();
    println!("MIDI Outputs:");
    for (index, port) in midi_out.ports().iter().enumerate() {
        println!("  [{}] {}", index, midi_out.port_name(port)?);
    }

    Ok(())
}

pub fn run_forwarder(
    input_selector: PortSelector,
    output_selector: PortSelector,
    mapper: VelocityMapper,
    monitor_enabled: bool,
    drop_active_sensing: bool,
    shutdown_rx: Receiver<()>,
) -> Result<()> {
    let mut midi_in = MidiInput::new("keyestra-input")?;
    midi_in.ignore(Ignore::None);
    let midi_out = MidiOutput::new("keyestra-output")?;

    let input_port = select_input_port(&midi_in, &input_selector)?;
    let output_port = select_output_port(&midi_out, &output_selector)?;
    let input_name = midi_in.port_name(&input_port)?;
    let output_name = midi_out.port_name(&output_port)?;

    println!("Input:  {}", input_name);
    println!("Output: {}", output_name);
    println!("Press Ctrl+C to stop.");

    let output_conn = OutputForwarder::connect(output_name.clone())
        .with_context(|| format!("Failed to connect MIDI output {}", output_name))?;
    let output_conn = Arc::new(Mutex::new(output_conn));

    let callback_output = Arc::clone(&output_conn);
    let callback_mapper = mapper.clone();
    let _input_conn = midi_in
        .connect(
            &input_port,
            "keyestra-input",
            move |_timestamp, message, _| {
                let (mapped, report) = callback_mapper.map_message(message);
                if !drop_active_sensing || !is_active_sensing(&mapped) {
                    if let Ok(mut output) = callback_output.lock() {
                        if let Err(err) = output.send(&mapped) {
                            eprintln!("Failed to send MIDI message: {err}");
                        }
                    }
                }
                if monitor_enabled {
                    monitor::print_event(message, report);
                }
            },
            (),
        )
        .with_context(|| format!("Failed to connect MIDI input {}", input_name))?;

    let _ = shutdown_rx.recv();
    println!("Stopping Keyestra.");
    Ok(())
}

struct OutputForwarder {
    output_name: String,
    connection: MidiOutputConnection,
}

impl OutputForwarder {
    fn connect(output_name: String) -> Result<Self> {
        let midi_out = MidiOutput::new("keyestra-output")?;
        let output_port = select_output_port(&midi_out, &PortSelector::name(&output_name))?;
        let connection = midi_out
            .connect(&output_port, "keyestra-output")
            .with_context(|| format!("Failed to connect MIDI output {output_name}"))?;
        Ok(Self {
            output_name,
            connection,
        })
    }

    fn send(&mut self, message: &[u8]) -> Result<()> {
        match self.connection.send(message) {
            Ok(()) => Ok(()),
            Err(first_error) => {
                eprintln!(
                    "MIDI output {} failed ({first_error}); reconnecting and retrying once",
                    self.output_name
                );
                let mut replacement = Self::connect(self.output_name.clone())?;
                replacement
                    .connection
                    .send(message)
                    .map_err(anyhow::Error::new)?;
                *self = replacement;
                eprintln!("Reconnected MIDI output {}", self.output_name);
                Ok(())
            }
        }
    }
}

fn is_active_sensing(message: &[u8]) -> bool {
    message == [0xFE]
}

fn select_input_port(midi_in: &MidiInput, selector: &PortSelector) -> Result<MidiInputPort> {
    let ports = midi_in.ports();
    match selector {
        PortSelector::Index(index) => ports
            .get(*index)
            .cloned()
            .ok_or_else(|| port_error("input", selector, midi_in, &ports)),
        PortSelector::Name(name) => {
            for port in &ports {
                let port_name = midi_in.port_name(port)?;
                if port_name.to_lowercase().contains(&name.to_lowercase()) {
                    return Ok(port.clone());
                }
            }
            Err(port_error("input", selector, midi_in, &ports))
        }
    }
}

fn select_output_port(midi_out: &MidiOutput, selector: &PortSelector) -> Result<MidiOutputPort> {
    let ports = midi_out.ports();
    match selector {
        PortSelector::Index(index) => ports
            .get(*index)
            .cloned()
            .ok_or_else(|| output_port_error("output", selector, midi_out, &ports)),
        PortSelector::Name(name) => {
            for candidate in port_name_candidates(name) {
                for port in &ports {
                    let port_name = midi_out.port_name(port)?;
                    if port_name.to_lowercase().contains(&candidate.to_lowercase()) {
                        return Ok(port.clone());
                    }
                }
            }
            Err(output_port_error("output", selector, midi_out, &ports))
        }
    }
}

fn port_name_candidates(name: &str) -> Vec<&str> {
    if name.eq_ignore_ascii_case(DEFAULT_OUTPUT_NAME) {
        vec![name, LEGACY_OUTPUT_NAME]
    } else {
        vec![name]
    }
}

fn port_error(
    kind: &str,
    selector: &PortSelector,
    midi_in: &MidiInput,
    ports: &[MidiInputPort],
) -> anyhow::Error {
    let mut lines = vec![format!(
        "Could not find MIDI {} port {:?}. Available:",
        kind, selector
    )];
    for (index, port) in ports.iter().enumerate() {
        let name = midi_in
            .port_name(port)
            .unwrap_or_else(|_| "<unreadable>".to_string());
        lines.push(format!("  [{}] {}", index, name));
    }
    anyhow::anyhow!(lines.join("\n"))
}

fn output_port_error(
    kind: &str,
    selector: &PortSelector,
    midi_out: &MidiOutput,
    ports: &[MidiOutputPort],
) -> anyhow::Error {
    let mut lines = vec![format!(
        "Could not find MIDI {} port {:?}. Available:",
        kind, selector
    )];
    for (index, port) in ports.iter().enumerate() {
        let name = midi_out
            .port_name(port)
            .unwrap_or_else(|_| "<unreadable>".to_string());
        lines.push(format!("  [{}] {}", index, name));
    }
    anyhow::anyhow!(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_falls_back_to_the_legacy_port() {
        assert_eq!(
            port_name_candidates(DEFAULT_OUTPUT_NAME),
            vec![DEFAULT_OUTPUT_NAME, LEGACY_OUTPUT_NAME]
        );
        assert_eq!(port_name_candidates("Custom Port"), vec!["Custom Port"]);
    }

    #[test]
    fn active_sensing_is_identified_without_matching_other_realtime_messages() {
        assert!(is_active_sensing(&[0xFE]));
        assert!(!is_active_sensing(&[0x90, 60, 100]));
        assert!(!is_active_sensing(&[0xF8]));
    }
}

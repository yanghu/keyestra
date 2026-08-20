use crate::mapping::MappingReport;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn print_event(message: &[u8], report: MappingReport) {
    match report {
        MappingReport::Mapped {
            channel,
            note,
            input,
            output,
        } => {
            println!(
                "Note On  time_ms={} ch={} note={} velocity={} -> {}",
                now_ms(),
                channel,
                note,
                input,
                output
            );
        }
        MappingReport::Passthrough => print_passthrough(message),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn print_passthrough(message: &[u8]) {
    if message.is_empty() {
        println!("Empty message passthrough");
        return;
    }

    let status = message[0];
    match status & 0xF0 {
        0x80 if message.len() >= 3 => println!("Note Off note={} passthrough", message[1]),
        0x90 if message.len() >= 3 && message[2] == 0 => {
            println!("Note On  note={} velocity=0 passthrough", message[1])
        }
        0xB0 if message.len() >= 3 => println!(
            "CC       cc={} value={} passthrough",
            message[1], message[2]
        ),
        _ => println!("MIDI     {:02X?} passthrough", message),
    }
}

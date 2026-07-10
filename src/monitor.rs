use crate::mapping::MappingReport;

pub fn print_event(message: &[u8], report: MappingReport) {
    match report {
        MappingReport::Mapped {
            channel,
            note,
            input,
            output,
        } => {
            println!(
                "Note On  ch={} note={} velocity={} -> {}",
                channel, note, input, output
            );
        }
        MappingReport::Passthrough => print_passthrough(message),
    }
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

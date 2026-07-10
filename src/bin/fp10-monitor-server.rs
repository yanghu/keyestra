#![cfg_attr(windows, windows_subsystem = "console")]

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use midir::{Ignore, MidiInput, MidiInputPort};

#[derive(Debug, Parser)]
#[command(name = "fp10-monitor-server")]
#[command(about = "Native MIDI monitor backend with a local web dashboard")]
struct Cli {
    #[arg(long, help = "List MIDI input ports")]
    list: bool,

    #[arg(
        long = "in",
        default_value = "FP10 Mapped",
        help = "MIDI input port name substring or index"
    )]
    input: String,

    #[arg(long, default_value_t = 8770, help = "Local HTTP port")]
    port: u16,

    #[arg(long, default_value = "0.0.0.0", help = "HTTP bind host")]
    host: String,
}

#[derive(Debug, Clone)]
enum PortSelector {
    Index(usize),
    Name(String),
}

#[derive(Debug, Clone)]
struct NoteEvent {
    note: u8,
    name: String,
    velocity: u8,
    mark: &'static str,
}

#[derive(Debug, Clone)]
struct ChordGroup {
    id: u64,
    time_ms: u128,
    notes: Vec<NoteEvent>,
    average: u8,
    spread: u8,
}

#[derive(Debug, Clone)]
struct RawMidiEvent {
    time_ms: u128,
    kind: String,
    channel: String,
    bytes: String,
}

#[derive(Debug)]
struct MonitorState {
    input_name: String,
    message_count: u64,
    last_note: Option<NoteEvent>,
    current_chord: Option<ChordGroup>,
    chord_buffer: Vec<(u128, NoteEvent)>,
    history: Vec<ChordGroup>,
    raw: Vec<RawMidiEvent>,
    next_group_id: u64,
}

impl MonitorState {
    fn new(input_name: String) -> Self {
        Self {
            input_name,
            message_count: 0,
            last_note: None,
            current_chord: None,
            chord_buffer: Vec::new(),
            history: Vec::new(),
            raw: Vec::new(),
            next_group_id: 1,
        }
    }

    fn ingest(&mut self, message: &[u8]) {
        self.ingest_at(now_ms(), message);
    }

    fn ingest_at(&mut self, now: u128, message: &[u8]) {
        self.message_count += 1;
        self.raw.insert(0, raw_event(now, message));
        self.raw.truncate(80);

        if message.len() < 3 {
            return;
        }

        let status = message[0];
        let command = status & 0xF0;
        let note = message[1];
        let velocity = message[2];

        if command == 0x90 && velocity > 0 {
            let note_event = NoteEvent {
                note,
                name: note_name(note),
                velocity,
                mark: dynamic_mark(velocity),
            };
            self.last_note = Some(note_event.clone());
            self.chord_buffer.push((now, note_event));
            self.flush_ready_chord(now);
        } else {
            self.flush_ready_chord(now);
        }
    }

    fn flush_ready_chord(&mut self, now: u128) {
        if self.chord_buffer.is_empty() {
            return;
        }

        let first_time = self.chord_buffer[0].0;
        if now.saturating_sub(first_time) < 52 {
            return;
        }

        let mut notes: Vec<NoteEvent> = self
            .chord_buffer
            .drain(..)
            .map(|(_, event)| event)
            .collect();
        notes.sort_by_key(|event| event.note);

        let min = notes.iter().map(|event| event.velocity).min().unwrap_or(0);
        let max = notes.iter().map(|event| event.velocity).max().unwrap_or(0);
        let sum: u16 = notes.iter().map(|event| event.velocity as u16).sum();
        let average = ((sum as f32) / (notes.len() as f32)).round() as u8;

        let group = ChordGroup {
            id: self.next_group_id,
            time_ms: first_time,
            notes,
            average,
            spread: max.saturating_sub(min),
        };
        self.next_group_id += 1;
        self.current_chord = Some(group.clone());
        self.history.insert(0, group);
        self.history.truncate(80);
    }

    fn to_json(&self) -> String {
        let mut out = String::with_capacity(8192);
        out.push('{');
        write!(
            out,
            "\"inputName\":\"{}\",\"messageCount\":{},",
            escape_json(&self.input_name),
            self.message_count
        )
        .unwrap();

        out.push_str("\"lastNote\":");
        write_note_or_null(&mut out, self.last_note.as_ref());
        out.push(',');

        out.push_str("\"currentChord\":");
        write_group_or_null(&mut out, self.current_chord.as_ref());
        out.push(',');

        out.push_str("\"history\":[");
        for (index, group) in self.history.iter().take(40).enumerate() {
            if index > 0 {
                out.push(',');
            }
            write_group(&mut out, group);
        }
        out.push_str("],\"raw\":[");
        for (index, event) in self.raw.iter().take(60).enumerate() {
            if index > 0 {
                out.push(',');
            }
            write!(
                out,
                "{{\"timeMs\":{},\"kind\":\"{}\",\"channel\":\"{}\",\"bytes\":\"{}\"}}",
                event.time_ms,
                escape_json(&event.kind),
                escape_json(&event.channel),
                escape_json(&event.bytes)
            )
            .unwrap();
        }
        out.push_str("]}");
        out
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.list {
        list_inputs()?;
        return Ok(());
    }

    let mut midi_in = MidiInput::new("fp10-monitor-input")?;
    midi_in.ignore(Ignore::None);
    let selector = PortSelector::parse(&cli.input);
    let input_port = select_input_port(&midi_in, &selector)?;
    let input_name = midi_in.port_name(&input_port)?;
    let state = Arc::new(Mutex::new(MonitorState::new(input_name.clone())));
    let midi_state = Arc::clone(&state);

    let _conn = midi_in
        .connect(
            &input_port,
            "fp10-monitor-input",
            move |_timestamp, message, _| {
                if let Ok(mut state) = midi_state.lock() {
                    state.ingest(message);
                }
            },
            (),
        )
        .with_context(|| format!("Failed to open MIDI input {}", input_name))?;

    let address = format!("{}:{}", cli.host, cli.port);
    let listener =
        TcpListener::bind(&address).with_context(|| format!("Failed to bind {}", address))?;
    println!("Monitoring MIDI input: {}", input_name);
    println!("Dashboard: http://localhost:{}/", cli.port);
    if cli.host == "0.0.0.0" {
        if let Some(ip) = local_lan_ip() {
            println!("Phone/LAN dashboard: http://{}:{}/", ip, cli.port);
        }
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(error) = handle_client(stream, state) {
                        eprintln!("HTTP error: {}", error);
                    }
                });
            }
            Err(error) => eprintln!("HTTP accept error: {}", error),
        }
    }

    Ok(())
}

impl PortSelector {
    fn parse(value: &str) -> Self {
        match usize::from_str(value) {
            Ok(index) => Self::Index(index),
            Err(_) => Self::Name(value.to_string()),
        }
    }
}

fn list_inputs() -> Result<()> {
    let midi_in = MidiInput::new("fp10-monitor-list-inputs")?;
    println!("MIDI Inputs:");
    for (index, port) in midi_in.ports().iter().enumerate() {
        println!("  [{}] {}", index, midi_in.port_name(port)?);
    }
    Ok(())
}

fn select_input_port(midi_in: &MidiInput, selector: &PortSelector) -> Result<MidiInputPort> {
    let ports = midi_in.ports();
    match selector {
        PortSelector::Index(index) => ports
            .get(*index)
            .cloned()
            .ok_or_else(|| input_port_error(selector, midi_in, &ports)),
        PortSelector::Name(name) => {
            for port in &ports {
                let port_name = midi_in.port_name(port)?;
                if port_name.to_lowercase().contains(&name.to_lowercase()) {
                    return Ok(port.clone());
                }
            }
            Err(input_port_error(selector, midi_in, &ports))
        }
    }
}

fn input_port_error(
    selector: &PortSelector,
    midi_in: &MidiInput,
    ports: &[MidiInputPort],
) -> anyhow::Error {
    let mut lines = vec![format!(
        "Could not find MIDI input port {:?}. Available:",
        selector
    )];
    for (index, port) in ports.iter().enumerate() {
        let name = midi_in
            .port_name(port)
            .unwrap_or_else(|_| "<unreadable>".to_string());
        lines.push(format!("  [{}] {}", index, name));
    }
    anyhow::anyhow!(lines.join("\n"))
}

fn handle_client(mut stream: TcpStream, state: Arc<Mutex<MonitorState>>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;

    let mut buffer = [0_u8; 2048];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|path| path.split('?').next())
        .unwrap_or("/");

    match path {
        "/" | "/index.html" => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            INDEX_HTML,
        ),
        "/assets/monitor.css" => respond(
            &mut stream,
            "200 OK",
            "text/css; charset=utf-8",
            MONITOR_CSS,
        ),
        "/assets/monitor.js" => respond(
            &mut stream,
            "200 OK",
            "application/javascript; charset=utf-8",
            MONITOR_JS,
        ),
        "/state" => {
            let json = state
                .lock()
                .map(|mut state| {
                    state.flush_ready_chord(now_ms());
                    state.to_json()
                })
                .unwrap_or_else(|_| "{\"error\":\"state lock poisoned\"}".to_string());
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &json,
            )
        }
        "/events" => respond_events(stream, state),
        "/seed-log" => {
            let body = match seed_from_tray_log(&state) {
                Ok(count) => format!("{{\"loaded\":{}}}", count),
                Err(error) => format!("{{\"error\":\"{}\"}}", escape_json(&error.to_string())),
            };
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        _ => respond(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "Not found",
        ),
    }
}

fn respond_events(mut stream: TcpStream, state: Arc<Mutex<MonitorState>>) -> Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n"
    )?;
    stream.flush()?;

    let mut last_json = String::new();
    let mut ticks_since_heartbeat = 0_u8;
    loop {
        let json = state
            .lock()
            .map(|mut state| {
                state.flush_ready_chord(now_ms());
                state.to_json()
            })
            .unwrap_or_else(|_| "{\"error\":\"state lock poisoned\"}".to_string());

        if json != last_json {
            write!(stream, "event: state\ndata: {}\n\n", json)?;
            stream.flush()?;
            last_json = json;
            ticks_since_heartbeat = 0;
        } else {
            ticks_since_heartbeat = ticks_since_heartbeat.saturating_add(1);
            if ticks_since_heartbeat >= 25 {
                write!(stream, ": heartbeat\n\n")?;
                stream.flush()?;
                ticks_since_heartbeat = 0;
            }
        }

        thread::sleep(Duration::from_millis(120));
    }
}

fn seed_from_tray_log(state: &Arc<Mutex<MonitorState>>) -> Result<usize> {
    let appdata = env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?;
    let log_path = std::path::PathBuf::from(appdata)
        .join("fp10-map")
        .join("tray.log");
    let text = fs::read_to_string(&log_path)
        .with_context(|| format!("Failed to read {}", log_path.display()))?;
    let events: Vec<(u8, u8)> = text
        .lines()
        .filter_map(parse_mapped_note)
        .rev()
        .take(36)
        .collect();
    let mut state = state
        .lock()
        .map_err(|_| anyhow::anyhow!("state lock poisoned"))?;
    state.history.clear();
    state.raw.clear();
    state.chord_buffer.clear();
    state.current_chord = None;
    state.last_note = None;

    let mut time = now_ms().saturating_sub(events.len() as u128 * 160);
    for (index, (note, velocity)) in events.iter().rev().enumerate() {
        let gap = if index % 9 == 0 {
            520
        } else if index % 4 == 0 {
            210
        } else {
            72
        };
        time += gap;
        state.ingest_at(time, &[0x90, *note, *velocity]);
        state.flush_ready_chord(time + 80);
    }
    state.flush_ready_chord(now_ms());
    Ok(events.len())
}

fn parse_mapped_note(line: &str) -> Option<(u8, u8)> {
    if !line.starts_with("Note On") {
        return None;
    }
    let note = parse_after(line, "note=")?;
    let mapped = line.rsplit_once("->")?.1.trim().parse().ok()?;
    Some((note, mapped))
}

fn parse_after(line: &str, marker: &str) -> Option<u8> {
    let rest = line.split_once(marker)?.1;
    rest.split_whitespace().next()?.parse().ok()
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.as_bytes().len(),
        body
    )?;
    Ok(())
}

fn local_lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

fn raw_event(time_ms: u128, message: &[u8]) -> RawMidiEvent {
    let status = message.first().copied().unwrap_or(0);
    let channel = if status < 0xF0 {
        ((status & 0x0F) + 1).to_string()
    } else {
        "-".to_string()
    };
    RawMidiEvent {
        time_ms,
        kind: midi_message_name(status).to_string(),
        channel,
        bytes: message
            .iter()
            .map(|byte| format!("{:02X}", byte))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn midi_message_name(status: u8) -> &'static str {
    match status & 0xF0 {
        0x80 => "note off",
        0x90 => "note on",
        0xA0 => "poly pressure",
        0xB0 => "control",
        0xC0 => "program",
        0xD0 => "pressure",
        0xE0 => "pitch bend",
        _ if status == 0xF8 => "clock",
        _ if status == 0xFE => "active sensing",
        _ => "midi",
    }
}

fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (note / 12) as i16 - 1;
    format!("{}{}", NAMES[(note % 12) as usize], octave)
}

fn dynamic_mark(value: u8) -> &'static str {
    if value == 0 {
        "-"
    } else if value <= 27 {
        "pp"
    } else if value <= 30 {
        "pp/p"
    } else if value <= 47 {
        "p"
    } else if value <= 50 {
        "p/mp"
    } else if value <= 67 {
        "mp"
    } else if value <= 70 {
        "mp/mf"
    } else if value <= 87 {
        "mf"
    } else if value <= 90 {
        "mf/f"
    } else if value <= 107 {
        "f"
    } else if value <= 110 {
        "f/ff"
    } else {
        "ff"
    }
}

fn write_note_or_null(out: &mut String, note: Option<&NoteEvent>) {
    match note {
        Some(note) => write_note(out, note),
        None => out.push_str("null"),
    }
}

fn write_group_or_null(out: &mut String, group: Option<&ChordGroup>) {
    match group {
        Some(group) => write_group(out, group),
        None => out.push_str("null"),
    }
}

fn write_note(out: &mut String, note: &NoteEvent) {
    write!(
        out,
        "{{\"note\":{},\"name\":\"{}\",\"velocity\":{},\"mark\":\"{}\"}}",
        note.note,
        escape_json(&note.name),
        note.velocity,
        note.mark
    )
    .unwrap();
}

fn write_group(out: &mut String, group: &ChordGroup) {
    write!(
        out,
        "{{\"id\":{},\"timeMs\":{},\"average\":{},\"spread\":{},\"notes\":[",
        group.id, group.time_ms, group.average, group.spread
    )
    .unwrap();
    for (index, note) in group.notes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_note(out, note);
    }
    out.push_str("]}");
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

const INDEX_HTML: &str = include_str!("../monitor_web/index.html");
const MONITOR_CSS: &str = include_str!("../monitor_web/monitor.css");
const MONITOR_JS: &str = include_str!("../monitor_web/monitor.js");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_mark_uses_overlapping_working_ranges() {
        assert_eq!(dynamic_mark(0), "-");
        assert_eq!(dynamic_mark(1), "pp");
        assert_eq!(dynamic_mark(27), "pp");
        assert_eq!(dynamic_mark(28), "pp/p");
        assert_eq!(dynamic_mark(30), "pp/p");
        assert_eq!(dynamic_mark(31), "p");
        assert_eq!(dynamic_mark(47), "p");
        assert_eq!(dynamic_mark(48), "p/mp");
        assert_eq!(dynamic_mark(50), "p/mp");
        assert_eq!(dynamic_mark(51), "mp");
        assert_eq!(dynamic_mark(67), "mp");
        assert_eq!(dynamic_mark(68), "mp/mf");
        assert_eq!(dynamic_mark(70), "mp/mf");
        assert_eq!(dynamic_mark(71), "mf");
        assert_eq!(dynamic_mark(87), "mf");
        assert_eq!(dynamic_mark(88), "mf/f");
        assert_eq!(dynamic_mark(90), "mf/f");
        assert_eq!(dynamic_mark(91), "f");
        assert_eq!(dynamic_mark(107), "f");
        assert_eq!(dynamic_mark(108), "f/ff");
        assert_eq!(dynamic_mark(110), "f/ff");
        assert_eq!(dynamic_mark(111), "ff");
        assert_eq!(dynamic_mark(127), "ff");
    }
}

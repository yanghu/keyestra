#![cfg_attr(windows, windows_subsystem = "console")]

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
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

    #[arg(long = "in", default_value = "FP10 Mapped", help = "MIDI input port name substring or index")]
    input: String,

    #[arg(long, default_value_t = 8770, help = "Local HTTP port")]
    port: u16,
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

        let mut notes: Vec<NoteEvent> = self.chord_buffer.drain(..).map(|(_, event)| event).collect();
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

    let address = format!("127.0.0.1:{}", cli.port);
    let listener = TcpListener::bind(&address).with_context(|| format!("Failed to bind {}", address))?;
    println!("Monitoring MIDI input: {}", input_name);
    println!("Dashboard: http://localhost:{}/", cli.port);

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

fn input_port_error(selector: &PortSelector, midi_in: &MidiInput, ports: &[MidiInputPort]) -> anyhow::Error {
    let mut lines = vec![format!("Could not find MIDI input port {:?}. Available:", selector)];
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
        .unwrap_or("/");

    match path {
        "/" | "/index.html" => respond(&mut stream, "200 OK", "text/html; charset=utf-8", INDEX_HTML_V2),
        "/state" => {
            let json = state
                .lock()
                .map(|mut state| {
                    state.flush_ready_chord(now_ms());
                    state.to_json()
                })
                .unwrap_or_else(|_| "{\"error\":\"state lock poisoned\"}".to_string());
            respond(&mut stream, "200 OK", "application/json; charset=utf-8", &json)
        }
        "/seed-log" => {
            let body = match seed_from_tray_log(&state) {
                Ok(count) => format!("{{\"loaded\":{}}}", count),
                Err(error) => format!("{{\"error\":\"{}\"}}", escape_json(&error.to_string())),
            };
            respond(&mut stream, "200 OK", "application/json; charset=utf-8", &body)
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", "Not found"),
    }
}

fn seed_from_tray_log(state: &Arc<Mutex<MonitorState>>) -> Result<usize> {
    let appdata = env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?;
    let log_path = std::path::PathBuf::from(appdata).join("fp10-map").join("tray.log");
    let text = fs::read_to_string(&log_path).with_context(|| format!("Failed to read {}", log_path.display()))?;
    let events: Vec<(u8, u8)> = text.lines().filter_map(parse_mapped_note).rev().take(36).collect();
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
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (note / 12) as i16 - 1;
    format!("{}{}", NAMES[(note % 12) as usize], octave)
}

fn dynamic_mark(value: u8) -> &'static str {
    if value <= 20 {
        "pp"
    } else if value <= 40 {
        "p"
    } else if value <= 60 {
        "mp"
    } else if value <= 80 {
        "mf"
    } else if value <= 100 {
        "f"
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

#[allow(dead_code)]
const INDEX_HTML: &str = r###"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>FP-10 Native MIDI Monitor</title>
  <style>
    :root { color-scheme: light; --bg:#f6f7f4; --panel:#fff; --ink:#18201d; --muted:#66706b; --line:#dbe1dc; --accent:#146c6c; --red:#b2412f; --soft:#e8efeb; }
    * { box-sizing: border-box; }
    body { margin:0; min-height:100vh; background:var(--bg); color:var(--ink); font-family:ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; }
    main { width:min(1280px, calc(100vw - 32px)); margin:0 auto; padding:24px 0 32px; }
    header { display:flex; align-items:end; justify-content:space-between; gap:20px; margin-bottom:16px; }
    h1,h2,p { margin:0; } h1 { font-size:clamp(28px,4vw,42px); line-height:1; } h2 { font-size:17px; }
    .subtitle,.summary { color:var(--muted); font-size:14px; }
    .status { border:1px solid var(--line); border-radius:8px; padding:8px 10px; background:#fbfcfa; color:var(--accent); font-weight:750; }
    .grid { display:grid; grid-template-columns:minmax(0,1.6fr) minmax(300px,.8fr); gap:16px; }
    .lower { display:grid; grid-template-columns:minmax(0,1fr) minmax(300px,.8fr); gap:16px; margin-top:16px; }
    .panel { background:var(--panel); border:1px solid var(--line); border-radius:8px; padding:16px; box-shadow:0 16px 40px rgba(24,32,29,.08); }
    .heading { display:flex; align-items:center; justify-content:space-between; gap:12px; margin-bottom:14px; }
    .keyboard { position:relative; height:180px; border:1px solid var(--line); border-radius:8px; background:linear-gradient(90deg,rgba(24,32,29,.05) 1px,transparent 1px) 0 0/calc(100%/52) 100%,#fbfcfa; overflow:hidden; }
    .guide { position:absolute; left:0; right:0; height:1px; border-top:1px dashed rgba(24,32,29,.22); z-index:1; }
    .guide span,.guide b { position:absolute; top:-10px; padding:1px 5px; border-radius:5px; background:rgba(255,255,255,.88); color:var(--muted); font-size:11px; }
    .guide span { left:7px; font-weight:750; } .guide b { right:7px; font-weight:650; }
    .key { position:absolute; bottom:0; width:18px; min-height:2px; transform:translateX(-50%); border-radius:7px 7px 0 0; color:#fff; display:flex; align-items:center; justify-content:center; font-size:11px; font-weight:750; z-index:2; }
    .key .value { writing-mode:vertical-rl; max-height:130px; overflow:hidden; }
    .cards { display:grid; grid-template-columns:repeat(auto-fit,minmax(112px,1fr)); gap:10px; margin-top:12px; }
    .card { border:1px solid var(--line); border-radius:8px; padding:10px; background:#fbfcfa; } .card strong { display:block; font-size:18px; margin-bottom:8px; }
    .mini { display:grid; grid-template-columns:42px 1fr 34px; align-items:center; gap:8px; color:var(--muted); font-size:12px; }
    .track { height:9px; border-radius:999px; background:var(--soft); overflow:hidden; } .fill { height:100%; border-radius:inherit; background:var(--accent); }
    .meter strong { display:block; font-size:54px; line-height:1; margin:8px 0; } .meter .bigbar { height:24px; border-radius:999px; background:var(--soft); overflow:hidden; }
    .dynamic { margin-top:14px; color:var(--accent); font-size:28px; font-weight:800; }
    .timeline { margin-top:16px; }
    .timeline-grid { display:flex; gap:5px; align-items:end; min-height:136px; overflow:hidden; }
    .timeline-group { flex:0 0 var(--group-width,38px); min-width:0; display:grid; grid-template-rows:104px auto auto; gap:2px; padding:4px 3px; border:1px solid var(--line); border-radius:6px; background:#fbfcfa; }
    .timeline-bars { display:flex; align-items:flex-end; justify-content:center; gap:1px; border-bottom:1px solid var(--line); }
    .timeline-bars span { width:7px; min-height:6px; max-height:98px; border-radius:4px 4px 0 0; }
    .timeline-group strong { text-align:center; font-size:13px; line-height:1; } .timeline-group small { overflow:hidden; color:var(--muted); font-size:10px; font-style:normal; text-align:center; text-overflow:ellipsis; white-space:nowrap; }
    table { width:100%; border-collapse:collapse; font-size:14px; } th,td { padding:9px 10px; border-bottom:1px solid var(--line); text-align:left; white-space:nowrap; } th { position:sticky; top:0; background:#eef3ef; }
    .table-wrap,.raw { max-height:390px; overflow:auto; border:1px solid var(--line); border-radius:8px; }
    .group-row td { background:#eef3ef; color:#34403a; } .group-meta { display:flex; align-items:center; gap:12px; flex-wrap:wrap; }
    .pill { display:inline-flex; min-width:38px; min-height:24px; align-items:center; justify-content:center; border-radius:6px; background:var(--soft); font-weight:750; }
    .history-meter { display:grid; grid-template-columns:minmax(82px,1fr) 32px; align-items:center; gap:8px; }
    .history-meter span { display:block; height:9px; border-radius:999px; }
    .raw { display:grid; gap:6px; max-height:210px; padding:8px; background:#fbfcfa; }
    .raw-row { display:grid; grid-template-columns:92px 110px 48px minmax(100px,160px); gap:10px; min-height:28px; color:var(--muted); font-size:13px; align-items:center; }
    .raw-row strong { color:var(--ink); } code { color:var(--red); font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace; }
    .velocity-pp { background:#5d7ea8; } .velocity-p { background:#4f8f9f; } .velocity-mp { background:#3f8f67; } .velocity-mf { background:#8b8a35; } .velocity-f { background:#b4642f; } .velocity-ff { background:#b2412f; }
    @media (max-width:860px){ header,.grid,.lower{grid-template-columns:1fr;display:grid;align-items:start;} .raw-row{grid-template-columns:86px 1fr 44px;} code{grid-column:1/-1;} }
  </style>
</head>
<body>
  <main>
    <header>
      <div>
        <h1>FP-10 Native MIDI Monitor</h1>
        <p class="subtitle">Rust 后端读取 MIDI，浏览器只显示数据。</p>
      </div>
      <div class="status" id="status">连接中</div>
    </header>
    <section class="grid">
      <section class="panel">
        <div class="heading"><h2>当前和弦</h2><div class="summary" id="chordSummary">等待输入</div></div>
        <div class="keyboard" id="keyboard"></div>
        <div class="cards" id="cards"></div>
      </section>
      <section class="panel meter">
        <div class="heading"><h2>最近一次击键</h2><div class="summary" id="lastName">-</div></div>
        <strong id="lastVelocity">0</strong>
        <div class="bigbar"><div class="fill" id="lastBar" style="width:0%"></div></div>
        <div class="dynamic" id="lastMark">-</div>
      </section>
    </section>
    <section class="panel timeline">
      <div class="heading"><h2>最近演奏</h2><div class="summary">从左到右显示最近的击键/和弦力度</div></div>
      <div class="timeline-grid" id="timeline"></div>
    </section>
    <section class="lower">
      <section class="panel">
        <div class="heading"><h2>击键 Log</h2><div class="summary" id="historySummary">0 groups</div></div>
        <div class="table-wrap"><table><thead><tr><th>时间</th><th>音</th><th>力度</th><th>动态</th></tr></thead><tbody id="history"></tbody></table></div>
      </section>
      <section class="panel">
        <div class="heading"><h2>原始 MIDI</h2><div class="summary" id="rawSummary">0 messages</div></div>
        <div class="raw" id="raw"></div>
      </section>
    </section>
  </main>
  <script>
    const state = { lastGroupId: 0 };
    const bands = [{value:20,label:"pp"},{value:40,label:"p"},{value:60,label:"mp"},{value:80,label:"mf"},{value:100,label:"f"},{value:127,label:"ff"}];
    const $ = (id) => document.getElementById(id);
    function cls(v){ if(v<=20)return"velocity-pp"; if(v<=40)return"velocity-p"; if(v<=60)return"velocity-mp"; if(v<=80)return"velocity-mf"; if(v<=100)return"velocity-f"; return"velocity-ff"; }
    function notePos(note){ const names=["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"]; const isBlack=(n)=>names[n%12].includes("#"); let count=0; for(let n=21;n<Math.max(21,Math.min(108,note));n++) if(!isBlack(n)) count++; const center=isBlack(note)?count:count+.5; return center/52*100; }
    function time(ms){ return new Date(Number(ms)).toLocaleTimeString("zh-CN",{hour12:false,hour:"2-digit",minute:"2-digit",second:"2-digit",fractionalSecondDigits:2}); }
    function render(data){
      $("status").textContent = `${data.inputName} · ${data.messageCount} messages`;
      const last = data.lastNote;
      if(last){ $("lastName").textContent=last.name; $("lastVelocity").textContent=last.velocity; $("lastBar").style.width=`${last.velocity/127*100}%`; $("lastBar").className=`fill ${cls(last.velocity)}`; $("lastMark").textContent=last.mark; }
      renderChord(data.currentChord);
      renderTimeline(data.history || []);
      renderHistory(data.history || []);
      renderRaw(data.raw || [], data.messageCount);
    }
    function renderChord(group){
      const keyboard=$("keyboard"); const cards=$("cards"); keyboard.innerHTML=""; cards.innerHTML="";
      const chordGraphHeight = 160;
      for(const band of bands){ const g=document.createElement("div"); g.className="guide"; g.style.bottom=`${band.value/127*chordGraphHeight}px`; g.innerHTML=`<span>${band.label}</span><b>${band.value}</b>`; keyboard.append(g); }
      if(!group){ $("chordSummary").textContent="等待输入"; return; }
      $("chordSummary").textContent=`${group.notes.length} 音 · 平均 ${group.average} · 差距 ${group.spread}`;
      for(const note of group.notes){ const marker=document.createElement("div"); marker.className=`key ${cls(note.velocity)}`; marker.style.left=`${notePos(note.note)}%`; marker.style.height=`${Math.max(2,note.velocity/127*chordGraphHeight)}px`; marker.innerHTML=`<span class="value">${note.name} ${note.velocity}</span>`; keyboard.append(marker);
        const card=document.createElement("article"); card.className="card"; card.innerHTML=`<strong>${note.name}</strong><div class="mini"><span>力度</span><div class="track"><div class="fill ${cls(note.velocity)}" style="width:${note.velocity/127*100}%"></div></div><b>${note.velocity}</b></div>`; cards.append(card); }
    }
    function renderTimeline(history){
      const timeline=$("timeline"); const groups=history.slice(0,28).reverse();
      timeline.innerHTML = groups.length ? groups.map(group=>`<article class="timeline-group" style="--group-width:${Math.max(34, Math.min(86, 24 + group.notes.length * 11))}px"><div class="timeline-bars">${group.notes.map(note=>`<span class="${cls(note.velocity)}" title="${note.name} ${note.velocity}" style="height:${Math.max(6,Math.round(note.velocity/127*98))}px"></span>`).join("")}</div><strong>${group.average}</strong><small>${group.notes.map(n=>n.name).join(" ")}</small></article>`).join("") : `<div class="timeline-empty">等待输入</div>`;
    }
    function renderHistory(history){
      $("historySummary").textContent=`${history.length} groups`;
      $("history").innerHTML = history.map((group,index)=>`<tr class="group-row"><td colspan="4"><div class="group-meta"><strong>#${history.length-index}</strong><span>${time(group.timeMs)}</span><span>${group.notes.length} 音 · ${group.notes.map(n=>n.name).join(" ")}</span><span>平均 ${group.average}</span><span>差距 ${group.spread}</span></div></td></tr>${group.notes.map(note=>`<tr><td></td><td><span class="pill">${note.name}</span></td><td><div class="history-meter"><span class="${cls(note.velocity)}" style="width:${note.velocity/127*100}%"></span><b>${note.velocity}</b></div></td><td>${note.mark}</td></tr>`).join("")}`).join("");
    }
    function renderRaw(raw,count){ $("rawSummary").textContent=`${count} messages`; $("raw").innerHTML=raw.map(e=>`<div class="raw-row"><span>${time(e.timeMs)}</span><strong>${e.kind}</strong><span>ch ${e.channel}</span><code>${e.bytes}</code></div>`).join(""); }
    async function tick(){ try{ const data=await fetch("/state",{cache:"no-store"}).then(r=>r.json()); render(data); } catch(e){ $("status").textContent="backend offline"; } setTimeout(tick,120); }
    tick();
  </script>
</body>
</html>"###;

const INDEX_HTML_V2: &str = r####"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>FP-10 Native MIDI Monitor</title>
  <style>
    :root { color-scheme: light; --bg:#f6f7f4; --panel:#fff; --ink:#18201d; --muted:#66706b; --line:#dbe1dc; --accent:#146c6c; --red:#b2412f; --soft:#e8efeb; --pp:#6d7f95; --p:#4991ad; --mp:#39a58d; --mf:#c0a736; --f:#df7a2d; --ff:#c83d32; }
    * { box-sizing:border-box; }
    body { margin:0; min-height:100vh; background:var(--bg); color:var(--ink); font-family:ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; }
    main { width:min(1280px, calc(100vw - 32px)); margin:0 auto; padding:24px 0 32px; }
    header { display:flex; align-items:end; justify-content:space-between; gap:20px; margin-bottom:16px; }
    h1,h2,p { margin:0; } h1 { font-size:clamp(28px,4vw,42px); line-height:1; } h2 { font-size:17px; }
    .subtitle,.summary { color:var(--muted); font-size:14px; }
    .status { border:1px solid var(--line); border-radius:8px; padding:8px 10px; background:#fbfcfa; color:var(--accent); font-weight:750; }
    .grid { display:grid; grid-template-columns:minmax(0,1.6fr) minmax(300px,.8fr); gap:16px; }
    .lower { display:grid; grid-template-columns:minmax(0,1fr) minmax(300px,.8fr); gap:16px; margin-top:16px; }
    .panel { background:var(--panel); border:1px solid var(--line); border-radius:8px; padding:16px; box-shadow:0 16px 40px rgba(24,32,29,.08); }
    .heading { display:flex; align-items:center; justify-content:space-between; gap:12px; margin-bottom:14px; }
    .keyboard { position:relative; height:180px; border:1px solid var(--line); border-radius:8px; background:linear-gradient(90deg,rgba(24,32,29,.05) 1px,transparent 1px) 0 0/calc(100%/52) 100%,#fbfcfa; overflow:hidden; }
    .guide { position:absolute; left:0; right:0; height:1px; border-top:1px dashed rgba(24,32,29,.22); z-index:1; }
    .guide span,.guide b { position:absolute; top:-10px; padding:1px 5px; border-radius:5px; background:rgba(255,255,255,.88); color:var(--muted); font-size:11px; }
    .guide span { left:7px; font-weight:750; } .guide b { right:7px; font-weight:650; }
    .key { position:absolute; bottom:0; width:18px; min-height:2px; transform:translateX(-50%); border-radius:7px 7px 0 0; color:#fff; display:flex; align-items:center; justify-content:center; font-size:11px; font-weight:750; z-index:2; }
    .key .value { writing-mode:vertical-rl; max-height:130px; overflow:hidden; }
    .cards { display:grid; grid-template-columns:repeat(auto-fit,minmax(112px,1fr)); gap:10px; margin-top:12px; }
    .card { border:1px solid var(--line); border-radius:8px; padding:10px; background:#fbfcfa; } .card strong { display:block; font-size:18px; margin-bottom:8px; }
    .mini { display:grid; grid-template-columns:42px 1fr 34px; align-items:center; gap:8px; color:var(--muted); font-size:12px; }
    .track { height:9px; border-radius:999px; background:var(--soft); overflow:hidden; } .fill { height:100%; border-radius:inherit; }
    .meter strong { display:block; font-size:54px; line-height:1; margin:8px 0; } .meter .bigbar { height:24px; border-radius:999px; background:var(--soft); overflow:hidden; }
    .dynamic { margin-top:14px; color:var(--accent); font-size:28px; font-weight:800; }
    .timeline { margin-top:16px; }
    .timeline .heading { align-items:start; }
    .tools { display:flex; align-items:center; justify-content:flex-end; gap:8px; flex-wrap:wrap; }
    .ghost { height:30px; border:1px solid var(--line); border-radius:7px; background:#fff; color:var(--accent); padding:0 9px; font:inherit; font-size:13px; font-weight:750; cursor:pointer; }
    select { height:30px; border:1px solid var(--line); border-radius:7px; background:#fff; color:var(--ink); padding:0 8px; font:inherit; font-size:13px; }
    .plot { position:relative; height:360px; overflow:hidden; border:1px solid var(--line); border-radius:8px; background:linear-gradient(to top, rgba(24,32,29,.05) 1px, transparent 1px) 0 96px/100% 36px,#fbfcfa; }
    .plot-note { position:absolute; width:12px; height:12px; transform:translate(-50%,-50%); border-radius:50%; border:2px solid rgba(255,255,255,.92); box-shadow:0 1px 5px rgba(24,32,29,.22); z-index:4; }
    .plot-note.latest { width:15px; height:15px; box-shadow:0 0 0 4px rgba(20,108,108,.12),0 2px 7px rgba(24,32,29,.25); }
    .staff-line { position:absolute; left:8px; right:22px; height:1px; background:#aeb8b0; }
    .staff-middle { position:absolute; left:8px; right:22px; height:1px; border-top:1px dashed rgba(24,32,29,.32); }
    .staff-ledger { position:absolute; width:22px; height:1px; transform:translateX(-50%); background:#8f9b92; z-index:3; }
    .staff-clef { position:absolute; left:12px; color:rgba(24,32,29,.26); font-family:Georgia,serif; pointer-events:none; }
    .staff-clef.treble { top:46px; font-size:56px; }
    .staff-clef.bass { top:120px; font-size:43px; }
    .key-sig { position:absolute; transform:translate(-50%,-50%); color:rgba(24,32,29,.58); font-family:Georgia,serif; font-size:16px; font-weight:700; pointer-events:none; }
    .accidental { position:absolute; transform:translate(-135%,-50%); color:var(--ink); font-family:Georgia,serif; font-size:13px; font-weight:700; z-index:5; }
    .octave-mark { position:absolute; transform:translate(-50%,-50%); color:var(--accent); font-size:10px; font-weight:800; z-index:5; white-space:nowrap; }
    .velocity-lane { position:absolute; left:78px; right:22px; bottom:38px; height:82px; border-top:1px solid var(--line); border-bottom:1px solid var(--line); background:rgba(24,32,29,.025); }
    .velocity-grid-line { position:absolute; left:78px; right:22px; height:1px; border-top:1px dashed rgba(24,32,29,.18); z-index:2; }
    .velocity-grid-label { position:absolute; left:50px; transform:translateY(50%); color:var(--muted); font-size:10px; font-weight:700; z-index:2; }
    .velocity-bar { position:absolute; bottom:38px; width:4px; min-height:3px; transform:translateX(-50%); border-radius:3px 3px 0 0; opacity:.9; z-index:4; }
    .velocity-title { position:absolute; left:10px; bottom:100px; color:var(--muted); font-size:11px; font-weight:750; }
    .ruler-tick { position:absolute; bottom:24px; width:1px; height:8px; background:#bfc8c0; z-index:2; }
    .ruler-label { position:absolute; bottom:8px; transform:translateX(-50%); color:var(--muted); font-size:10px; font-weight:700; white-space:nowrap; z-index:2; }
    .timeline-empty { display:flex; align-items:center; justify-content:center; height:100%; color:var(--muted); font-weight:750; }
    table { width:100%; border-collapse:collapse; font-size:14px; } th,td { padding:9px 10px; border-bottom:1px solid var(--line); text-align:left; white-space:nowrap; } th { position:sticky; top:0; background:#eef3ef; }
    .table-wrap,.raw { max-height:390px; overflow:auto; border:1px solid var(--line); border-radius:8px; }
    .group-row td { background:#eef3ef; color:#34403a; } .group-meta { display:flex; align-items:center; gap:12px; flex-wrap:wrap; }
    .pill { display:inline-flex; min-width:38px; min-height:24px; align-items:center; justify-content:center; border-radius:6px; background:var(--soft); font-weight:750; }
    .history-meter { display:grid; grid-template-columns:minmax(82px,1fr) 32px; align-items:center; gap:8px; }
    .history-meter span { display:block; height:9px; border-radius:999px; }
    .raw { display:grid; gap:6px; max-height:210px; padding:8px; background:#fbfcfa; }
    .raw-row { display:grid; grid-template-columns:92px 110px 48px minmax(100px,160px); gap:10px; min-height:28px; color:var(--muted); font-size:13px; align-items:center; }
    .raw-row strong { color:var(--ink); } code { color:var(--red); font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace; }
    .velocity-pp { background:var(--pp); } .velocity-p { background:var(--p); } .velocity-mp { background:var(--mp); } .velocity-mf { background:var(--mf); } .velocity-f { background:var(--f); } .velocity-ff { background:var(--ff); }
    @media (max-width:860px){ header,.grid,.lower{grid-template-columns:1fr;display:grid;align-items:start;} header{align-items:start;} .tools{justify-content:flex-start;} .raw-row{grid-template-columns:86px 1fr 44px;} code{grid-column:1/-1;} }
  </style>
</head>
<body>
  <main>
    <header>
      <div>
        <h1>FP-10 Native MIDI Monitor</h1>
        <p class="subtitle">Rust 后端读取 MIDI，浏览器只显示数据。</p>
      </div>
      <div class="status" id="status">连接中</div>
    </header>
    <section class="grid">
      <section class="panel">
        <div class="heading"><h2>当前和弦</h2><div class="summary" id="chordSummary">等待输入</div></div>
        <div class="keyboard" id="keyboard"></div>
        <div class="cards" id="cards"></div>
      </section>
      <section class="panel meter">
        <div class="heading"><h2>最近一次击键</h2><div class="summary" id="lastName">-</div></div>
        <strong id="lastVelocity">0</strong>
        <div class="bigbar"><div class="fill" id="lastBar" style="width:0%"></div></div>
        <div class="dynamic" id="lastMark">-</div>
      </section>
    </section>
    <section class="panel timeline">
      <div class="heading">
        <div><h2>最近演奏</h2><div class="summary" id="timelineSummary">时间从左到右，最新在右侧</div></div>
        <div class="tools">
          <select id="keySelect" aria-label="key">
            <option value="C">C</option><option value="G">G</option><option value="D">D</option><option value="A">A</option><option value="E">E</option><option value="F">F</option><option value="Bb">Bb</option><option value="Eb">Eb</option>
          </select>
          <button type="button" class="ghost" id="clearView">清屏</button>
          <button type="button" class="ghost" id="seedLog">载入日志</button>
        </div>
      </div>
      <div class="plot" id="timeline"></div>
    </section>
    <section class="lower">
      <section class="panel">
        <div class="heading"><h2>击键 Log</h2><div class="summary" id="historySummary">0 groups</div></div>
        <div class="table-wrap"><table><thead><tr><th>时间</th><th>音</th><th>力度</th><th>动态</th></tr></thead><tbody id="history"></tbody></table></div>
      </section>
      <section class="panel">
        <div class="heading"><h2>原始 MIDI</h2><div class="summary" id="rawSummary">0 messages</div></div>
        <div class="raw" id="raw"></div>
      </section>
    </section>
  </main>
  <script>
    const state = { cutoffTimeMs: 0, cutoffMessageCount: 0 };
    const bands = [{value:20,label:"pp"},{value:40,label:"p"},{value:60,label:"mp"},{value:80,label:"mf"},{value:100,label:"f"},{value:127,label:"ff"}];
    const $ = (id) => document.getElementById(id);
    function cls(v){ if(v<=20)return"velocity-pp"; if(v<=40)return"velocity-p"; if(v<=60)return"velocity-mp"; if(v<=80)return"velocity-mf"; if(v<=100)return"velocity-f"; return"velocity-ff"; }
    function color(v){ return getComputedStyle(document.documentElement).getPropertyValue(`--${cls(v).replace("velocity-","")}`).trim(); }
    function notePos(note){ const names=["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"]; const isBlack=(n)=>names[n%12].includes("#"); let count=0; for(let n=21;n<Math.max(21,Math.min(108,note));n++) if(!isBlack(n)) count++; const center=isBlack(note)?count:count+.5; return center/52*100; }
    function time(ms){ return new Date(Number(ms)).toLocaleTimeString("zh-CN",{hour12:false,hour:"2-digit",minute:"2-digit",second:"2-digit",fractionalSecondDigits:2}); }
    function spell(note){ return spellingFor(note).name; }
    function spellingFor(note){
      const pc=note%12; const octave=Math.floor(note/12)-1; const info=keyInfo();
      if(info.scale[pc]){
        const s=info.scale[pc];
        return {letter:s.letter,letterIndex:s.index,octave,accidental:"",name:`${s.letter}${s.keyAccidental||""}${octave}`};
      }
      const naturals={0:["C",0],2:["D",1],4:["E",2],5:["F",3],7:["G",4],9:["A",5],11:["B",6]};
      if(naturals[pc]){
        const [letter,index]=naturals[pc];
        return {letter,letterIndex:index,octave,accidental:"♮",name:`${letter}${octave}`};
      }
      const sharp={1:["C",0],3:["D",1],6:["F",3],8:["G",4],10:["A",5]};
      const flat={1:["D",1],3:["E",2],6:["G",4],8:["A",5],10:["B",6]};
      const [letter,index]=(info.prefer==="flat"?flat:sharp)[pc];
      const accidental=info.prefer==="flat"?"♭":"♯";
      return {letter,letterIndex:index,octave,accidental,name:`${letter}${accidental}${octave}`};
    }
    function render(data){
      $("status").textContent = `${data.inputName} · ${data.messageCount} messages`;
      const last = data.lastNote;
      const history = visibleHistory(data.history || []);
      const raw = (data.raw || []).filter(e=>Number(e.timeMs)>state.cutoffTimeMs);
      if(last && history.length){ $("lastName").textContent=last.name; $("lastVelocity").textContent=last.velocity; $("lastBar").style.width=`${last.velocity/127*100}%`; $("lastBar").className=`fill ${cls(last.velocity)}`; $("lastMark").textContent=last.mark; }
      else { $("lastName").textContent="-"; $("lastVelocity").textContent="0"; $("lastBar").style.width="0%"; $("lastMark").textContent="-"; }
      renderChord(data.currentChord);
      renderTimeline(history);
      renderHistory(history);
      renderRaw(raw, Math.max(0, data.messageCount - state.cutoffMessageCount));
    }
    function visibleHistory(history){ return history.filter(group=>Number(group.timeMs)>state.cutoffTimeMs); }
    function renderChord(group){
      const keyboard=$("keyboard"); const cards=$("cards"); keyboard.innerHTML=""; cards.innerHTML="";
      const chordGraphHeight = 160;
      for(const band of bands){ const g=document.createElement("div"); g.className="guide"; g.style.bottom=`${band.value/127*chordGraphHeight}px`; g.innerHTML=`<span>${band.label}</span><b>${band.value}</b>`; keyboard.append(g); }
      if(!group){ $("chordSummary").textContent="等待输入"; return; }
      $("chordSummary").textContent=`${group.notes.length} 音 · 平均 ${group.average} · 差距 ${group.spread}`;
      for(const note of group.notes){ const marker=document.createElement("div"); marker.className=`key ${cls(note.velocity)}`; marker.style.left=`${notePos(note.note)}%`; marker.style.height=`${Math.max(2,note.velocity/127*chordGraphHeight)}px`; marker.innerHTML=`<span class="value">${note.name} ${note.velocity}</span>`; keyboard.append(marker);
        const card=document.createElement("article"); card.className="card"; card.innerHTML=`<strong>${note.name}</strong><div class="mini"><span>力度</span><div class="track"><div class="fill ${cls(note.velocity)}" style="width:${note.velocity/127*100}%"></div></div><b>${note.velocity}</b></div>`; cards.append(card); }
    }
    function renderTimeline(history){
      const timeline=$("timeline"); const groups=visibleGroups(history); timeline.innerHTML="";
      if(!groups.length){ timeline.innerHTML=`<div class="timeline-empty">等待输入</div>`; return; }
      const span=(groups[groups.length-1].timeMs-groups[0].timeMs)/1000;
      $("timelineSummary").textContent = `${$("keySelect").value} 调 · ${groups.length} 组 · ${span.toFixed(1)}s · 上方音高，下方力度`;
      renderStaffPlot(timeline, groups);
    }
    function visibleGroups(history){
      const ordered=history.slice(0,48).reverse();
      if(ordered.length<2) return ordered;
      const latest=ordered[ordered.length-1].timeMs;
      const earliest=ordered[0].timeMs;
      const fullSpan=Math.max(1,latest-earliest);
      const targetWindow=Math.min(Math.max(fullSpan,4500),22000);
      const start=latest-targetWindow;
      return ordered.filter(group=>group.timeMs>=start);
    }
    function timeScale(groups){
      const left=12, right=4;
      if(groups.length<=1) return {left,right,start:0,span:1};
      const latest=groups[groups.length-1].timeMs;
      const earliest=groups[0].timeMs;
      const span=Math.max(4500,latest-earliest);
      return {left,right,start:latest-span,span};
    }
    function xForTime(group,scale){ return scale.left + ((group.timeMs-scale.start)/scale.span) * (100-scale.left-scale.right); }
    function add(el, clsName, style, text){ const node=document.createElement("div"); node.className=clsName; Object.assign(node.style, style); if(text!==undefined) node.textContent=text; el.append(node); return node; }
    function drawRuler(el, scale){
      const seconds=Math.ceil(scale.span/1000); const step=seconds>12?2:1;
      for(let s=0;s<=seconds;s+=step){
        const ms=scale.start+s*1000; const x=scale.left+((ms-scale.start)/scale.span)*(100-scale.left-scale.right);
        if(x<scale.left-.1 || x>100-scale.right+.1) continue;
        add(el,"ruler-tick",{left:`${x}%`});
        add(el,"ruler-label",{left:`${x}%`},`${Math.round((ms-(scale.start+scale.span))/1000)}s`);
      }
    }
    function renderStaffPlot(el, groups){
      const scale=timeScale(groups);
      drawRuler(el, scale);
      [62,74,86,98,110,134,146,158,170,182].forEach(y=>add(el,"staff-line",{top:`${y}px`}));
      add(el,"staff-middle",{top:"122px"});
      add(el,"staff-clef treble",{},"𝄞");
      add(el,"staff-clef bass",{},"𝄢");
      renderKeySignature(el);
      add(el,"velocity-lane",{});
      renderVelocityGrid(el);
      add(el,"velocity-title",{},"velocity");
      const accidentalMemory = new Map();
      let eventIndex = 0;
      groups.forEach((group,index)=>{
        const x=xForTime(group,scale); const latest=index===groups.length-1;
        group.notes.forEach((note,noteIndex)=>{
          const spelling=spellingFor(note.note);
          const display=displayStaffPosition(spelling);
          const y=display.y; const dx=(noteIndex-(group.notes.length-1)/2)*9; const c=color(note.velocity);
          for(const lineY of ledgerLines(y)){ add(el,"staff-ledger",{left:`calc(${x}% + ${dx}px)`,top:`${lineY}px`}); }
          const marker=add(el,`plot-note ${latest?"latest":""}`,{left:`calc(${x}% + ${dx}px)`,top:`${y}px`,background:c},"");
          marker.title=`${spelling.name}${display.mark ? " · " + display.mark : ""} · ${note.velocity}`;
          const accidental=accidentalForSpelling(accidentalMemory, spelling, group.timeMs, eventIndex);
          if(accidental) add(el,"accidental",{left:`calc(${x}% + ${dx - noteIndex * 5}px)`,top:`${y}px`},accidental);
          if(display.mark) add(el,"octave-mark",{left:`calc(${x}% + ${dx}px)`,top:`${display.markY}px`},display.mark);
          add(el,"velocity-bar",{left:`calc(${x}% + ${dx}px)`,height:`${Math.max(3,note.velocity/127*82)}px`,background:c}).title=`${spelling.name} · ${note.velocity}`;
          eventIndex += 1;
        });
      });
    }
    function renderVelocityGrid(el){
      const laneBottom=38, laneHeight=82;
      for(const value of [0,20,40,60,80,100,127]){
        const bottom=laneBottom + value/127*laneHeight;
        add(el,"velocity-grid-line",{bottom:`${bottom}px`});
        add(el,"velocity-grid-label",{bottom:`${bottom}px`},String(value));
      }
    }
    function staffStepFromSpelling(spelling){ return spelling.octave*7+spelling.letterIndex; }
    function staffYFromSpelling(spelling){ return 122-(staffStepFromSpelling(spelling)-28)*6; }
    function staffY(note){ return staffYFromSpelling(spellingFor(note)); }
    function displayStaffPosition(spelling){
      let y=staffYFromSpelling(spelling); let octaveShift=0;
      while(y<26){ y+=42; octaveShift+=1; }
      while(y>224){ y-=42; octaveShift-=1; }
      const mark=octaveShift===0 ? "" : octaveShift===1 ? "8va" : octaveShift>=2 ? "15ma" : octaveShift===-1 ? "8vb" : "15mb";
      return {y,mark,markY:octaveShift>0?Math.max(14,y-18):Math.min(232,y+18)};
    }
    function ledgerLines(y){
      const lines=[];
      if(Math.abs(y-122)<1) lines.push(122);
      if(y<62){ for(let lineY=50; lineY>=Math.max(26,y-1); lineY-=12) lines.push(lineY); }
      if(y>182){ for(let lineY=194; lineY<=Math.min(224,y+1); lineY+=12) lines.push(lineY); }
      return lines;
    }
    function keyInfo(){
      const natural={0:["C",0,""],2:["D",1,""],4:["E",2,""],5:["F",3,""],7:["G",4,""],9:["A",5,""],11:["B",6,""]};
      const configs={
        C:{prefer:"sharp",alter:{}},
        G:{prefer:"sharp",alter:{6:["F",3,"♯"]}},
        D:{prefer:"sharp",alter:{6:["F",3,"♯"],1:["C",0,"♯"]}},
        A:{prefer:"sharp",alter:{6:["F",3,"♯"],1:["C",0,"♯"],8:["G",4,"♯"]}},
        E:{prefer:"sharp",alter:{6:["F",3,"♯"],1:["C",0,"♯"],8:["G",4,"♯"],3:["D",1,"♯"]}},
        F:{prefer:"flat",alter:{10:["B",6,"♭"]}},
        Bb:{prefer:"flat",alter:{10:["B",6,"♭"],3:["E",2,"♭"]}},
        Eb:{prefer:"flat",alter:{10:["B",6,"♭"],3:["E",2,"♭"],8:["A",5,"♭"]}}
      };
      const config=configs[$("keySelect").value] || configs.C;
      const scale={};
      for(const [pc,parts] of Object.entries(natural)){
        scale[pc]={letter:parts[0],index:parts[1],keyAccidental:parts[2]};
      }
      for(const [pc,parts] of Object.entries(config.alter)){
        scale[pc]={letter:parts[0],index:parts[1],keyAccidental:parts[2]};
        for(const naturalPc of Object.keys(scale)){
          if(scale[naturalPc].letter===parts[0] && naturalPc!==pc) delete scale[naturalPc];
        }
      }
      return {prefer:config.prefer,scale};
    }
    function renderKeySignature(el){
      for(const item of keySignatureMarks()){
        add(el,"key-sig",{left:`${55 + item.index * 8}px`,top:`${staffY(item.treble)}px`},item.symbol);
        add(el,"key-sig",{left:`${55 + item.index * 8}px`,top:`${staffY(item.bass)}px`},item.symbol);
      }
    }
    function keySignatureMarks(){
      const key=$("keySelect").value;
      const sharps=[
        {treble:78,bass:42},
        {treble:73,bass:37},
        {treble:80,bass:44},
        {treble:75,bass:39}
      ];
      const flats=[
        {treble:70,bass:46},
        {treble:75,bass:51},
        {treble:68,bass:44}
      ];
      const sharpCount={G:1,D:2,A:3,E:4}[key]||0;
      const flatCount={F:1,Bb:2,Eb:3}[key]||0;
      if(sharpCount) return sharps.slice(0,sharpCount).map((mark,index)=>({...mark,index,symbol:"♯"}));
      if(flatCount) return flats.slice(0,flatCount).map((mark,index)=>({...mark,index,symbol:"♭"}));
      return [];
    }
    function accidentalForSpelling(memory, spelling, timeMs, eventIndex){
      const key=spelling.letter;
      const last=memory.get(key);
      const accidental=spelling.accidental;
      let show=false;
      if(accidental){
        show=!last || last.accidental!==accidental || timeMs-last.timeMs>1100 || eventIndex-last.index>4;
      } else if(last && last.accidental && (timeMs-last.timeMs<2200 || eventIndex-last.index<=6)) {
        show=true;
      }
      memory.set(key,{accidental,timeMs,index:eventIndex});
      return show ? (accidental || "♮") : "";
    }
    function renderHistory(history){
      $("historySummary").textContent=`${history.length} groups`;
      $("history").innerHTML = history.map((group,index)=>`<tr class="group-row"><td colspan="4"><div class="group-meta"><strong>#${history.length-index}</strong><span>${time(group.timeMs)}</span><span>${group.notes.length} 音 · ${group.notes.map(n=>n.name).join(" ")}</span><span>平均 ${group.average}</span><span>差距 ${group.spread}</span></div></td></tr>${group.notes.map(note=>`<tr><td></td><td><span class="pill">${note.name}</span></td><td><div class="history-meter"><span class="${cls(note.velocity)}" style="width:${note.velocity/127*100}%"></span><b>${note.velocity}</b></div></td><td>${note.mark}</td></tr>`).join("")}`).join("");
    }
    function renderRaw(raw,count){ $("rawSummary").textContent=`${count} messages`; $("raw").innerHTML=raw.map(e=>`<div class="raw-row"><span>${time(e.timeMs)}</span><strong>${e.kind}</strong><span>ch ${e.channel}</span><code>${e.bytes}</code></div>`).join(""); }
    $("clearView").addEventListener("click",async()=>{ try{ const data=await fetch("/state",{cache:"no-store"}).then(r=>r.json()); state.cutoffTimeMs=Date.now(); state.cutoffMessageCount=data.messageCount||0; render({...data,history:[],raw:[],lastNote:null,currentChord:null}); } catch(e){ state.cutoffTimeMs=Date.now(); } });
    $("seedLog").addEventListener("click",async()=>{ const button=$("seedLog"); button.disabled=true; button.textContent="载入中"; try{ const result=await fetch("/seed-log",{cache:"no-store"}).then(r=>r.json()); button.textContent=result.loaded ? `已载入 ${result.loaded}` : "无日志"; } catch(e){ button.textContent="载入失败"; } setTimeout(()=>{ button.disabled=false; button.textContent="载入日志"; },1400); });
    async function tick(){ try{ const data=await fetch("/state",{cache:"no-store"}).then(r=>r.json()); render(data); } catch(e){ $("status").textContent="backend offline"; } setTimeout(tick,120); }
    tick();
  </script>
</body>
</html>"####;

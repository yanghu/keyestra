#![cfg_attr(windows, windows_subsystem = "console")]

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use midir::{Ignore, MidiInput, MidiInputPort};

#[path = "../cfx_render.rs"]
mod cfx_render;
#[path = "../metronome.rs"]
mod metronome;
#[path = "../midi_clip.rs"]
mod midi_clip;
#[path = "../midi_preview.rs"]
mod midi_preview;
#[path = "../pianoteq.rs"]
mod pianoteq;
#[path = "../pianoteq_favorites.rs"]
mod pianoteq_favorites;
#[path = "../practice.rs"]
mod practice;
#[path = "../reaper.rs"]
mod reaper;
#[path = "../recorder.rs"]
mod recorder;

use cfx_render::{CfxRenderConfig, CfxRenderFormat, CfxRenderer};
use metronome::{Metronome, MetronomePatch};
use midi_preview::{MidiPreview, PreviewError, PreviewErrorCode};
use pianoteq::PianoteqClient;
use pianoteq_favorites::PianoteqFavoriteStore;
use practice::{Practice, PracticeSettings};
use reaper::ReaperClient;
use recorder::{Recorder, RecorderError, RecorderErrorCode, TimelineRequest};

const DEFAULT_MIDI_PORT: &str = "Keyestra MIDI";
const LEGACY_MIDI_PORT: &str = "FP10 Mapped";

#[derive(Debug, Parser)]
#[command(name = "keyestra-monitor")]
#[command(about = "Keyestra monitor backend with a local web dashboard")]
struct Cli {
    #[arg(long, help = "List MIDI input ports")]
    list: bool,

    #[arg(
        long = "in",
        default_value = "Keyestra MIDI",
        help = "MIDI input port name substring or index"
    )]
    input: String,

    #[arg(long, default_value_t = 8770, help = "Local HTTP port")]
    port: u16,

    #[arg(long, default_value = "0.0.0.0", help = "HTTP bind host")]
    host: String,

    #[arg(
        long,
        default_value = "127.0.0.1:8081",
        help = "Pianoteq JSON-RPC host and port"
    )]
    pianoteq_rpc: String,

    #[arg(
        long,
        help = "REAPER Piano Compare TOML config (defaults to %APPDATA%\\keyestra\\reaper-pianos.toml)"
    )]
    reaper_config: Option<PathBuf>,

    #[arg(long, help = "Directory for saved MIDI recordings")]
    recordings_dir: Option<PathBuf>,

    #[arg(long, help = "Path to reaper.exe for CFX audio rendering")]
    reaper: Option<PathBuf>,

    #[arg(
        long,
        help = "Path to the Keyestra CFX WAV Render REAPER project template"
    )]
    cfx_template: Option<PathBuf>,

    #[arg(
        long,
        help = "Path to the Keyestra CFX MP3 Render REAPER project template"
    )]
    cfx_mp3_template: Option<PathBuf>,
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

    let mut midi_in = MidiInput::new("keyestra-monitor-input")?;
    midi_in.ignore(Ignore::None);
    let selector = PortSelector::parse(&cli.input);
    let input_port = select_input_port(&midi_in, &selector)?;
    let input_name = midi_in.port_name(&input_port)?;
    let state = Arc::new(Mutex::new(MonitorState::new(input_name.clone())));
    let metronome = Arc::new(Metronome::new());
    let practice = Arc::new(Practice::new());
    let recordings_dir = cli
        .recordings_dir
        .map(Ok)
        .unwrap_or_else(Recorder::default_directory)?;
    let recorder = Arc::new(Recorder::new(recordings_dir.clone())?);
    let cfx_renderer = Arc::new(CfxRenderer::new(CfxRenderConfig::discover(
        recordings_dir,
        cli.reaper,
        cli.cfx_template,
        cli.cfx_mp3_template,
    )));
    let preview = Arc::new(MidiPreview::new(Arc::clone(&recorder)));
    let recorder_control = Arc::new(Mutex::new(()));
    let pianoteq = Arc::new(PianoteqClient::new(cli.pianoteq_rpc));
    let pianoteq_favorites = Arc::new(PianoteqFavoriteStore::discover()?);
    let reaper = Arc::new(ReaperClient::discover(cli.reaper_config));
    let midi_state = Arc::clone(&state);
    let midi_recorder = Arc::clone(&recorder);
    let midi_metronome = Arc::clone(&metronome);
    let midi_practice = Arc::clone(&practice);

    let _conn = midi_in
        .connect(
            &input_port,
            "keyestra-monitor-input",
            move |_timestamp, message, _| {
                let timing = midi_metronome.grid_timing(Instant::now());
                midi_practice.ingest(message, timing);
                midi_recorder.ingest(message);
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
                let metronome = Arc::clone(&metronome);
                let practice = Arc::clone(&practice);
                let recorder = Arc::clone(&recorder);
                let preview = Arc::clone(&preview);
                let cfx_renderer = Arc::clone(&cfx_renderer);
                let recorder_control = Arc::clone(&recorder_control);
                let pianoteq = Arc::clone(&pianoteq);
                let pianoteq_favorites = Arc::clone(&pianoteq_favorites);
                let reaper = Arc::clone(&reaper);
                thread::spawn(move || {
                    if let Err(error) = handle_client(
                        stream,
                        state,
                        metronome,
                        practice,
                        recorder,
                        preview,
                        cfx_renderer,
                        recorder_control,
                        pianoteq,
                        pianoteq_favorites,
                        reaper,
                    ) {
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
    let midi_in = MidiInput::new("keyestra-monitor-list-inputs")?;
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
            for candidate in input_name_candidates(name) {
                for port in &ports {
                    let port_name = midi_in.port_name(port)?;
                    if port_name.to_lowercase().contains(&candidate.to_lowercase()) {
                        return Ok(port.clone());
                    }
                }
            }
            Err(input_port_error(selector, midi_in, &ports))
        }
    }
}

fn input_name_candidates(name: &str) -> Vec<&str> {
    if name.eq_ignore_ascii_case(DEFAULT_MIDI_PORT) {
        vec![name, LEGACY_MIDI_PORT]
    } else {
        vec![name]
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

fn handle_client(
    mut stream: TcpStream,
    state: Arc<Mutex<MonitorState>>,
    metronome: Arc<Metronome>,
    practice: Arc<Practice>,
    recorder: Arc<Recorder>,
    preview: Arc<MidiPreview>,
    cfx_renderer: Arc<CfxRenderer>,
    recorder_control: Arc<Mutex<()>>,
    pianoteq: Arc<PianoteqClient>,
    pianoteq_favorites: Arc<PianoteqFavoriteStore>,
    reaper: Arc<ReaperClient>,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;

    let mut buffer = [0_u8; 2048];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let request_line = request.lines().next().unwrap_or("");
    let method = request_line.split_whitespace().next().unwrap_or("GET");
    let target = request_line.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = split_target(target);
    let has_control_header = request.lines().skip(1).any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            (name.eq_ignore_ascii_case("X-Keyestra-Control")
                || name.eq_ignore_ascii_case("X-FP10-Control"))
                && value.trim() == "1"
        })
    });
    let range_header = request_header(&request, "Range");

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
        "/assets/keyestra-app.svg" => respond(
            &mut stream,
            "200 OK",
            "image/svg+xml; charset=utf-8",
            KEYESTRA_APP_ICON,
        ),
        "/assets/keyestra-tray.svg" | "/favicon.svg" => respond(
            &mut stream,
            "200 OK",
            "image/svg+xml; charset=utf-8",
            KEYESTRA_TRAY_ICON,
        ),
        "/version" => {
            let body = serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "build": env!("KEYESTRA_BUILD_ID"),
            })
            .to_string();
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        "/pianoteq" => {
            if method == "GET" {
                let body = serde_json::to_string(&pianoteq.snapshot())?;
                respond(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    &body,
                )
            } else if method != "POST" {
                respond(
                    &mut stream,
                    "405 Method Not Allowed",
                    "text/plain; charset=utf-8",
                    "Pianoteq control requires GET or POST",
                )
            } else if !has_control_header {
                respond(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain; charset=utf-8",
                    "Missing X-Keyestra-Control: 1 header",
                )
            } else {
                let action = query_value(query, "action").unwrap_or_else(|| "load".to_string());
                if action == "parameter" {
                    let id = query_value(query, "id").unwrap_or_default();
                    let result = query_value(query, "value")
                        .and_then(|value| value.parse::<f64>().ok())
                        .ok_or_else(|| anyhow::anyhow!("Missing or invalid control value"))
                        .and_then(|value| pianoteq.set_parameter(&id, value))
                        .map(|controls| serde_json::json!({"controls": controls}));
                    result_json_response(&mut stream, result)
                } else if action == "volume" {
                    let result = query_value(query, "value")
                        .and_then(|value| value.parse::<f64>().ok())
                        .ok_or_else(|| anyhow::anyhow!("Missing or invalid volume value"))
                        .and_then(|value| pianoteq.set_volume_db(value))
                        .map(|controls| serde_json::json!({"controls": controls}));
                    result_json_response(&mut stream, result)
                } else if action == "reverb" {
                    let name = query_value(query, "name").unwrap_or_default();
                    let bank = query_value(query, "bank").unwrap_or_default();
                    let result = pianoteq
                        .load_reverb_preset(&name, &bank)
                        .and_then(|snapshot| serde_json::to_value(snapshot).map_err(Into::into));
                    result_json_response(&mut stream, result)
                } else if action == "load" {
                    let name = query_value(query, "name").unwrap_or_default();
                    let bank = query_value(query, "bank").unwrap_or_default();
                    let result = pianoteq
                        .load_preset(&name, &bank)
                        .and_then(|snapshot| serde_json::to_value(snapshot).map_err(Into::into));
                    result_json_response(&mut stream, result)
                } else {
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "application/json; charset=utf-8",
                        &serde_json::json!({"error": "Unknown Pianoteq action"}).to_string(),
                    )
                }
            }
        }
        "/pianoteq-favorites" => {
            if method == "GET" {
                result_json_response(
                    &mut stream,
                    pianoteq_favorites
                        .snapshot()
                        .and_then(|favorites| serde_json::to_value(favorites).map_err(Into::into)),
                )
            } else if method != "POST" {
                respond(
                    &mut stream,
                    "405 Method Not Allowed",
                    "text/plain; charset=utf-8",
                    "Pianoteq favorites require GET or POST",
                )
            } else if !has_control_header {
                respond(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain; charset=utf-8",
                    "Missing X-Keyestra-Control: 1 header",
                )
            } else {
                let kind = query_value(query, "kind").unwrap_or_default();
                let key = query_value(query, "key").unwrap_or_default();
                let favorite = query_value(query, "favorite")
                    .and_then(|value| value.parse::<bool>().ok())
                    .ok_or_else(|| anyhow::anyhow!("Missing or invalid favorite value"));
                let result = favorite
                    .and_then(|favorite| pianoteq_favorites.set(&kind, &key, favorite))
                    .and_then(|favorites| serde_json::to_value(favorites).map_err(Into::into));
                result_json_response(&mut stream, result)
            }
        }
        "/reaper-pianos" => {
            if method == "GET" {
                let body = serde_json::to_string(&reaper.snapshot())?;
                respond(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    &body,
                )
            } else if method != "POST" {
                respond(
                    &mut stream,
                    "405 Method Not Allowed",
                    "text/plain; charset=utf-8",
                    "REAPER piano control requires GET or POST",
                )
            } else if !has_control_header {
                respond(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain; charset=utf-8",
                    "Missing X-Keyestra-Control: 1 header",
                )
            } else {
                let piano_id = query_value(query, "id").unwrap_or_default();
                let result = reaper
                    .activate(&piano_id)
                    .and_then(|snapshot| serde_json::to_value(snapshot).map_err(Into::into));
                result_json_response(&mut stream, result)
            }
        }
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
        "/metronome-events" => respond_metronome_events(stream, metronome),
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
        "/metronome" => {
            if !query.is_empty() {
                metronome.apply_patch(parse_metronome_patch(query));
            }
            let json = metronome.to_json();
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &json,
            )
        }
        "/practice" => {
            if method == "GET" {
                let body = practice.to_json(metronome.grid_timing(Instant::now()));
                respond(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    &body,
                )
            } else if method != "POST" {
                respond_api_error(
                    &mut stream,
                    "405 Method Not Allowed",
                    "invalid_parameter",
                    "Practice mutations must use POST",
                    None,
                )
            } else if !has_control_header {
                respond_api_error(
                    &mut stream,
                    "400 Bad Request",
                    "invalid_parameter",
                    "Missing X-Keyestra-Control: 1 header",
                    None,
                )
            } else {
                let action = query_value(query, "action");
                let result = match action.as_deref() {
                    Some("start") => {
                        let settings = parse_practice_settings(query);
                        practice.start(settings);
                        metronome.apply_patch(MetronomePatch {
                            running: Some(true),
                            bpm: Some(settings.bpm),
                            beats_per_bar: Some(settings.beats_per_bar),
                            subdivision: Some(settings.subdivision),
                            volume: None,
                            output_device: None,
                        });
                        Ok(())
                    }
                    Some("pause") => {
                        practice.pause(metronome.grid_timing(Instant::now()));
                        metronome.apply_patch(MetronomePatch {
                            running: Some(false),
                            bpm: None,
                            beats_per_bar: None,
                            subdivision: None,
                            volume: None,
                            output_device: None,
                        });
                        Ok(())
                    }
                    Some("resume") => {
                        practice.resume();
                        metronome.apply_patch(MetronomePatch {
                            running: Some(true),
                            bpm: None,
                            beats_per_bar: None,
                            subdivision: None,
                            volume: None,
                            output_device: None,
                        });
                        Ok(())
                    }
                    Some("finish") => {
                        practice.finish(metronome.grid_timing(Instant::now()));
                        metronome.apply_patch(MetronomePatch {
                            running: Some(false),
                            bpm: None,
                            beats_per_bar: None,
                            subdivision: None,
                            volume: None,
                            output_device: None,
                        });
                        Ok(())
                    }
                    Some("reset") => {
                        practice.reset();
                        metronome.apply_patch(MetronomePatch {
                            running: Some(false),
                            bpm: None,
                            beats_per_bar: None,
                            subdivision: None,
                            volume: None,
                            output_device: None,
                        });
                        Ok(())
                    }
                    _ => Err("action must be start, pause, resume, finish, or reset"),
                };
                match result {
                    Ok(()) => {
                        let body = practice.to_json(metronome.grid_timing(Instant::now()));
                        respond(
                            &mut stream,
                            "200 OK",
                            "application/json; charset=utf-8",
                            &body,
                        )
                    }
                    Err(message) => respond_api_error(
                        &mut stream,
                        "400 Bad Request",
                        "invalid_parameter",
                        message,
                        None,
                    ),
                }
            }
        }
        "/recorder" => {
            let action = query_value(query, "action");
            let _control = action
                .as_ref()
                .map(|_| {
                    recorder_control
                        .lock()
                        .map_err(|_| anyhow::anyhow!("recorder control lock poisoned"))
                })
                .transpose()?;
            if action.as_deref() == Some("start") {
                if let Some(preview_id) = preview.snapshot().preview_id {
                    let _ = preview.stop(preview_id);
                    for _ in 0..25 {
                        if !recorder.capture_suspended() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                }
            }
            let result = match action.as_deref() {
                Some("start") => recorder.start_take(),
                Some("stop") => recorder.stop_take(),
                Some("save-take") => recorder.save_take().map(|_| ()),
                Some("save-recent") => {
                    let seconds = query_value(query, "seconds")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(300);
                    recorder.save_recent(seconds).map(|_| ())
                }
                Some(_) => Err(anyhow::anyhow!("Unknown recorder action")),
                None => Ok(()),
            };
            let body = match result {
                Ok(()) => recorder.status_json(),
                Err(error) => format!("{{\"error\":\"{}\"}}", escape_json(&error.to_string())),
            };
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                &body,
            )
        }
        "/recorder/timeline" => {
            if method != "GET" {
                respond_api_error(
                    &mut stream,
                    "405 Method Not Allowed",
                    "invalid_parameter",
                    "Timeline requests must use GET",
                    None,
                )
            } else if let Some(session_id) = query_value(query, "sessionId") {
                if session_id != recorder.session_id() {
                    respond_api_error(
                        &mut stream,
                        "409 Conflict",
                        "stale_session",
                        "Recorder session changed; refresh the selection",
                        None,
                    )
                } else {
                    respond_timeline(&mut stream, &recorder, query)
                }
            } else {
                respond_timeline(&mut stream, &recorder, query)
            }
        }
        "/recorder/save-range" => {
            if method != "POST" {
                respond_api_error(
                    &mut stream,
                    "405 Method Not Allowed",
                    "invalid_parameter",
                    "Save range requests must use POST",
                    None,
                )
            } else if !has_control_header {
                respond_api_error(
                    &mut stream,
                    "400 Bad Request",
                    "invalid_parameter",
                    "Missing X-Keyestra-Control: 1 header",
                    None,
                )
            } else {
                let result =
                    required_range_parameters(query).and_then(|(session_id, start_us, end_us)| {
                        recorder.save_range(&session_id, start_us, end_us)
                    });
                match result {
                    Ok(result) => {
                        let body = serde_json::json!({
                            "ok": true,
                            "recording": result.recording,
                            "warnings": result.warnings,
                        })
                        .to_string();
                        respond(
                            &mut stream,
                            "200 OK",
                            "application/json; charset=utf-8",
                            &body,
                        )
                    }
                    Err(error) => respond_recorder_error(&mut stream, &error),
                }
            }
        }
        "/recorder/preview" => {
            if method == "GET" {
                let body = serde_json::json!({
                    "ok": true,
                    "outputs": preview.outputs(),
                    "preview": preview.snapshot(),
                })
                .to_string();
                respond(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    &body,
                )
            } else if method != "POST" {
                respond_api_error(
                    &mut stream,
                    "405 Method Not Allowed",
                    "invalid_parameter",
                    "Preview mutations must use POST",
                    None,
                )
            } else if !has_control_header {
                respond_api_error(
                    &mut stream,
                    "400 Bad Request",
                    "invalid_parameter",
                    "Missing X-Keyestra-Control: 1 header",
                    None,
                )
            } else {
                let _control = recorder_control
                    .lock()
                    .map_err(|_| anyhow::anyhow!("recorder control lock poisoned"))?;
                match query_value(query, "action").as_deref() {
                    Some("play") => {
                        if recorder.take_is_recording() {
                            let error = RecorderError::new(
                                RecorderErrorCode::TakeInProgress,
                                "Stop the active Take before previewing",
                            );
                            respond_recorder_error(&mut stream, &error)
                        } else {
                            handle_preview_play(&mut stream, query, &recorder, &preview)
                        }
                    }
                    Some("stop") => {
                        let result = required_u64(query, "previewId")
                            .map_err(|message| PreviewError {
                                code: PreviewErrorCode::PreviewReplaced,
                                message,
                            })
                            .and_then(|preview_id| preview.stop(preview_id));
                        match result {
                            Ok(()) => {
                                let body = serde_json::json!({
                                    "ok": true,
                                    "preview": preview.snapshot()
                                })
                                .to_string();
                                respond(
                                    &mut stream,
                                    "200 OK",
                                    "application/json; charset=utf-8",
                                    &body,
                                )
                            }
                            Err(error) => respond_preview_error(&mut stream, &error),
                        }
                    }
                    _ => respond_api_error(
                        &mut stream,
                        "400 Bad Request",
                        "invalid_parameter",
                        "action must be play or stop",
                        None,
                    ),
                }
            }
        }
        "/render/cfx" => {
            if method == "GET" {
                let body = serde_json::to_string(&cfx_renderer.snapshot())?;
                respond(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    &body,
                )
            } else if method != "POST" {
                respond_api_error(
                    &mut stream,
                    "405 Method Not Allowed",
                    "invalid_parameter",
                    "CFX render mutations must use POST",
                    None,
                )
            } else if !has_control_header {
                respond_api_error(
                    &mut stream,
                    "400 Bad Request",
                    "invalid_parameter",
                    "Missing X-Keyestra-Control: 1 header",
                    None,
                )
            } else if let Some(name) = query_value(query, "name") {
                let format = query_value(query, "format").unwrap_or_else(|| "mp3".to_string());
                let Some(format) = CfxRenderFormat::parse(&format) else {
                    return respond_api_error(
                        &mut stream,
                        "400 Bad Request",
                        "invalid_parameter",
                        "format must be mp3 or wav",
                        None,
                    );
                };
                match cfx_renderer.request(&name, format) {
                    Ok(job) => {
                        let body = serde_json::json!({ "ok": true, "job": job }).to_string();
                        respond(
                            &mut stream,
                            "202 Accepted",
                            "application/json; charset=utf-8",
                            &body,
                        )
                    }
                    Err(error) => respond_api_error(
                        &mut stream,
                        "400 Bad Request",
                        "cfx_render_failed",
                        &error.to_string(),
                        None,
                    ),
                }
            } else {
                respond_api_error(
                    &mut stream,
                    "400 Bad Request",
                    "invalid_parameter",
                    "Missing recording name",
                    None,
                )
            }
        }
        "/renders/download" => {
            if method != "GET" && method != "HEAD" {
                return respond_api_error(
                    &mut stream,
                    "405 Method Not Allowed",
                    "invalid_parameter",
                    "Render downloads must use GET or HEAD",
                    None,
                );
            }
            let format = query_value(query, "format")
                .as_deref()
                .and_then(CfxRenderFormat::parse)
                .unwrap_or(CfxRenderFormat::Mp3);
            let result = query_value(query, "name")
                .ok_or_else(|| anyhow::anyhow!("Missing recording name"))
                .and_then(|name| cfx_renderer.rendered_path(&name, format));
            match result {
                Ok(path) => respond_file_download(
                    &mut stream,
                    &path,
                    format.content_type(),
                    method == "HEAD",
                    range_header,
                ),
                Err(error) => respond(
                    &mut stream,
                    "404 Not Found",
                    "text/plain; charset=utf-8",
                    &error.to_string(),
                ),
            }
        }
        "/recordings/download" => {
            let result = query_value(query, "name")
                .ok_or_else(|| anyhow::anyhow!("Missing recording name"))
                .and_then(|name| recorder.read_recording(&name).map(|bytes| (name, bytes)));
            match result {
                Ok((name, bytes)) => respond_download(&mut stream, &name, &bytes),
                Err(error) => respond(
                    &mut stream,
                    "404 Not Found",
                    "text/plain; charset=utf-8",
                    &error.to_string(),
                ),
            }
        }
        _ => respond(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "Not found",
        ),
    }
}

fn split_target(target: &str) -> (&str, &str) {
    match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    }
}

fn request_header<'a>(request: &'a str, desired_name: &str) -> Option<&'a str> {
    request.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(desired_name)
            .then(|| value.trim())
    })
}

fn query_value(query: &str, desired_key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == desired_key).then(|| percent_decode(value))
    })
}

fn optional_u64(query: &str, key: &str) -> std::result::Result<Option<u64>, String> {
    query_value(query, key)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{} must be an unsigned integer", key))
        })
        .transpose()
}

fn required_u64(query: &str, key: &str) -> std::result::Result<u64, String> {
    optional_u64(query, key)?.ok_or_else(|| format!("Missing {}", key))
}

fn required_range_parameters(query: &str) -> recorder::RecorderResult<(String, u64, u64)> {
    let session_id = query_value(query, "sessionId").ok_or_else(|| {
        RecorderError::new(RecorderErrorCode::InvalidParameter, "Missing sessionId")
    })?;
    let start_us = required_u64(query, "startUs")
        .map_err(|message| RecorderError::new(RecorderErrorCode::InvalidParameter, message))?;
    let end_us = required_u64(query, "endUs")
        .map_err(|message| RecorderError::new(RecorderErrorCode::InvalidParameter, message))?;
    Ok((session_id, start_us, end_us))
}

fn respond_timeline(stream: &mut TcpStream, recorder: &Recorder, query: &str) -> Result<()> {
    let parsed = (|| {
        let start_us = optional_u64(query, "startUs")?;
        let end_us = optional_u64(query, "endUs")?;
        let bins = query_value(query, "bins")
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| "bins must be an unsigned integer".to_string())
            })
            .transpose()?;
        let notes = match query_value(query, "notes").as_deref() {
            None | Some("0") | Some("false") => false,
            Some("1") | Some("true") => true,
            Some(_) => return Err("notes must be 0 or 1".to_string()),
        };
        Ok(TimelineRequest {
            start_us,
            end_us,
            bins,
            notes,
        })
    })();
    match parsed {
        Ok(request) => match recorder.timeline(request) {
            Ok(snapshot) => {
                let body = serde_json::to_string(&snapshot)?;
                respond(stream, "200 OK", "application/json; charset=utf-8", &body)
            }
            Err(error) => respond_recorder_error(stream, &error),
        },
        Err(message) => respond_api_error(
            stream,
            "400 Bad Request",
            "invalid_parameter",
            &message,
            None,
        ),
    }
}

fn handle_preview_play(
    stream: &mut TcpStream,
    query: &str,
    recorder: &Recorder,
    preview: &MidiPreview,
) -> Result<()> {
    let (session_id, start_us, end_us) = match required_range_parameters(query) {
        Ok(range) => range,
        Err(error) => return respond_recorder_error(stream, &error),
    };
    let position_us = match optional_u64(query, "positionUs") {
        Ok(value) => value.unwrap_or(start_us),
        Err(message) => {
            return respond_api_error(
                stream,
                "400 Bad Request",
                "invalid_parameter",
                &message,
                None,
            )
        }
    };
    if position_us < start_us || position_us >= end_us {
        return respond_api_error(
            stream,
            "400 Bad Request",
            "invalid_range",
            "positionUs must be inside [startUs, endUs)",
            None,
        );
    }
    let output_name = query_value(query, "outputName");
    if output_name.as_ref().is_some_and(|name| name.len() > 128) {
        return respond_api_error(
            stream,
            "400 Bad Request",
            "invalid_parameter",
            "outputName is too long",
            None,
        );
    }
    if let Some(name) = output_name.as_ref() {
        let wanted = name.to_lowercase();
        if !preview
            .outputs()
            .iter()
            .any(|candidate| candidate.to_lowercase().contains(&wanted))
        {
            return respond_api_error(
                stream,
                "404 Not Found",
                "output_not_found",
                "The selected MIDI output was not found",
                None,
            );
        }
    }
    let clip = match recorder.prepare_range(&session_id, position_us, end_us) {
        Ok(clip) => clip,
        Err(error) => return respond_recorder_error(stream, &error),
    };
    match preview.play(session_id, clip, position_us, output_name) {
        Ok(preview_id) => {
            let body = serde_json::json!({
                "ok": true,
                "previewId": preview_id,
                "preview": preview.snapshot(),
            })
            .to_string();
            respond(stream, "200 OK", "application/json; charset=utf-8", &body)
        }
        Err(error) => respond_preview_error(stream, &error),
    }
}

fn respond_recorder_error(stream: &mut TcpStream, error: &RecorderError) -> Result<()> {
    let status = match error.code {
        RecorderErrorCode::InvalidParameter | RecorderErrorCode::InvalidRange => "400 Bad Request",
        RecorderErrorCode::StaleSession
        | RecorderErrorCode::RangeExpired
        | RecorderErrorCode::RangeEmpty
        | RecorderErrorCode::TakeInProgress
        | RecorderErrorCode::StateLockFailed => "409 Conflict",
        RecorderErrorCode::InternalError => "500 Internal Server Error",
    };
    respond_api_error(
        stream,
        status,
        error.code.as_str(),
        &error.message,
        error.available_start_us,
    )
}

fn respond_preview_error(stream: &mut TcpStream, error: &PreviewError) -> Result<()> {
    let status = match error.code {
        PreviewErrorCode::OutputNotFound => "404 Not Found",
        PreviewErrorCode::PreviewReplaced => "409 Conflict",
        PreviewErrorCode::OutputConnectFailed | PreviewErrorCode::PreviewFailed => {
            "500 Internal Server Error"
        }
    };
    respond_api_error(stream, status, error.code.as_str(), &error.message, None)
}

fn respond_api_error(
    stream: &mut TcpStream,
    status: &str,
    code: &str,
    message: &str,
    available_start_us: Option<u64>,
) -> Result<()> {
    let details = available_start_us
        .map(|value| serde_json::json!({"availableStartUs": value}))
        .unwrap_or_else(|| serde_json::json!({}));
    let body = serde_json::json!({
        "ok": false,
        "error": {
            "code": code,
            "message": message,
            "details": details,
        }
    })
    .to_string();
    respond(stream, status, "application/json; charset=utf-8", &body)
}

fn parse_metronome_patch(query: &str) -> MetronomePatch {
    let mut patch = MetronomePatch {
        running: None,
        bpm: None,
        beats_per_bar: None,
        subdivision: None,
        volume: None,
        output_device: None,
    };
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = percent_decode(value);
        match key {
            "running" => {
                patch.running = match value.as_str() {
                    "1" | "true" | "on" => Some(true),
                    "0" | "false" | "off" => Some(false),
                    _ => None,
                };
            }
            "bpm" => patch.bpm = value.parse().ok(),
            "beatsPerBar" => patch.beats_per_bar = value.parse().ok(),
            "subdivision" => patch.subdivision = value.parse().ok(),
            "volume" => patch.volume = value.parse().ok(),
            "outputDevice" => {
                patch.output_device = if value.trim().is_empty() {
                    Some(None)
                } else {
                    Some(Some(value))
                };
            }
            _ => {}
        }
    }
    patch
}

fn parse_practice_settings(query: &str) -> PracticeSettings {
    let defaults = PracticeSettings::default();
    PracticeSettings {
        bpm: query_value(query, "bpm")
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.bpm),
        beats_per_bar: query_value(query, "beatsPerBar")
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.beats_per_bar),
        subdivision: query_value(query, "subdivision")
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.subdivision),
        target_rounds: query_value(query, "targetRounds")
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.target_rounds),
        count_in_bars: query_value(query, "countInBars")
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.count_in_bars),
        hit_window_ms: query_value(query, "hitWindowMs")
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.hit_window_ms),
    }
    .normalized()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
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

fn respond_metronome_events(mut stream: TcpStream, metronome: Arc<Metronome>) -> Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n"
    )?;
    stream.flush()?;

    let mut last_key = String::new();
    let mut ticks_since_heartbeat = 0_u8;
    loop {
        let key = metronome.event_key();
        if key != last_key {
            write!(
                stream,
                "event: metronome\ndata: {}\n\n",
                metronome.event_json()
            )?;
            stream.flush()?;
            last_key = key;
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
    let appdata = std::path::PathBuf::from(appdata);
    let current_log = appdata.join("keyestra").join("tray.log");
    let legacy_log = appdata.join("fp10-map").join("tray.log");
    let log_path = if current_log.exists() {
        current_log
    } else {
        legacy_log
    };
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
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.as_bytes().len(),
        body
    )?;
    Ok(())
}

fn result_json_response(stream: &mut TcpStream, result: Result<serde_json::Value>) -> Result<()> {
    match result {
        Ok(value) => respond(
            stream,
            "200 OK",
            "application/json; charset=utf-8",
            &value.to_string(),
        ),
        Err(error) => respond(
            stream,
            "502 Bad Gateway",
            "application/json; charset=utf-8",
            &serde_json::json!({"error": error.to_string()}).to_string(),
        ),
    }
}

fn respond_download(stream: &mut TcpStream, name: &str, body: &[u8]) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: audio/midi\r\nContent-Disposition: attachment; filename=\"{}\"\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        name,
        body.len()
    )?;
    stream.write_all(body)?;
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
const KEYESTRA_APP_ICON: &str = include_str!("../../assets/icons/keyestra-app.svg");
const KEYESTRA_TRAY_ICON: &str = include_str!("../../assets/icons/keyestra-tray.svg");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_monitor_input_falls_back_to_the_legacy_port() {
        assert_eq!(
            input_name_candidates(DEFAULT_MIDI_PORT),
            vec![DEFAULT_MIDI_PORT, LEGACY_MIDI_PORT]
        );
        assert_eq!(input_name_candidates("Custom Port"), vec!["Custom Port"]);
    }

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

    #[test]
    fn practice_settings_are_parsed_and_clamped() {
        let settings = parse_practice_settings(
            "bpm=999&beatsPerBar=4&subdivision=3&targetRounds=0&countInBars=2&hitWindowMs=5",
        );
        assert_eq!(settings.bpm, 240);
        assert_eq!(settings.beats_per_bar, 4);
        assert_eq!(settings.subdivision, 3);
        assert_eq!(settings.target_rounds, 1);
        assert_eq!(settings.count_in_bars, 2);
        assert_eq!(settings.hit_window_ms, 10.0);
    }

    #[test]
    fn request_headers_are_case_insensitive() {
        let request = "GET / HTTP/1.1\r\nrAnGe: bytes=10-19\r\n\r\n";
        assert_eq!(request_header(request, "Range"), Some("bytes=10-19"));
    }

    #[test]
    fn byte_ranges_support_bounded_open_and_suffix_forms() {
        assert_eq!(
            parse_byte_range(Some("bytes=10-19"), 100),
            Ok(Some(ByteRange { start: 10, end: 19 }))
        );
        assert_eq!(
            parse_byte_range(Some("bytes=90-"), 100),
            Ok(Some(ByteRange { start: 90, end: 99 }))
        );
        assert_eq!(
            parse_byte_range(Some("bytes=-10"), 100),
            Ok(Some(ByteRange { start: 90, end: 99 }))
        );
        assert_eq!(
            parse_byte_range(Some("bytes=90-200"), 100),
            Ok(Some(ByteRange { start: 90, end: 99 }))
        );
    }

    #[test]
    fn invalid_or_unsatisfiable_byte_ranges_are_rejected() {
        assert_eq!(parse_byte_range(Some("bytes=100-"), 100), Err(()));
        assert_eq!(parse_byte_range(Some("bytes=20-10"), 100), Err(()));
        assert_eq!(parse_byte_range(Some("bytes=0-1,5-6"), 100), Err(()));
        assert_eq!(parse_byte_range(Some("bytes=-0"), 100), Err(()));
        assert_eq!(parse_byte_range(Some("bytes=0-"), 0), Err(()));
    }

    #[test]
    fn file_download_returns_partial_content_and_head_has_no_body() {
        let partial = capture_file_download(false, Some("bytes=2-4"));
        let (partial_headers, partial_body) = split_http_response(&partial);
        assert!(partial_headers.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(partial_headers.contains("Accept-Ranges: bytes\r\n"));
        assert!(partial_headers.contains("Content-Range: bytes 2-4/10\r\n"));
        assert!(partial_headers.contains("Content-Length: 3\r\n"));
        assert!(partial_headers.contains("ETag: \""));
        assert_eq!(partial_body, b"234");

        let head = capture_file_download(true, None);
        let (head_headers, head_body) = split_http_response(&head);
        assert!(head_headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(head_headers.contains("Content-Length: 10\r\n"));
        assert!(head_body.is_empty());
    }

    fn capture_file_download(head_only: bool, range: Option<&str>) -> Vec<u8> {
        let path = env::temp_dir().join(format!(
            "keyestra-download-test-{}-{}.mp3",
            std::process::id(),
            now_ms()
        ));
        fs::write(&path, b"0123456789").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let reader = thread::spawn(move || {
            let mut client = TcpStream::connect(address).unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).unwrap();
            response
        });
        let (mut server, _) = listener.accept().unwrap();
        respond_file_download(&mut server, &path, "audio/mpeg", head_only, range).unwrap();
        drop(server);
        let response = reader.join().unwrap();
        fs::remove_file(path).unwrap();
        response
    }

    fn split_http_response(response: &[u8]) -> (&str, &[u8]) {
        let separator = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        (
            std::str::from_utf8(&response[..separator + 4]).unwrap(),
            &response[separator + 4..],
        )
    }
}

fn respond_file_download(
    stream: &mut TcpStream,
    path: &std::path::Path,
    content_type: &str,
    head_only: bool,
    range_header: Option<&str>,
) -> Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(60)))?;
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    let total_length = metadata.len();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Download file name is not valid UTF-8"))?;
    let safe_name = name.replace(['"', '\r', '\n'], "");
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let etag = format!("\"{:x}-{:x}\"", total_length, modified);

    let requested_range = match parse_byte_range(range_header, total_length) {
        Ok(range) => range,
        Err(()) => {
            write!(
                stream,
                "HTTP/1.1 416 Range Not Satisfiable\r\nAccept-Ranges: bytes\r\nContent-Range: bytes */{}\r\nETag: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                total_length, etag
            )?;
            return Ok(());
        }
    };

    let (status, start, end, content_length) = match requested_range {
        Some(range) => (
            "206 Partial Content",
            range.start,
            range.end,
            range.end - range.start + 1,
        ),
        None => ("200 OK", 0, total_length.saturating_sub(1), total_length),
    };
    let content_range = requested_range
        .map(|range| {
            format!(
                "Content-Range: bytes {}-{}/{}\r\n",
                range.start, range.end, total_length
            )
        })
        .unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Disposition: attachment; filename=\"{}\"\r\nCache-Control: private, no-cache\r\nAccept-Ranges: bytes\r\n{}ETag: {}\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        safe_name,
        content_range,
        etag,
        content_length
    )?;
    if !head_only && content_length > 0 {
        file.seek(SeekFrom::Start(start))?;
        std::io::copy(&mut file.take(end - start + 1), stream)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteRange {
    start: u64,
    end: u64,
}

fn parse_byte_range(
    value: Option<&str>,
    total_length: u64,
) -> std::result::Result<Option<ByteRange>, ()> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(specification) = value.strip_prefix("bytes=") else {
        return Ok(None);
    };
    if total_length == 0 || specification.contains(',') {
        return Err(());
    }
    let (start, end) = specification.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix_length = end.parse::<u64>().map_err(|_| ())?;
        if suffix_length == 0 {
            return Err(());
        }
        let start = total_length.saturating_sub(suffix_length);
        return Ok(Some(ByteRange {
            start,
            end: total_length - 1,
        }));
    }

    let start = start.parse::<u64>().map_err(|_| ())?;
    if start >= total_length {
        return Err(());
    }
    let end = if end.is_empty() {
        total_length - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(total_length - 1)
    };
    if end < start {
        return Err(());
    }
    Ok(Some(ByteRange { start, end }))
}

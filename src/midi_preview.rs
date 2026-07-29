use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use midir::{MidiOutput, MidiOutputConnection};
use serde::Serialize;

use crate::midi_clip::PreparedClip;
use crate::recorder::Recorder;

const DEFAULT_OUTPUT: &str = "Keyestra MIDI";
const LEGACY_OUTPUT: &str = "FP10 Mapped";
const CAPTURE_RESUME_DELAY: Duration = Duration::from_millis(150);

trait MidiSink {
    fn send_midi(&mut self, message: &[u8]) -> Result<(), String>;
}

impl MidiSink for MidiOutputConnection {
    fn send_midi(&mut self, message: &[u8]) -> Result<(), String> {
        self.send(message).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewErrorCode {
    PreviewReplaced,
    OutputNotFound,
    OutputConnectFailed,
    PreviewFailed,
}

impl PreviewErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreviewReplaced => "preview_replaced",
            Self::OutputNotFound => "output_not_found",
            Self::OutputConnectFailed => "output_connect_failed",
            Self::PreviewFailed => "preview_failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreviewError {
    pub code: PreviewErrorCode,
    pub message: String,
}

impl PreviewError {
    fn new(code: PreviewErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PreviewError {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSnapshot {
    pub state: String,
    pub preview_id: Option<u64>,
    pub session_id: Option<String>,
    pub selection_start_us: Option<u64>,
    pub selection_end_us: Option<u64>,
    pub position_us: Option<u64>,
    pub output_name: String,
    pub capture_suspended: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
struct PreviewState {
    state: String,
    preview_id: Option<u64>,
    session_id: Option<String>,
    selection_start_us: Option<u64>,
    selection_end_us: Option<u64>,
    position_us: Option<u64>,
    output_name: String,
    error: Option<String>,
    started_at: Option<Instant>,
    started_position_us: Option<u64>,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            state: "stopped".into(),
            preview_id: None,
            session_id: None,
            selection_start_us: None,
            selection_end_us: None,
            position_us: None,
            output_name: DEFAULT_OUTPUT.into(),
            error: None,
            started_at: None,
            started_position_us: None,
        }
    }
}

enum Command {
    Play(PlayCommand),
    Stop(u64),
    Shutdown,
}

struct PlayCommand {
    preview_id: u64,
    session_id: String,
    clip: PreparedClip,
    position_us: u64,
    output_name: String,
    ready: Sender<Result<(), PreviewError>>,
}

pub struct MidiPreview {
    sender: Sender<Command>,
    state: Arc<Mutex<PreviewState>>,
    recorder: Arc<Recorder>,
    next_id: AtomicU64,
}

impl MidiPreview {
    pub fn new(recorder: Arc<Recorder>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let state = Arc::new(Mutex::new(PreviewState::default()));
        let worker_state = Arc::clone(&state);
        let worker_recorder = Arc::clone(&recorder);
        thread::Builder::new()
            .name("keyestra-midi-preview".into())
            .spawn(move || worker(receiver, worker_state, worker_recorder))
            .expect("failed to start MIDI preview worker");
        Self {
            sender,
            state,
            recorder,
            next_id: AtomicU64::new(1),
        }
    }

    pub fn outputs(&self) -> Vec<String> {
        output_names().unwrap_or_default()
    }

    pub fn play(
        &self,
        session_id: String,
        clip: PreparedClip,
        position_us: u64,
        output_name: Option<String>,
    ) -> Result<u64, PreviewError> {
        let preview_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let output_name = output_name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_OUTPUT.into());
        {
            let mut state = self.state.lock().map_err(|_| {
                PreviewError::new(
                    PreviewErrorCode::PreviewFailed,
                    "Preview state lock poisoned",
                )
            })?;
            state.state = "starting".into();
            state.preview_id = Some(preview_id);
            state.session_id = Some(session_id.clone());
            state.selection_start_us = Some(clip.selection_start_us);
            state.selection_end_us = Some(clip.selection_end_us);
            state.position_us = Some(position_us);
            state.output_name = output_name.clone();
            state.error = None;
            state.started_at = None;
            state.started_position_us = None;
        }
        let (ready, ready_receiver) = mpsc::channel();
        self.sender
            .send(Command::Play(PlayCommand {
                preview_id,
                session_id,
                clip,
                position_us,
                output_name,
                ready,
            }))
            .map_err(|_| {
                if let Ok(mut state) = self.state.lock() {
                    state.state = "error".into();
                    state.error = Some("Preview worker is unavailable".into());
                }
                PreviewError::new(
                    PreviewErrorCode::PreviewFailed,
                    "Preview worker is unavailable",
                )
            })?;
        match ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => result.map(|()| preview_id),
            Err(_) => {
                let _ = self.sender.send(Command::Stop(preview_id));
                Err(PreviewError::new(
                    PreviewErrorCode::PreviewFailed,
                    "Preview output did not become ready",
                ))
            }
        }
    }

    pub fn stop(&self, preview_id: u64) -> Result<(), PreviewError> {
        let current = self
            .state
            .lock()
            .map_err(|_| {
                PreviewError::new(
                    PreviewErrorCode::PreviewFailed,
                    "Preview state lock poisoned",
                )
            })?
            .preview_id;
        if current != Some(preview_id) {
            return Err(PreviewError::new(
                PreviewErrorCode::PreviewReplaced,
                "A newer preview has replaced this preview",
            ));
        }
        self.sender.send(Command::Stop(preview_id)).map_err(|_| {
            PreviewError::new(
                PreviewErrorCode::PreviewFailed,
                "Preview worker is unavailable",
            )
        })
    }

    pub fn snapshot(&self) -> PreviewSnapshot {
        let Ok(state) = self.state.lock() else {
            return PreviewSnapshot {
                state: "error".into(),
                preview_id: None,
                session_id: None,
                selection_start_us: None,
                selection_end_us: None,
                position_us: None,
                output_name: DEFAULT_OUTPUT.into(),
                capture_suspended: self.recorder.capture_suspended(),
                error: Some("Preview state lock poisoned".into()),
            };
        };
        let position_us = match (
            state.started_at,
            state.started_position_us,
            state.selection_end_us,
        ) {
            (Some(started), Some(position), Some(end)) if state.state == "playing" => Some(
                position
                    .saturating_add(started.elapsed().as_micros().min(u64::MAX as u128) as u64)
                    .min(end),
            ),
            _ => state.position_us,
        };
        PreviewSnapshot {
            state: state.state.clone(),
            preview_id: state.preview_id,
            session_id: state.session_id.clone(),
            selection_start_us: state.selection_start_us,
            selection_end_us: state.selection_end_us,
            position_us,
            output_name: state.output_name.clone(),
            capture_suspended: self.recorder.capture_suspended(),
            error: state.error.clone(),
        }
    }
}

impl Drop for MidiPreview {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Shutdown);
    }
}

fn worker(receiver: Receiver<Command>, state: Arc<Mutex<PreviewState>>, recorder: Arc<Recorder>) {
    let mut pending: Option<Command> = None;
    loop {
        let command = pending.take().or_else(|| receiver.recv().ok());
        match command {
            Some(Command::Play(play)) => {
                pending = run_play(play, &receiver, &state, &recorder);
            }
            Some(Command::Stop(_)) => {}
            Some(Command::Shutdown) | None => {
                recorder.set_capture_suspended(false);
                break;
            }
        }
    }
}

fn run_play(
    play: PlayCommand,
    receiver: &Receiver<Command>,
    state: &Arc<Mutex<PreviewState>>,
    recorder: &Arc<Recorder>,
) -> Option<Command> {
    recorder.set_capture_suspended(true);
    set_playing_state(state, &play);
    let connection = connect_output(&play.output_name);
    let mut connection = match connection {
        Ok(connection) => connection,
        Err(error) => {
            set_error_state(state, &play, &error.to_string());
            release_capture(recorder);
            let _ = play.ready.send(Err(error));
            return receiver.try_recv().ok();
        }
    };

    send_channel_cleanup(&mut connection, &play.clip, true);
    let _ = play.ready.send(Ok(()));
    let start_offset = play
        .position_us
        .saturating_sub(play.clip.selection_start_us);
    let started = Instant::now();
    for event in play
        .clip
        .events
        .iter()
        .filter(|event| event.offset_us >= start_offset)
    {
        let deadline = started + Duration::from_micros(event.offset_us - start_offset);
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(timeout) {
                Ok(Command::Stop(id)) if id == play.preview_id => {
                    send_channel_cleanup(&mut connection, &play.clip, true);
                    set_stopped_state(state, &play, play.position_us, None);
                    release_capture(recorder);
                    return receiver.try_recv().ok();
                }
                Ok(Command::Play(next)) => {
                    send_channel_cleanup(&mut connection, &play.clip, true);
                    set_stopped_state(state, &play, play.position_us, None);
                    release_capture(recorder);
                    return Some(Command::Play(next));
                }
                Ok(Command::Shutdown) => {
                    send_channel_cleanup(&mut connection, &play.clip, true);
                    recorder.set_capture_suspended(false);
                    return Some(Command::Shutdown);
                }
                Ok(Command::Stop(_)) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    send_channel_cleanup(&mut connection, &play.clip, true);
                    recorder.set_capture_suspended(false);
                    return Some(Command::Shutdown);
                }
            }
        }
        if let Err(error) = connection.send_midi(&event.message) {
            send_channel_cleanup(&mut connection, &play.clip, true);
            set_stopped_state(
                state,
                &play,
                play.position_us,
                Some(format!("MIDI preview send failed: {}", error)),
            );
            release_capture(recorder);
            return receiver.try_recv().ok();
        }
    }
    set_stopped_state(state, &play, play.clip.selection_end_us, None);
    release_capture(recorder);
    receiver.try_recv().ok()
}

fn release_capture(recorder: &Recorder) {
    thread::sleep(CAPTURE_RESUME_DELAY);
    recorder.set_capture_suspended(false);
}

fn set_playing_state(state: &Mutex<PreviewState>, play: &PlayCommand) {
    if let Ok(mut state) = state.lock() {
        state.state = "playing".into();
        state.preview_id = Some(play.preview_id);
        state.session_id = Some(play.session_id.clone());
        state.selection_start_us = Some(play.clip.selection_start_us);
        state.selection_end_us = Some(play.clip.selection_end_us);
        state.position_us = Some(play.position_us);
        state.output_name = play.output_name.clone();
        state.error = None;
        state.started_at = Some(Instant::now());
        state.started_position_us = Some(play.position_us);
    }
}

fn set_error_state(state: &Mutex<PreviewState>, play: &PlayCommand, error: &str) {
    set_stopped_state(state, play, play.position_us, Some(error.into()));
}

fn set_stopped_state(
    state: &Mutex<PreviewState>,
    play: &PlayCommand,
    position_us: u64,
    error: Option<String>,
) {
    if let Ok(mut state) = state.lock() {
        state.state = if error.is_some() { "error" } else { "stopped" }.into();
        state.preview_id = Some(play.preview_id);
        state.session_id = Some(play.session_id.clone());
        state.selection_start_us = Some(play.clip.selection_start_us);
        state.selection_end_us = Some(play.clip.selection_end_us);
        state.position_us = Some(position_us);
        state.output_name = play.output_name.clone();
        state.error = error;
        state.started_at = None;
        state.started_position_us = None;
    }
}

fn output_names() -> Result<Vec<String>, PreviewError> {
    let midi_output = MidiOutput::new("keyestra-preview-list")
        .map_err(|error| PreviewError::new(PreviewErrorCode::PreviewFailed, error.to_string()))?;
    Ok(midi_output
        .ports()
        .iter()
        .filter_map(|port| midi_output.port_name(port).ok())
        .collect())
}

fn connect_output(output_name: &str) -> Result<MidiOutputConnection, PreviewError> {
    let midi_output = MidiOutput::new("keyestra-preview-output").map_err(|error| {
        PreviewError::new(PreviewErrorCode::OutputConnectFailed, error.to_string())
    })?;
    let ports = midi_output.ports();
    let wanted = output_name.to_lowercase();
    let port = find_output_port(&midi_output, &ports, &wanted)
        .or_else(|| {
            output_name
                .eq_ignore_ascii_case(DEFAULT_OUTPUT)
                .then(|| find_output_port(&midi_output, &ports, &LEGACY_OUTPUT.to_lowercase()))
                .flatten()
        })
        .ok_or_else(|| {
            PreviewError::new(
                PreviewErrorCode::OutputNotFound,
                format!("MIDI output '{}' was not found", output_name),
            )
        })?;
    midi_output
        .connect(&port, "keyestra-preview")
        .map_err(|error| {
            PreviewError::new(PreviewErrorCode::OutputConnectFailed, error.to_string())
        })
}

fn find_output_port(
    midi_output: &MidiOutput,
    ports: &[midir::MidiOutputPort],
    wanted: &str,
) -> Option<midir::MidiOutputPort> {
    ports
        .iter()
        .find(|port| {
            midi_output
                .port_name(port)
                .is_ok_and(|name| name.to_lowercase() == wanted)
        })
        .or_else(|| {
            ports.iter().find(|port| {
                midi_output
                    .port_name(port)
                    .is_ok_and(|name| name.to_lowercase().contains(wanted))
            })
        })
        .cloned()
}

fn send_channel_cleanup<S: MidiSink>(connection: &mut S, clip: &PreparedClip, all_sound_off: bool) {
    for channel in &clip.channels {
        let _ = connection.send_midi(&[0xB0 | channel, 64, 0]);
        let _ = connection.send_midi(&[0xB0 | channel, 123, 0]);
        if all_sound_off {
            let _ = connection.send_midi(&[0xB0 | channel, 120, 0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi_clip::{prepare_clip, TimedMidiEvent};

    #[derive(Default)]
    struct FakeSink(Vec<Vec<u8>>);

    impl MidiSink for FakeSink {
        fn send_midi(&mut self, message: &[u8]) -> Result<(), String> {
            self.0.push(message.to_vec());
            Ok(())
        }
    }

    #[test]
    fn stale_preview_id_is_rejected_without_touching_worker() {
        let directory = std::env::temp_dir().join(format!(
            "keyestra-preview-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let recorder = Arc::new(Recorder::new(directory.clone()).unwrap());
        let preview = MidiPreview::new(recorder);
        let error = preview.stop(999).unwrap_err();
        assert_eq!(error.code, PreviewErrorCode::PreviewReplaced);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn emergency_cleanup_covers_every_involved_channel() {
        let clip = prepare_clip(
            &[
                TimedMidiEvent {
                    at_us: 1,
                    message: vec![0x90, 60, 90],
                },
                TimedMidiEvent {
                    at_us: 2,
                    message: vec![0x92, 64, 80],
                },
            ],
            0,
            10,
            true,
        )
        .unwrap();
        let mut sink = FakeSink::default();
        send_channel_cleanup(&mut sink, &clip, true);
        assert_eq!(
            sink.0,
            vec![
                vec![0xB0, 64, 0],
                vec![0xB0, 123, 0],
                vec![0xB0, 120, 0],
                vec![0xB2, 64, 0],
                vec![0xB2, 123, 0],
                vec![0xB2, 120, 0],
            ]
        );
    }
}

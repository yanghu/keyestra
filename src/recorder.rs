use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::midi_clip::{
    encode_smf, is_note_on, prepare_clip, ClipWarning, PreparedClip, TimedMidiEvent,
};

const DEFAULT_BUFFER_MINUTES: u64 = 60;
const DEFAULT_TIMELINE_SECONDS: u64 = 60;
pub const MIN_TIMELINE_BINS: usize = 50;
pub const MAX_TIMELINE_BINS: usize = 1_200;
pub const MAX_NOTE_MARKERS: usize = 5_000;
pub const MAX_NOTE_WINDOW_US: u64 = 5 * 60 * 1_000_000;

#[derive(Debug, Clone)]
struct TakeRange {
    started_us: u64,
    stopped_us: Option<u64>,
}

#[derive(Debug)]
struct RecorderState {
    events: VecDeque<TimedMidiEvent>,
    take: Option<TakeRange>,
    ignored_messages: u64,
    revision: u64,
    trimmed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRecording {
    pub name: String,
    pub size: u64,
    pub modified_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_us: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedRecordingResult {
    pub recording: SavedRecording,
    pub warnings: Vec<ClipWarning>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TimelineRequest {
    pub start_us: Option<u64>,
    pub end_us: Option<u64>,
    pub bins: Option<usize>,
    pub notes: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineBin {
    pub start_us: u64,
    pub end_us: u64,
    pub event_count: usize,
    pub note_on_count: usize,
    pub velocity_average: u8,
    pub velocity_maximum: u8,
    pub minimum_note: Option<u8>,
    pub maximum_note: Option<u8>,
    pub pedal_down_at_end: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteMarker {
    pub at_us: u64,
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineBuffer {
    pub available_start_us: u64,
    pub available_end_us: u64,
    pub limit_us: u64,
    pub event_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineRange {
    pub start_us: u64,
    pub end_us: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineTake {
    pub start_us: u64,
    pub end_us: u64,
    pub recording: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSnapshot {
    pub ok: bool,
    pub schema_version: u8,
    pub session_id: String,
    pub revision: u64,
    pub generated_at_us: u64,
    pub buffer: TimelineBuffer,
    pub range: TimelineRange,
    pub take: Option<TimelineTake>,
    pub bins: Vec<TimelineBin>,
    pub notes: Vec<NoteMarker>,
    pub notes_truncated: bool,
    pub state_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderErrorCode {
    InvalidParameter,
    InvalidRange,
    StaleSession,
    RangeExpired,
    RangeEmpty,
    TakeInProgress,
    StateLockFailed,
    InternalError,
}

impl RecorderErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParameter => "invalid_parameter",
            Self::InvalidRange => "invalid_range",
            Self::StaleSession => "stale_session",
            Self::RangeExpired => "range_expired",
            Self::RangeEmpty => "range_empty",
            Self::TakeInProgress => "take_in_progress",
            Self::StateLockFailed => "state_lock_failed",
            Self::InternalError => "internal_error",
        }
    }
}

#[derive(Debug)]
pub struct RecorderError {
    pub code: RecorderErrorCode,
    pub message: String,
    pub available_start_us: Option<u64>,
}

impl RecorderError {
    pub fn new(code: RecorderErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            available_start_us: None,
        }
    }

    fn expired(available_start_us: u64) -> Self {
        Self {
            code: RecorderErrorCode::RangeExpired,
            message: "Selected range is no longer in the rolling buffer".into(),
            available_start_us: Some(available_start_us),
        }
    }
}

impl std::fmt::Display for RecorderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RecorderError {}

pub type RecorderResult<T> = std::result::Result<T, RecorderError>;

pub struct Recorder {
    clock: Instant,
    epoch_started_ms: u128,
    session_id: String,
    buffer_duration: Duration,
    recordings_dir: PathBuf,
    capture_suspended: AtomicBool,
    state: Mutex<RecorderState>,
}

impl Recorder {
    pub fn new(recordings_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&recordings_dir).with_context(|| {
            format!(
                "Failed to create recording directory {}",
                recordings_dir.display()
            )
        })?;
        let epoch_started_ms = unix_time_ms();
        Ok(Self {
            clock: Instant::now(),
            epoch_started_ms,
            session_id: format!("{}-{}", epoch_started_ms, std::process::id()),
            buffer_duration: Duration::from_secs(DEFAULT_BUFFER_MINUTES * 60),
            recordings_dir,
            capture_suspended: AtomicBool::new(false),
            state: Mutex::new(RecorderState {
                events: VecDeque::new(),
                take: None,
                ignored_messages: 0,
                revision: 0,
                trimmed: false,
            }),
        })
    }

    pub fn default_directory() -> Result<PathBuf> {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let appdata = PathBuf::from(appdata);
            let directory = appdata.join("keyestra").join("recordings");
            migrate_legacy_recordings(&appdata.join("fp10-map").join("recordings"), &directory)?;
            return Ok(directory);
        }
        Ok(std::env::current_dir()
            .context("Failed to locate the current directory")?
            .join("recordings"))
    }

    pub fn ingest(&self, message: &[u8]) {
        if self.capture_suspended() {
            return;
        }
        let now_us = self.elapsed_us();
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if is_recordable_message(message) {
            state.events.push_back(TimedMidiEvent {
                at_us: now_us,
                message: message.to_vec(),
            });
            state.revision = state.revision.saturating_add(1);
            state.trimmed |= trim_old_events(&mut state.events, now_us, self.buffer_duration);
        } else {
            state.ignored_messages = state.ignored_messages.saturating_add(1);
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn capture_suspended(&self) -> bool {
        self.capture_suspended.load(Ordering::Acquire)
    }

    pub fn set_capture_suspended(&self, suspended: bool) {
        self.capture_suspended.store(suspended, Ordering::Release);
    }

    pub fn start_take(&self) -> Result<()> {
        let now_us = self.elapsed_us();
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("recorder state lock poisoned"))?;
        state.take = Some(TakeRange {
            started_us: now_us,
            stopped_us: None,
        });
        Ok(())
    }

    pub fn stop_take(&self) -> Result<()> {
        let now_us = self.elapsed_us();
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("recorder state lock poisoned"))?;
        let take = state
            .take
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No take has been started"))?;
        if take.stopped_us.is_none() {
            take.stopped_us = Some(now_us);
        }
        Ok(())
    }

    pub fn take_is_recording(&self) -> bool {
        self.state
            .lock()
            .map(|state| {
                state
                    .take
                    .as_ref()
                    .is_some_and(|take| take.stopped_us.is_none())
            })
            .unwrap_or(false)
    }

    pub fn save_take(&self) -> Result<SavedRecording> {
        let now_us = self.elapsed_us();
        let (start_us, end_us) = {
            let state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("recorder state lock poisoned"))?;
            let take = state
                .take
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No take has been started"))?;
            (take.started_us, take.stopped_us.unwrap_or(now_us))
        };
        self.save_range(&self.session_id, start_us, end_us)
            .map(|result| result.recording)
            .map_err(anyhow::Error::new)
    }

    pub fn save_recent(&self, seconds: u64) -> Result<SavedRecording> {
        let seconds = seconds.clamp(1, self.buffer_duration.as_secs());
        let end_us = self.elapsed_us();
        let start_us = end_us.saturating_sub(seconds.saturating_mul(1_000_000));
        self.save_range(&self.session_id, start_us, end_us)
            .map(|result| result.recording)
            .map_err(anyhow::Error::new)
    }

    pub fn timeline(&self, request: TimelineRequest) -> RecorderResult<TimelineSnapshot> {
        let now_us = self.elapsed_us();
        let bins_requested = request.bins.unwrap_or(600);
        if !(MIN_TIMELINE_BINS..=MAX_TIMELINE_BINS).contains(&bins_requested) {
            return Err(RecorderError::new(
                RecorderErrorCode::InvalidParameter,
                format!(
                    "bins must be between {} and {}",
                    MIN_TIMELINE_BINS, MAX_TIMELINE_BINS
                ),
            ));
        }
        let (events, take, revision, trimmed) = {
            let mut state = self.state.lock().map_err(|_| {
                RecorderError::new(
                    RecorderErrorCode::StateLockFailed,
                    "recorder state lock poisoned",
                )
            })?;
            state.trimmed |= trim_old_events(&mut state.events, now_us, self.buffer_duration);
            (
                state.events.iter().cloned().collect::<Vec<_>>(),
                state.take.clone(),
                state.revision,
                state.trimmed,
            )
        };
        let available_start_us = events.first().map(|event| event.at_us).unwrap_or(now_us);
        let available_end_us = now_us;
        let default_range = take
            .as_ref()
            .filter(|take| take.stopped_us.is_some())
            .map(|take| (take.started_us, take.stopped_us.unwrap()))
            .unwrap_or_else(|| {
                (
                    now_us.saturating_sub(DEFAULT_TIMELINE_SECONDS * 1_000_000),
                    now_us,
                )
            });
        let mut start_us = request.start_us.unwrap_or(default_range.0);
        let mut end_us = request.end_us.unwrap_or(default_range.1);
        start_us = start_us.max(available_start_us).min(available_end_us);
        end_us = end_us.max(available_start_us).min(available_end_us);
        if start_us >= end_us {
            start_us = available_start_us;
            end_us = available_end_us.max(start_us);
        }

        let bins = aggregate_bins(&events, start_us, end_us, bins_requested);
        let (notes, notes_truncated) = note_markers(&events, start_us, end_us, request.notes);
        Ok(TimelineSnapshot {
            ok: true,
            schema_version: 1,
            session_id: self.session_id.clone(),
            revision,
            generated_at_us: now_us,
            buffer: TimelineBuffer {
                available_start_us,
                available_end_us,
                limit_us: self.buffer_duration.as_micros() as u64,
                event_count: events.len(),
            },
            range: TimelineRange { start_us, end_us },
            take: take.map(|take| TimelineTake {
                start_us: take.started_us,
                end_us: take.stopped_us.unwrap_or(now_us),
                recording: take.stopped_us.is_none(),
            }),
            bins,
            notes,
            notes_truncated,
            state_complete: !trimmed,
        })
    }

    pub fn prepare_range(
        &self,
        session_id: &str,
        start_us: u64,
        end_us: u64,
    ) -> RecorderResult<PreparedClip> {
        if session_id != self.session_id {
            return Err(RecorderError::new(
                RecorderErrorCode::StaleSession,
                "Recorder session changed; refresh the selection",
            ));
        }
        if start_us >= end_us {
            return Err(RecorderError::new(
                RecorderErrorCode::InvalidRange,
                "startUs must be before endUs",
            ));
        }
        let now_us = self.elapsed_us();
        let (events, trimmed) = {
            let mut state = self.state.lock().map_err(|_| {
                RecorderError::new(
                    RecorderErrorCode::StateLockFailed,
                    "recorder state lock poisoned",
                )
            })?;
            state.trimmed |= trim_old_events(&mut state.events, now_us, self.buffer_duration);
            (
                state.events.iter().cloned().collect::<Vec<_>>(),
                state.trimmed,
            )
        };
        let available_start_us = events.first().map(|event| event.at_us).unwrap_or(now_us);
        if (trimmed && start_us < available_start_us) || end_us > now_us {
            return Err(RecorderError::expired(available_start_us));
        }
        let clip = prepare_clip(&events, start_us, end_us, !trimmed).map_err(|error| {
            RecorderError::new(RecorderErrorCode::InvalidRange, error.to_string())
        })?;
        if clip.original_event_count == 0 {
            return Err(RecorderError::new(
                RecorderErrorCode::RangeEmpty,
                "Selected range contains no MIDI events",
            ));
        }
        Ok(clip)
    }

    pub fn save_range(
        &self,
        session_id: &str,
        start_us: u64,
        end_us: u64,
    ) -> RecorderResult<SavedRecordingResult> {
        let clip = self.prepare_range(session_id, start_us, end_us)?;
        let bytes = encode_smf(&clip).map_err(|error| {
            RecorderError::new(RecorderErrorCode::InternalError, error.to_string())
        })?;
        let recording = self.save_bytes(bytes, start_us, end_us).map_err(|error| {
            RecorderError::new(RecorderErrorCode::InternalError, error.to_string())
        })?;
        Ok(SavedRecordingResult {
            recording,
            warnings: clip.warnings,
        })
    }

    pub fn status_json(&self) -> String {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Status {
            recording: bool,
            take_available: bool,
            event_count: usize,
            buffered_ms: u64,
            buffer_limit_minutes: u64,
            take_duration_ms: u64,
            ignored_messages: u64,
            take_started_at_ms: Option<u128>,
            last_event_ago_ms: Option<u64>,
            capture_suspended: bool,
            session_id: String,
            recordings: Vec<SavedRecording>,
        }

        let now_us = self.elapsed_us();
        let recordings = self.list_recordings().unwrap_or_default();
        let Ok(state) = self.state.lock() else {
            return r#"{"error":"recorder state lock poisoned"}"#.to_string();
        };
        let first_us = state.events.front().map(|event| event.at_us);
        let last_us = state.events.back().map(|event| event.at_us);
        let (recording, take_available, take_started_at_ms, take_duration_ms) =
            match state.take.as_ref() {
                Some(take) => {
                    let end = take.stopped_us.unwrap_or(now_us);
                    (
                        take.stopped_us.is_none(),
                        true,
                        Some(
                            self.epoch_started_ms
                                .saturating_add((take.started_us / 1_000) as u128),
                        ),
                        end.saturating_sub(take.started_us) / 1_000,
                    )
                }
                None => (false, false, None, 0),
            };
        serde_json::to_string(&Status {
            recording,
            take_available,
            event_count: state.events.len(),
            buffered_ms: first_us
                .map(|first| now_us.saturating_sub(first) / 1_000)
                .unwrap_or(0),
            buffer_limit_minutes: self.buffer_duration.as_secs() / 60,
            take_duration_ms,
            ignored_messages: state.ignored_messages,
            take_started_at_ms,
            last_event_ago_ms: last_us.map(|last| now_us.saturating_sub(last) / 1_000),
            capture_suspended: self.capture_suspended(),
            session_id: self.session_id.clone(),
            recordings,
        })
        .unwrap_or_else(|error| format!(r#"{{"error":"{}"}}"#, error))
    }

    pub fn read_recording(&self, name: &str) -> Result<Vec<u8>> {
        let path = safe_recording_path(&self.recordings_dir, name)?;
        fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))
    }

    fn save_bytes(&self, bytes: Vec<u8>, start_us: u64, end_us: u64) -> Result<SavedRecording> {
        let name = self.unique_recording_name();
        let final_path = self.recordings_dir.join(&name);
        let temporary_path = self.recordings_dir.join(format!(".{}.tmp", name));
        fs::write(&temporary_path, bytes)
            .with_context(|| format!("Failed to write {}", temporary_path.display()))?;
        if let Err(error) = fs::rename(&temporary_path, &final_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(error)
                .with_context(|| format!("Failed to finalize recording {}", final_path.display()));
        }
        let mut saved = recording_metadata(final_path)?;
        saved.start_us = Some(start_us);
        saved.end_us = Some(end_us);
        saved.duration_us = Some(end_us - start_us);
        Ok(saved)
    }

    fn list_recordings(&self) -> Result<Vec<SavedRecording>> {
        let mut recordings = fs::read_dir(&self.recordings_dir)
            .with_context(|| format!("Failed to list {}", self.recordings_dir.display()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| recording_metadata(entry.path()).ok())
            .collect::<Vec<_>>();
        recordings.sort_by(|left, right| right.modified_ms.cmp(&left.modified_ms));
        recordings.truncate(20);
        Ok(recordings)
    }

    fn unique_recording_name(&self) -> String {
        let timestamp = unix_time_ms();
        for suffix in 0..1_000_u16 {
            let name = if suffix == 0 {
                format!("keyestra-{}.mid", timestamp)
            } else {
                format!("keyestra-{}-{}.mid", timestamp, suffix)
            };
            if !self.recordings_dir.join(&name).exists() {
                return name;
            }
        }
        format!("keyestra-{}-overflow.mid", timestamp)
    }

    fn elapsed_us(&self) -> u64 {
        self.clock.elapsed().as_micros().min(u64::MAX as u128) as u64
    }
}

fn migrate_legacy_recordings(legacy: &Path, destination: &Path) -> Result<()> {
    const MARKER: &str = ".fp10-recordings-migrated";

    fs::create_dir_all(destination)
        .with_context(|| format!("Failed to create {}", destination.display()))?;
    let marker = destination.join(MARKER);
    if marker.exists() || !legacy.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(legacy)
        .with_context(|| format!("Failed to list legacy recordings {}", legacy.display()))?
    {
        let entry = entry?;
        let source = entry.path();
        let Some(name) = source.file_name() else {
            continue;
        };
        if !source.is_file()
            || source
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("mid"))
        {
            continue;
        }
        let target = destination.join(name);
        if !target.exists() {
            fs::copy(&source, &target).with_context(|| {
                format!(
                    "Failed to migrate recording {} to {}",
                    source.display(),
                    target.display()
                )
            })?;
        }
    }

    fs::write(
        &marker,
        b"Legacy FP10 recordings copied; source files were retained.\n",
    )
    .with_context(|| format!("Failed to write migration marker {}", marker.display()))
}

fn trim_old_events(events: &mut VecDeque<TimedMidiEvent>, now_us: u64, duration: Duration) -> bool {
    let cutoff = now_us.saturating_sub(duration.as_micros().min(u64::MAX as u128) as u64);
    let mut trimmed = false;
    while events.front().is_some_and(|event| event.at_us < cutoff) {
        events.pop_front();
        trimmed = true;
    }
    trimmed
}

fn aggregate_bins(
    events: &[TimedMidiEvent],
    start_us: u64,
    end_us: u64,
    requested: usize,
) -> Vec<TimelineBin> {
    if start_us >= end_us {
        return Vec::new();
    }
    let duration = end_us - start_us;
    let bin_count = requested.min(duration.max(1) as usize);
    let mut bins = (0..bin_count)
        .map(|index| {
            let bin_start =
                start_us + (duration as u128 * index as u128 / bin_count as u128) as u64;
            let bin_end =
                start_us + (duration as u128 * (index + 1) as u128 / bin_count as u128) as u64;
            TimelineBin {
                start_us: bin_start,
                end_us: bin_end,
                event_count: 0,
                note_on_count: 0,
                velocity_average: 0,
                velocity_maximum: 0,
                minimum_note: None,
                maximum_note: None,
                pedal_down_at_end: false,
            }
        })
        .collect::<Vec<_>>();
    let mut velocity_sums = vec![0_u64; bin_count];
    let mut pedal_down = false;
    for event in events.iter().filter(|event| event.at_us < end_us) {
        if event.message.len() >= 3 && event.message[0] & 0xF0 == 0xB0 && event.message[1] == 64 {
            pedal_down = event.message[2] >= 64;
        }
        if event.at_us < start_us {
            continue;
        }
        let relative = event.at_us - start_us;
        let index = ((relative as u128 * bin_count as u128) / duration as u128)
            .min((bin_count - 1) as u128) as usize;
        let bin = &mut bins[index];
        bin.event_count += 1;
        if is_note_on(&event.message) {
            let note = event.message[1];
            let velocity = event.message[2];
            bin.note_on_count += 1;
            velocity_sums[index] += velocity as u64;
            bin.velocity_maximum = bin.velocity_maximum.max(velocity);
            bin.minimum_note = Some(bin.minimum_note.map_or(note, |value| value.min(note)));
            bin.maximum_note = Some(bin.maximum_note.map_or(note, |value| value.max(note)));
        }
        bin.pedal_down_at_end = pedal_down;
    }
    let mut last_pedal = events
        .iter()
        .filter(|event| event.at_us < start_us)
        .filter(|event| {
            event.message.len() >= 3 && event.message[0] & 0xF0 == 0xB0 && event.message[1] == 64
        })
        .next_back()
        .is_some_and(|event| event.message[2] >= 64);
    for (index, bin) in bins.iter_mut().enumerate() {
        if bin.note_on_count > 0 {
            bin.velocity_average = (velocity_sums[index] / bin.note_on_count as u64).min(127) as u8;
        }
        if bin.event_count == 0 {
            bin.pedal_down_at_end = last_pedal;
        } else {
            last_pedal = bin.pedal_down_at_end;
        }
    }
    bins
}

fn note_markers(
    events: &[TimedMidiEvent],
    start_us: u64,
    end_us: u64,
    requested: bool,
) -> (Vec<NoteMarker>, bool) {
    if !requested || end_us.saturating_sub(start_us) > MAX_NOTE_WINDOW_US {
        return (Vec::new(), false);
    }
    let mut notes = Vec::new();
    let mut truncated = false;
    for event in events
        .iter()
        .filter(|event| event.at_us >= start_us && event.at_us < end_us)
        .filter(|event| is_note_on(&event.message))
    {
        if notes.len() == MAX_NOTE_MARKERS {
            truncated = true;
            break;
        }
        notes.push(NoteMarker {
            at_us: event.at_us,
            channel: event.message[0] & 0x0F,
            note: event.message[1],
            velocity: event.message[2],
        });
    }
    (notes, truncated)
}

fn is_recordable_message(message: &[u8]) -> bool {
    let Some(status) = message.first().copied() else {
        return false;
    };
    matches!(status, 0x80..=0xEF | 0xF0 | 0xF7)
}

fn safe_recording_path(directory: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || !name.ends_with(".mid")
        || Path::new(name).file_name().and_then(|value| value.to_str()) != Some(name)
    {
        anyhow::bail!("Invalid recording name");
    }
    Ok(directory.join(name))
}

fn recording_metadata(path: PathBuf) -> Result<SavedRecording> {
    if path.extension().and_then(|value| value.to_str()) != Some("mid") {
        anyhow::bail!("Not a MIDI recording");
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("Recording name is not valid UTF-8"))?
        .to_string();
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        anyhow::bail!("Recording is not a file");
    }
    let modified_ms = metadata
        .modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Ok(SavedRecording {
        name,
        size: metadata.len(),
        modified_ms,
        start_us: None,
        end_us: None,
        duration_us: None,
    })
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(at_us: u64, message: &[u8]) -> TimedMidiEvent {
        TimedMidiEvent {
            at_us,
            message: message.to_vec(),
        }
    }

    #[test]
    fn boundary_event_belongs_to_one_bin() {
        let bins = aggregate_bins(&[timed(50, &[0x90, 60, 90])], 0, 100, 50);
        assert_eq!(bins.iter().map(|bin| bin.event_count).sum::<usize>(), 1);
        assert_eq!(bins[25].event_count, 1);
    }

    #[test]
    fn bin_totals_velocity_pitch_and_pedal_are_aggregated() {
        let bins = aggregate_bins(
            &[
                timed(5, &[0xB0, 64, 127]),
                timed(11, &[0x90, 40, 60]),
                timed(12, &[0x90, 80, 100]),
            ],
            10,
            20,
            50,
        );
        assert_eq!(bins.iter().map(|bin| bin.event_count).sum::<usize>(), 2);
        let populated = bins.iter().find(|bin| bin.note_on_count > 0).unwrap();
        assert!(populated.pedal_down_at_end);
        assert_eq!(populated.minimum_note, Some(40));
        assert_eq!(populated.maximum_note, Some(40));
    }

    #[test]
    fn marker_limit_is_reported() {
        let events = (0..=MAX_NOTE_MARKERS)
            .map(|index| timed(index as u64, &[0x90, 60, 1]))
            .collect::<Vec<_>>();
        let (notes, truncated) = note_markers(&events, 0, 10_000, true);
        assert_eq!(notes.len(), MAX_NOTE_MARKERS);
        assert!(truncated);
    }

    #[test]
    fn recording_paths_cannot_escape_directory() {
        let directory = Path::new("recordings");
        assert!(safe_recording_path(directory, "keyestra-123.mid").is_ok());
        assert!(safe_recording_path(directory, "../secret.mid").is_err());
    }

    #[test]
    fn legacy_recordings_are_copied_once_and_the_source_is_retained() {
        let root = std::env::temp_dir().join(format!(
            "keyestra-recorder-migration-test-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let legacy = root.join("fp10-map").join("recordings");
        let destination = root.join("keyestra").join("recordings");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("fp10-old.mid"), b"midi").unwrap();
        fs::write(legacy.join("ignore.txt"), b"text").unwrap();

        migrate_legacy_recordings(&legacy, &destination).unwrap();
        migrate_legacy_recordings(&legacy, &destination).unwrap();

        assert_eq!(fs::read(destination.join("fp10-old.mid")).unwrap(), b"midi");
        assert!(destination.join(".fp10-recordings-migrated").exists());
        assert!(!destination.join("ignore.txt").exists());
        assert!(legacy.join("fp10-old.mid").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn realtime_messages_are_not_recorded() {
        assert!(is_recordable_message(&[0x90, 60, 100]));
        assert!(is_recordable_message(&[0xF0, 1, 2, 0xF7]));
        assert!(!is_recordable_message(&[0xFE]));
        assert!(!is_recordable_message(&[]));
    }

    #[test]
    fn stale_session_and_empty_ranges_have_typed_errors() {
        let directory = std::env::temp_dir().join(format!(
            "keyestra-recorder-range-test-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let recorder = Recorder::new(directory.clone()).unwrap();
        let end_us = recorder.elapsed_us().max(1);
        let stale = recorder
            .prepare_range("old-session", 0, end_us)
            .unwrap_err();
        assert_eq!(stale.code, RecorderErrorCode::StaleSession);
        let empty = recorder
            .prepare_range(recorder.session_id(), 0, end_us)
            .unwrap_err();
        assert_eq!(empty.code, RecorderErrorCode::RangeEmpty);
        fs::remove_dir_all(directory).unwrap();
    }
}

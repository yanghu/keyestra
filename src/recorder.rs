use std::collections::{BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

const DEFAULT_BUFFER_MINUTES: u64 = 60;
const MIDI_TICKS_PER_QUARTER: u16 = 1_000;
const MIDI_TEMPO_US_PER_QUARTER: u64 = 500_000;
const MIDI_US_PER_TICK: u64 = MIDI_TEMPO_US_PER_QUARTER / MIDI_TICKS_PER_QUARTER as u64;

#[derive(Debug, Clone)]
struct TimedMidiEvent {
    at_us: u64,
    message: Vec<u8>,
}

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
}

#[derive(Debug, Clone)]
pub struct SavedRecording {
    pub name: String,
    pub size: u64,
    pub modified_ms: u128,
}

pub struct Recorder {
    clock: Instant,
    epoch_started_ms: u128,
    buffer_duration: Duration,
    recordings_dir: PathBuf,
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
        Ok(Self {
            clock: Instant::now(),
            epoch_started_ms: unix_time_ms(),
            buffer_duration: Duration::from_secs(DEFAULT_BUFFER_MINUTES * 60),
            recordings_dir,
            state: Mutex::new(RecorderState {
                events: VecDeque::new(),
                take: None,
                ignored_messages: 0,
            }),
        })
    }

    pub fn default_directory() -> Result<PathBuf> {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return Ok(PathBuf::from(appdata).join("fp10-map").join("recordings"));
        }
        Ok(std::env::current_dir()
            .context("Failed to locate the current directory")?
            .join("recordings"))
    }

    pub fn ingest(&self, message: &[u8]) {
        let now_us = self.elapsed_us();
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if is_recordable_message(message) {
            state.events.push_back(TimedMidiEvent {
                at_us: now_us,
                message: message.to_vec(),
            });
            trim_old_events(&mut state.events, now_us, self.buffer_duration);
        } else {
            state.ignored_messages = state.ignored_messages.saturating_add(1);
        }
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

    pub fn save_take(&self) -> Result<SavedRecording> {
        let now_us = self.elapsed_us();
        let (start_us, end_us, events) = {
            let state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("recorder state lock poisoned"))?;
            let take = state
                .take
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("No take has been started"))?;
            let end_us = take.stopped_us.unwrap_or(now_us);
            (
                take.started_us,
                end_us,
                events_in_range(&state.events, take.started_us, end_us),
            )
        };
        self.save_events(&events, start_us, end_us)
    }

    pub fn save_recent(&self, seconds: u64) -> Result<SavedRecording> {
        let seconds = seconds.clamp(1, self.buffer_duration.as_secs());
        let end_us = self.elapsed_us();
        let start_us = end_us.saturating_sub(seconds.saturating_mul(1_000_000));
        let events = {
            let state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("recorder state lock poisoned"))?;
            events_in_range(&state.events, start_us, end_us)
        };
        self.save_events(&events, start_us, end_us)
    }

    pub fn status_json(&self) -> String {
        let now_us = self.elapsed_us();
        let recordings = self.list_recordings().unwrap_or_default();
        let Ok(state) = self.state.lock() else {
            return "{\"error\":\"recorder state lock poisoned\"}".to_string();
        };
        let first_us = state.events.front().map(|event| event.at_us);
        let last_us = state.events.back().map(|event| event.at_us);
        let buffered_ms = first_us
            .map(|first| now_us.saturating_sub(first) / 1_000)
            .unwrap_or(0);
        let last_event_ago_ms = last_us.map(|last| now_us.saturating_sub(last) / 1_000);
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

        let mut out = String::with_capacity(2048);
        write!(
            out,
            "{{\"recording\":{},\"takeAvailable\":{},\"eventCount\":{},\"bufferedMs\":{},\"bufferLimitMinutes\":{},\"takeDurationMs\":{},\"ignoredMessages\":{},",
            recording,
            take_available,
            state.events.len(),
            buffered_ms,
            self.buffer_duration.as_secs() / 60,
            take_duration_ms,
            state.ignored_messages
        )
        .unwrap();
        out.push_str("\"takeStartedAtMs\":");
        write_optional_u128(&mut out, take_started_at_ms);
        out.push_str(",\"lastEventAgoMs\":");
        write_optional_u64(&mut out, last_event_ago_ms);
        out.push_str(",\"recordings\":[");
        for (index, recording) in recordings.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            write!(
                out,
                "{{\"name\":\"{}\",\"size\":{},\"modifiedMs\":{}}}",
                escape_json(&recording.name),
                recording.size,
                recording.modified_ms
            )
            .unwrap();
        }
        out.push_str("]}");
        out
    }

    pub fn read_recording(&self, name: &str) -> Result<Vec<u8>> {
        let path = safe_recording_path(&self.recordings_dir, name)?;
        fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))
    }

    fn save_events(
        &self,
        events: &[TimedMidiEvent],
        start_us: u64,
        end_us: u64,
    ) -> Result<SavedRecording> {
        if events.is_empty() {
            anyhow::bail!("No MIDI events in the selected time range");
        }
        let bytes = encode_smf(events, start_us, end_us)?;
        let name = self.unique_recording_name();
        let final_path = self.recordings_dir.join(&name);
        let temporary_path = self.recordings_dir.join(format!(".{}.tmp", name));
        fs::write(&temporary_path, bytes)
            .with_context(|| format!("Failed to write {}", temporary_path.display()))?;
        fs::rename(&temporary_path, &final_path).with_context(|| {
            format!(
                "Failed to finalize recording {}",
                final_path.as_path().display()
            )
        })?;
        recording_metadata(final_path)
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
                format!("fp10-{}.mid", timestamp)
            } else {
                format!("fp10-{}-{}.mid", timestamp, suffix)
            };
            if !self.recordings_dir.join(&name).exists() {
                return name;
            }
        }
        format!("fp10-{}-overflow.mid", timestamp)
    }

    fn elapsed_us(&self) -> u64 {
        self.clock.elapsed().as_micros().min(u64::MAX as u128) as u64
    }
}

fn trim_old_events(events: &mut VecDeque<TimedMidiEvent>, now_us: u64, duration: Duration) {
    let cutoff = now_us.saturating_sub(duration.as_micros().min(u64::MAX as u128) as u64);
    while events.front().is_some_and(|event| event.at_us < cutoff) {
        events.pop_front();
    }
}

fn events_in_range(
    events: &VecDeque<TimedMidiEvent>,
    start_us: u64,
    end_us: u64,
) -> Vec<TimedMidiEvent> {
    events
        .iter()
        .filter(|event| event.at_us >= start_us && event.at_us <= end_us)
        .cloned()
        .collect()
}

fn is_recordable_message(message: &[u8]) -> bool {
    let Some(status) = message.first().copied() else {
        return false;
    };
    matches!(status, 0x80..=0xEF | 0xF0 | 0xF7)
}

fn encode_smf(events: &[TimedMidiEvent], start_us: u64, end_us: u64) -> Result<Vec<u8>> {
    let mut track = Vec::with_capacity(events.len().saturating_mul(5).saturating_add(128));
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
    let track_name = b"FP10 Mapped";
    track.extend_from_slice(&[0x00, 0xFF, 0x03, track_name.len() as u8]);
    track.extend_from_slice(track_name);

    let mut previous_tick = 0_u64;
    let mut channels = BTreeSet::new();
    let mut written_events = 0_usize;
    for event in events {
        if event.at_us < start_us || event.at_us > end_us {
            continue;
        }
        let tick = us_to_tick(event.at_us.saturating_sub(start_us));
        let delta = tick.saturating_sub(previous_tick);
        let Some(encoded) = encode_midi_message(&event.message, &mut channels) else {
            continue;
        };
        write_variable_length(&mut track, delta)?;
        track.extend_from_slice(&encoded);
        previous_tick = tick;
        written_events += 1;
    }
    if written_events == 0 {
        anyhow::bail!("The selected range has no exportable MIDI events");
    }

    let end_tick = us_to_tick(end_us.saturating_sub(start_us)).max(previous_tick);
    let mut first_cleanup = true;
    for channel in channels {
        write_variable_length(
            &mut track,
            if first_cleanup {
                end_tick.saturating_sub(previous_tick)
            } else {
                0
            },
        )?;
        track.extend_from_slice(&[0xB0 | channel, 64, 0]);
        write_variable_length(&mut track, 0)?;
        track.extend_from_slice(&[0xB0 | channel, 123, 0]);
        first_cleanup = false;
        previous_tick = end_tick;
    }
    write_variable_length(&mut track, end_tick.saturating_sub(previous_tick))?;
    track.extend_from_slice(&[0xFF, 0x2F, 0x00]);

    let mut file = Vec::with_capacity(track.len().saturating_add(22));
    file.extend_from_slice(b"MThd");
    file.extend_from_slice(&6_u32.to_be_bytes());
    file.extend_from_slice(&0_u16.to_be_bytes());
    file.extend_from_slice(&1_u16.to_be_bytes());
    file.extend_from_slice(&MIDI_TICKS_PER_QUARTER.to_be_bytes());
    file.extend_from_slice(b"MTrk");
    file.extend_from_slice(&(track.len() as u32).to_be_bytes());
    file.extend_from_slice(&track);
    Ok(file)
}

fn encode_midi_message(message: &[u8], channels: &mut BTreeSet<u8>) -> Option<Vec<u8>> {
    let status = *message.first()?;
    match status {
        0x80..=0xEF => {
            let expected_length = match status & 0xF0 {
                0xC0 | 0xD0 => 2,
                _ => 3,
            };
            if message.len() < expected_length {
                return None;
            }
            channels.insert(status & 0x0F);
            Some(message[..expected_length].to_vec())
        }
        0xF0 | 0xF7 => {
            let payload = &message[1..];
            let mut encoded = vec![status];
            write_variable_length(&mut encoded, payload.len() as u64).ok()?;
            encoded.extend_from_slice(payload);
            Some(encoded)
        }
        _ => None,
    }
}

fn us_to_tick(us: u64) -> u64 {
    us.saturating_add(MIDI_US_PER_TICK / 2) / MIDI_US_PER_TICK
}

fn write_variable_length(out: &mut Vec<u8>, value: u64) -> Result<()> {
    if value > 0x0FFF_FFFF {
        anyhow::bail!("MIDI delta time is too large");
    }
    let mut value = value as u32;
    let mut buffer = [0_u8; 4];
    let mut index = 3;
    buffer[index] = (value & 0x7F) as u8;
    while {
        value >>= 7;
        value > 0
    } {
        index -= 1;
        buffer[index] = ((value & 0x7F) as u8) | 0x80;
    }
    out.extend_from_slice(&buffer[index..]);
    Ok(())
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
    })
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn write_optional_u64(out: &mut String, value: Option<u64>) {
    match value {
        Some(value) => write!(out, "{}", value).unwrap(),
        None => out.push_str("null"),
    }
}

fn write_optional_u128(out: &mut String, value: Option<u128>) {
    match value {
        Some(value) => write!(out, "{}", value).unwrap(),
        None => out.push_str("null"),
    }
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_length_encoding_matches_midi_examples() {
        let cases = [
            (0, vec![0x00]),
            (127, vec![0x7F]),
            (128, vec![0x81, 0x00]),
            (16_383, vec![0xFF, 0x7F]),
            (16_384, vec![0x81, 0x80, 0x00]),
            (0x0FFF_FFFF, vec![0xFF, 0xFF, 0xFF, 0x7F]),
        ];
        for (value, expected) in cases {
            let mut actual = Vec::new();
            write_variable_length(&mut actual, value).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn smf_preserves_channel_events_and_adds_cleanup() {
        let events = vec![
            TimedMidiEvent {
                at_us: 1_000,
                message: vec![0x90, 60, 100],
            },
            TimedMidiEvent {
                at_us: 251_000,
                message: vec![0xB0, 64, 127],
            },
            TimedMidiEvent {
                at_us: 501_000,
                message: vec![0x80, 60, 0],
            },
        ];
        let file = encode_smf(&events, 1_000, 601_000).unwrap();
        assert_eq!(&file[..4], b"MThd");
        assert_eq!(&file[8..14], &[0, 0, 0, 1, 0x03, 0xE8]);
        assert!(file.windows(3).any(|bytes| bytes == [0x90, 60, 100]));
        assert!(file.windows(3).any(|bytes| bytes == [0xB0, 64, 127]));
        assert!(file.windows(3).any(|bytes| bytes == [0x80, 60, 0]));
        assert!(file.windows(3).any(|bytes| bytes == [0xB0, 64, 0]));
        assert!(file.windows(3).any(|bytes| bytes == [0xB0, 123, 0]));
        assert!(file.ends_with(&[0xFF, 0x2F, 0x00]));
    }

    #[test]
    fn realtime_messages_are_not_recorded() {
        assert!(is_recordable_message(&[0x90, 60, 100]));
        assert!(is_recordable_message(&[0xF0, 1, 2, 0xF7]));
        assert!(!is_recordable_message(&[0xFE]));
        assert!(!is_recordable_message(&[0xF8]));
        assert!(!is_recordable_message(&[]));
    }

    #[test]
    fn recording_paths_cannot_escape_directory() {
        let directory = Path::new("recordings");
        assert!(safe_recording_path(directory, "fp10-123.mid").is_ok());
        assert!(safe_recording_path(directory, "../secret.mid").is_err());
        assert!(safe_recording_path(directory, "not-midi.txt").is_err());
    }

    #[test]
    fn recorder_saves_and_reads_a_take() {
        let directory = std::env::temp_dir().join(format!(
            "fp10-recorder-test-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let recorder = Recorder::new(directory.clone()).unwrap();
        recorder.start_take().unwrap();
        recorder.ingest(&[0x90, 60, 100]);
        recorder.ingest(&[0xB0, 64, 127]);
        recorder.ingest(&[0x80, 60, 0]);
        recorder.stop_take().unwrap();
        let saved = recorder.save_take().unwrap();
        let bytes = recorder.read_recording(&saved.name).unwrap();
        assert_eq!(&bytes[..4], b"MThd");
        fs::remove_dir_all(directory).unwrap();
    }
}

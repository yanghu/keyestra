use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamError};

#[derive(Debug, Clone)]
pub struct MetronomeSettings {
    pub running: bool,
    pub bpm: u16,
    pub beats_per_bar: u8,
    pub subdivision: u8,
    pub volume: f32,
    pub output_device: Option<String>,
}

impl Default for MetronomeSettings {
    fn default() -> Self {
        Self {
            running: false,
            bpm: 80,
            beats_per_bar: 4,
            subdivision: 1,
            volume: 0.35,
            output_device: None,
        }
    }
}

#[derive(Debug)]
struct MetronomeState {
    settings: MetronomeSettings,
    current_beat: u8,
    current_subdivision: u8,
    devices: Vec<String>,
    active_device: Option<String>,
    error: Option<String>,
    started_at_ms: Option<u128>,
    started_at: Option<Instant>,
    stream_generation: u64,
    timing_generation: u64,
}

#[derive(Debug)]
pub struct MetronomePatch {
    pub running: Option<bool>,
    pub bpm: Option<u16>,
    pub beats_per_bar: Option<u8>,
    pub subdivision: Option<u8>,
    pub volume: Option<f32>,
    pub output_device: Option<Option<String>>,
}

pub struct Metronome {
    state: Arc<Mutex<MetronomeState>>,
}

#[derive(Debug, Clone, Copy)]
pub struct GridTiming {
    pub generation: u64,
    pub grid_index: u64,
    pub nearest_grid_index: u64,
    pub error_ms: f64,
    pub phase_ms: f64,
    pub interval_ms: f64,
    pub bpm: u16,
    pub beats_per_bar: u8,
    pub subdivision: u8,
}

#[derive(Debug, Default)]
struct PlaybackState {
    was_running: bool,
    samples_until_tick: f64,
    tick_remaining: usize,
    phase: f32,
    frequency: f32,
    next_beat: u8,
    next_subdivision: u8,
}

impl Metronome {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(MetronomeState {
            settings: MetronomeSettings::default(),
            current_beat: 0,
            current_subdivision: 0,
            devices: list_output_devices(),
            active_device: None,
            error: None,
            started_at_ms: None,
            started_at: None,
            stream_generation: 1,
            timing_generation: 0,
        }));
        start_audio_thread(Arc::clone(&state));
        Self { state }
    }

    pub fn apply_patch(&self, patch: MetronomePatch) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(running) = patch.running {
                if running && !state.settings.running {
                    state.started_at_ms = None;
                    state.started_at = None;
                    state.current_beat = 0;
                    state.current_subdivision = 0;
                    state.stream_generation = state.stream_generation.wrapping_add(1);
                } else if !running {
                    state.started_at_ms = None;
                    state.started_at = None;
                    state.current_beat = 0;
                    state.current_subdivision = 0;
                    state.stream_generation = state.stream_generation.wrapping_add(1);
                }
                state.settings.running = running;
            }
            if let Some(bpm) = patch.bpm {
                state.settings.bpm = bpm.clamp(30, 240);
                if state.settings.running {
                    state.started_at_ms = None;
                    state.started_at = None;
                    state.current_beat = 0;
                    state.current_subdivision = 0;
                    state.stream_generation = state.stream_generation.wrapping_add(1);
                }
            }
            if let Some(beats_per_bar) = patch.beats_per_bar {
                state.settings.beats_per_bar = beats_per_bar.clamp(1, 12);
                if state.settings.running {
                    state.started_at_ms = None;
                    state.started_at = None;
                    state.current_beat = 0;
                    state.current_subdivision = 0;
                    state.stream_generation = state.stream_generation.wrapping_add(1);
                }
            }
            if let Some(subdivision) = patch.subdivision {
                state.settings.subdivision = subdivision.clamp(1, 4);
                if state.settings.running {
                    state.started_at_ms = None;
                    state.started_at = None;
                    state.current_beat = 0;
                    state.current_subdivision = 0;
                    state.stream_generation = state.stream_generation.wrapping_add(1);
                }
            }
            if let Some(volume) = patch.volume {
                state.settings.volume = volume.clamp(0.0, 1.0);
            }
            if let Some(output_device) = patch.output_device {
                state.settings.output_device = output_device;
                state.stream_generation = state.stream_generation.wrapping_add(1);
            }
        }
    }

    pub fn to_json(&self) -> String {
        self.state
            .lock()
            .map(|state| state_json(&state, true, true))
            .unwrap_or_else(|_| "{\"error\":\"metronome state lock poisoned\"}".to_string())
    }

    pub fn event_key(&self) -> String {
        self.state
            .lock()
            .map(|state| state_json(&state, false, false))
            .unwrap_or_else(|_| "metronome state lock poisoned".to_string())
    }

    pub fn event_json(&self) -> String {
        self.state
            .lock()
            .map(|state| state_json(&state, true, false))
            .unwrap_or_else(|_| "{\"error\":\"metronome state lock poisoned\"}".to_string())
    }

    pub fn grid_timing(&self, at: Instant) -> Option<GridTiming> {
        let state = self.state.lock().ok()?;
        if !state.settings.running {
            return None;
        }
        let started_at = state.started_at?;
        let elapsed = at.checked_duration_since(started_at)?.as_secs_f64();
        let subdivision = state.settings.subdivision.max(1);
        let interval_ms = 60_000.0 / state.settings.bpm.max(1) as f64 / subdivision as f64;
        let position = elapsed * 1_000.0 / interval_ms;
        let grid_index = position.floor().max(0.0) as u64;
        let nearest_grid_index = position.round().max(0.0) as u64;
        Some(GridTiming {
            generation: state.timing_generation,
            grid_index,
            nearest_grid_index,
            error_ms: (position - nearest_grid_index as f64) * interval_ms,
            phase_ms: (position - grid_index as f64) * interval_ms,
            interval_ms,
            bpm: state.settings.bpm,
            beats_per_bar: state.settings.beats_per_bar,
            subdivision,
        })
    }
}

fn start_audio_thread(state: Arc<Mutex<MetronomeState>>) {
    std::thread::spawn(move || {
        let mut stream: Option<Stream> = None;
        let mut seen_generation = 0_u64;
        loop {
            let (generation, running) = state
                .lock()
                .map(|state| (state.stream_generation, state.settings.running))
                .unwrap_or((seen_generation, false));
            if !running {
                if stream.take().is_some() {
                    if let Ok(mut state) = state.lock() {
                        state.active_device = None;
                        state.current_beat = 0;
                        state.current_subdivision = 0;
                        state.started_at_ms = None;
                        state.started_at = None;
                    }
                }
                seen_generation = generation;
            } else if stream.is_none() || generation != seen_generation {
                match build_stream(Arc::clone(&state)) {
                    Ok(new_stream) => {
                        stream = Some(new_stream);
                        seen_generation = generation;
                    }
                    Err(error) => {
                        stream = None;
                        seen_generation = generation;
                        if let Ok(mut state) = state.lock() {
                            state.active_device = None;
                            state.error = Some(error.to_string());
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    });
}

fn build_stream(state: Arc<Mutex<MetronomeState>>) -> Result<Stream> {
    let host = cpal::default_host();
    let desired_device = state
        .lock()
        .ok()
        .and_then(|state| state.settings.output_device.clone());
    let device = select_output_device(&host, desired_device.as_deref())?;
    let active_name = device
        .name()
        .unwrap_or_else(|_| "Unknown output".to_string());
    let config = device
        .default_output_config()
        .context("Failed to read default output config")?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let stream_config = config.config();
    let shared = Arc::clone(&state);
    let mut playback = PlaybackState::default();
    let err_shared = Arc::clone(&state);
    let err_fn = move |error: StreamError| {
        if let Ok(mut state) = err_shared.lock() {
            state.error = Some(error.to_string());
        }
    };

    let stream = match config.sample_format() {
        SampleFormat::F32 => device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _| {
                write_samples(data, channels, sample_rate, &shared, &mut playback)
            },
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_output_stream(
            &stream_config,
            move |data: &mut [i16], _| {
                write_samples(data, channels, sample_rate, &shared, &mut playback)
            },
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_output_stream(
            &stream_config,
            move |data: &mut [u16], _| {
                write_samples(data, channels, sample_rate, &shared, &mut playback)
            },
            err_fn,
            None,
        )?,
        _ => anyhow::bail!("Unsupported audio sample format"),
    };
    stream
        .play()
        .context("Failed to start metronome output stream")?;
    if let Ok(mut state) = state.lock() {
        state.active_device = Some(active_name);
        state.error = None;
    }
    Ok(stream)
}

trait MeterSample {
    fn from_meter(value: f32) -> Self;
}

impl MeterSample for f32 {
    fn from_meter(value: f32) -> Self {
        value
    }
}

impl MeterSample for i16 {
    fn from_meter(value: f32) -> Self {
        (value.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
    }
}

impl MeterSample for u16 {
    fn from_meter(value: f32) -> Self {
        ((value.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16
    }
}

fn write_samples<T: MeterSample>(
    data: &mut [T],
    channels: usize,
    sample_rate: u32,
    shared: &Arc<Mutex<MetronomeState>>,
    playback: &mut PlaybackState,
) {
    let settings = shared
        .lock()
        .map(|state| state.settings.clone())
        .unwrap_or_default();
    let running = settings.running;
    if !running {
        playback.was_running = false;
        playback.samples_until_tick = 0.0;
        playback.tick_remaining = 0;
        for sample in data {
            *sample = T::from_meter(0.0);
        }
        return;
    }

    if !playback.was_running {
        playback.samples_until_tick = 0.0;
        playback.next_beat = 0;
        playback.next_subdivision = 0;
        playback.was_running = true;
        if let Ok(mut state) = shared.lock() {
            state.started_at = Some(Instant::now());
            state.started_at_ms = Some(now_ms());
            state.timing_generation = state.timing_generation.wrapping_add(1);
        }
    }

    let frames_per_click =
        sample_rate as f64 * 60.0 / settings.bpm.max(1) as f64 / settings.subdivision.max(1) as f64;
    let tick_samples = (sample_rate as f32 * 0.045) as usize;
    for frame in data.chunks_mut(channels.max(1)) {
        if playback.samples_until_tick <= 0.0 {
            let accent = playback.next_beat == 0 && playback.next_subdivision == 0;
            playback.tick_remaining = tick_samples;
            playback.phase = 0.0;
            playback.frequency = if accent { 1760.0 } else { 1180.0 };
            if let Ok(mut state) = shared.lock() {
                state.current_beat = playback.next_beat;
                state.current_subdivision = playback.next_subdivision;
            }
            advance_position(
                &mut playback.next_beat,
                &mut playback.next_subdivision,
                settings.beats_per_bar,
                settings.subdivision,
            );
            playback.samples_until_tick += frames_per_click;
        }

        let sample = if playback.tick_remaining > 0 {
            let env = playback.tick_remaining as f32 / tick_samples.max(1) as f32;
            let value = playback.phase.sin() * env * settings.volume;
            playback.phase += std::f32::consts::TAU * playback.frequency / sample_rate as f32;
            playback.tick_remaining -= 1;
            value
        } else {
            0.0
        };

        for out in frame {
            *out = T::from_meter(sample);
        }
        playback.samples_until_tick -= 1.0;
    }
}

fn advance_position(beat: &mut u8, subdivision: &mut u8, beats_per_bar: u8, subdivisions: u8) {
    *subdivision += 1;
    if *subdivision >= subdivisions.max(1) {
        *subdivision = 0;
        *beat = (*beat + 1) % beats_per_bar.max(1);
    }
}

fn select_output_device(host: &cpal::Host, desired: Option<&str>) -> Result<cpal::Device> {
    let devices = host
        .output_devices()
        .context("Failed to list output devices")?;
    if let Some(desired) = desired.filter(|value| !value.trim().is_empty()) {
        let desired_lower = desired.to_lowercase();
        for device in devices {
            let name = device.name().unwrap_or_default();
            if name.to_lowercase() == desired_lower || name.to_lowercase().contains(&desired_lower)
            {
                return Ok(device);
            }
        }
    }
    host.default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No default output device"))
}

fn list_output_devices() -> Vec<String> {
    cpal::default_host()
        .output_devices()
        .map(|devices| {
            devices
                .filter_map(|device| device.name().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn state_json(state: &MetronomeState, include_server_time: bool, include_beat: bool) -> String {
    let mut out = String::with_capacity(1024);
    out.push('{');
    write!(
        out,
        "\"running\":{},\"bpm\":{},\"beatsPerBar\":{},\"subdivision\":{},\"volume\":{:.3},",
        state.settings.running,
        state.settings.bpm,
        state.settings.beats_per_bar,
        state.settings.subdivision,
        state.settings.volume
    )
    .unwrap();
    if include_beat {
        write!(
            out,
            "\"currentBeat\":{},\"currentSubdivision\":{},",
            state.current_beat, state.current_subdivision
        )
        .unwrap();
    }
    out.push_str("\"startedAtMs\":");
    match state.started_at_ms {
        Some(value) => write!(out, "{}", value).unwrap(),
        None => out.push_str("null"),
    }
    if include_server_time {
        write!(out, ",\"serverTimeMs\":{}", now_ms()).unwrap();
    }
    out.push_str(",\"outputDevice\":");
    write_json_string_or_null(&mut out, state.settings.output_device.as_deref());
    out.push_str(",\"activeDevice\":");
    write_json_string_or_null(&mut out, state.active_device.as_deref());
    out.push_str(",\"error\":");
    write_json_string_or_null(&mut out, state.error.as_deref());
    out.push_str(",\"devices\":[");
    for (index, device) in state.devices.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_json_string(&mut out, device);
    }
    out.push_str("]}");
    out
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

fn write_json_string_or_null(out: &mut String, value: Option<&str>) {
    match value {
        Some(value) => write_json_string(out, value),
        None => out.push_str("null"),
    }
}

fn write_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_metronome(started_at: Instant) -> Metronome {
        Metronome {
            state: Arc::new(Mutex::new(MetronomeState {
                settings: MetronomeSettings {
                    running: true,
                    bpm: 120,
                    beats_per_bar: 4,
                    subdivision: 2,
                    volume: 0.35,
                    output_device: None,
                },
                current_beat: 0,
                current_subdivision: 0,
                devices: Vec::new(),
                active_device: None,
                error: None,
                started_at_ms: Some(0),
                started_at: Some(started_at),
                stream_generation: 1,
                timing_generation: 9,
            })),
        }
    }

    #[test]
    fn grid_timing_uses_negative_for_early_and_positive_for_late() {
        let started_at = Instant::now();
        let metronome = running_metronome(started_at);

        let early = metronome
            .grid_timing(started_at + Duration::from_millis(225))
            .unwrap();
        assert_eq!(early.nearest_grid_index, 1);
        assert!((early.error_ms + 25.0).abs() < 0.001);

        let late = metronome
            .grid_timing(started_at + Duration::from_millis(275))
            .unwrap();
        assert_eq!(late.nearest_grid_index, 1);
        assert!((late.error_ms - 25.0).abs() < 0.001);
    }
}

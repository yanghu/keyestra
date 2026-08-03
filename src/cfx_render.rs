use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;

const DEFAULT_WAV_TEMPLATE_NAME: &str = "Keyestra CFX Render.rpp";
const DEFAULT_MP3_TEMPLATE_NAME: &str = "Keyestra CFX Render MP3.rpp";
const DEFAULT_REAPER_PATH: &str = r"C:\Program Files\REAPER (x64)\reaper.exe";
const RENDER_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_RENDER_TIME: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone)]
pub struct CfxRenderConfig {
    pub reaper_path: PathBuf,
    pub wav_template_path: PathBuf,
    pub mp3_template_path: PathBuf,
    pub recordings_dir: PathBuf,
}

impl CfxRenderConfig {
    pub fn discover(
        recordings_dir: PathBuf,
        reaper_path: Option<PathBuf>,
        wav_template_path: Option<PathBuf>,
        mp3_template_path: Option<PathBuf>,
    ) -> Self {
        Self {
            reaper_path: reaper_path.unwrap_or_else(|| PathBuf::from(DEFAULT_REAPER_PATH)),
            wav_template_path: wav_template_path
                .unwrap_or_else(|| default_template_path(DEFAULT_WAV_TEMPLATE_NAME)),
            mp3_template_path: mp3_template_path
                .unwrap_or_else(|| default_template_path(DEFAULT_MP3_TEMPLATE_NAME)),
            recordings_dir,
        }
    }

    fn template_path(&self, format: CfxRenderFormat) -> &Path {
        match format {
            CfxRenderFormat::Mp3 => &self.mp3_template_path,
            CfxRenderFormat::Wav => &self.wav_template_path,
        }
    }

    fn availability_error(&self, format: CfxRenderFormat) -> Option<String> {
        if !self.reaper_path.is_file() {
            return Some(format!(
                "REAPER was not found at {}",
                self.reaper_path.display()
            ));
        }
        if !self.template_path(format).is_file() {
            return Some(format!(
                "CFX {} render template was not found at {}",
                format.as_str().to_uppercase(),
                self.template_path(format).display()
            ));
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CfxRenderFormat {
    Mp3,
    Wav,
}

impl CfxRenderFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "mp3" => Some(Self::Mp3),
            "wav" => Some(Self::Wav),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Wav => "audio/wav",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfxRenderJob {
    pub recording_name: String,
    pub format: CfxRenderFormat,
    pub state: CfxRenderState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CfxRenderState {
    Queued,
    Rendering,
    Ready,
    Failed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfxRenderSnapshot {
    pub ok: bool,
    pub available: bool,
    pub reaper_path: String,
    pub formats: Vec<CfxRenderFormatStatus>,
    pub jobs: Vec<CfxRenderJob>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfxRenderFormatStatus {
    pub format: CfxRenderFormat,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    pub template_path: String,
}

#[derive(Debug)]
struct RenderRequest {
    recording_name: String,
    format: CfxRenderFormat,
}

pub struct CfxRenderer {
    config: CfxRenderConfig,
    jobs: Arc<Mutex<BTreeMap<String, CfxRenderJob>>>,
    sender: mpsc::Sender<RenderRequest>,
}

impl CfxRenderer {
    pub fn new(config: CfxRenderConfig) -> Self {
        let jobs = Arc::new(Mutex::new(BTreeMap::new()));
        let (sender, receiver) = mpsc::channel();
        let worker_jobs = Arc::clone(&jobs);
        let worker_config = config.clone();
        thread::Builder::new()
            .name("keyestra-cfx-render".into())
            .spawn(move || render_worker(receiver, worker_config, worker_jobs))
            .expect("failed to start CFX render worker");
        Self {
            config,
            jobs,
            sender,
        }
    }

    pub fn request(&self, recording_name: &str, format: CfxRenderFormat) -> Result<CfxRenderJob> {
        if let Some(error) = self.config.availability_error(format) {
            anyhow::bail!(error);
        }
        validate_recording_name(recording_name)?;
        let midi_path = self.config.recordings_dir.join(recording_name);
        if !midi_path.is_file() {
            anyhow::bail!("MIDI recording was not found");
        }

        let output_path = output_path(&self.config.recordings_dir, recording_name, format)?;
        if is_valid_render_file(&output_path, format) {
            return Ok(ready_job(recording_name, format, &output_path));
        }

        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("CFX render state lock poisoned"))?;
        let key = job_key(recording_name, format);
        if let Some(job) = jobs.get(&key) {
            if matches!(
                job.state,
                CfxRenderState::Queued | CfxRenderState::Rendering
            ) {
                return Ok(job.clone());
            }
        }
        let job = CfxRenderJob {
            recording_name: recording_name.to_string(),
            format,
            state: CfxRenderState::Queued,
            output_name: None,
            output_size: None,
            error: None,
        };
        jobs.insert(key, job.clone());
        drop(jobs);
        self.sender
            .send(RenderRequest {
                recording_name: recording_name.to_string(),
                format,
            })
            .context("CFX render worker is unavailable")?;
        Ok(job)
    }

    pub fn snapshot(&self) -> CfxRenderSnapshot {
        let formats = [CfxRenderFormat::Mp3, CfxRenderFormat::Wav]
            .into_iter()
            .map(|format| {
                let unavailable_reason = self.config.availability_error(format);
                CfxRenderFormatStatus {
                    format,
                    available: unavailable_reason.is_none(),
                    unavailable_reason,
                    template_path: self.config.template_path(format).display().to_string(),
                }
            })
            .collect::<Vec<_>>();
        let mut jobs = self
            .jobs
            .lock()
            .map(|jobs| jobs.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        if let Ok(entries) = fs::read_dir(&self.config.recordings_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let Some((recording_name, format)) = recording_and_format_from_output(name) else {
                    continue;
                };
                if is_valid_render_file(&path, format)
                    && !jobs
                        .iter()
                        .any(|job| job.recording_name == recording_name && job.format == format)
                {
                    jobs.push(ready_job(&recording_name, format, &path));
                }
            }
        }
        jobs.sort_by(|left, right| {
            left.recording_name
                .cmp(&right.recording_name)
                .then(left.format.cmp(&right.format))
        });
        CfxRenderSnapshot {
            ok: true,
            available: formats.iter().any(|format| format.available),
            reaper_path: self.config.reaper_path.display().to_string(),
            formats,
            jobs,
        }
    }

    pub fn rendered_path(&self, recording_name: &str, format: CfxRenderFormat) -> Result<PathBuf> {
        let path = output_path(&self.config.recordings_dir, recording_name, format)?;
        if !is_valid_render_file(&path, format) {
            anyhow::bail!("Rendered {} was not found", format.as_str().to_uppercase());
        }
        Ok(path)
    }
}

fn render_worker(
    receiver: mpsc::Receiver<RenderRequest>,
    config: CfxRenderConfig,
    jobs: Arc<Mutex<BTreeMap<String, CfxRenderJob>>>,
) {
    while let Ok(request) = receiver.recv() {
        update_job(&jobs, &request.recording_name, request.format, |job| {
            job.state = CfxRenderState::Rendering;
            job.error = None;
        });
        match render_recording(&config, &request.recording_name, request.format) {
            Ok(path) => update_job(&jobs, &request.recording_name, request.format, |job| {
                job.state = CfxRenderState::Ready;
                job.output_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string);
                job.output_size = path.metadata().ok().map(|metadata| metadata.len());
                job.error = None;
            }),
            Err(error) => update_job(&jobs, &request.recording_name, request.format, |job| {
                job.state = CfxRenderState::Failed;
                job.error = Some(error.to_string());
            }),
        }
    }
}

fn update_job(
    jobs: &Mutex<BTreeMap<String, CfxRenderJob>>,
    recording_name: &str,
    format: CfxRenderFormat,
    update: impl FnOnce(&mut CfxRenderJob),
) {
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&job_key(recording_name, format)) {
            update(job);
        }
    }
}

fn render_recording(
    config: &CfxRenderConfig,
    recording_name: &str,
    format: CfxRenderFormat,
) -> Result<PathBuf> {
    let midi_path = config.recordings_dir.join(recording_name);
    let output_path = output_path(&config.recordings_dir, recording_name, format)?;
    let output_stem = output_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .context("Rendered audio name is not valid UTF-8")?;
    let jobs_dir = config.recordings_dir.join(".cfx-render-jobs");
    fs::create_dir_all(&jobs_dir)
        .with_context(|| format!("Failed to create {}", jobs_dir.display()))?;
    let job_id = format!("{}-{}", unix_time_ms(), std::process::id());
    let render_stem = format!("{}-{}", output_stem, job_id);
    let project_path = jobs_dir.join(format!("{}.rpp", render_stem));
    let temporary_output = jobs_dir.join(format!("{}.{}", render_stem, format.as_str()));

    let template_path = config.template_path(format);
    let template = fs::read_to_string(template_path)
        .with_context(|| format!("Failed to read {}", template_path.display()))?;
    let midi =
        fs::read(&midi_path).with_context(|| format!("Failed to read {}", midi_path.display()))?;
    let project = build_render_project(&template, &midi, &jobs_dir, &render_stem)?;
    fs::write(&project_path, project)
        .with_context(|| format!("Failed to write {}", project_path.display()))?;

    let mut command = Command::new(&config.reaper_path);
    command
        .arg("-newinst")
        .arg("-nosplash")
        .arg("-noactivate")
        .arg("-renderproject")
        .arg(&project_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Failed to start {}", config.reaper_path.display()))?;
    let started = Instant::now();
    let exit_status = loop {
        if let Some(status) = child.try_wait().context("Failed to wait for REAPER")? {
            break status;
        }
        if started.elapsed() >= MAX_RENDER_TIME {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("REAPER render timed out after 30 minutes");
        }
        thread::sleep(RENDER_POLL_INTERVAL);
    };
    if !exit_status.success() {
        anyhow::bail!("REAPER render failed with {}", exit_status);
    }
    if !is_valid_render_file(&temporary_output, format) {
        anyhow::bail!(
            "REAPER exited successfully but did not create {}",
            temporary_output.display()
        );
    }
    if output_path.exists() {
        anyhow::bail!("Refusing to overwrite {}", output_path.display());
    }
    fs::rename(&temporary_output, &output_path).with_context(|| {
        format!(
            "Failed to finalize {} as {}",
            temporary_output.display(),
            output_path.display()
        )
    })?;
    Ok(output_path)
}

fn build_render_project(
    template: &str,
    midi: &[u8],
    output_directory: &Path,
    output_stem: &str,
) -> Result<String> {
    let normalized_template;
    let template = if template.contains("\r\n") {
        normalized_template = template.replace("\r\n", "\n");
        normalized_template.as_str()
    } else {
        template
    };
    if !template.contains("NAME \"CFX Render\"") {
        anyhow::bail!("Template does not contain a track named 'CFX Render'");
    }
    if !template.contains("<VST \"VSTi: CFX Concert Grand") {
        anyhow::bail!("Template does not contain the CFX Concert Grand VSTi");
    }
    let parsed = parse_keyestra_midi(midi)?;
    let item = rpp_midi_item(&parsed);
    let mut project = replace_project_setting(
        template,
        "RENDER_FILE",
        &format!("RENDER_FILE \"{}\"", rpp_escape(output_directory)),
    )?;
    project = replace_project_setting(
        &project,
        "RENDER_PATTERN",
        &format!("RENDER_PATTERN \"{}\"", output_stem.replace('"', "")),
    )?;
    let track_end = find_named_track_end(&project, "CFX Render")?;
    project.insert_str(track_end, &item);
    Ok(project)
}

fn replace_project_setting(project: &str, key: &str, replacement: &str) -> Result<String> {
    let prefix = format!("  {} ", key);
    let start = project
        .lines()
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((current, line))
        })
        .find_map(|(offset, line)| line.starts_with(&prefix).then_some(offset))
        .with_context(|| format!("Template is missing {}", key))?;
    let end = project[start..]
        .find('\n')
        .map(|relative| start + relative)
        .unwrap_or(project.len());
    let mut result = project.to_string();
    result.replace_range(start..end, &format!("  {}", replacement));
    Ok(result)
}

fn find_named_track_end(project: &str, name: &str) -> Result<usize> {
    let track_name = format!("    NAME \"{}\"", name);
    let name_offset = project
        .find(&track_name)
        .with_context(|| format!("Template does not contain track '{}'", name))?;
    let track_start = project[..name_offset]
        .rfind("  <TRACK ")
        .context("Could not find the start of the CFX track")?;
    let relative_end = project[track_start..]
        .find("\n  >\n")
        .context("Could not find the end of the CFX track")?;
    Ok(track_start + relative_end + 1)
}

#[derive(Debug)]
struct ParsedMidi {
    events: Vec<(u64, Vec<u8>)>,
    end_tick: u64,
    ticks_per_quarter: u16,
}

fn parse_keyestra_midi(bytes: &[u8]) -> Result<ParsedMidi> {
    let mut cursor = io::Cursor::new(bytes);
    let mut tag = [0_u8; 4];
    cursor.read_exact(&mut tag)?;
    if &tag != b"MThd" {
        anyhow::bail!("Recording is not a standard MIDI file");
    }
    let header_length = read_u32(&mut cursor)? as usize;
    if header_length < 6 {
        anyhow::bail!("MIDI header is invalid");
    }
    let format = read_u16(&mut cursor)?;
    let tracks = read_u16(&mut cursor)?;
    let division = read_u16(&mut cursor)?;
    if format != 0 || tracks != 1 || division & 0x8000 != 0 {
        anyhow::bail!("Only single-track Keyestra MIDI recordings are supported");
    }
    if header_length > 6 {
        cursor.set_position(cursor.position() + (header_length - 6) as u64);
    }
    cursor.read_exact(&mut tag)?;
    if &tag != b"MTrk" {
        anyhow::bail!("MIDI track is missing");
    }
    let track_length = read_u32(&mut cursor)? as u64;
    let track_end = cursor.position().saturating_add(track_length);
    if track_end > bytes.len() as u64 {
        anyhow::bail!("MIDI track is truncated");
    }

    let mut absolute_tick = 0_u64;
    let mut running_status = None;
    let mut events = Vec::new();
    while cursor.position() < track_end {
        absolute_tick = absolute_tick.saturating_add(read_variable_length(&mut cursor)?);
        let first = read_u8(&mut cursor)?;
        let status = if first & 0x80 != 0 {
            first
        } else {
            running_status.context("MIDI running status is invalid")?
        };
        if status == 0xFF {
            running_status = None;
            let kind = read_u8(&mut cursor)?;
            let length = read_variable_length(&mut cursor)?;
            if kind == 0x51 && length == 3 {
                let mut tempo = [0_u8; 3];
                cursor.read_exact(&mut tempo)?;
                let micros = ((tempo[0] as u32) << 16) | ((tempo[1] as u32) << 8) | tempo[2] as u32;
                if micros != 500_000 {
                    anyhow::bail!("MIDI tempo must be Keyestra's fixed 120 BPM");
                }
            } else {
                cursor.set_position(cursor.position().saturating_add(length));
            }
            if kind == 0x2F {
                break;
            }
            continue;
        }
        if status == 0xF0 || status == 0xF7 {
            running_status = None;
            let length = read_variable_length(&mut cursor)?;
            cursor.set_position(cursor.position().saturating_add(length));
            continue;
        }
        if !(0x80..=0xEF).contains(&status) {
            anyhow::bail!("Unsupported MIDI status {:02X}", status);
        }
        running_status = Some(status);
        let data_length = if matches!(status & 0xF0, 0xC0 | 0xD0) {
            1
        } else {
            2
        };
        let mut message = Vec::with_capacity(data_length + 1);
        message.push(status);
        if first & 0x80 == 0 {
            message.push(first);
        }
        while message.len() < data_length + 1 {
            message.push(read_u8(&mut cursor)?);
        }
        events.push((absolute_tick, message));
    }
    if events.is_empty() {
        anyhow::bail!("MIDI recording contains no renderable events");
    }
    Ok(ParsedMidi {
        events,
        end_tick: absolute_tick,
        ticks_per_quarter: division,
    })
}

fn rpp_midi_item(midi: &ParsedMidi) -> String {
    let guid = render_guid();
    let duration = midi.end_tick as f64 / midi.ticks_per_quarter as f64 * 0.5;
    let mut item = format!(
        "    <ITEM\n      POSITION 0\n      SNAPOFFS 0\n      LENGTH {:.6}\n      LOOP 0\n      ALLTAKES 0\n      FADEIN 1 0 0 1 0 0 0\n      FADEOUT 1 0 0 1 0 0 0\n      MUTE 0 0\n      SEL 1\n      IGUID {}\n      IID 1\n      NAME \"Keyestra MIDI\"\n      VOLPAN 1 0 1 -1\n      SOFFS 0 0\n      PLAYRATE 1 1 0 -1 0 0.0025\n      CHANMODE 0\n      GUID {}\n      <SOURCE MIDI\n        HASDATA 1 {} QN\n        CCINTERP 32\n",
        duration, guid, guid, midi.ticks_per_quarter
    );
    let mut previous_tick = 0_u64;
    for (tick, message) in &midi.events {
        let delta = tick.saturating_sub(previous_tick);
        previous_tick = *tick;
        let bytes = message
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<Vec<_>>()
            .join(" ");
        item.push_str(&format!("        E {} {}\n", delta, bytes));
    }
    item.push_str(&format!(
        "        CCINTERP 32\n        CHASE_CC_TAKEOFFS 1\n        GUID {}\n        IGNTEMPO 0 120 4 4\n        SRCCOLOR 0\n        EVTFILTER 0 -1 -1 -1 -1 0 0 0 0 -1 -1 -1 -1 0 -1 0 -1 -1\n      >\n    >\n",
        guid
    ));
    item
}

fn validate_recording_name(name: &str) -> Result<()> {
    if name.is_empty()
        || Path::new(name).file_name().and_then(|value| value.to_str()) != Some(name)
        || !name.to_ascii_lowercase().ends_with(".mid")
    {
        anyhow::bail!("Invalid MIDI recording name");
    }
    Ok(())
}

fn output_path(
    recordings_dir: &Path,
    recording_name: &str,
    format: CfxRenderFormat,
) -> Result<PathBuf> {
    validate_recording_name(recording_name)?;
    let stem = Path::new(recording_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .context("MIDI recording name must end in .mid")?;
    Ok(recordings_dir.join(format!("{}_natural.{}", stem, format.as_str())))
}

fn recording_and_format_from_output(output_name: &str) -> Option<(String, CfxRenderFormat)> {
    [CfxRenderFormat::Mp3, CfxRenderFormat::Wav]
        .into_iter()
        .find_map(|format| {
            output_name
                .strip_suffix(&format!("_natural.{}", format.as_str()))
                .map(|stem| (format!("{}.mid", stem), format))
        })
}

fn ready_job(recording_name: &str, format: CfxRenderFormat, path: &Path) -> CfxRenderJob {
    CfxRenderJob {
        recording_name: recording_name.to_string(),
        format,
        state: CfxRenderState::Ready,
        output_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string),
        output_size: path.metadata().ok().map(|metadata| metadata.len()),
        error: None,
    }
}

fn is_valid_render_file(path: &Path, format: CfxRenderFormat) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    match format {
        CfxRenderFormat::Wav => {
            if metadata.len() < 44 {
                return false;
            }
            let mut header = [0_u8; 12];
            file.read_exact(&mut header).is_ok()
                && &header[..4] == b"RIFF"
                && &header[8..] == b"WAVE"
        }
        CfxRenderFormat::Mp3 => {
            if metadata.len() < 128 {
                return false;
            }
            let mut header = [0_u8; 3];
            file.read_exact(&mut header).is_ok()
                && (&header == b"ID3" || (header[0] == 0xFF && header[1] & 0xE0 == 0xE0))
        }
    }
}

fn job_key(recording_name: &str, format: CfxRenderFormat) -> String {
    format!("{}|{}", recording_name, format.as_str())
}

fn default_template_path(template_name: &str) -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("REAPER")
        .join("ProjectTemplates")
        .join(template_name)
}

fn rpp_escape(path: &Path) -> String {
    path.display().to_string().replace('"', "")
}

fn render_guid() -> String {
    let value = unix_time_ms();
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:04X}-{:012X}}}",
        value as u32,
        (value >> 32) as u16,
        std::process::id() as u16,
        0xCF00_u16 | (value as u16 & 0x00FF),
        value as u64 & 0x0000_FFFF_FFFF_FFFF
    )
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn read_u8(reader: &mut impl Read) -> Result<u8> {
    let mut value = [0_u8; 1];
    reader.read_exact(&mut value)?;
    Ok(value[0])
}

fn read_u16(reader: &mut impl Read) -> Result<u16> {
    let mut value = [0_u8; 2];
    reader.read_exact(&mut value)?;
    Ok(u16::from_be_bytes(value))
}

fn read_u32(reader: &mut impl Read) -> Result<u32> {
    let mut value = [0_u8; 4];
    reader.read_exact(&mut value)?;
    Ok(u32::from_be_bytes(value))
}

fn read_variable_length(reader: &mut impl Read) -> Result<u64> {
    let mut value = 0_u64;
    for _ in 0..4 {
        let byte = read_u8(reader)?;
        value = (value << 7) | (byte & 0x7F) as u64;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    anyhow::bail!("MIDI variable-length value is invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi_clip::{encode_smf, ClipEvent, PreparedClip};
    use std::collections::BTreeSet;

    fn test_midi() -> Vec<u8> {
        encode_smf(&PreparedClip {
            selection_start_us: 0,
            selection_end_us: 1_500_000,
            events: vec![
                ClipEvent {
                    offset_us: 0,
                    message: vec![0x90, 60, 100],
                    generated: false,
                },
                ClipEvent {
                    offset_us: 1_000_000,
                    message: vec![0x80, 60, 0],
                    generated: false,
                },
            ],
            channels: BTreeSet::from([0]),
            warnings: vec![],
            original_event_count: 2,
        })
        .unwrap()
    }

    #[test]
    fn embeds_midi_at_project_start_and_keeps_render_settings() {
        let template = r#"<REAPER_PROJECT 0.1 "7.75/win64"
  RENDER_FILE ""
  RENDER_PATTERN $project
  <TRACK {ABC}
    NAME "CFX Render"
    <FXCHAIN
      <VST "VSTi: CFX Concert Grand (Garritan)" "CFX.dll"
      >
    >
  >
>
"#;
        let project =
            build_render_project(template, &test_midi(), Path::new("C:\\out"), "take_natural")
                .unwrap();
        assert!(project.contains("RENDER_FILE \"C:\\out\""));
        assert!(project.contains("RENDER_PATTERN \"take_natural\""));
        assert!(project.contains("POSITION 0"));
        assert!(project.contains("LENGTH 1.500000"));
        assert!(project.contains("E 0 90 3c 64"));
        assert!(project.contains("E 2000 80 3c 00"));
    }

    #[test]
    fn accepts_windows_line_endings_in_reaper_templates() {
        let template = "<REAPER_PROJECT 0.1\r\n  RENDER_FILE \"\"\r\n  RENDER_PATTERN $project\r\n  <TRACK {ABC}\r\n    NAME \"CFX Render\"\r\n    <FXCHAIN\r\n      <VST \"VSTi: CFX Concert Grand (Garritan)\" \"CFX.dll\"\r\n      >\r\n    >\r\n  >\r\n>\r\n";
        assert!(
            build_render_project(template, &test_midi(), Path::new("C:\\out"), "take")
                .unwrap()
                .contains("POSITION 0")
        );
    }

    #[test]
    fn rejects_paths_that_escape_the_recordings_directory() {
        assert!(output_path(Path::new("recordings"), "../take.mid", CfxRenderFormat::Wav).is_err());
        assert!(output_path(Path::new("recordings"), "take.wav", CfxRenderFormat::Wav).is_err());
        assert_eq!(
            output_path(Path::new("recordings"), "take.mid", CfxRenderFormat::Wav).unwrap(),
            Path::new("recordings").join("take_natural.wav")
        );
        assert_eq!(
            output_path(Path::new("recordings"), "take.mid", CfxRenderFormat::Mp3).unwrap(),
            Path::new("recordings").join("take_natural.mp3")
        );
    }
}

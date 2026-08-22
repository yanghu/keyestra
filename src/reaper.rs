use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::pianoteq_midi;

const DEFAULT_ADDRESS: &str = "127.0.0.1:8080";
const DEFAULT_OSC_ADDRESS: &str = "127.0.0.1:9000";
const DEFAULT_OSC_FEEDBACK_ADDRESS: &str = "127.0.0.1:9001";
const TOGGLE_MASTER_MUTE_COMMAND: &str = "14";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReaperConfig {
    #[serde(default)]
    reaper: ReaperConnectionConfig,
    #[serde(rename = "piano")]
    pianos: Vec<PianoConfig>,
    pianoteq_vst: Option<PianoteqVstConfig>,
    // Accepted only so existing installations keep working after the CFX
    // calibration control was removed. It is deliberately never acted on.
    #[serde(default, rename = "cfx_vst")]
    _legacy_cfx_vst: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReaperConnectionConfig {
    #[serde(default = "default_address")]
    address: String,
    username: Option<String>,
    password: Option<String>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

impl Default for ReaperConnectionConfig {
    fn default() -> Self {
        Self {
            address: default_address(),
            username: None,
            password: None,
            timeout_ms: default_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PianoConfig {
    id: String,
    name: String,
    tracks: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PianoteqVstConfig {
    piano_id: String,
    track: String,
    #[serde(default = "default_fx_number")]
    fx: usize,
    #[serde(default = "default_osc_address")]
    osc_address: String,
    #[serde(default = "default_osc_feedback_address")]
    osc_feedback_address: String,
    #[serde(default = "pianoteq_midi::default_control_output")]
    midi_control_output: String,
    // Retained solely for backward-compatible config parsing; preset loading
    // now leaves the saved Pianoteq Dynamics value untouched.
    #[serde(default, rename = "dynamics_after_preset")]
    _dynamics_after_preset: Option<f32>,
    // Old host-preset slots remain parseable so an installed config can move
    // forward without an all-at-once migration. Native preset selection uses
    // the centralized Pianoteq MIDI mapping instead.
    #[serde(default, rename = "preset")]
    _legacy_presets: Vec<PianoteqVstPresetConfig>,
    #[serde(default, rename = "reverb")]
    reverbs: Vec<PianoteqVstReverbConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PianoteqVstPresetConfig {
    #[serde(rename = "id")]
    _id: String,
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "reaper_preset")]
    _reaper_preset: String,
    #[serde(default, rename = "demo")]
    _demo: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PianoteqVstReverbConfig {
    id: String,
    name: String,
    duration: f32,
    mix: f32,
    room_dimensions: f32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReaperPiano {
    pub id: String,
    pub name: String,
    pub tracks: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaperSnapshot {
    pub configured: bool,
    pub available: bool,
    pub active_piano_id: Option<String>,
    pub pianos: Vec<ReaperPiano>,
    pub config_path: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReaperPianoteqPreset {
    pub id: String,
    pub name: String,
    pub demo: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReaperPianoteqReverb {
    pub id: String,
    pub name: String,
    pub duration: f32,
    pub mix: f32,
    pub room_dimensions: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaperPianoteqSnapshot {
    pub configured: bool,
    pub available: bool,
    pub piano_id: Option<String>,
    pub track: Option<String>,
    pub fx: Option<usize>,
    pub midi_control_output: Option<String>,
    pub presets: Vec<ReaperPianoteqPreset>,
    pub reverbs: Vec<ReaperPianoteqReverb>,
    pub parameters: HashMap<String, f32>,
    pub parameters_fresh: bool,
    pub parameter_error: Option<String>,
    pub config_path: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReaperClient {
    config_path: PathBuf,
    config: std::result::Result<ReaperConfig, String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ReaperTrack {
    index: usize,
    name: String,
    muted: bool,
}

impl ReaperClient {
    pub fn discover(explicit_path: Option<PathBuf>) -> Self {
        let config_path = explicit_path.unwrap_or_else(default_config_path);
        let config = load_config(&config_path).map_err(|error| error.to_string());
        Self {
            config_path,
            config,
        }
    }

    pub fn snapshot(&self) -> ReaperSnapshot {
        let config = match &self.config {
            Ok(config) => config,
            Err(error) => return self.unavailable(Vec::new(), false, error.clone()),
        };
        let pianos = public_pianos(config);
        match fetch_tracks(&config.reaper) {
            Ok(tracks) => match resolve_tracks(config, &tracks) {
                Ok(_) => match analyze(config, &tracks) {
                    Ok(active_piano_id) => ReaperSnapshot {
                        configured: true,
                        available: true,
                        active_piano_id,
                        pianos,
                        config_path: self.config_path.display().to_string(),
                        error: None,
                    },
                    Err(error) => ReaperSnapshot {
                        configured: true,
                        available: true,
                        active_piano_id: None,
                        pianos,
                        config_path: self.config_path.display().to_string(),
                        error: Some(error.to_string()),
                    },
                },
                Err(error) => ReaperSnapshot {
                    configured: true,
                    available: false,
                    active_piano_id: None,
                    pianos,
                    config_path: self.config_path.display().to_string(),
                    error: Some(error.to_string()),
                },
            },
            Err(error) => self.unavailable(pianos, true, error.to_string()),
        }
    }

    pub fn activate(&self, piano_id: &str) -> Result<ReaperSnapshot> {
        let config = self
            .config
            .as_ref()
            .map_err(|error| anyhow!(error.clone()))?;
        if !config.pianos.iter().any(|piano| piano.id == piano_id) {
            return Err(anyhow!("Unknown REAPER piano {piano_id:?}"));
        }

        let tracks = fetch_tracks(&config.reaper)?;
        let resolved = resolve_tracks(config, &tracks)?;
        let selected = resolved
            .get(piano_id)
            .ok_or_else(|| anyhow!("Unknown REAPER piano {piano_id:?}"))?;
        let selected_indices = selected.iter().copied().collect::<HashSet<_>>();

        // Silence every other configured piano before opening the selected path.
        let mut commands = Vec::new();
        for index in resolved.values().flatten() {
            if !selected_indices.contains(index) {
                commands.push(format!("SET/TRACK/{index}/MUTE/1"));
            }
        }
        for index in selected {
            commands.push(format!("SET/TRACK/{index}/MUTE/0"));
        }
        commands.push("TRACK".to_string());

        let body = reaper_request(&config.reaper, &commands.join(";"))?;
        let updated_tracks = parse_tracks(&body)?;
        let active_piano_id = analyze(config, &updated_tracks)?;
        if active_piano_id.as_deref() != Some(piano_id) {
            return Err(anyhow!("REAPER did not confirm the requested piano switch"));
        }

        Ok(ReaperSnapshot {
            configured: true,
            available: true,
            active_piano_id,
            pianos: public_pianos(config),
            config_path: self.config_path.display().to_string(),
            error: None,
        })
    }

    pub fn master_muted(&self) -> Result<bool> {
        let config = self
            .config
            .as_ref()
            .map_err(|error| anyhow!(error.clone()))?;
        let body = reaper_request(&config.reaper, "TRACK")?;
        parse_master_muted(&body)
    }

    pub fn set_master_muted(&self, muted: bool) -> Result<bool> {
        let config = self
            .config
            .as_ref()
            .map_err(|error| anyhow!(error.clone()))?;
        if self.master_muted()? == muted {
            return Ok(muted);
        }

        let body = reaper_request(
            &config.reaper,
            &format!("{TOGGLE_MASTER_MUTE_COMMAND};TRACK"),
        )?;
        let confirmed = parse_master_muted(&body)?;
        if confirmed != muted {
            return Err(anyhow!(
                "REAPER did not confirm the requested Master mute state"
            ));
        }
        Ok(confirmed)
    }

    pub fn pianoteq_snapshot(&self) -> ReaperPianoteqSnapshot {
        let config = match &self.config {
            Ok(config) => config,
            Err(error) => return self.unavailable_pianoteq(false, error.clone()),
        };
        let vst = match &config.pianoteq_vst {
            Some(vst) => vst,
            None => {
                return self.unavailable_pianoteq(
                    false,
                    "REAPER Pianoteq VST control is not configured".to_string(),
                )
            }
        };
        let snapshot = match fetch_tracks(&config.reaper)
            .and_then(|tracks| resolve_vst_track(vst, &tracks))
            .and_then(|_| pianoteq_midi::ensure_control_output(&vst.midi_control_output))
        {
            Ok(_) => public_pianoteq_snapshot(&self.config_path, vst, true, None),
            Err(error) => {
                public_pianoteq_snapshot(&self.config_path, vst, false, Some(error.to_string()))
            }
        };
        snapshot
    }

    pub fn set_pianoteq_parameter(
        &self,
        parameter_id: &str,
        normalized_value: f32,
    ) -> Result<ReaperPianoteqSnapshot> {
        if !normalized_value.is_finite() || !(0.0..=1.0).contains(&normalized_value) {
            return Err(anyhow!(
                "Pianoteq VST control value must be between 0 and 1"
            ));
        }
        let parameter_index = pianoteq_parameter_index(parameter_id)
            .ok_or_else(|| anyhow!("Unsupported Pianoteq VST control"))?;
        let (vst, track_index) = self.resolve_pianoteq_target()?;
        let address = format!(
            "/track/{track_index}/fx/{}/fxparam/{}/value",
            vst.fx,
            parameter_index + 1
        );
        send_osc_float(&vst.osc_address, &address, normalized_value)?;
        Ok(public_pianoteq_snapshot(&self.config_path, vst, true, None))
    }

    pub fn load_pianoteq_preset(&self, preset_id: &str) -> Result<ReaperPianoteqSnapshot> {
        let (vst, track_index) = self.resolve_pianoteq_target()?;
        let preset = pianoteq_midi::preset(preset_id)
            .ok_or_else(|| anyhow!("Unknown REAPER Pianoteq preset {preset_id:?}"))?;
        // Bind before loading the preset so no REAPER feedback can race past us.
        // Failure to open feedback must not turn a successful sound change into
        // a failed request; the snapshot reports that the displayed values are
        // not fresh instead.
        let feedback = open_osc_feedback(&vst.osc_feedback_address);
        pianoteq_midi::send_program_change(&vst.midi_control_output, preset.program_change)?;
        // Make the loaded Pianoteq sound audible after the state change was sent.
        self.activate(&vst.piano_id)?;
        let mut snapshot = public_pianoteq_snapshot(&self.config_path, vst, true, None);
        match feedback.and_then(|socket| fetch_pianoteq_parameters(&socket, vst, track_index)) {
            Ok(parameters) => {
                snapshot.parameters = parameters;
                snapshot.parameters_fresh = true;
            }
            Err(error) => snapshot.parameter_error = Some(error.to_string()),
        }
        Ok(snapshot)
    }

    pub fn reload_pianoteq_preset(&self) -> Result<ReaperPianoteqSnapshot> {
        let (vst, _) = self.resolve_pianoteq_target()?;
        pianoteq_midi::send_program_change(
            &vst.midi_control_output,
            pianoteq_midi::RELOAD_CURRENT_PRESET_PROGRAM_CHANGE,
        )?;
        self.activate(&vst.piano_id)?;
        Ok(public_pianoteq_snapshot(&self.config_path, vst, true, None))
    }

    pub fn load_pianoteq_reverb(&self, reverb_id: &str) -> Result<ReaperPianoteqSnapshot> {
        let (vst, track_index) = self.resolve_pianoteq_target()?;
        let reverb = vst
            .reverbs
            .iter()
            .find(|reverb| reverb.id == reverb_id)
            .ok_or_else(|| anyhow!("Unknown REAPER Pianoteq reverb {reverb_id:?}"))?;
        for (parameter, value) in [
            ("reverb_duration", reverb.duration),
            ("reverb_mix", reverb.mix),
            ("room_dimensions", reverb.room_dimensions),
            ("reverb_switch", 1.0),
        ] {
            let parameter_index = pianoteq_parameter_index(parameter)
                .expect("built-in Pianoteq reverb parameter must be supported");
            let address = format!(
                "/track/{track_index}/fx/{}/fxparam/{}/value",
                vst.fx,
                parameter_index + 1
            );
            send_osc_float(&vst.osc_address, &address, value)?;
        }
        Ok(public_pianoteq_snapshot(&self.config_path, vst, true, None))
    }

    fn resolve_pianoteq_target(&self) -> Result<(&PianoteqVstConfig, usize)> {
        let config = self
            .config
            .as_ref()
            .map_err(|error| anyhow!(error.clone()))?;
        let vst = config
            .pianoteq_vst
            .as_ref()
            .ok_or_else(|| anyhow!("REAPER Pianoteq VST control is not configured"))?;
        let tracks = fetch_tracks(&config.reaper)?;
        let track_index = resolve_vst_track(vst, &tracks)?;
        Ok((vst, track_index))
    }

    fn unavailable(
        &self,
        pianos: Vec<ReaperPiano>,
        configured: bool,
        error: String,
    ) -> ReaperSnapshot {
        ReaperSnapshot {
            configured,
            available: false,
            active_piano_id: None,
            pianos,
            config_path: self.config_path.display().to_string(),
            error: Some(error),
        }
    }

    fn unavailable_pianoteq(&self, configured: bool, error: String) -> ReaperPianoteqSnapshot {
        ReaperPianoteqSnapshot {
            configured,
            available: false,
            piano_id: None,
            track: None,
            fx: None,
            midi_control_output: None,
            presets: Vec::new(),
            reverbs: Vec::new(),
            parameters: HashMap::new(),
            parameters_fresh: false,
            parameter_error: None,
            config_path: self.config_path.display().to_string(),
            error: Some(error),
        }
    }
}

fn default_address() -> String {
    DEFAULT_ADDRESS.to_string()
}

fn default_timeout_ms() -> u64 {
    800
}

fn default_fx_number() -> usize {
    1
}

fn default_osc_address() -> String {
    DEFAULT_OSC_ADDRESS.to_string()
}

fn default_osc_feedback_address() -> String {
    DEFAULT_OSC_FEEDBACK_ADDRESS.to_string()
}

fn default_config_path() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("keyestra")
        .join("reaper-pianos.toml")
}

fn load_config(path: &Path) -> Result<ReaperConfig> {
    let text = fs::read_to_string(path).with_context(|| {
        format!(
            "REAPER Piano Compare is not configured; create {}",
            path.display()
        )
    })?;
    let config: ReaperConfig = toml::from_str(&text)
        .with_context(|| format!("Failed to parse REAPER piano config {}", path.display()))?;
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &ReaperConfig) -> Result<()> {
    if config.reaper.address.trim().is_empty() {
        return Err(anyhow!("REAPER web address cannot be empty"));
    }
    if config.reaper.timeout_ms == 0 {
        return Err(anyhow!("REAPER timeout_ms must be greater than zero"));
    }
    if config.pianos.is_empty() {
        return Err(anyhow!("Configure at least one [[piano]] entry"));
    }
    let mut ids = HashSet::new();
    let mut tracks = HashSet::new();
    for piano in &config.pianos {
        if piano.id.trim().is_empty() || piano.name.trim().is_empty() {
            return Err(anyhow!("Every piano needs a non-empty id and name"));
        }
        if !ids.insert(piano.id.to_lowercase()) {
            return Err(anyhow!("Duplicate piano id {:?}", piano.id));
        }
        if piano.tracks.is_empty() {
            return Err(anyhow!("Piano {:?} has no REAPER tracks", piano.name));
        }
        for track in &piano.tracks {
            if track.trim().is_empty() {
                return Err(anyhow!("Piano {:?} has an empty track name", piano.name));
            }
            if !tracks.insert(track.to_lowercase()) {
                return Err(anyhow!(
                    "REAPER track {:?} is assigned to more than one piano",
                    track
                ));
            }
        }
    }
    if let Some(vst) = &config.pianoteq_vst {
        if vst.piano_id.trim().is_empty() || vst.track.trim().is_empty() {
            return Err(anyhow!(
                "pianoteq_vst needs non-empty piano_id and track values"
            ));
        }
        if vst.fx == 0 {
            return Err(anyhow!(
                "pianoteq_vst.fx is one-based and must be at least 1"
            ));
        }
        if vst.midi_control_output.trim().is_empty() {
            return Err(anyhow!("pianoteq_vst.midi_control_output cannot be empty"));
        }
        vst.osc_address
            .to_socket_addrs()
            .with_context(|| format!("Invalid REAPER OSC address {}", vst.osc_address))?
            .next()
            .ok_or_else(|| anyhow!("REAPER OSC address did not resolve"))?;
        vst.osc_feedback_address
            .to_socket_addrs()
            .with_context(|| {
                format!(
                    "Invalid REAPER OSC feedback address {}",
                    vst.osc_feedback_address
                )
            })?
            .next()
            .ok_or_else(|| anyhow!("REAPER OSC feedback address did not resolve"))?;
        let piano = config
            .pianos
            .iter()
            .find(|piano| piano.id == vst.piano_id)
            .ok_or_else(|| anyhow!("pianoteq_vst.piano_id must match a configured piano"))?;
        if !piano
            .tracks
            .iter()
            .any(|track| track.eq_ignore_ascii_case(&vst.track))
        {
            return Err(anyhow!(
                "pianoteq_vst.track must belong to its configured piano"
            ));
        }
        let mut reverb_ids = HashSet::new();
        for reverb in &vst.reverbs {
            if reverb.id.trim().is_empty() || reverb.name.trim().is_empty() {
                return Err(anyhow!(
                    "Every pianoteq_vst reverb needs a non-empty id and name"
                ));
            }
            if !reverb_ids.insert(reverb.id.to_lowercase()) {
                return Err(anyhow!("Duplicate Pianoteq VST reverb id {:?}", reverb.id));
            }
            for value in [reverb.duration, reverb.mix, reverb.room_dimensions] {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(anyhow!(
                        "Pianoteq VST reverb values must be between 0 and 1"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn public_pianoteq_snapshot(
    config_path: &Path,
    vst: &PianoteqVstConfig,
    available: bool,
    error: Option<String>,
) -> ReaperPianoteqSnapshot {
    ReaperPianoteqSnapshot {
        configured: true,
        available,
        piano_id: Some(vst.piano_id.clone()),
        track: Some(vst.track.clone()),
        fx: Some(vst.fx),
        midi_control_output: Some(vst.midi_control_output.clone()),
        presets: pianoteq_midi::PRESET_MAPPINGS
            .iter()
            .map(|preset| ReaperPianoteqPreset {
                id: preset.id.to_string(),
                name: preset.name.to_string(),
                demo: false,
            })
            .collect(),
        reverbs: vst
            .reverbs
            .iter()
            .map(|reverb| ReaperPianoteqReverb {
                id: reverb.id.clone(),
                name: reverb.name.clone(),
                duration: reverb.duration,
                mix: reverb.mix,
                room_dimensions: reverb.room_dimensions,
            })
            .collect(),
        parameters: HashMap::new(),
        parameters_fresh: false,
        parameter_error: None,
        config_path: config_path.display().to_string(),
        error,
    }
}

fn resolve_vst_track(vst: &PianoteqVstConfig, tracks: &[ReaperTrack]) -> Result<usize> {
    let matches = tracks
        .iter()
        .filter(|track| track.name.eq_ignore_ascii_case(&vst.track))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [track] => Ok(track.index),
        [] => Err(anyhow!(
            "REAPER Pianoteq track {:?} was not found",
            vst.track
        )),
        _ => Err(anyhow!(
            "REAPER Pianoteq track {:?} is ambiguous",
            vst.track
        )),
    }
}

fn pianoteq_parameter_index(id: &str) -> Option<usize> {
    match id {
        "volume" => Some(2),
        "reverb_switch" => Some(201),
        "reverb_duration" => Some(202),
        "reverb_mix" => Some(203),
        "room_dimensions" => Some(204),
        _ => None,
    }
}

fn send_osc_float(destination: &str, address: &str, value: f32) -> Result<()> {
    let mut packet = osc_string(address);
    packet.extend_from_slice(&osc_string(",f"));
    packet.extend_from_slice(&value.to_bits().to_be_bytes());
    send_osc(destination, &packet)
}

fn send_osc_int(destination: &str, address: &str, value: i32) -> Result<()> {
    let mut packet = osc_string(address);
    packet.extend_from_slice(&osc_string(",i"));
    packet.extend_from_slice(&value.to_be_bytes());
    send_osc(destination, &packet)
}

fn open_osc_feedback(address: &str) -> Result<UdpSocket> {
    let address = address
        .to_socket_addrs()
        .with_context(|| format!("Invalid REAPER OSC feedback address {address}"))?
        .next()
        .ok_or_else(|| anyhow!("REAPER OSC feedback address did not resolve"))?;
    let socket = UdpSocket::bind(address)
        .with_context(|| format!("Failed to listen for REAPER OSC feedback on {address}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(15)))
        .context("Failed to set the REAPER OSC feedback timeout")?;
    Ok(socket)
}

fn fetch_pianoteq_parameters(
    socket: &UdpSocket,
    vst: &PianoteqVstConfig,
    track_index: usize,
) -> Result<HashMap<String, f32>> {
    send_osc_int(
        &vst.osc_address,
        "/device/track/select",
        i32::try_from(track_index).context("REAPER track index is too large")?,
    )?;
    send_osc_int(
        &vst.osc_address,
        "/device/fx/select",
        i32::try_from(vst.fx).context("REAPER FX index is too large")?,
    )?;

    let mut parameters = HashMap::new();
    // Pianoteq's reverb controls (parameters 202-205 in OSC's one-based
    // numbering) are in REAPER OSC bank 13.
    for bank in [13_usize] {
        send_osc_int(&vst.osc_address, "/device/fxparam/bank/select", bank as i32)?;
        collect_pianoteq_feedback(
            socket,
            track_index,
            vst.fx,
            bank,
            Instant::now() + Duration::from_millis(180),
            &mut parameters,
        )?;
    }

    let missing = pianoteq_display_parameters()
        .iter()
        .filter_map(|(id, _)| (!parameters.contains_key(*id)).then_some(*id))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(anyhow!(
            "REAPER did not return fresh Pianoteq values for {}. Confirm OSC feedback uses 127.0.0.1:9001 and reload the Keyestra pattern file",
            missing.join(", ")
        ));
    }
    Ok(parameters)
}

fn pianoteq_display_parameters() -> [(&'static str, usize); 4] {
    [
        ("reverb_switch", 202),
        ("reverb_duration", 203),
        ("reverb_mix", 204),
        ("room_dimensions", 205),
    ]
}

fn collect_pianoteq_feedback(
    socket: &UdpSocket,
    track_index: usize,
    fx: usize,
    bank: usize,
    deadline: Instant,
    parameters: &mut HashMap<String, f32>,
) -> Result<()> {
    let mut packet = [0_u8; 4096];
    while Instant::now() < deadline {
        match socket.recv_from(&mut packet) {
            Ok((length, _)) => {
                parse_osc_packet(&packet[..length], &mut |address, value| {
                    let Some(number) =
                        osc_feedback_parameter_number(address, track_index, fx, bank)
                    else {
                        return;
                    };
                    if let Some((id, _)) = pianoteq_display_parameters()
                        .iter()
                        .find(|(_, parameter)| *parameter == number)
                    {
                        parameters.insert((*id).to_string(), value.clamp(0.0, 1.0));
                    }
                })?;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error).context("Failed to receive REAPER OSC feedback"),
        }
    }
    Ok(())
}

fn osc_feedback_parameter_number(
    address: &str,
    track_index: usize,
    fx: usize,
    bank: usize,
) -> Option<usize> {
    let prefix = format!("/track/{track_index}/fx/{fx}/fxparam/");
    let absolute_number = address
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix("/value"))
        .and_then(|number| number.parse::<usize>().ok());
    let selected_fx_prefix = format!("/fx/{fx}/fxparam/");
    let bank_slot = address
        .strip_prefix(&selected_fx_prefix)
        .or_else(|| address.strip_prefix("/fxparam/"))
        .and_then(|rest| rest.strip_suffix("/value"))
        .and_then(|number| number.parse::<usize>().ok());
    absolute_number.or_else(|| {
        bank_slot.map(|slot| {
            if slot <= 16 {
                (bank - 1) * 16 + slot
            } else {
                slot
            }
        })
    })
}

fn parse_osc_packet(packet: &[u8], callback: &mut impl FnMut(&str, f32)) -> Result<()> {
    if packet.starts_with(b"#bundle\0") {
        if packet.len() < 16 {
            return Err(anyhow!("REAPER sent an incomplete OSC bundle"));
        }
        let mut offset = 16;
        while offset < packet.len() {
            if offset + 4 > packet.len() {
                return Err(anyhow!("REAPER sent an incomplete OSC bundle element"));
            }
            let length =
                u32::from_be_bytes(packet[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;
            if offset + length > packet.len() {
                return Err(anyhow!("REAPER sent an invalid OSC bundle element length"));
            }
            parse_osc_packet(&packet[offset..offset + length], callback)?;
            offset += length;
        }
        return Ok(());
    }

    let mut offset = 0;
    let address = read_osc_string(packet, &mut offset)?;
    let types = read_osc_string(packet, &mut offset)?;
    if types == ",f" && offset + 4 <= packet.len() {
        let value = f32::from_bits(u32::from_be_bytes(
            packet[offset..offset + 4].try_into().unwrap(),
        ));
        if value.is_finite() {
            callback(address, value);
        }
    }
    Ok(())
}

fn read_osc_string<'a>(packet: &'a [u8], offset: &mut usize) -> Result<&'a str> {
    let start = *offset;
    let end = packet[start..]
        .iter()
        .position(|byte| *byte == 0)
        .map(|length| start + length)
        .ok_or_else(|| anyhow!("REAPER sent an unterminated OSC string"))?;
    *offset = (end + 4) & !3;
    if *offset > packet.len() {
        return Err(anyhow!("REAPER sent an invalid padded OSC string"));
    }
    std::str::from_utf8(&packet[start..end]).context("REAPER sent a non-UTF-8 OSC string")
}

fn send_osc(destination: &str, packet: &[u8]) -> Result<()> {
    let address = destination
        .to_socket_addrs()
        .with_context(|| format!("Invalid REAPER OSC address {destination}"))?
        .next()
        .ok_or_else(|| anyhow!("REAPER OSC address did not resolve"))?;
    let bind_address = if address.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let socket = UdpSocket::bind(bind_address).context("Failed to open REAPER OSC socket")?;
    socket
        .send_to(packet, address)
        .with_context(|| format!("Failed to send REAPER OSC to {destination}"))?;
    Ok(())
}

fn osc_string(value: &str) -> Vec<u8> {
    let mut output = value.as_bytes().to_vec();
    output.push(0);
    while output.len() % 4 != 0 {
        output.push(0);
    }
    output
}

fn public_pianos(config: &ReaperConfig) -> Vec<ReaperPiano> {
    config
        .pianos
        .iter()
        .map(|piano| ReaperPiano {
            id: piano.id.clone(),
            name: piano.name.clone(),
            tracks: piano.tracks.clone(),
        })
        .collect()
}

fn fetch_tracks(config: &ReaperConnectionConfig) -> Result<Vec<ReaperTrack>> {
    parse_tracks(&reaper_request(config, "TRACK")?)
}

fn reaper_request(config: &ReaperConnectionConfig, command: &str) -> Result<String> {
    let mut addresses = config
        .address
        .to_socket_addrs()
        .with_context(|| format!("Invalid REAPER web address {}", config.address))?;
    let address = addresses
        .next()
        .ok_or_else(|| anyhow!("REAPER web address did not resolve"))?;
    let timeout = Duration::from_millis(config.timeout_ms);
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .with_context(|| format!("REAPER web control is not available at {}", config.address))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let authorization = config.username.as_ref().map(|username| {
        let credentials = format!("{}:{}", username, config.password.as_deref().unwrap_or(""));
        format!(
            "Authorization: Basic {}\r\n",
            base64(credentials.as_bytes())
        )
    });
    write!(
        stream,
        "GET /_/{} HTTP/1.1\r\nHost: {}\r\n{}Connection: close\r\n\r\n",
        command,
        config.address,
        authorization.as_deref().unwrap_or("")
    )?;

    let mut response = Vec::new();
    if let Err(error) = stream.read_to_end(&mut response) {
        // Some small HTTP servers close with a TCP reset after sending a complete
        // response. Preserve a response we already received and validate it below.
        if response.is_empty() {
            return Err(error.into());
        }
    }
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("Invalid HTTP response from REAPER"))?;
    let headers = String::from_utf8_lossy(&response[..separator]);
    let status = headers.lines().next().unwrap_or_default();
    if !status.contains(" 200 ") {
        return Err(anyhow!("REAPER web control returned {status}"));
    }
    String::from_utf8(response[separator + 4..].to_vec())
        .context("REAPER web control returned invalid UTF-8")
}

fn parse_tracks(body: &str) -> Result<Vec<ReaperTrack>> {
    let mut tracks = Vec::new();
    for line in body.lines() {
        let fields = line.trim_end_matches('\r').split('\t').collect::<Vec<_>>();
        if fields.first() != Some(&"TRACK") {
            continue;
        }
        if fields.len() < 4 {
            return Err(anyhow!("REAPER returned an incomplete TRACK response"));
        }
        let index = fields[1]
            .parse::<usize>()
            .context("REAPER returned an invalid track index")?;
        if index == 0 {
            continue;
        }
        let flags = fields[3]
            .parse::<u32>()
            .context("REAPER returned invalid track flags")?;
        tracks.push(ReaperTrack {
            index,
            name: decode_reaper_string(fields[2]),
            muted: flags & 8 != 0,
        });
    }
    if tracks.is_empty() {
        return Err(anyhow!("REAPER project has no tracks"));
    }
    Ok(tracks)
}

fn parse_master_muted(body: &str) -> Result<bool> {
    for line in body.lines() {
        let fields = line.trim_end_matches('\r').split('\t').collect::<Vec<_>>();
        if fields.first() != Some(&"TRACK") || fields.get(1) != Some(&"0") {
            continue;
        }
        let flags = fields
            .get(3)
            .ok_or_else(|| anyhow!("REAPER returned an incomplete Master track response"))?
            .parse::<u32>()
            .context("REAPER returned invalid Master track flags")?;
        return Ok(flags & 8 != 0);
    }
    Err(anyhow!("REAPER did not return its Master track state"))
}

fn decode_reaper_string(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some('\\') => output.push('\\'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn resolve_tracks(
    config: &ReaperConfig,
    tracks: &[ReaperTrack],
) -> Result<HashMap<String, Vec<usize>>> {
    let mut resolved = HashMap::new();
    for piano in &config.pianos {
        let mut indices = Vec::new();
        for desired_name in &piano.tracks {
            let matches = tracks
                .iter()
                .filter(|track| track.name.eq_ignore_ascii_case(desired_name))
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [track] => indices.push(track.index),
                [] => return Err(anyhow!("REAPER track {:?} was not found", desired_name)),
                _ => {
                    return Err(anyhow!(
                        "REAPER track name {:?} is ambiguous; track names must be unique",
                        desired_name
                    ))
                }
            }
        }
        resolved.insert(piano.id.clone(), indices);
    }
    Ok(resolved)
}

fn analyze(config: &ReaperConfig, tracks: &[ReaperTrack]) -> Result<Option<String>> {
    let resolved = resolve_tracks(config, tracks)?;
    let by_index = tracks
        .iter()
        .map(|track| (track.index, track))
        .collect::<HashMap<_, _>>();
    let mut active = Vec::new();
    for piano in &config.pianos {
        let mut muted = resolved[&piano.id]
            .iter()
            .map(|index| by_index[index].muted);
        let first = muted.next().expect("validated piano has tracks");
        if muted.any(|value| value != first) {
            return Err(anyhow!(
                "REAPER piano {:?} has inconsistent track mute states",
                piano.name
            ));
        }
        if !first {
            active.push(piano.id.clone());
        }
    }
    match active.as_slice() {
        [] => Err(anyhow!("No configured REAPER piano is active")),
        [id] => Ok(Some(id.clone())),
        _ => Err(anyhow!(
            "Multiple configured REAPER pianos are active; select one in Keyestra"
        )),
    }
}

fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let value = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        output.push(TABLE[((value >> 18) & 63) as usize] as char);
        output.push(TABLE[((value >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Shutdown, TcpListener};
    use std::sync::mpsc;
    use std::thread;

    fn config() -> ReaperConfig {
        toml::from_str(
            r#"
                [[piano]]
                id = "cfx"
                name = "Garritan CFX"
                tracks = ["Garritan CFX"]

                [[piano]]
                id = "sk-ex"
                name = "Pianoteq SK-EX"
                tracks = ["SK-EX Close", "SK-EX Room"]
            "#,
        )
        .unwrap()
    }

    #[test]
    fn shipped_config_uses_the_two_live_piano_identities() {
        let shipped: ReaperConfig =
            toml::from_str(include_str!("../examples/reaper-pianos.toml")).unwrap();
        validate_config(&shipped).unwrap();
        let identities = shipped
            .pianos
            .iter()
            .map(|piano| (piano.id.as_str(), piano.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            identities,
            vec![("cfx", "Garritan CFX"), ("pianoteq-live", "Pianoteq Live"),]
        );

        let bootstrap = include_str!("../scripts/reaper/keyestra-piano-live-simplify.lua");
        for (id, name) in identities {
            assert!(bootstrap.contains(name));
            assert!(id == "cfx" || id == "pianoteq-live");
        }
    }

    #[test]
    fn parses_reaper_track_flags_and_escaped_names() {
        let tracks = parse_tracks(
            "TRACK\t0\tMASTER\t0\t1\nTRACK\t1\tGarritan CFX\t4\t1\nTRACK\t2\tSK-EX\\tClose\t8\t1\n",
        )
        .unwrap();
        assert_eq!(tracks.len(), 2);
        assert!(!tracks[0].muted);
        assert_eq!(tracks[1].name, "SK-EX\tClose");
        assert!(tracks[1].muted);
    }

    #[test]
    fn parses_reaper_master_mute_flag() {
        assert!(!parse_master_muted("TRACK\t0\tMASTER\t512\t1\n").unwrap());
        assert!(parse_master_muted("TRACK\t0\tMASTER\t520\t1\n").unwrap());
        assert!(parse_master_muted("TRACK\t1\tPiano\t4\t1\n").is_err());
    }

    #[test]
    fn detects_exactly_one_active_piano() {
        let config = config();
        let tracks = vec![
            ReaperTrack {
                index: 1,
                name: "Garritan CFX".into(),
                muted: true,
            },
            ReaperTrack {
                index: 2,
                name: "SK-EX Close".into(),
                muted: false,
            },
            ReaperTrack {
                index: 3,
                name: "SK-EX Room".into(),
                muted: false,
            },
        ];
        assert_eq!(analyze(&config, &tracks).unwrap(), Some("sk-ex".into()));
    }

    #[test]
    fn rejects_a_partly_unmuted_multitrack_piano() {
        let config = config();
        let tracks = vec![
            ReaperTrack {
                index: 1,
                name: "Garritan CFX".into(),
                muted: false,
            },
            ReaperTrack {
                index: 2,
                name: "SK-EX Close".into(),
                muted: false,
            },
            ReaperTrack {
                index: 3,
                name: "SK-EX Room".into(),
                muted: true,
            },
        ];
        assert!(analyze(&config, &tracks)
            .unwrap_err()
            .to_string()
            .contains("inconsistent"));
    }

    #[test]
    fn rejects_missing_duplicate_and_shared_tracks() {
        let config = config();
        let missing = vec![ReaperTrack {
            index: 1,
            name: "Garritan CFX".into(),
            muted: false,
        }];
        assert!(resolve_tracks(&config, &missing)
            .unwrap_err()
            .to_string()
            .contains("not found"));

        let duplicate = vec![
            ReaperTrack {
                index: 1,
                name: "Garritan CFX".into(),
                muted: false,
            },
            ReaperTrack {
                index: 2,
                name: "garritan cfx".into(),
                muted: true,
            },
        ];
        assert!(resolve_tracks(&config, &duplicate)
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));

        let shared: ReaperConfig = toml::from_str(
            r#"
                [[piano]]
                id="a"
                name="A"
                tracks=["Shared"]
                [[piano]]
                id="b"
                name="B"
                tracks=["shared"]
            "#,
        )
        .unwrap();
        assert!(validate_config(&shared)
            .unwrap_err()
            .to_string()
            .contains("more than one"));
    }

    #[test]
    fn basic_auth_encoding_is_standard() {
        assert_eq!(
            base64(b"Aladdin:open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn osc_packets_are_padded_and_use_network_byte_order() {
        let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
        listener
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let destination = listener.local_addr().unwrap().to_string();
        send_osc_float(&destination, "/track/2/fx/1/fxparam/2/value", 0.5).unwrap();

        let mut packet = [0_u8; 256];
        let (length, _) = listener.recv_from(&mut packet).unwrap();
        let expected_address = osc_string("/track/2/fx/1/fxparam/2/value");
        let expected_type = osc_string(",f");
        assert_eq!(&packet[..expected_address.len()], expected_address);
        assert_eq!(
            &packet[expected_address.len()..expected_address.len() + expected_type.len()],
            expected_type
        );
        assert_eq!(
            &packet[length - 4..length],
            &0.5_f32.to_bits().to_be_bytes()
        );
    }

    #[test]
    fn validates_pianoteq_vst_target_and_legacy_host_presets() {
        let config: ReaperConfig = toml::from_str(
            r#"
                [pianoteq_vst]
                piano_id = "live"
                track = "Pianoteq Live"
                dynamics_after_preset = 0.45
                [[pianoteq_vst.preset]]
                id = "sk-ex"
                name = "SK-EX Player"
                reaper_preset = "SK-EX Player"
                [[pianoteq_vst.reverb]]
                id = "studio"
                name = "Studio"
                duration = 0.3
                mix = 0.2
                room_dimensions = 0.4

                [cfx_vst]
                piano_id = "cfx"
                track = "CFX"
                fx = 1
                master_volume_parameter = 7
                osc_address = "127.0.0.1:9000"

                [[piano]]
                id = "cfx"
                name = "CFX"
                tracks = ["CFX"]
                [[piano]]
                id = "live"
                name = "Pianoteq"
                tracks = ["Pianoteq Live"]
            "#,
        )
        .unwrap();
        validate_config(&config).unwrap();
        let vst = config.pianoteq_vst.as_ref().unwrap();
        assert_eq!(vst.fx, 1);
        assert_eq!(vst.osc_address, DEFAULT_OSC_ADDRESS);
        assert_eq!(vst.osc_feedback_address, DEFAULT_OSC_FEEDBACK_ADDRESS);
        assert_eq!(
            vst.midi_control_output,
            pianoteq_midi::DEFAULT_CONTROL_WRITE_PORT
        );
        assert_eq!(vst.reverbs.len(), 1);
        assert_eq!(pianoteq_parameter_index("not-a-control"), None);

        let snapshot = public_pianoteq_snapshot(Path::new("test.toml"), vst, true, None);
        assert_eq!(snapshot.presets.len(), 15);
        assert_eq!(snapshot.presets[0].id, "sk-ex-player");
    }

    #[test]
    fn parses_reaper_osc_parameter_feedback_messages_and_bundles() {
        let address = osc_string("/track/2/fx/1/fxparam/203/value");
        let types = osc_string(",f");
        let mut message = address;
        message.extend_from_slice(&types);
        message.extend_from_slice(&0.375_f32.to_bits().to_be_bytes());

        let mut values = Vec::new();
        parse_osc_packet(&message, &mut |address, value| {
            values.push((address.to_string(), value));
        })
        .unwrap();
        assert_eq!(
            values,
            vec![("/track/2/fx/1/fxparam/203/value".to_string(), 0.375)]
        );

        let mut bundle = b"#bundle\0".to_vec();
        bundle.extend_from_slice(&[0_u8; 8]);
        bundle.extend_from_slice(&(message.len() as u32).to_be_bytes());
        bundle.extend_from_slice(&message);
        values.clear();
        parse_osc_packet(&bundle, &mut |address, value| {
            values.push((address.to_string(), value));
        })
        .unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].1, 0.375);
    }

    #[test]
    fn maps_absolute_and_banked_reaper_osc_parameter_addresses() {
        assert_eq!(
            osc_feedback_parameter_number("/track/2/fx/1/fxparam/203/value", 2, 1, 13),
            Some(203)
        );
        assert_eq!(
            osc_feedback_parameter_number("/fx/1/fxparam/11/value", 2, 1, 13),
            Some(203)
        );
        assert_eq!(
            osc_feedback_parameter_number("/fxparam/12/value", 2, 1, 13),
            Some(204)
        );
        assert_eq!(
            osc_feedback_parameter_number("/track/3/fx/1/fxparam/203/value", 2, 1, 13),
            None
        );
    }

    #[test]
    fn activation_mutes_other_pianos_before_unmuting_and_confirms_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            for body in [
                "TRACK\t1\tGarritan CFX\t0\nTRACK\t2\tSK-EX Close\t0\nTRACK\t3\tSK-EX Room\t0\n",
                "TRACK\t1\tGarritan CFX\t8\nTRACK\t2\tSK-EX Close\t0\nTRACK\t3\tSK-EX Room\t0\n",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 256];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let bytes_read = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..bytes_read]);
                }
                request_tx
                    .send(String::from_utf8_lossy(&request).to_string())
                    .unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
                stream.flush().unwrap();
                stream.shutdown(Shutdown::Write).unwrap();
            }
        });

        let mut config = config();
        config.reaper.address = address;
        let client = ReaperClient {
            config_path: PathBuf::from("test.toml"),
            config: Ok(config),
        };
        let snapshot = client.activate("sk-ex").unwrap();
        assert_eq!(snapshot.active_piano_id.as_deref(), Some("sk-ex"));

        let first = request_rx.recv().unwrap();
        assert!(first.contains("GET /_/TRACK HTTP/1.1"), "{first:?}");
        let second = request_rx.recv().unwrap();
        assert!(
            second.contains("/_/SET/TRACK/1/MUTE/1;SET/TRACK/2/MUTE/0;SET/TRACK/3/MUTE/0;TRACK")
        );
        server.join().unwrap();
    }
}

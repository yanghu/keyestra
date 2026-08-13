use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

const DEFAULT_ADDRESS: &str = "127.0.0.1:8080";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReaperConfig {
    #[serde(default)]
    reaper: ReaperConnectionConfig,
    #[serde(rename = "piano")]
    pianos: Vec<PianoConfig>,
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
}

fn default_address() -> String {
    DEFAULT_ADDRESS.to_string()
}

fn default_timeout_ms() -> u64 {
    800
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
    Ok(())
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
    fn shipped_config_and_bootstrap_use_the_same_piano_identities() {
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
            vec![
                ("cfx", "Garritan CFX"),
                ("sk-ex", "Pianoteq SK-EX"),
                ("steinway-d", "Pianoteq Steinway D"),
                ("280vc", "Pianoteq Bösendorfer 280VC"),
            ]
        );

        let bootstrap = include_str!("../scripts/reaper/keyestra-piano-compare-bootstrap.lua");
        for (id, name) in identities {
            assert!(bootstrap.contains(&format!("id = \"{id}\"")));
            assert!(bootstrap.contains(&format!("name = \"{name}\"")));
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

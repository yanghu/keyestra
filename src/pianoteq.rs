use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PianoteqPreset {
    pub name: String,
    pub bank: String,
    pub display_name: String,
    pub instrument: String,
    pub collection: String,
    pub license: String,
    pub license_status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PianoteqReverbPreset {
    pub name: String,
    pub bank: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PianoteqControl {
    pub id: String,
    pub name: String,
    pub normalized_value: f64,
    pub text: String,
    pub unit: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PianoteqSnapshot {
    pub available: bool,
    pub current_preset: Option<String>,
    pub current_preset_name: Option<String>,
    pub current_preset_bank: Option<String>,
    pub current_instrument: Option<String>,
    pub current_reverb_preset: Option<String>,
    pub presets: Vec<PianoteqPreset>,
    pub reverb_presets: Vec<PianoteqReverbPreset>,
    pub controls: Vec<PianoteqControl>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PianoteqClient {
    address: String,
}

impl PianoteqClient {
    pub fn new(address: String) -> Self {
        Self { address }
    }

    pub fn snapshot(&self) -> PianoteqSnapshot {
        match self.fetch_snapshot() {
            Ok((
                current_preset,
                current_preset_name,
                current_preset_bank,
                current_instrument,
                current_reverb_preset,
                presets,
                reverb_presets,
                controls,
            )) => PianoteqSnapshot {
                available: true,
                current_preset,
                current_preset_name,
                current_preset_bank,
                current_instrument,
                current_reverb_preset,
                presets,
                reverb_presets,
                controls,
                error: None,
            },
            Err(error) => PianoteqSnapshot {
                available: false,
                current_preset: None,
                current_preset_name: None,
                current_preset_bank: None,
                current_instrument: None,
                current_reverb_preset: None,
                presets: Vec::new(),
                reverb_presets: Vec::new(),
                controls: Vec::new(),
                error: Some(error.to_string()),
            },
        }
    }

    pub fn load_preset(&self, name: &str, bank: &str) -> Result<PianoteqSnapshot> {
        if name.trim().is_empty() {
            return Err(anyhow!("Preset name cannot be empty"));
        }
        self.rpc("loadPreset", json!({ "name": name, "bank": bank }))?;
        let mut snapshot = self.snapshot();
        if snapshot.available && snapshot.current_preset.is_none() {
            snapshot.current_preset = Some(display_name(name, bank));
            snapshot.current_preset_name = Some(name.to_string());
            snapshot.current_preset_bank = Some(bank.to_string());
        }
        Ok(snapshot)
    }

    pub fn save_preset(&self, name: &str, bank: &str) -> Result<PianoteqSnapshot> {
        let name = name.trim();
        let bank = bank.trim();
        if name.is_empty() {
            return Err(anyhow!("Preset name cannot be empty"));
        }
        if bank.is_empty() {
            return Err(anyhow!("A user preset bank is required"));
        }
        self.rpc(
            "savePreset",
            json!({ "name": name, "bank": bank, "preset_type": "full" }),
        )?;
        let mut snapshot = self.snapshot();
        if snapshot.available {
            snapshot.current_preset = Some(display_name(name, bank));
            snapshot.current_preset_name = Some(name.to_string());
            snapshot.current_preset_bank = Some(bank.to_string());
        }
        Ok(snapshot)
    }

    pub fn load_reverb_preset(&self, name: &str, bank: &str) -> Result<PianoteqSnapshot> {
        if name.trim().is_empty() {
            return Err(anyhow!("Reverb preset name cannot be empty"));
        }
        self.rpc(
            "loadPreset",
            json!({ "name": name, "bank": bank, "preset_type": "reverb" }),
        )?;
        let mut snapshot = self.snapshot();
        if snapshot.available && snapshot.current_reverb_preset.is_none() {
            snapshot.current_reverb_preset = Some(display_name(name, bank));
        }
        Ok(snapshot)
    }

    pub fn set_volume_db(&self, value_db: f64) -> Result<Vec<PianoteqControl>> {
        if !value_db.is_finite() || !(-60.0..=12.0).contains(&value_db) {
            return Err(anyhow!("Pianoteq volume must be between -60 and +12 dB"));
        }
        self.rpc(
            "setParameters",
            json!({ "list": [{ "id": "volume", "text": format!("{value_db:.1}") }] }),
        )?;
        self.fetch_controls()
    }

    pub fn set_parameter(&self, id: &str, normalized_value: f64) -> Result<Vec<PianoteqControl>> {
        const ALLOWED: [&str; 5] = [
            "volume",
            "dynamics",
            "reverb_switch",
            "reverb_duration",
            "reverb_mix",
        ];
        if !ALLOWED.contains(&id) {
            return Err(anyhow!("Unsupported Pianoteq control"));
        }
        if !normalized_value.is_finite() || !(0.0..=1.0).contains(&normalized_value) {
            return Err(anyhow!("Pianoteq control value must be between 0 and 1"));
        }
        self.rpc(
            "setParameters",
            json!({ "list": [{ "id": id, "normalized_value": normalized_value }] }),
        )?;
        self.fetch_controls()
    }

    fn fetch_snapshot(
        &self,
    ) -> Result<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Vec<PianoteqPreset>,
        Vec<PianoteqReverbPreset>,
        Vec<PianoteqControl>,
    )> {
        let presets = parse_presets(self.rpc("getListOfPresets", json!({}))?);
        let reverb_presets =
            parse_reverb_presets(self.rpc("getListOfPresets", json!({ "preset_type": "reverb" }))?);
        let info = self.rpc("getInfo", json!({})).ok();
        let (current_preset, current_preset_name, current_preset_bank, current_instrument) = info
            .as_ref()
            .map(find_current_preset_info)
            .unwrap_or_default();
        let current_reverb_preset = info.as_ref().and_then(find_current_reverb_preset);
        let controls = self.fetch_controls().unwrap_or_default();
        Ok((
            current_preset,
            current_preset_name,
            current_preset_bank,
            current_instrument,
            current_reverb_preset,
            presets,
            reverb_presets,
            controls,
        ))
    }

    fn fetch_controls(&self) -> Result<Vec<PianoteqControl>> {
        Ok(parse_controls(self.rpc("getParameters", json!({}))?))
    }

    fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let request_body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        })
        .to_string();
        let mut addresses = self
            .address
            .to_socket_addrs()
            .with_context(|| format!("Invalid Pianoteq RPC address {}", self.address))?;
        let address = addresses
            .next()
            .ok_or_else(|| anyhow!("Pianoteq RPC address did not resolve"))?;
        let timeout = Duration::from_secs(2);
        let mut stream = TcpStream::connect_timeout(&address, timeout)
            .with_context(|| format!("Pianoteq is not listening on {}", self.address))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        write!(
            stream,
            "POST /jsonrpc HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.address,
            request_body.len(),
            request_body
        )?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let separator = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| anyhow!("Invalid HTTP response from Pianoteq"))?;
        let headers = String::from_utf8_lossy(&response[..separator]);
        let status = headers.lines().next().unwrap_or_default();
        if !status.contains(" 200 ") {
            return Err(anyhow!("Pianoteq RPC returned {}", status));
        }
        let response: Value = serde_json::from_slice(&response[separator + 4..])
            .context("Pianoteq returned invalid JSON")?;
        if let Some(error) = response.get("error") {
            return Err(anyhow!("Pianoteq RPC error: {}", error));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("Pianoteq RPC response has no result"))
    }
}

fn parse_presets(value: Value) -> Vec<PianoteqPreset> {
    let entries = value
        .as_array()
        .or_else(|| value.get("presets").and_then(Value::as_array));
    let mut presets = entries
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            if let Some(name) = entry.as_str() {
                return Some(PianoteqPreset {
                    name: name.to_string(),
                    bank: String::new(),
                    display_name: name.to_string(),
                    instrument: "Other".to_string(),
                    collection: String::new(),
                    license: String::new(),
                    license_status: String::new(),
                });
            }
            let name = entry.get("name")?.as_str()?.to_string();
            let bank = entry
                .get("bank")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(PianoteqPreset {
                display_name: display_name(&name, &bank),
                instrument: entry
                    .get("instr")
                    .and_then(Value::as_str)
                    .unwrap_or("Other")
                    .to_string(),
                collection: entry
                    .get("collection")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                license: entry
                    .get("license")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                license_status: entry
                    .get("license_status")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name,
                bank,
            })
        })
        .collect::<Vec<_>>();
    presets.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
    });
    presets
}

fn parse_reverb_presets(value: Value) -> Vec<PianoteqReverbPreset> {
    let entries = value
        .as_array()
        .or_else(|| value.get("presets").and_then(Value::as_array));
    entries
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            if let Some(name) = entry.as_str() {
                return Some(PianoteqReverbPreset {
                    name: name.to_string(),
                    bank: String::new(),
                    display_name: name.to_string(),
                });
            }
            let name = entry.get("name")?.as_str()?.to_string();
            let bank = entry
                .get("bank")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(PianoteqReverbPreset {
                display_name: display_name(&name, &bank),
                name,
                bank,
            })
        })
        .collect()
}

fn parse_controls(value: Value) -> Vec<PianoteqControl> {
    const CONTROL_IDS: [&str; 5] = [
        "volume",
        "dynamics",
        "reverb_switch",
        "reverb_duration",
        "reverb_mix",
    ];
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?;
            CONTROL_IDS.contains(&id).then(|| PianoteqControl {
                id: id.to_string(),
                name: entry
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_string(),
                normalized_value: entry
                    .get("normalized_value")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
                text: entry
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                unit: entry
                    .get("unit")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect()
}

fn display_name(name: &str, bank: &str) -> String {
    if bank.is_empty() {
        name.to_string()
    } else {
        format!("{} / {}", bank, name)
    }
}

fn find_current_preset_info(
    value: &Value,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    if let Some(object) = value.as_object() {
        for key in ["current_preset", "preset_name", "presetName", "preset"] {
            if let Some(candidate) = object.get(key) {
                if let Some(name) = candidate.as_str() {
                    return (Some(name.to_string()), Some(name.to_string()), None, None);
                }
                if let Some(name) = candidate.get("name").and_then(Value::as_str) {
                    let bank = candidate
                        .get("bank")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let instrument = candidate
                        .get("instrument")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    return (
                        Some(display_name(name, bank)),
                        Some(name.to_string()),
                        Some(bank.to_string()),
                        instrument,
                    );
                }
            }
        }
        for child in object.values() {
            let info = find_current_preset_info(child);
            if info.0.is_some() {
                return info;
            }
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            let info = find_current_preset_info(child);
            if info.0.is_some() {
                return info;
            }
        }
    }
    (None, None, None, None)
}

fn find_current_reverb_preset(value: &Value) -> Option<String> {
    if let Some(object) = value.as_object() {
        if let Some(reverb) = object
            .get("mini_presets")
            .and_then(|presets| presets.get("reverb"))
        {
            if let Some(name) = reverb.as_str() {
                return Some(name.to_string());
            }
            if let Some(name) = reverb.get("name").and_then(Value::as_str) {
                let bank = reverb
                    .get("bank")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                return Some(display_name(name, bank));
            }
        }
        object.values().find_map(find_current_reverb_preset)
    } else {
        value
            .as_array()
            .and_then(|array| array.iter().find_map(find_current_reverb_preset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_list_preserves_bank_for_custom_presets() {
        let presets = parse_presets(json!([
            {"name":"Recording", "bank":"My Presets"},
            {"name":"Steinway D", "bank":""}
        ]));
        assert_eq!(presets[0].display_name, "My Presets / Recording");
        assert_eq!(presets[0].bank, "My Presets");
        assert_eq!(presets[1].display_name, "Steinway D");
    }

    #[test]
    fn preset_list_exposes_license_metadata_for_ui_grouping() {
        let presets = parse_presets(json!([{
            "name":"Pleyel 1926",
            "bank":"",
            "instr":"Pleyel Model F",
            "collection":"KIViR",
            "license":"KIViR",
            "license_status":"ok"
        }]));
        assert_eq!(presets[0].collection, "KIViR");
        assert_eq!(presets[0].license, "KIViR");
        assert_eq!(presets[0].license_status, "ok");
    }

    #[test]
    fn reverb_presets_preserve_names_and_banks() {
        let presets = parse_reverb_presets(json!([
            {"name":"Piano room 2", "bank":""},
            {"name":"My Hall", "bank":"My Presets"}
        ]));
        assert_eq!(presets[0].display_name, "Piano room 2");
        assert_eq!(presets[1].display_name, "My Presets / My Hall");
    }

    #[test]
    fn current_preset_accepts_nested_info_shapes() {
        assert_eq!(
            find_current_preset_info(
                &json!({"info":{"preset":{"name":"Warm", "bank":"Mine", "instrument":"SK-EX"}}})
            ),
            (
                Some("Mine / Warm".to_string()),
                Some("Warm".to_string()),
                Some("Mine".to_string()),
                Some("SK-EX".to_string())
            )
        );
        assert_eq!(
            find_current_preset_info(&json!([{"preset_name":"Steinway D"}])),
            (
                Some("Steinway D".to_string()),
                Some("Steinway D".to_string()),
                None,
                None
            )
        );
        assert_eq!(
            find_current_preset_info(&json!([{"current_preset":{"name":"Player", "bank":""}}])),
            (
                Some("Player".to_string()),
                Some("Player".to_string()),
                Some(String::new()),
                None
            )
        );
    }

    #[test]
    fn current_reverb_preset_accepts_pianoteq_info_shape() {
        assert_eq!(
            find_current_reverb_preset(&json!({
                "current_preset": {"mini_presets": {"reverb": "Piano room 2"}}
            })),
            Some("Piano room 2".to_string())
        );
    }

    #[test]
    fn saving_requires_a_name_and_user_bank_before_contacting_pianoteq() {
        let client = PianoteqClient::new("127.0.0.1:1".to_string());
        assert!(client.save_preset("", "My Presets").is_err());
        assert!(client.save_preset("My Sound", "").is_err());
    }

    #[test]
    fn controls_are_limited_to_the_remote_surface() {
        let controls = parse_controls(json!([
            {"id":"volume", "name":"Volume", "normalized_value":0.7, "text":"-1.0", "unit":"dB"},
            {"id":"hammer_hardness", "name":"Hammer", "normalized_value":0.5, "text":"0", "unit":""}
        ]));
        assert_eq!(controls.len(), 1);
        assert_eq!(controls[0].id, "volume");
    }
}

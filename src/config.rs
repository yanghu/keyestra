use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::midi::PortSelector;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub input: Option<PortConfig>,
    pub output: Option<PortConfig>,
    pub mapping: Option<MappingConfig>,
}

#[derive(Debug, Deserialize)]
pub struct PortConfig {
    pub name: Option<String>,
    pub index: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MappingConfig {
    #[serde(default = "default_mode")]
    pub mode: MappingMode,
    pub gamma: Option<f32>,
    pub min_out: Option<u8>,
    pub max_out: Option<u8>,
    pub velocity_table: Option<Vec<u8>>,
    pub points: Option<Vec<[u8; 2]>>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingMode {
    Curve,
    Table,
    Piecewise,
}

impl Default for MappingMode {
    fn default() -> Self {
        Self::Curve
    }
}

fn default_mode() -> MappingMode {
    MappingMode::Curve
}

impl AppConfig {
    pub fn from_path(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file {}", path.display()))?;
        toml::from_str(&text)
            .with_context(|| format!("Failed to parse config file {}", path.display()))
    }

    pub fn input_selector(&self) -> Option<PortSelector> {
        self.input.as_ref().and_then(PortConfig::selector)
    }

    pub fn output_selector(&self) -> Option<PortSelector> {
        self.output.as_ref().and_then(PortConfig::selector)
    }
}

impl PortConfig {
    fn selector(&self) -> Option<PortSelector> {
        if let Some(index) = self.index {
            Some(PortSelector::Index(index))
        } else {
            self.name.as_deref().map(PortSelector::name)
        }
    }
}

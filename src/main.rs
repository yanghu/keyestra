mod config;
mod mapping;
mod midi;
mod monitor;

use std::path::PathBuf;
use std::sync::mpsc;

use anyhow::Result;
use clap::Parser;

use crate::config::AppConfig;
use crate::mapping::VelocityMapper;
use crate::midi::{list_ports, run_forwarder, PortSelector};

#[derive(Debug, Parser)]
#[command(name = "keyestra")]
#[command(about = "Keyestra digital piano MIDI companion")]
struct Cli {
    #[arg(long, help = "List MIDI input and output ports")]
    list: bool,

    #[arg(long = "in", help = "MIDI input port name substring or index")]
    input: Option<String>,

    #[arg(long = "out", help = "MIDI output port name substring or index")]
    output: Option<String>,

    #[arg(long = "curve", help = "TOML config file path")]
    curve: Option<PathBuf>,

    #[arg(long, help = "Print mapped Note On and passthrough events")]
    monitor: bool,

    #[arg(long, help = "Forward every MIDI message unchanged")]
    bypass: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.list {
        list_ports()?;
        return Ok(());
    }

    let config = match &cli.curve {
        Some(path) => Some(AppConfig::from_path(path)?),
        None => None,
    };

    let input = cli
        .input
        .as_deref()
        .map(PortSelector::parse)
        .or_else(|| config.as_ref().and_then(|c| c.input_selector()));
    let output = cli
        .output
        .as_deref()
        .map(PortSelector::parse)
        .or_else(|| config.as_ref().and_then(|c| c.output_selector()));

    let input = input.ok_or_else(|| {
        anyhow::anyhow!("Missing input. Use --in or configure [input].name/index.")
    })?;
    let output = output.ok_or_else(|| {
        anyhow::anyhow!("Missing output. Use --out or configure [output].name/index.")
    })?;

    let mapper = if cli.bypass {
        VelocityMapper::bypass()
    } else {
        VelocityMapper::from_config(config.as_ref().and_then(|c| c.mapping.as_ref()))?
    };

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(());
    })?;

    run_forwarder(input, output, mapper, cli.monitor, shutdown_rx)
}

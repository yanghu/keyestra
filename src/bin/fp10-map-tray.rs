#![cfg_attr(windows, windows_subsystem = "windows")]

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use midir::{MidiInput, MidiOutput};
use serde::{Deserialize, Serialize};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoop};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
const STABLE_RUN_DURATION: Duration = Duration::from_secs(30);
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(name = "fp10-map-tray")]
#[command(about = "Windows tray launcher for fp10-map")]
struct Cli {
    #[arg(long = "in", default_value = "Roland Digital Piano")]
    input: String,

    #[arg(long = "out", default_value = "FP10 Mapped")]
    output: String,

    #[arg(long = "monitor-in", default_value = "FP10 Mapped")]
    monitor_input: String,

    #[arg(long = "monitor-port", default_value_t = 8770)]
    monitor_port: u16,

    #[arg(long = "monitor-host", default_value = "0.0.0.0")]
    monitor_host: String,

    #[arg(long)]
    curve: Option<PathBuf>,

    #[arg(long)]
    install_startup: bool,

    #[arg(long)]
    uninstall_startup: bool,
}

struct MapperProcess {
    child: Option<Child>,
    input: String,
    output: String,
    curve: PathBuf,
    log: Arc<Mutex<RollingLog>>,
}

struct MonitorProcess {
    child: Option<Child>,
    input: String,
    host: String,
    port: u16,
    log_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesiredState {
    Running,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeState {
    Running,
    Stopped,
    Waiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurveChoice {
    Forum,
    MidControl,
    Custom,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct TraySettings {
    curve: Option<PathBuf>,
}

struct RollingLog {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    max_bytes: u64,
}

struct Supervisor {
    mapper: MapperProcess,
    monitor: MonitorProcess,
    settings_path: PathBuf,
    mapper_desired: DesiredState,
    monitor_desired: DesiredState,
    mapper_runtime: RuntimeState,
    monitor_runtime: RuntimeState,
    mapper_last_attempt: Option<Instant>,
    mapper_started_at: Option<Instant>,
    mapper_failures: u32,
    monitor_last_attempt: Option<Instant>,
    mapper_last_error: Option<String>,
    monitor_last_error: Option<String>,
}

impl MapperProcess {
    fn new(input: String, output: String, curve: PathBuf, log: Arc<Mutex<RollingLog>>) -> Self {
        Self {
            child: None,
            input,
            output,
            curve,
            log,
        }
    }

    fn start(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }

        let mapper_exe = mapper_exe_path()?;

        let mut command = Command::new(mapper_exe);
        command
            .arg("--in")
            .arg(&self.input)
            .arg("--out")
            .arg(&self.output)
            .arg("--curve")
            .arg(&self.curve)
            .arg("--monitor")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        let mut child = command.spawn().context("Failed to start fp10-map.exe")?;
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, Arc::clone(&self.log));
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, Arc::clone(&self.log));
        }
        self.child = Some(child);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn set_curve(&mut self, curve: PathBuf) {
        self.curve = curve;
    }

    fn probe_ports(&self) -> std::result::Result<(), String> {
        probe_midi_ports(&self.input, &self.output)
    }

    fn log_status(&self, message: &str) {
        if let Ok(mut log) = self.log.lock() {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs();
            let _ = log.write_line(format!("[{}] {}\n", timestamp, message).as_bytes());
        }
    }

    fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) | Err(_) => {
                    self.child = None;
                    false
                }
                Ok(None) => true,
            },
            None => false,
        }
    }
}

impl MonitorProcess {
    fn new(input: String, host: String, port: u16, log_path: PathBuf) -> Self {
        Self {
            child: None,
            input,
            host,
            port,
            log_path,
        }
    }

    fn start(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }

        let monitor_exe = monitor_exe_path()?;
        let stdout = append_log_file(&self.log_path)?;
        let stderr = stdout.try_clone()?;

        let mut command = Command::new(monitor_exe);
        command
            .arg("--in")
            .arg(&self.input)
            .arg("--host")
            .arg(&self.host)
            .arg("--port")
            .arg(self.port.to_string())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));

        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        self.child = Some(
            command
                .spawn()
                .context("Failed to start fp10-monitor-server.exe")?,
        );
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) | Err(_) => {
                    self.child = None;
                    false
                }
                Ok(None) => true,
            },
            None => false,
        }
    }
}

impl RollingLog {
    fn new(path: PathBuf, max_bytes: u64) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let bytes = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let mut log = Self {
            path,
            file: None,
            bytes,
            max_bytes,
        };
        if log.bytes >= log.max_bytes {
            log.rotate()?;
        } else {
            log.open()?;
        }
        Ok(log)
    }

    fn write_line(&mut self, bytes: &[u8]) -> Result<()> {
        let bytes = if bytes.len() as u64 > self.max_bytes {
            &bytes[bytes.len() - self.max_bytes as usize..]
        } else {
            bytes
        };
        if self.bytes.saturating_add(bytes.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        if self.file.is_none() {
            self.open()?;
        }
        if let Some(file) = self.file.as_mut() {
            file.write_all(bytes)?;
            file.flush()?;
        }
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        Ok(())
    }

    fn open(&mut self) -> Result<()> {
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .with_context(|| format!("Failed to open log file {}", self.path.display()))?,
        );
        Ok(())
    }

    fn rotate(&mut self) -> Result<()> {
        self.file.take();
        let backup = self.path.with_extension("log.1");
        if backup.exists() {
            fs::remove_file(&backup)
                .with_context(|| format!("Failed to remove old log {}", backup.display()))?;
        }
        if self.path.exists() {
            fs::rename(&self.path, &backup).with_context(|| {
                format!(
                    "Failed to rotate log {} to {}",
                    self.path.display(),
                    backup.display()
                )
            })?;
        }
        self.bytes = 0;
        self.open()
    }
}

impl Supervisor {
    fn new(mapper: MapperProcess, monitor: MonitorProcess, settings_path: PathBuf) -> Self {
        Self {
            mapper,
            monitor,
            settings_path,
            mapper_desired: DesiredState::Running,
            monitor_desired: DesiredState::Running,
            mapper_runtime: RuntimeState::Stopped,
            monitor_runtime: RuntimeState::Stopped,
            mapper_last_attempt: None,
            mapper_started_at: None,
            mapper_failures: 0,
            monitor_last_attempt: None,
            mapper_last_error: None,
            monitor_last_error: None,
        }
    }

    fn tick(&mut self) {
        self.tick_mapper();
        self.tick_monitor();
    }

    fn tick_mapper(&mut self) {
        if self.mapper_desired == DesiredState::Stopped {
            self.set_mapper_state(RuntimeState::Stopped, None);
            return;
        }

        if let Err(reason) = self.mapper.probe_ports() {
            self.mapper.stop();
            self.mapper_started_at = None;
            self.mapper_last_attempt = None;
            self.mapper_failures = 0;
            self.set_mapper_state(RuntimeState::Waiting, Some(reason));
            return;
        }

        if self.mapper.is_running() {
            if self
                .mapper_started_at
                .map(|started| started.elapsed() >= STABLE_RUN_DURATION)
                .unwrap_or(false)
            {
                self.mapper_failures = 0;
            }
            self.set_mapper_state(RuntimeState::Running, None);
            return;
        }

        if self.mapper_started_at.take().is_some() {
            self.mapper_failures = self.mapper_failures.saturating_add(1);
            self.set_mapper_state(
                RuntimeState::Waiting,
                Some("Mapper exited; waiting to retry".to_string()),
            );
        }

        let delay = retry_delay(self.mapper_failures);
        let should_try = self
            .mapper_last_attempt
            .map(|last| last.elapsed() >= delay)
            .unwrap_or(true);
        if !should_try {
            return;
        }

        let now = Instant::now();
        self.mapper_last_attempt = Some(now);
        match self.mapper.start() {
            Ok(()) => {
                self.mapper_started_at = Some(now);
                if self.mapper.is_running() {
                    self.set_mapper_state(RuntimeState::Running, None);
                } else {
                    self.mapper_started_at = None;
                    self.mapper_failures = self.mapper_failures.saturating_add(1);
                    self.set_mapper_state(
                        RuntimeState::Waiting,
                        Some("Mapper exited immediately; waiting to retry".to_string()),
                    );
                }
            }
            Err(error) => {
                self.mapper_failures = self.mapper_failures.saturating_add(1);
                self.set_mapper_state(RuntimeState::Waiting, Some(error.to_string()));
            }
        }
    }

    fn set_mapper_state(&mut self, runtime: RuntimeState, error: Option<String>) {
        let changed = self.mapper_runtime != runtime || self.mapper_last_error != error;
        self.mapper_runtime = runtime;
        self.mapper_last_error = error;
        if changed {
            self.mapper
                .log_status(&self.mapper_status_text().replace("Mapper: ", "Mapper "));
        }
    }

    fn tick_monitor(&mut self) {
        if self.monitor.is_running() {
            self.monitor_runtime = RuntimeState::Running;
            self.monitor_last_error = None;
            return;
        }

        if self.monitor_desired == DesiredState::Stopped {
            self.monitor_runtime = RuntimeState::Stopped;
            return;
        }

        self.monitor_runtime = RuntimeState::Waiting;
        let should_try = self
            .monitor_last_attempt
            .map(|last| last.elapsed() >= Duration::from_secs(5))
            .unwrap_or(true);

        if should_try {
            self.monitor_last_attempt = Some(Instant::now());
            match self.monitor.start() {
                Ok(()) => {
                    if self.monitor.is_running() {
                        self.monitor_runtime = RuntimeState::Running;
                        self.monitor_last_error = None;
                    }
                }
                Err(error) => {
                    self.monitor_last_error = Some(error.to_string());
                }
            }
        }
    }

    fn start_mapper(&mut self) {
        self.mapper_desired = DesiredState::Running;
        self.mapper_last_attempt = None;
        self.mapper_started_at = None;
        self.mapper_failures = 0;
        self.tick_mapper();
    }

    fn stop_mapper(&mut self) {
        self.mapper_desired = DesiredState::Stopped;
        self.mapper.stop();
        self.mapper_last_attempt = None;
        self.mapper_started_at = None;
        self.mapper_failures = 0;
        self.set_mapper_state(RuntimeState::Stopped, None);
    }

    fn restart_mapper(&mut self) {
        self.mapper_desired = DesiredState::Running;
        self.mapper.stop();
        self.mapper_last_attempt = None;
        self.mapper_started_at = None;
        self.mapper_failures = 0;
        self.tick_mapper();
    }

    fn select_curve(&mut self, curve: PathBuf) {
        self.mapper.set_curve(curve);
        let _ = save_tray_settings(&self.settings_path, &self.mapper.curve);
        self.restart_mapper();
    }

    fn start_monitor(&mut self) {
        self.monitor_desired = DesiredState::Running;
        self.monitor_last_attempt = None;
        self.tick_monitor();
    }

    fn stop_monitor(&mut self) {
        self.monitor_desired = DesiredState::Stopped;
        self.monitor.stop();
        self.monitor_runtime = RuntimeState::Stopped;
        self.monitor_last_error = None;
    }

    fn restart_monitor(&mut self) {
        self.monitor_desired = DesiredState::Running;
        self.monitor.stop();
        self.monitor_last_attempt = None;
        self.tick_monitor();
    }

    fn restart_all(&mut self) {
        self.restart_mapper();
        self.restart_monitor();
    }

    fn stop_all(&mut self) {
        self.stop_mapper();
        self.stop_monitor();
    }

    fn mapper_status_text(&self) -> String {
        status_text(
            "Mapper",
            self.mapper_runtime,
            self.mapper_last_error.as_deref(),
        )
    }

    fn monitor_status_text(&self) -> String {
        status_text(
            "Monitor",
            self.monitor_runtime,
            self.monitor_last_error.as_deref(),
        )
    }

    fn tooltip_text(&self) -> String {
        format!(
            "FP-10 tools - mapper: {}, monitor: {}, curve: {}",
            runtime_label(self.mapper_runtime),
            runtime_label(self.monitor_runtime),
            self.curve_label()
        )
    }

    fn curve_choice(&self) -> CurveChoice {
        curve_choice(&self.mapper.curve)
    }

    fn curve_label(&self) -> &'static str {
        curve_label(self.curve_choice())
    }

    fn monitor_url(&self) -> String {
        format!("http://localhost:{}/", self.monitor.port)
    }
}

fn retry_delay(failures: u32) -> Duration {
    if failures == 0 {
        return Duration::ZERO;
    }

    let exponent = failures.saturating_sub(1).min(4);
    INITIAL_RETRY_DELAY
        .saturating_mul(1_u32 << exponent)
        .min(MAX_RETRY_DELAY)
}

fn probe_midi_ports(
    input_selector: &str,
    output_selector: &str,
) -> std::result::Result<(), String> {
    let midi_in = MidiInput::new("fp10-map-tray-probe-input").map_err(|error| error.to_string())?;
    let input_names = midi_in
        .ports()
        .iter()
        .map(|port| midi_in.port_name(port).ok())
        .collect::<Vec<_>>();
    if !selector_available(input_selector, &input_names) {
        return Err(format!("input '{}' is not available", input_selector));
    }

    let midi_out =
        MidiOutput::new("fp10-map-tray-probe-output").map_err(|error| error.to_string())?;
    let output_names = midi_out
        .ports()
        .iter()
        .map(|port| midi_out.port_name(port).ok())
        .collect::<Vec<_>>();
    if !selector_available(output_selector, &output_names) {
        return Err(format!("output '{}' is not available", output_selector));
    }

    Ok(())
}

fn selector_available(selector: &str, names: &[Option<String>]) -> bool {
    match selector.parse::<usize>() {
        Ok(index) => index < names.len(),
        Err(_) => {
            let selector = selector.to_lowercase();
            names
                .iter()
                .flatten()
                .any(|name| name.to_lowercase().contains(selector.as_str()))
        }
    }
}

fn spawn_log_reader<R>(reader: R, log: Arc<Mutex<RollingLog>>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(mut log) = log.lock() {
                        let _ = log.write_line(&line);
                    }
                }
            }
        }
    });
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.install_startup {
        install_startup()?;
        return Ok(());
    }

    if cli.uninstall_startup {
        uninstall_startup()?;
        return Ok(());
    }

    let app_data = app_data_dir()?;
    let mapper_log_path = app_data.join("tray.log");
    let monitor_log_path = app_data.join("monitor.log");
    let settings_path = app_data.join("tray-settings.toml");
    let mapper_log = Arc::new(Mutex::new(RollingLog::new(mapper_log_path, MAX_LOG_BYTES)?));
    let curve = cli
        .curve
        .or_else(|| {
            load_tray_settings(&settings_path)
                .ok()
                .and_then(|settings| settings.curve)
        })
        .unwrap_or_else(default_curve_path);
    let mapper = MapperProcess::new(cli.input, cli.output, curve, mapper_log);
    let monitor = MonitorProcess::new(
        cli.monitor_input,
        cli.monitor_host,
        cli.monitor_port,
        monitor_log_path,
    );

    run_tray(Supervisor::new(mapper, monitor, settings_path))
}

fn run_tray(mut supervisor: Supervisor) -> Result<()> {
    let event_loop = EventLoop::new();
    let tray_menu = Menu::new();

    let mapper_status_item = MenuItem::new("Mapper: Starting", false, None);
    let monitor_status_item = MenuItem::new("Monitor: Starting", false, None);
    let open_monitor_item = MenuItem::new("Open monitor page", true, None);
    let restart_all_item = MenuItem::new("Restart all", true, None);
    let start_item = MenuItem::new("Start mapper", true, None);
    let stop_item = MenuItem::new("Stop mapper", true, None);
    let restart_item = MenuItem::new("Restart mapper", true, None);
    let curve_status_item =
        MenuItem::new(format!("Curve: {}", supervisor.curve_label()), false, None);
    let forum_curve_item = CheckMenuItem::new("Forum curve", true, false, None);
    let mid_control_curve_item = CheckMenuItem::new("Mid-control curve", true, false, None);
    let custom_curve_item = CheckMenuItem::new("Custom curve", false, false, None);
    let start_monitor_item = MenuItem::new("Start monitor", true, None);
    let stop_monitor_item = MenuItem::new("Stop monitor", true, None);
    let restart_monitor_item = MenuItem::new("Restart monitor", true, None);
    let install_item = MenuItem::new("Install startup", true, None);
    let uninstall_item = MenuItem::new("Uninstall startup", true, None);
    let quit_item = MenuItem::new("Exit", true, None);

    tray_menu.append_items(&[
        &mapper_status_item,
        &monitor_status_item,
        &open_monitor_item,
        &restart_all_item,
        &PredefinedMenuItem::separator(),
        &start_item,
        &stop_item,
        &restart_item,
        &PredefinedMenuItem::separator(),
        &curve_status_item,
        &forum_curve_item,
        &mid_control_curve_item,
        &custom_curve_item,
        &PredefinedMenuItem::separator(),
        &start_monitor_item,
        &stop_monitor_item,
        &restart_monitor_item,
        &PredefinedMenuItem::separator(),
        &install_item,
        &uninstall_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ])?;

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("FP-10 mapper")
        .with_icon(tray_icon())
        .build()?;

    let menu_rx = MenuEvent::receiver();
    let mut last_mapper_status = String::new();
    let mut last_monitor_status = String::new();
    let mut last_curve_choice = CurveChoice::Custom;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1));

        if matches!(
            event,
            Event::NewEvents(StartCause::Init)
                | Event::NewEvents(StartCause::ResumeTimeReached { .. })
        ) {
            let _keep_alive = &tray_icon;
            supervisor.tick();
            let mapper_status = supervisor.mapper_status_text();
            let monitor_status = supervisor.monitor_status_text();
            let curve_choice = supervisor.curve_choice();
            if mapper_status != last_mapper_status
                || monitor_status != last_monitor_status
                || curve_choice != last_curve_choice
            {
                mapper_status_item.set_text(&mapper_status);
                monitor_status_item.set_text(&monitor_status);
                curve_status_item.set_text(format!("Curve: {}", curve_label(curve_choice)));
                forum_curve_item.set_checked(curve_choice == CurveChoice::Forum);
                mid_control_curve_item.set_checked(curve_choice == CurveChoice::MidControl);
                custom_curve_item.set_checked(curve_choice == CurveChoice::Custom);
                let _ = tray_icon.set_tooltip(Some(supervisor.tooltip_text()));
                last_mapper_status = mapper_status;
                last_monitor_status = monitor_status;
                last_curve_choice = curve_choice;
            }
        }

        while let Ok(event) = menu_rx.try_recv() {
            if event.id == start_item.id() {
                supervisor.start_mapper();
            } else if event.id == stop_item.id() {
                supervisor.stop_mapper();
            } else if event.id == restart_item.id() {
                supervisor.restart_mapper();
            } else if event.id == forum_curve_item.id() {
                supervisor.select_curve(default_curve_path());
            } else if event.id == mid_control_curve_item.id() {
                supervisor.select_curve(mid_control_curve_path());
            } else if event.id == start_monitor_item.id() {
                supervisor.start_monitor();
            } else if event.id == stop_monitor_item.id() {
                supervisor.stop_monitor();
            } else if event.id == restart_monitor_item.id() {
                supervisor.restart_monitor();
            } else if event.id == restart_all_item.id() {
                supervisor.restart_all();
            } else if event.id == open_monitor_item.id() {
                let _ = open_url(&supervisor.monitor_url());
            } else if event.id == install_item.id() {
                let _ = install_startup();
            } else if event.id == uninstall_item.id() {
                let _ = uninstall_startup();
            } else if event.id == quit_item.id() {
                supervisor.stop_all();
                *control_flow = ControlFlow::Exit;
            }
        }
    });
}

fn status_text(name: &str, runtime: RuntimeState, last_error: Option<&str>) -> String {
    match runtime {
        RuntimeState::Running => format!("{}: Running", name),
        RuntimeState::Stopped => format!("{}: Stopped", name),
        RuntimeState::Waiting => match last_error {
            Some(error) => format!("{}: Waiting - {}", name, first_line(error)),
            None => format!("{}: Waiting for MIDI ports", name),
        },
    }
}

fn runtime_label(runtime: RuntimeState) -> &'static str {
    match runtime {
        RuntimeState::Running => "running",
        RuntimeState::Stopped => "stopped",
        RuntimeState::Waiting => "waiting",
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

fn mapper_exe_path() -> Result<PathBuf> {
    let exe = env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Could not find tray exe directory"))?;
    Ok(dir.join("fp10-map.exe"))
}

fn monitor_exe_path() -> Result<PathBuf> {
    let exe = env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Could not find tray exe directory"))?;
    Ok(dir.join("fp10-monitor-server.exe"))
}

fn open_url(url: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command.creation_flags(CREATE_NO_WINDOW);
        command.spawn()?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn()?;
        return Ok(());
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(url).spawn()?;
        return Ok(());
    }
}

fn default_curve_path() -> PathBuf {
    curve_path("curve.toml")
}

fn mid_control_curve_path() -> PathBuf {
    curve_path("curve-mid-control.toml")
}

fn curve_path(file_name: &str) -> PathBuf {
    let exe = env::current_exe().ok();
    let Some(exe) = exe else {
        return PathBuf::from("examples").join(file_name);
    };
    let Some(dir) = exe.parent() else {
        return PathBuf::from("examples").join(file_name);
    };

    let beside_exe = dir.join("examples").join(file_name);
    if beside_exe.exists() {
        return beside_exe;
    }

    let workspace_example = dir.join("..").join("..").join("examples").join(file_name);
    if workspace_example.exists() {
        return workspace_example;
    }

    PathBuf::from("examples").join(file_name)
}

fn curve_choice(path: &Path) -> CurveChoice {
    if same_path(path, &default_curve_path()) {
        CurveChoice::Forum
    } else if same_path(path, &mid_control_curve_path()) {
        CurveChoice::MidControl
    } else {
        CurveChoice::Custom
    }
}

fn curve_label(choice: CurveChoice) -> &'static str {
    match choice {
        CurveChoice::Forum => "Forum",
        CurveChoice::MidControl => "Mid-control",
        CurveChoice::Custom => "Custom",
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

fn app_data_dir() -> Result<PathBuf> {
    let appdata = env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?;
    let dir = PathBuf::from(appdata).join("fp10-map");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn load_tray_settings(path: &Path) -> Result<TraySettings> {
    if !path.exists() {
        return Ok(TraySettings::default());
    }

    let text = fs::read_to_string(path)
        .with_context(|| format!("Failed to read tray settings {}", path.display()))?;
    toml::from_str(&text)
        .with_context(|| format!("Failed to parse tray settings {}", path.display()))
}

fn save_tray_settings(path: &Path, curve: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let settings = TraySettings {
        curve: Some(curve.to_path_buf()),
    };
    let text = toml::to_string_pretty(&settings)?;
    fs::write(path, text)
        .with_context(|| format!("Failed to write tray settings {}", path.display()))
}

fn startup_script_path() -> Result<PathBuf> {
    let appdata = env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?;
    Ok(PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("fp10-map-tray.vbs"))
}

fn install_startup() -> Result<()> {
    let exe = env::current_exe()?;
    let script_path = startup_script_path()?;
    let script = format!(
        "Set shell = CreateObject(\"WScript.Shell\")\r\nshell.Run \"\"\"{}\"\"\", 0, False\r\n",
        exe.display()
    );
    fs::write(script_path, script)?;
    Ok(())
}

fn uninstall_startup() -> Result<()> {
    let script_path = startup_script_path()?;
    if script_path.exists() {
        fs::remove_file(script_path)?;
    }
    Ok(())
}

fn append_log_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open log file {}", path.display()))
}

fn tray_icon() -> Icon {
    let size = 32;
    let mut rgba = Vec::with_capacity(size * size * 4);

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let distance = (dx * dx + dy * dy).sqrt();
            let inside = distance < 14.0;
            let highlight = x > 9 && x < 23 && y > 7 && y < 24;

            let (r, g, b, a) = if !inside {
                (0, 0, 0, 0)
            } else if highlight {
                (245, 255, 250, 255)
            } else {
                (20, 108, 108, 255)
            };

            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }

    Icon::from_rgba(rgba, size as u32, size as u32).expect("valid tray icon")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_backs_off_and_caps_at_one_minute() {
        let seconds = (0..=7)
            .map(|failures| retry_delay(failures).as_secs())
            .collect::<Vec<_>>();
        assert_eq!(seconds, vec![0, 5, 10, 20, 40, 60, 60, 60]);
    }

    #[test]
    fn selector_accepts_index_or_case_insensitive_substring() {
        let names = vec![
            Some("FP10 Mapped".to_string()),
            Some("Roland Digital Piano".to_string()),
        ];

        assert!(selector_available("0", &names));
        assert!(selector_available("roland DIGITAL", &names));
        assert!(!selector_available("2", &names));
        assert!(!selector_available("missing", &names));
    }

    #[test]
    fn unreadable_name_still_counts_for_numeric_selection() {
        let names = vec![None];

        assert!(selector_available("0", &names));
        assert!(!selector_available("Roland", &names));
    }

    #[test]
    fn rolling_log_keeps_one_bounded_backup() {
        let directory =
            env::temp_dir().join(format!("fp10-map-tray-log-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("tray.log");
        let backup = directory.join("tray.log.1");

        {
            let mut log = RollingLog::new(path.clone(), 10).unwrap();
            log.write_line(b"first\n").unwrap();
            log.write_line(b"second\n").unwrap();
        }

        assert_eq!(fs::read(&path).unwrap(), b"second\n");
        assert_eq!(fs::read(&backup).unwrap(), b"first\n");
        fs::remove_file(path).unwrap();
        fs::remove_file(backup).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}

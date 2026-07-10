#![cfg_attr(windows, windows_subsystem = "windows")]

use std::env;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoop};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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
    log_path: PathBuf,
}

struct MonitorProcess {
    child: Option<Child>,
    input: String,
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

struct Supervisor {
    mapper: MapperProcess,
    monitor: MonitorProcess,
    mapper_desired: DesiredState,
    monitor_desired: DesiredState,
    mapper_runtime: RuntimeState,
    monitor_runtime: RuntimeState,
    mapper_last_attempt: Option<Instant>,
    monitor_last_attempt: Option<Instant>,
    mapper_last_error: Option<String>,
    monitor_last_error: Option<String>,
}

impl MapperProcess {
    fn new(input: String, output: String, curve: PathBuf, log_path: PathBuf) -> Self {
        Self {
            child: None,
            input,
            output,
            curve,
            log_path,
        }
    }

    fn start(&mut self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }

        let mapper_exe = mapper_exe_path()?;
        let stdout = append_log_file(&self.log_path)?;
        let stderr = stdout.try_clone()?;

        let mut command = Command::new(mapper_exe);
        command
            .arg("--in")
            .arg(&self.input)
            .arg("--out")
            .arg(&self.output)
            .arg("--curve")
            .arg(&self.curve)
            .arg("--monitor")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));

        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);

        self.child = Some(command.spawn().context("Failed to start fp10-map.exe")?);
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

impl MonitorProcess {
    fn new(input: String, port: u16, log_path: PathBuf) -> Self {
        Self {
            child: None,
            input,
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

impl Supervisor {
    fn new(mapper: MapperProcess, monitor: MonitorProcess) -> Self {
        Self {
            mapper,
            monitor,
            mapper_desired: DesiredState::Running,
            monitor_desired: DesiredState::Running,
            mapper_runtime: RuntimeState::Stopped,
            monitor_runtime: RuntimeState::Stopped,
            mapper_last_attempt: None,
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
        if self.mapper.is_running() {
            self.mapper_runtime = RuntimeState::Running;
            self.mapper_last_error = None;
            return;
        }

        if self.mapper_desired == DesiredState::Stopped {
            self.mapper_runtime = RuntimeState::Stopped;
            return;
        }

        self.mapper_runtime = RuntimeState::Waiting;
        let should_try = self
            .mapper_last_attempt
            .map(|last| last.elapsed() >= Duration::from_secs(5))
            .unwrap_or(true);

        if should_try {
            self.mapper_last_attempt = Some(Instant::now());
            match self.mapper.start() {
                Ok(()) => {
                    if self.mapper.is_running() {
                        self.mapper_runtime = RuntimeState::Running;
                        self.mapper_last_error = None;
                    }
                }
                Err(error) => {
                    self.mapper_last_error = Some(error.to_string());
                }
            }
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
        self.tick_mapper();
    }

    fn stop_mapper(&mut self) {
        self.mapper_desired = DesiredState::Stopped;
        self.mapper.stop();
        self.mapper_runtime = RuntimeState::Stopped;
        self.mapper_last_error = None;
    }

    fn restart_mapper(&mut self) {
        self.mapper_desired = DesiredState::Running;
        self.mapper.stop();
        self.mapper_last_attempt = None;
        self.tick_mapper();
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
        status_text("Mapper", self.mapper_runtime, self.mapper_last_error.as_deref())
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
            "FP-10 tools - mapper: {}, monitor: {}",
            runtime_label(self.mapper_runtime),
            runtime_label(self.monitor_runtime)
        )
    }

    fn monitor_url(&self) -> String {
        format!("http://localhost:{}/", self.monitor.port)
    }
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
    let curve = cli.curve.unwrap_or_else(default_curve_path);
    let mapper = MapperProcess::new(cli.input, cli.output, curve, mapper_log_path);
    let monitor = MonitorProcess::new(cli.monitor_input, cli.monitor_port, monitor_log_path);

    run_tray(Supervisor::new(mapper, monitor))
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
    let start_monitor_item = MenuItem::new("Start monitor", true, None);
    let stop_monitor_item = MenuItem::new("Stop monitor", true, None);
    let restart_monitor_item = MenuItem::new("Restart monitor", true, None);
    let install_item = MenuItem::new("Install startup", true, None);
    let uninstall_item = MenuItem::new("Uninstall startup", true, None);
    let quit_item = MenuItem::new("Exit", true, None);

    tray_menu.append_items(&[
        &mapper_status_item,
        &monitor_status_item,
        &PredefinedMenuItem::separator(),
        &open_monitor_item,
        &restart_all_item,
        &PredefinedMenuItem::separator(),
        &start_item,
        &stop_item,
        &restart_item,
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

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1));

        if matches!(
            event,
            Event::NewEvents(StartCause::Init) | Event::NewEvents(StartCause::ResumeTimeReached { .. })
        ) {
            let _keep_alive = &tray_icon;
            supervisor.tick();
            let mapper_status = supervisor.mapper_status_text();
            let monitor_status = supervisor.monitor_status_text();
            if mapper_status != last_mapper_status || monitor_status != last_monitor_status {
                mapper_status_item.set_text(&mapper_status);
                monitor_status_item.set_text(&monitor_status);
                let _ = tray_icon.set_tooltip(Some(supervisor.tooltip_text()));
                last_mapper_status = mapper_status;
                last_monitor_status = monitor_status;
            }
        }

        while let Ok(event) = menu_rx.try_recv() {
            if event.id == start_item.id() {
                supervisor.start_mapper();
            } else if event.id == stop_item.id() {
                supervisor.stop_mapper();
            } else if event.id == restart_item.id() {
                supervisor.restart_mapper();
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
    let exe = env::current_exe().ok();
    let Some(exe) = exe else {
        return PathBuf::from("examples/curve.toml");
    };
    let Some(dir) = exe.parent() else {
        return PathBuf::from("examples/curve.toml");
    };

    let beside_exe = dir.join("examples").join("curve.toml");
    if beside_exe.exists() {
        return beside_exe;
    }

    let workspace_example = dir.join("..").join("..").join("examples").join("curve.toml");
    if workspace_example.exists() {
        return workspace_example;
    }

    PathBuf::from("examples/curve.toml")
}

fn app_data_dir() -> Result<PathBuf> {
    let appdata = env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?;
    let dir = PathBuf::from(appdata).join("fp10-map");
    fs::create_dir_all(&dir)?;
    Ok(dir)
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

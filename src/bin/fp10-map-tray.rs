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
    desired: DesiredState,
    runtime: RuntimeState,
    last_attempt: Option<Instant>,
    last_error: Option<String>,
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

impl Supervisor {
    fn new(mapper: MapperProcess) -> Self {
        Self {
            mapper,
            desired: DesiredState::Running,
            runtime: RuntimeState::Stopped,
            last_attempt: None,
            last_error: None,
        }
    }

    fn tick(&mut self) {
        if self.mapper.is_running() {
            self.runtime = RuntimeState::Running;
            self.last_error = None;
            return;
        }

        if self.desired == DesiredState::Stopped {
            self.runtime = RuntimeState::Stopped;
            return;
        }

        self.runtime = RuntimeState::Waiting;
        let should_try = self
            .last_attempt
            .map(|last| last.elapsed() >= Duration::from_secs(5))
            .unwrap_or(true);

        if should_try {
            self.last_attempt = Some(Instant::now());
            match self.mapper.start() {
                Ok(()) => {
                    if self.mapper.is_running() {
                        self.runtime = RuntimeState::Running;
                        self.last_error = None;
                    }
                }
                Err(error) => {
                    self.last_error = Some(error.to_string());
                }
            }
        }
    }

    fn start(&mut self) {
        self.desired = DesiredState::Running;
        self.last_attempt = None;
        self.tick();
    }

    fn stop(&mut self) {
        self.desired = DesiredState::Stopped;
        self.mapper.stop();
        self.runtime = RuntimeState::Stopped;
        self.last_error = None;
    }

    fn restart(&mut self) {
        self.desired = DesiredState::Running;
        self.mapper.stop();
        self.last_attempt = None;
        self.tick();
    }

    fn status_text(&self) -> String {
        match self.runtime {
            RuntimeState::Running => "Status: Running".to_string(),
            RuntimeState::Stopped => "Status: Stopped".to_string(),
            RuntimeState::Waiting => match &self.last_error {
                Some(error) => format!("Status: Waiting - {}", first_line(error)),
                None => "Status: Waiting for MIDI ports".to_string(),
            },
        }
    }

    fn tooltip_text(&self) -> String {
        format!("FP-10 mapper - {}", self.status_text().trim_start_matches("Status: "))
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

    let log_path = app_data_dir()?.join("tray.log");
    let curve = cli.curve.unwrap_or_else(default_curve_path);
    let mapper = MapperProcess::new(cli.input, cli.output, curve, log_path);

    run_tray(Supervisor::new(mapper))
}

fn run_tray(mut supervisor: Supervisor) -> Result<()> {
    let event_loop = EventLoop::new();
    let tray_menu = Menu::new();

    let status_item = MenuItem::new("Status: Starting", false, None);
    let start_item = MenuItem::new("Start mapper", true, None);
    let stop_item = MenuItem::new("Stop mapper", true, None);
    let restart_item = MenuItem::new("Restart mapper", true, None);
    let install_item = MenuItem::new("Install startup", true, None);
    let uninstall_item = MenuItem::new("Uninstall startup", true, None);
    let quit_item = MenuItem::new("Exit", true, None);

    tray_menu.append_items(&[
        &status_item,
        &PredefinedMenuItem::separator(),
        &start_item,
        &stop_item,
        &restart_item,
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
    let mut last_status = String::new();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1));

        if matches!(
            event,
            Event::NewEvents(StartCause::Init) | Event::NewEvents(StartCause::ResumeTimeReached { .. })
        ) {
            let _keep_alive = &tray_icon;
            supervisor.tick();
            let status = supervisor.status_text();
            if status != last_status {
                status_item.set_text(&status);
                let _ = tray_icon.set_tooltip(Some(supervisor.tooltip_text()));
                last_status = status;
            }
        }

        while let Ok(event) = menu_rx.try_recv() {
            if event.id == start_item.id() {
                supervisor.start();
            } else if event.id == stop_item.id() {
                supervisor.stop();
            } else if event.id == restart_item.id() {
                supervisor.restart();
            } else if event.id == install_item.id() {
                let _ = install_startup();
            } else if event.id == uninstall_item.id() {
                let _ = uninstall_startup();
            } else if event.id == quit_item.id() {
                supervisor.stop();
                *control_flow = ControlFlow::Exit;
            }
        }
    });
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

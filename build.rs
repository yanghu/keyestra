use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=assets/icons/keyestra.ico");
    println!("cargo:rerun-if-env-changed=KEYESTRA_BUILD_ID_OVERRIDE");
    if let Ok(head) = fs::read_to_string(".git/HEAD") {
        if let Some(reference) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=.git/{reference}");
        }
    }

    let build_id = std::env::var("KEYESTRA_BUILD_ID_OVERRIDE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(git_build_id);
    println!("cargo:rustc-env=KEYESTRA_BUILD_ID={build_id}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/keyestra.ico")
            .compile()
            .expect("failed to embed the Keyestra Windows icon");
    }
}

fn git_build_id() -> String {
    let Some(revision) = git_stdout(&["rev-parse", "--short=8", "HEAD"]) else {
        return format!("snapshot-{}", unix_timestamp());
    };
    match git_stdout(&["status", "--porcelain"]) {
        Some(status) if status.is_empty() => revision,
        Some(_) => format!("{revision}-dirty"),
        None => format!("{revision}-state-unknown"),
    }
}

fn git_stdout(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

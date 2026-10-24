//! Best-effort Unix support (Foreground/title discovery uses X11 or macOS
//! scripting when available and degrades to priority-only detection otherwise).

use super::{SysSnapshot, WindowInfo};
use std::process::Command;
use std::sync::OnceLock;

pub fn snapshot() -> SysSnapshot {
    let procs = list_procs();
    let (foreground_pid, window) = foreground();

    let windows = if let Some((pid, title)) = window.filter(|(_, t)| !t.trim().is_empty()) {
        vec![WindowInfo {
            pid,
            title,
            fullscreen: false,
        }]
    } else {
        Vec::new()
    };

    SysSnapshot {
        procs,
        windows,
        foreground_pid,
        idle_secs: idle_seconds(),
        has_foreground: foreground_pid != 0,
    }
}

fn in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn have_xdotool() -> bool {
    static HAVE: OnceLock<bool> = OnceLock::new();
    *HAVE.get_or_init(|| in_path("xdotool"))
}

#[cfg(target_os = "linux")]
fn foreground() -> (u32, Option<(u32, String)>) {
    if !have_xdotool() {
        return (0, None);
    }
    let pid = Command::new("xdotool")
        .args(["getactivewindow", "getwindowpid"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok())
        .unwrap_or(0);
    let title = Command::new("xdotool")
        .args(["getactivewindow", "getwindowname"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if pid == 0 {
        (0, None)
    } else {
        (pid, Some((pid, title)))
    }
}

#[cfg(target_os = "linux")]
fn list_procs() -> Vec<(u32, String)> {
    let mut out = Vec::with_capacity(256);
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            let exe = comm.trim();
            if !exe.is_empty() {
                out.push((pid, exe.to_string()));
            }
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn idle_seconds() -> u64 {
    static HAVE: OnceLock<bool> = OnceLock::new();
    if !*HAVE.get_or_init(|| in_path("xprintidle")) {
        return 0;
    }
    Command::new("xprintidle")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok())
        .map(|ms| ms / 1000)
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn foreground() -> (u32, Option<(u32, String)>) {
    let script = r#"tell application "System Events"
set p to first application process whose frontmost is true
set t to ""
try
set t to title of front window of p
end try
return (unix id of p as text) & "\n" & t
end tell"#;
    let out = Command::new("osascript").args(["-e", script]).output().ok();
    let Some(out) = out.filter(|o| o.status.success()) else {
        return (0, None);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.trim().splitn(2, '\n');
    let pid = lines.next().unwrap_or("").trim().parse::<u32>().unwrap_or(0);
    let title = lines.next().unwrap_or("").trim().to_string();
    if pid == 0 {
        (0, None)
    } else {
        (pid, Some((pid, title)))
    }
}

#[cfg(target_os = "macos")]
fn list_procs() -> Vec<(u32, String)> {
    let mut out = Vec::with_capacity(256);
    let Ok(output) = Command::new("ps").args(["-axo", "pid=,comm="]).output() else {
        return out;
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim_start();
        let Some((pid, path)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else { continue };
        let name = path
            .trim()
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .to_string();
        if !name.is_empty() {
            out.push((pid, name));
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn idle_seconds() -> u64 {
    0
}

pub fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

pub fn cancel_io(_file: &std::fs::File) {}

pub fn process_path(_pid: u32) -> Option<String> {
    None
}

pub fn attach_parent_console() -> bool {
    true
}

pub fn install_shutdown_handler() {}

pub fn shutdown_requested() -> bool {
    false
}

pub fn message_box(title: &str, text: &str, error: bool) {
    if error {
        eprintln!("{title}: {text}");
    } else {
        println!("{title}: {text}");
    }
}

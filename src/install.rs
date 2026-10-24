//! Native installer / uninstaller (no PowerShell, no external setup tool).
//!
//! `pulserpc --install` copies the binary beside a portable config folder,
//! registers a per-user uninstall entry and (optionally) an autostart entry.

use crate::config;
use std::path::{Path, PathBuf};
use std::process::Command;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\PulseRPC";

pub fn default_install_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local)
                .join("Programs")
                .join("PulseRPC");
        }
    }
    config::base_dir()
}

pub fn install(dir: Option<PathBuf>, autostart: bool, start: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        install_windows(dir.unwrap_or_else(default_install_dir), autostart, start)
    }
    #[cfg(unix)]
    {
        install_unix(dir, autostart, start)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (dir, autostart, start);
        Err("unsupported platform".to_string())
    }
}

pub fn uninstall(dir: Option<PathBuf>) -> Result<(), String> {
    #[cfg(windows)]
    {
        uninstall_windows(dir.unwrap_or_else(default_install_dir))
    }
    #[cfg(unix)]
    {
        uninstall_unix(dir)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = dir;
        Err("unsupported platform".to_string())
    }
}

// ───────────────────────────────── Windows ─────────────────────────────────

#[cfg(windows)]
fn install_windows(dir: PathBuf, autostart: bool, start: bool) -> Result<(), String> {
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate executable: {e}"))?;
    let target = dir.join("pulserpc.exe");

    if !same_file(&exe, &target) {
        std::fs::copy(&exe, &target)
            .map_err(|e| format!("cannot copy executable to {}: {e}", target.display()))?;
        println!("  binary      {}", target.display());
    } else {
        println!("  binary      {} (already in place)", target.display());
    }

    let config_dir = dir.join("config");
    let written = config::ensure_defaults(&config_dir)
        .map_err(|e| format!("cannot write config to {}: {e}", config_dir.display()))?;
    if written.is_empty() {
        println!("  config      {} (kept existing)", config_dir.display());
    } else {
        println!("  config      {} ({} files)", config_dir.display(), written.len());
    }

    if autostart {
        let quoted = format!("\"{}\" --tray", target.display());
        reg(&[
            "add", RUN_KEY, "/v", "PulseRPC", "/t", "REG_SZ", "/d", &quoted, "/f",
        ])?;
        println!("  autostart   HKCU\\...\\Run\\PulseRPC (starts in the tray)");
    } else {
        println!("  autostart   skipped");
    }

    let uninstall_cmd = format!("\"{}\" --uninstall", target.display());
    let icon = format!("{},0", target.display());
    let location = dir.display().to_string();
    let version = env!("CARGO_PKG_VERSION");

    let values: [(&str, &str, &str); 8] = [
        ("DisplayName", "REG_SZ", "PulseRPC"),
        ("DisplayVersion", "REG_SZ", version),
        ("Publisher", "REG_SZ", "PulseRPC"),
        ("InstallLocation", "REG_SZ", &location),
        ("DisplayIcon", "REG_SZ", &icon),
        ("UninstallString", "REG_SZ", &uninstall_cmd),
        ("NoModify", "REG_DWORD", "1"),
        ("NoRepair", "REG_DWORD", "1"),
    ];
    for (name, kind, data) in values {
        reg(&[
            "add", UNINSTALL_KEY, "/v", name, "/t", kind, "/d", data, "/f",
        ])?;
    }
    println!("  uninstall   Settings → Apps → PulseRPC");

    println!();
    println!("Installed PulseRPC {version} to {}", dir.display());
    println!(
        "Next: set presence.client_id in {}",
        config_dir.join("config.toml").display()
    );
    if start {
        Command::new(&target)
            .spawn()
            .map_err(|e| format!("installed, but could not start: {e}"))?;
        println!("Started in the background.");
    }
    Ok(())
}

#[cfg(windows)]
fn uninstall_windows(dir: PathBuf) -> Result<(), String> {
    stop_running_instance();

    let _ = reg(&["delete", RUN_KEY, "/v", "PulseRPC", "/f"]);
    let _ = reg(&["delete", UNINSTALL_KEY, "/f"]);
    println!("  removed startup + uninstall registry entries");

    let exe = std::env::current_exe().unwrap_or_default();
    let inside = exe.starts_with(&dir);
    if dir.exists() {
        let script = format!(
            "timeout /t 2 /nobreak >nul & rmdir /s /q \"{}\"",
            dir.display()
        );
        Command::new("cmd")
            .args(["/C", &script])
            .spawn()
            .map_err(|e| format!("cannot schedule removal of {}: {e}", dir.display()))?;
        println!("  removing {}", dir.display());
        if inside {
            println!("The app directory will be deleted after this process exits.");
        }
    } else {
        println!("  {} does not exist (already removed)", dir.display());
    }
    Ok(())
}

#[cfg(windows)]
fn reg(args: &[&str]) -> Result<(), String> {
    let output = Command::new("reg")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run reg.exe: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "reg.exe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

// ───────────────────────────── autostart (GUI) ─────────────────────────────

#[cfg(windows)]
pub fn autostart_enabled() -> bool {
    Command::new("reg")
        .args(["query", RUN_KEY, "/v", "PulseRPC"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if enabled {
        let exe = std::env::current_exe()
            .map_err(|e| format!("cannot locate executable: {e}"))?;
        let value = format!("\"{}\" --tray", exe.display());
        reg(&[
            "add", RUN_KEY, "/v", "PulseRPC", "/t", "REG_SZ", "/d", &value, "/f",
        ])
    } else {
        // Missing value is not an error: turning autostart off twice is fine.
        let _ = reg(&["delete", RUN_KEY, "/v", "PulseRPC", "/f"]);
        Ok(())
    }
}

#[cfg(unix)]
fn systemd_unit() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("systemd")
        .join("user")
        .join("pulserpc.service")
}

#[cfg(unix)]
pub fn autostart_enabled() -> bool {
    // The unit existing is the closest equivalent to "enabled" on Unix.
    systemd_unit().exists()
}

#[cfg(unix)]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let unit = systemd_unit();
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if let Some(parent) = unit.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = format!(
            "[Unit]\nDescription=PulseRPC Discord Rich Presence\n\n[Service]\nExecStart={}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
            exe.display()
        );
        std::fs::write(&unit, text).map_err(|e| format!("systemd unit: {e}"))?;
    } else {
        let _ = std::fs::remove_file(&unit);
    }
    Ok(())
}

#[cfg(not(any(windows, unix)))]
pub fn autostart_enabled() -> bool {
    false
}

#[cfg(not(any(windows, unix)))]
pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    Err("autostart is not supported on this platform".to_string())
}

/// Open a folder or file with the system handler (Explorer / xdg-open).
pub fn open_in_shell(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &std::ffi::OsStr| -> Vec<u16> {
            use std::os::windows::ffi::OsStrExt;
            s.encode_wide().chain(std::iter::once(0)).collect()
        };
        let operation = wide(std::ffi::OsStr::new("open"));
        let file = wide(path.as_os_str());
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize > 32 {
            Ok(())
        } else {
            Err(format!("cannot open {}", path.display()))
        }
    }
    #[cfg(unix)]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("cannot open {}: {e}", path.display()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        Err("opening folders is not supported on this platform".to_string())
    }
}

#[cfg(windows)]
fn stop_running_instance() {
    let lock = config::lock_path();
    let Ok(text) = std::fs::read_to_string(&lock) else {
        return;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        return;
    };
    if pid == std::process::id() || !crate::sys::process_is_alive(pid) {
        return;
    }
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
    println!("  stopped running instance (pid {pid})");
}

#[cfg(windows)]
fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

// ───────────────────────────────── Unix ─────────────────────────────────

#[cfg(unix)]
fn install_unix(dir: Option<PathBuf>, autostart: bool, start: bool) -> Result<(), String> {
    let dir = dir.unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("bin")
    });
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = dir.join("pulserpc");
    if exe != target {
        std::fs::copy(&exe, &target).map_err(|e| format!("copy failed: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
        }
    }
    let config_dir = config::config_dir();
    config::ensure_defaults(&config_dir).map_err(|e| format!("config: {e}"))?;
    println!("  binary      {}", target.display());
    println!("  config      {}", config_dir.display());

    if autostart {
        let unit_dir = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("systemd")
            .join("user");
        std::fs::create_dir_all(&unit_dir).map_err(|e| e.to_string())?;
        let unit = format!(
            "[Unit]\nDescription=PulseRPC Discord Rich Presence\n\n[Service]\nExecStart={}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
            target.display()
        );
        std::fs::write(unit_dir.join("pulserpc.service"), unit)
            .map_err(|e| format!("systemd unit: {e}"))?;
        println!("  autostart   ~/.config/systemd/user/pulserpc.service");
        println!("              enable with: systemctl --user enable --now pulserpc");
    }

    if start {
        Command::new(&target)
            .spawn()
            .map_err(|e| format!("could not start: {e}"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn uninstall_unix(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = dir.unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("bin")
    });
    let unit = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("systemd")
        .join("user")
        .join("pulserpc.service");
    let _ = std::fs::remove_file(unit);
    let target = dir.join("pulserpc");
    if target.exists() {
        std::fs::remove_file(&target).map_err(|e| format!("remove: {e}"))?;
    }
    println!("Removed PulseRPC. Config kept in {}.", config::config_dir().display());
    Ok(())
}

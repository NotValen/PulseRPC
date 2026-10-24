//! Platform layer: process list, top-level windows, foreground app, idle time.

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub pid: u32,
    pub title: String,
    pub fullscreen: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SysSnapshot {
    /// (pid, executable name)
    pub procs: Vec<(u32, String)>,
    pub windows: Vec<WindowInfo>,
    pub foreground_pid: u32,
    pub idle_secs: u64,
    /// False when the platform cannot report a foreground window reliably.
    pub has_foreground: bool,
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(not(any(windows, unix)))]
mod stub;
#[cfg(not(any(windows, unix)))]
pub use stub::*;

pub fn current_pid() -> u32 {
    std::process::id()
}

//! Placeholder platform for targets without a native backend.

use super::SysSnapshot;

pub fn snapshot() -> SysSnapshot {
    SysSnapshot::default()
}

pub fn process_is_alive(_pid: u32) -> bool {
    false
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
    let _ = error;
    eprintln!("{title}: {text}");
}

//! Tiny file + console logger. Avoids `log`/`env_logger` to keep the binary small.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl Level {
    pub fn parse(s: &str) -> Level {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" | "err" => Level::Error,
            "warn" | "warning" => Level::Warn,
            "debug" => Level::Debug,
            "trace" => Level::Trace,
            _ => Level::Info,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }
}

struct Logger {
    level: Level,
    file: Option<Mutex<File>>,
    console: bool,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn init(level: Level, log_path: Option<PathBuf>, console: bool, max_bytes: u64) {
    let mut file = None;
    if let Some(path) = log_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Single-generation rotation: keep the log bounded.
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > max_bytes {
                let _ = std::fs::rename(&path, path.with_extension("old.log"));
            }
        }
        if let Ok(f) = OpenOptions::new().create(true).append(true).open(&path) {
            file = Some(Mutex::new(f));
        }
    }
    let _ = LOGGER.set(Logger {
        level,
        file,
        console,
    });
}

pub fn log(level: Level, msg: &str) {
    let Some(logger) = LOGGER.get() else {
        // Before init (very early errors) still surface on stderr.
        if level <= Level::Warn {
            eprintln!("[{}] {}", level.tag(), msg);
        }
        return;
    };
    if level > logger.level {
        return;
    }

    let line = format!(
        "[{}] {} {}\n",
        timestamp(),
        level.tag(),
        msg.trim_end()
    );

    if logger.console {
        if level <= Level::Warn {
            eprint!("{line}");
        } else {
            print!("{line}");
        }
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
    }
    if let Some(file) = &logger.file {
        if let Ok(mut f) = file.lock() {
            let _ = f.write_all(line.as_bytes());
            // Flush warnings/errors immediately so crashes are diagnosable.
            if level <= Level::Warn {
                let _ = f.flush();
            }
        }
    }
}

fn timestamp() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Error, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Warn, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Info, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Debug, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Trace, &format!($($arg)*)) };
}

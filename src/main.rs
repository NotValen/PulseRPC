#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! PulseRPC — automatic Discord Rich Presence for 90+ desktop applications.
//!
//! Design goals: small (single-digit MB RSS), no background heavyweights,
//! config-driven detection, hot reload, and a native installer.

mod cli;
mod config;
mod detect;
mod discord;
mod install;
mod log;
mod render;
mod runtime;
mod sys;
mod text;
mod util;
#[cfg(windows)]
mod winui;

use crate::config::Config;
use crate::log::Level;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    // A release build is GUI-subsystem (no console window at login); when it is
    // launched from a terminal we re-attach so CLI output still works.
    #[cfg(windows)]
    let console = sys::attach_parent_console();
    #[cfg(not(windows))]
    let console = true;

    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            println!("error: {e}\n");
            print!("{}", cli::HELP);
            std::process::exit(2);
        }
    };

    // Startup errors should be visible even when no console is attached.
    let gui_mode = cfg!(windows)
        && !args.headless
        && !args.dry_run
        && !args.once
        && matches!(args.command, cli::Command::Run);

    let code = match run(args, console) {
        Ok(code) => code,
        Err(e) => {
            if gui_mode {
                sys::message_box("PulseRPC", &e, true);
            } else {
                eprintln!("error: {e}");
            }
            1
        }
    };
    let _ = std::io::stdout().flush();
    std::process::exit(code);
}

fn run(args: cli::Args, console: bool) -> Result<i32, String> {
    match args.command {
        cli::Command::Help => {
            print!("{}", cli::HELP);
            return Ok(0);
        }
        cli::Command::Version => {
            println!("PulseRPC {VERSION}");
            return Ok(0);
        }
        cli::Command::Install => {
            install::install(args.dir.clone(), !args.no_autostart, args.start)?;
            return Ok(0);
        }
        cli::Command::Uninstall => {
            install::uninstall(args.dir.clone())?;
            return Ok(0);
        }
        cli::Command::SetId => {
            let id = args
                .id
                .clone()
                .ok_or_else(|| "usage: pulserpc --set-id <17-20 digit id>".to_string())?;
            let dir = config::resolve_dir(args.config.as_deref())?;
            let path = config::set_client_id(&dir, &id)?;
            println!("client_id = \"{id}\" written to {}", path.display());
            return Ok(0);
        }
        _ => {}
    }

    let dir = config::resolve_dir(args.config.as_deref())?;
    let cfg = config::load_dir(&dir)?;

    let level = if args.verbose {
        Level::Debug
    } else if args.quiet {
        Level::Warn
    } else {
        Level::parse(&cfg.logging.level)
    };
    let log_path = if cfg.logging.file {
        Some(config::log_path())
    } else {
        None
    };
    log::init(level, log_path, console, cfg.logging.max_kb.saturating_mul(1024));
    sys::install_shutdown_handler();

    for warning in &cfg.warnings {
        log_warn!("config: {warning}");
    }

    match args.command {
        cli::Command::Validate => {
            for warning in &cfg.warnings {
                println!("warning: {warning}");
            }
            println!(
                "ok: {} rules, {} apps files, config {}",
                cfg.rules.len(),
                config::watched_files(&dir).len().saturating_sub(1),
                cfg.path.display()
            );
            return Ok(0);
        }
        cli::Command::Rules => {
            print_rules(&cfg);
            return Ok(0);
        }
        cli::Command::Probe => {
            print_probe(&cfg);
            return Ok(0);
        }
        _ => {}
    }

    if args.once {
        print_once(&cfg);
        return Ok(0);
    }

    if args.dry_run {
        log_info!("dry-run: no Discord connection will be made");
    }

    // On Windows the GUI is the default; `--headless` and `--dry-run` keep the
    // classic console loop.
    let gui = cfg!(windows) && !args.headless && !args.dry_run;

    if !config::client_id_looks_valid(
        args.id
            .as_deref()
            .unwrap_or(cfg.presence.client_id.as_str()),
    ) {
        log_warn!(
            "Discord application id is not set — edit {} (or run with --id) and it will reconnect automatically",
            cfg.path.display()
        );
    }

    let _lock = if args.dry_run {
        None
    } else {
        match InstanceLock::acquire() {
            Ok(lock) => Some(lock),
            Err(LockError::Running(pid)) => {
                #[cfg(windows)]
                if gui {
                    // Open the window of the copy that is already running.
                    winui::activate_existing();
                    return Ok(0);
                }
                log_info!("PulseRPC is already running (pid {pid})");
                return Ok(0);
            }
            Err(LockError::Io(e)) => return Err(e),
        }
    };

    #[cfg(windows)]
    if gui {
        log_info!(
            "PulseRPC {VERSION} starting — {} rules, config {}",
            cfg.rules.len(),
            cfg.path.display()
        );
        let config_dir = dir.clone();
        let start_hidden = args.tray;
        let runtime = runtime::Runtime::new(cfg, dir, args.id.clone(), false, !args.no_watch);
        return winui::run(runtime, config_dir, start_hidden);
    }
    #[cfg(not(windows))]
    let _ = gui;

    log_info!(
        "PulseRPC {VERSION} starting — {} rules, config {}",
        cfg.rules.len(),
        cfg.path.display()
    );

    let mut runtime = runtime::Runtime::new(
        cfg,
        dir,
        args.id.clone(),
        args.dry_run,
        !args.no_watch,
    );

    while !sys::shutdown_requested() {
        runtime.tick();
        std::thread::sleep(Duration::from_millis(250));
    }
    runtime.shutdown();
    Ok(0)
}

fn print_rules(cfg: &Config) {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<&str, Vec<&config::Rule>> = BTreeMap::new();
    for rule in &cfg.rules {
        groups.entry(rule.category.as_str()).or_default().push(rule);
    }

    println!(
        "PulseRPC {VERSION} — {} rules in {} categories\n",
        cfg.rules.len(),
        groups.len()
    );
    for (category, rules) in groups {
        println!("[{category}]");
        for rule in rules {
            let mut flags = Vec::new();
            if rule.requires_foreground {
                flags.push("foreground-only");
            }
            if rule.media {
                flags.push("media");
            }
            if rule.fullscreen_only {
                flags.push("fullscreen-only");
            }
            let flags = if flags.is_empty() {
                String::new()
            } else {
                format!("  ({})", flags.join(", "))
            };
            println!(
                "  {:<20} {:>4}  {}{}",
                rule.id,
                rule.priority,
                rule.process_display.join(", "),
                flags
            );
        }
        println!();
    }
}

fn print_probe(cfg: &Config) {
    let snapshot = detect::take(cfg);
    println!("PulseRPC {VERSION} — detection probe");
    println!("config: {}  ({} rules)", cfg.path.display(), cfg.rules.len());
    println!(
        "processes: {}   windows: {}   idle: {}s",
        snapshot.probes.len(),
        snapshot.probes.iter().filter(|p| !p.title.is_empty()).count(),
        snapshot.idle_secs
    );
    match snapshot.foreground() {
        Some(fg) => println!(
            "foreground: {} (pid {}) \"{}\"",
            fg.process, fg.pid, fg.title
        ),
        None => println!("foreground: unknown"),
    }
    if snapshot.idle_secs >= cfg.detection.idle_after_secs && cfg.idle.enabled {
        println!("idle threshold reached — idle presence would be shown");
    }
    println!();

    struct Row<'a> {
        probe: &'a detect::Probe,
        rule: Option<&'a config::Rule>,
        score: i32,
    }

    let mut rows: Vec<Row> = Vec::new();
    for probe in &snapshot.probes {
        let mut best: Option<(&config::Rule, i32)> = None;
        for rule in &cfg.rules {
            if let Some(score) = detect::score(rule, probe, cfg) {
                if best.map(|(_, s)| score > s).unwrap_or(true) {
                    best = Some((rule, score));
                }
            }
        }
        if best.is_none() && !probe.foreground && probe.title.is_empty() {
            continue;
        }
        rows.push(Row {
            probe,
            rule: best.map(|(r, _)| r),
            score: best.map(|(_, s)| s).unwrap_or(0),
        });
    }
    rows.sort_by(|a, b| {
        b.probe
            .foreground
            .cmp(&a.probe.foreground)
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.probe.process.cmp(&b.probe.process))
    });

    println!(
        "  {:<2} {:<28} {:<8} {:<44} {}",
        "", "process", "pid", "window title", "rule"
    );
    for row in rows.iter().take(40) {
        let marker = if row.probe.foreground { "▶" } else { " " };
        let title = util::truncate(&row.probe.title, 42);
        let rule = match row.rule {
            Some(rule) => format!("{} (p{}, score {})", rule.id, rule.priority, row.score),
            None => "—".to_string(),
        };
        println!(
            "{} {:<28} {:<8} {:<44} {}",
            marker, row.probe.process, row.probe.pid, title, rule
        );
    }

    if let Some(best) = detect::pick(&snapshot, cfg) {
        println!(
            "\nwinner: {} → {} (score {})",
            best.probe.process,
            best.rule.id,
            best.score
        );
    } else {
        println!("\nwinner: none (fallback presence)");
    }
}

fn print_once(cfg: &Config) {
    let snapshot = detect::take(cfg);
    let idle = cfg.idle.enabled
        && cfg.detection.idle_after_secs > 0
        && snapshot.idle_secs >= cfg.detection.idle_after_secs;
    let picked = if idle { None } else { detect::pick(&snapshot, cfg) };

    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let started = std::time::SystemTime::now();

    let rendered = if let Some(pick) = picked {
        eprintln!(
            "detected: {} ({}) → rule `{}` score {}",
            pick.probe.process, pick.probe.title, pick.rule.id, pick.score
        );
        render::render(
            cfg,
            &pick.rule.block,
            &pick.rule.id,
            pick.rule.label(),
            Some(pick.rule),
            Some(pick.probe),
            snapshot.idle_secs,
            started,
            now,
            0,
        )
    } else if idle {
        eprintln!("detected: idle ({}s without input)", snapshot.idle_secs);
        render::render(
            cfg,
            &cfg.idle.block,
            "__idle__",
            "Idle",
            None,
            None,
            snapshot.idle_secs,
            started,
            now,
            0,
        )
    } else if cfg.fallback.enabled {
        eprintln!("detected: no rule matched — fallback presence");
        render::render(
            cfg,
            &cfg.fallback.block,
            "__fallback__",
            "Desktop",
            None,
            None,
            snapshot.idle_secs,
            started,
            now,
            0,
        )
    } else {
        eprintln!("detected: nothing (fallback disabled)");
        return;
    };

    if let Ok(pretty) = serde_json::to_string_pretty(&rendered.to_json()) {
        println!("{pretty}");
    }
}

// ───────────────────────────── single instance ─────────────────────────────

enum LockError {
    Running(u32),
    Io(String),
}

struct InstanceLock {
    path: PathBuf,
    pid: u32,
}

impl InstanceLock {
    fn acquire() -> Result<InstanceLock, LockError> {
        let path = config::lock_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| LockError::Io(e.to_string()))?;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                if pid != std::process::id() && sys::process_is_alive(pid) {
                    return Err(LockError::Running(pid));
                }
            }
            let _ = std::fs::remove_file(&path);
        }

        let pid = std::process::id();
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let _ = write!(file, "{pid}");
                Ok(InstanceLock { path, pid })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(LockError::Running(0))
            }
            Err(e) => Err(LockError::Io(e.to_string())),
        }
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if let Ok(text) = std::fs::read_to_string(&self.path) {
            if text.trim() == self.pid.to_string() {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

//! The detection/presence state machine.

use crate::config::{self, Config};
use crate::detect::{self, Probe};
use crate::discord::Client;
use crate::render;
use crate::util;
use crate::{log_debug, log_error, log_info, log_warn};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use time::OffsetDateTime;

const IDLE_ID: &str = "__idle__";
const FALLBACK_ID: &str = "__fallback__";

struct Active {
    id: String,
    label: String,
    probe: Option<Probe>,
    started: SystemTime,
    last_seen: Instant,
}

struct Pending {
    id: String,
    since: Instant,
}

/// Everything the GUI needs to draw the status panel.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub paused: bool,
    pub connected: bool,
    pub has_id: bool,
    pub dry_run: bool,
    pub label: String,
    pub details: String,
    pub state: String,
    pub foreground_process: String,
    pub foreground_title: String,
    pub idle_secs: u64,
    pub sends: u32,
}

struct Sent {
    key: String,
    at: Instant,
}

#[derive(Default)]
struct Limiter {
    sends: VecDeque<Instant>,
}

impl Limiter {
    fn allow(&mut self, now: Instant, min_secs: u64) -> bool {
        while let Some(front) = self.sends.front() {
            if now.duration_since(*front) > Duration::from_secs(20) {
                self.sends.pop_front();
            } else {
                break;
            }
        }
        if self.sends.len() >= 5 {
            return false;
        }
        if let Some(last) = self.sends.back() {
            if now.duration_since(*last).as_secs() < min_secs {
                return false;
            }
        }
        true
    }

    fn record(&mut self, now: Instant) {
        self.sends.push_back(now);
    }
}

pub struct Runtime {
    cfg: Arc<Config>,
    client: Client,
    client_id: String,
    client_ready: bool,
    id_override: Option<String>,
    config_dir: PathBuf,
    stamp: u64,
    last_reload: Instant,
    last_detect: Option<Instant>,
    last_render: Option<Instant>,
    snapshot: Option<detect::Snapshot>,
    active: Option<Active>,
    pending: Option<Pending>,
    last_sent: Option<Sent>,
    printed: Option<String>,
    last_summary: Option<String>,
    limiter: Limiter,
    starts: HashMap<String, (u64, Instant)>,
    dry_run: bool,
    watch: bool,
    pid: u32,
    paused: bool,
    last_details: String,
    last_state: String,
    sends: u32,
    generation: u64,
}

impl Runtime {
    pub fn new(
        cfg: Config,
        config_dir: PathBuf,
        id_override: Option<String>,
        dry_run: bool,
        watch: bool,
    ) -> Runtime {
        let client_id = id_override
            .clone()
            .unwrap_or_else(|| cfg.presence.client_id.clone());
        let client_ready = config::client_id_looks_valid(&client_id);
        let stamp = config::stamp(&config_dir);
        Runtime {
            cfg: Arc::new(cfg),
            client: Client::new(client_id.clone()),
            client_id,
            client_ready,
            id_override,
            config_dir,
            stamp,
            last_reload: Instant::now(),
            last_detect: None,
            last_render: None,
            snapshot: None,
            active: None,
            pending: None,
            last_sent: None,
            printed: None,
            last_summary: None,
            limiter: Limiter::default(),
            starts: HashMap::new(),
            dry_run,
            watch,
            pid: std::process::id(),
            paused: false,
            last_details: String::new(),
            last_state: String::new(),
            sends: 0,
            generation: 0,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Bumped every time a new configuration is accepted. The GUI uses it to
    /// cache expensive renderings (like the rules list) instead of rebuilding
    /// them on every frame.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Stop or resume sharing. Pausing removes the activity immediately so a
    /// stale status never lingers on the profile.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        self.last_sent = None;
        if paused {
            if !self.dry_run {
                self.client.clear(self.pid);
            }
            log_info!("presence paused");
        } else {
            self.last_detect = None;
            log_info!("presence resumed");
        }
    }

    /// Re-read the config files now (used after the GUI saves a setting).
    pub fn reload_now(&mut self) {
        self.stamp = config::stamp(&self.config_dir);
        self.reload();
        self.last_detect = None;
        self.last_render = None;
    }

    pub fn status(&self) -> Status {
        let foreground = self.snapshot.as_ref().and_then(|s| s.foreground());
        Status {
            paused: self.paused,
            connected: self.client.is_connected(),
            has_id: self.client_ready,
            dry_run: self.dry_run,
            label: self
                .active
                .as_ref()
                .map(|a| a.label.clone())
                .unwrap_or_default(),
            details: self.last_details.clone(),
            state: self.last_state.clone(),
            foreground_process: foreground.map(|p| p.process.clone()).unwrap_or_default(),
            foreground_title: foreground.map(|p| p.title.clone()).unwrap_or_default(),
            idle_secs: self.snapshot.as_ref().map(|s| s.idle_secs).unwrap_or(0),
            sends: self.sends,
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        self.maybe_reload(now);

        if self.client_ready && !self.dry_run {
            let was_connected = self.client.is_connected();
            self.client.poll();
            if !was_connected && self.client.is_connected() {
                // Re-assert the activity after Discord (re)starts.
                self.last_sent = None;
            }
        }

        let poll = Duration::from_millis(self.cfg.detection.poll_ms.max(250));
        let detect_due = self
            .last_detect
            .map(|t| now.duration_since(t) >= poll)
            .unwrap_or(true);
        if detect_due {
            self.last_detect = Some(now);
            self.snapshot = Some(detect::take(&self.cfg));
            if !self.paused {
                self.evaluate(now);
            }
        }

        let render_due = self
            .last_render
            .map(|t| now.duration_since(t) >= Duration::from_millis(1000))
            .unwrap_or(true);
        if render_due {
            self.last_render = Some(now);
            if !self.paused {
                self.push(now);
            }
        }
    }

    fn maybe_reload(&mut self, now: Instant) {
        if !self.watch || now.duration_since(self.last_reload) < Duration::from_millis(1000) {
            return;
        }
        self.last_reload = now;
        let stamp = config::stamp(&self.config_dir);
        if stamp == self.stamp {
            return;
        }
        self.stamp = stamp;
        self.reload();
    }

    fn reload(&mut self) {
        match config::load_dir(&self.config_dir) {
            Ok(new_cfg) => {
                for warning in &new_cfg.warnings {
                    log_warn!("config: {warning}");
                }
                let new_id = self
                    .id_override
                    .clone()
                    .unwrap_or_else(|| new_cfg.presence.client_id.clone());
                if new_id != self.client_id {
                    log_info!("Discord application id changed — reconnecting");
                    self.client_id = new_id;
                    self.client_ready = config::client_id_looks_valid(&self.client_id);
                    self.client = Client::new(self.client_id.clone());
                    self.last_sent = None;
                }
                if let Some(active) = &self.active {
                    if !active.id.starts_with("__")
                        && !new_cfg.rules.iter().any(|r| r.id == active.id)
                    {
                        log_info!("active rule `{}` was removed", active.id);
                        self.active = None;
                        self.pending = None;
                    }
                }
                log_info!("config reloaded — {} rules", new_cfg.rules.len());
                self.cfg = Arc::new(new_cfg);
                self.generation = self.generation.wrapping_add(1);
                self.last_sent = None;
            }
            Err(e) => {
                log_error!("config reload failed, keeping previous config: {e}");
            }
        }
    }

    fn evaluate(&mut self, now: Instant) {
        let cfg = self.cfg.clone();
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };

        let idle_target = cfg.idle.enabled
            && cfg.detection.idle_after_secs > 0
            && snapshot.idle_secs >= cfg.detection.idle_after_secs;

        let picked = if idle_target {
            None
        } else {
            detect::pick(snapshot, &cfg).map(|p| {
                (
                    p.rule.id.clone(),
                    p.rule.label().to_string(),
                    Some(p.probe.clone()),
                )
            })
        };

        let (target_id, target_label, target_probe) = if idle_target {
            (IDLE_ID.to_string(), "Idle".to_string(), None)
        } else if let Some(picked) = picked {
            picked
        } else if cfg.fallback.enabled {
            (FALLBACK_ID.to_string(), "Desktop".to_string(), None)
        } else {
            return;
        };

        // Brief "keep last" window so taskbar clicks don't wipe the presence.
        if !idle_target && target_probe.is_none() {
            if let Some(active) = &self.active {
                if !active.id.starts_with("__")
                    && active.last_seen.elapsed().as_secs() < cfg.presence.keep_last_secs
                {
                    return;
                }
            }
        }

        if let Some(active) = &self.active {
            if active.id == target_id {
                let active = self.active.as_mut().unwrap();
                active.last_seen = now;
                if target_probe.is_some() {
                    active.probe = target_probe;
                }
                self.pending = None;
                return;
            }
        }

        let delay = if target_probe.is_none() || self.active.is_none() {
            Duration::ZERO
        } else {
            Duration::from_millis(cfg.presence.switch_delay_ms)
        };

        if delay.is_zero() {
            self.commit(target_id, target_label, target_probe, now);
            return;
        }

        match &self.pending {
            Some(pending) if pending.id == target_id => {
                if now.duration_since(pending.since) >= delay {
                    self.commit(target_id, target_label, target_probe, now);
                }
            }
            _ => {
                self.pending = Some(Pending {
                    id: target_id,
                    since: now,
                });
            }
        }
    }

    fn commit(&mut self, id: String, label: String, probe: Option<Probe>, now: Instant) {
        let cfg = self.cfg.clone();
        let resume_secs = cfg.presence.resume_window_secs;

        // Resume the previous timer when returning to an app quickly.
        let resumed = self
            .starts
            .get(&id)
            .filter(|(_, seen)| seen.elapsed().as_secs() < resume_secs)
            .map(|(start, _)| *start);
        let start_unix = resumed.unwrap_or_else(util::now_unix);
        self.starts.insert(id.clone(), (start_unix, now));
        self.starts
            .retain(|_, (_, seen)| seen.elapsed() < Duration::from_secs(3_600));

        let previous = self.active.as_ref().map(|a| a.label.clone());
        match previous {
            Some(previous) if previous != label => {
                log_info!("switch: {previous} → {label}");
            }
            None => log_info!("presence: {label}"),
            _ => log_debug!("still {label}"),
        }

        self.active = Some(Active {
            id,
            label,
            probe,
            started: util::unix_to_system(start_unix),
            last_seen: now,
        });
        self.pending = None;
        self.last_sent = None;
    }

    fn push(&mut self, now: Instant) {
        let cfg = self.cfg.clone();
        let Some(active) = self.active.as_ref() else {
            log_debug!("push: no active app");
            return;
        };

        let (block, label, rule) = match active.id.as_str() {
            IDLE_ID => (&cfg.idle.block, "Idle", None),
            FALLBACK_ID => (&cfg.fallback.block, "Desktop", None),
            _ => match cfg.rules.iter().find(|r| r.id == active.id) {
                Some(rule) => (&rule.block, rule.label(), Some(rule)),
                None => {
                    log_debug!("push: rule `{}` is not in the config", active.id);
                    return;
                }
            },
        };

        let now_local = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let rotate = block.rotate_secs.unwrap_or(cfg.presence.rotate_secs);
        let bucket = if rotate == 0 {
            0
        } else {
            util::now_unix() / rotate.max(5)
        };
        let idle_secs = self
            .snapshot
            .as_ref()
            .map(|s| s.idle_secs)
            .unwrap_or(0);

        let rendered = render::render(
            &cfg,
            block,
            &active.id,
            label,
            rule,
            active.probe.as_ref(),
            idle_secs,
            active.started,
            now_local,
            bucket,
        );

        self.last_details = rendered.details.clone();
        self.last_state = rendered.state.clone();

        if rendered.details.is_empty() && rendered.state.is_empty() {
            log_debug!("push: `{}` rendered an empty activity", active.id);
            return;
        }

        let json = rendered.to_json();
        let key = json.to_string();
        let unchanged = self
            .last_sent
            .as_ref()
            .map(|sent| sent.key == key)
            .unwrap_or(false);
        // Re-assert periodically even without changes so Discord keeps the
        // activity after a restart or transient IPC hiccup.
        let due = self
            .last_sent
            .as_ref()
            .map(|sent| now.duration_since(sent.at).as_secs() >= cfg.presence.refresh_secs.max(20))
            .unwrap_or(true);
        if unchanged && !due {
            return;
        }
        if !self.limiter.allow(now, cfg.presence.min_update_secs) {
            log_debug!("push: rate limited for `{}`", active.id);
            return;
        }

        if !self.dry_run && !self.client_ready {
            log_debug!("push: no client id yet");
            return;
        }

        let summary = rendered.summary();
        log_debug!("push: sending `{}` for {}", summary, active.id);
        if self.dry_run {
            if self.printed.as_deref() != Some(key.as_str()) {
                if let Ok(pretty) = serde_json::to_string_pretty(&json) {
                    println!("{pretty}");
                }
                self.printed = Some(key.clone());
            }
            self.last_sent = Some(Sent { key, at: now });
            self.limiter.record(now);
            return;
        }

        if self.client.set_activity(self.pid, json) {
            self.limiter.record(now);
            self.last_sent = Some(Sent { key, at: now });
            self.sends = self.sends.saturating_add(1);
            if self.last_summary.as_deref() != Some(summary.as_str()) {
                log_info!("{summary}");
                self.last_summary = Some(summary);
            }
        }
    }

    pub fn shutdown(&mut self) {
        if !self.dry_run {
            self.client.clear(self.pid);
        }
        log_info!("PulseRPC stopped");
    }
}

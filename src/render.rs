//! Template rendering and Discord activity payload construction.

use crate::config::{Block, Config, Rule};
use crate::detect::Probe;
use crate::text::{self, Prepared};
use crate::util;
use serde_json::{json, Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

pub struct Vars {
    pub app: String,
    pub process: String,
    pub pid: String,
    pub title: String,
    pub title_short: String,
    pub project: String,
    pub file: String,
    pub filename: String,
    pub artist: String,
    pub track: String,
    pub media: String,
    pub elapsed: String,
    pub elapsed_short: String,
    pub elapsed_mmss: String,
    pub started: String,
    pub time: String,
    pub date: String,
    pub day: String,
    pub time_of_day: String,
    pub idle_for: String,
    pub user: String,
    pub host: String,
    pub category: String,
    pub private: String,
    pub fullscreen: String,
    pub instances: String,
}

impl Vars {
    fn get(&self, key: &str) -> Option<&str> {
        Some(match key {
            "app" | "game" => &self.app,
            "process" | "exe" => &self.process,
            "pid" => &self.pid,
            "title" => &self.title,
            "title_short" => &self.title_short,
            "project" | "workspace" => &self.project,
            "file" => &self.file,
            "filename" => &self.filename,
            "artist" => &self.artist,
            "track" | "song" => &self.track,
            "media" | "now_playing" => &self.media,
            "elapsed" | "duration" => &self.elapsed,
            "elapsed_short" => &self.elapsed_short,
            "elapsed_mmss" => &self.elapsed_mmss,
            "started" => &self.started,
            "time" => &self.time,
            "date" => &self.date,
            "day" => &self.day,
            "time_of_day" | "tod" => &self.time_of_day,
            "idle_for" => &self.idle_for,
            "user" => &self.user,
            "host" | "hostname" => &self.host,
            "category" => &self.category,
            "private" => &self.private,
            "fullscreen" => &self.fullscreen,
            "instances" => &self.instances,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Rendered {
    pub details: String,
    pub state: String,
    pub large_image: String,
    pub large_text: String,
    pub small_image: String,
    pub small_text: String,
    pub activity_type: u8,
    pub buttons: Vec<(String, String)>,
    pub start_ts: Option<u64>,
}

impl Rendered {
    pub fn summary(&self) -> String {
        match (self.details.is_empty(), self.state.is_empty()) {
            (false, false) => format!("{} · {}", self.details, self.state),
            (false, true) => self.details.clone(),
            (true, false) => self.state.clone(),
            (true, true) => String::new(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        if !self.details.is_empty() {
            map.insert("details".into(), json!(self.details));
        }
        if !self.state.is_empty() {
            map.insert("state".into(), json!(self.state));
        }
        map.insert("type".into(), json!(self.activity_type));
        map.insert("instance".into(), json!(false));

        if let Some(start) = self.start_ts {
            map.insert("timestamps".into(), json!({ "start": start }));
        }

        if !self.large_image.is_empty() || !self.small_image.is_empty() {
            let mut assets = Map::new();
            if !self.large_image.is_empty() {
                assets.insert("large_image".into(), json!(self.large_image));
            }
            if !self.large_text.is_empty() {
                assets.insert("large_text".into(), json!(self.large_text));
            }
            if !self.small_image.is_empty() {
                assets.insert("small_image".into(), json!(self.small_image));
            }
            if !self.small_text.is_empty() {
                assets.insert("small_text".into(), json!(self.small_text));
            }
            map.insert("assets".into(), Value::Object(assets));
        }

        if !self.buttons.is_empty() {
            let buttons: Vec<Value> = self
                .buttons
                .iter()
                .map(|(label, url)| json!({ "label": label, "url": url }))
                .collect();
            map.insert("buttons".into(), Value::Array(buttons));
        }

        Value::Object(map)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    cfg: &Config,
    block: &Block,
    id: &str,
    label: &str,
    rule: Option<&Rule>,
    probe: Option<&Probe>,
    idle_secs: u64,
    started: SystemTime,
    now: OffsetDateTime,
    bucket: u64,
) -> Rendered {
    let prepared = match (rule, probe) {
        (Some(rule), Some(probe)) => text::prepare(Some(rule), &probe.title),
        _ => Prepared::default(),
    };
    let vars = build_vars(cfg, label, rule, probe, &prepared, idle_secs, started, now);

    let seed = util::hash64(id) ^ bucket.wrapping_mul(0x9E37_79B9_7F4A_7C15);

    let mut details = from_pool(block.details.list(), &vars, seed);
    if details.is_empty() {
        details = util::truncate(label, 128);
    }
    let state = from_pool(block.state.list(), &vars, seed.rotate_left(17));

    let large_image = image(&block.large_image, &cfg.presence.large_image, &vars);
    let large_text = if block.large_text.trim().is_empty() {
        util::truncate(label, 128)
    } else {
        sanitize(&substitute(&block.large_text, &vars), 128)
    };
    let small_image = image(&block.small_image, &cfg.presence.small_image, &vars);
    let small_text = sanitize(&substitute(&block.small_text, &vars), 128);

    let mut buttons = Vec::new();
    if cfg.presence.buttons {
        if let Some(rule) = rule {
            for button in rule.buttons.iter().take(2) {
                let label = sanitize(&substitute(&button.label, &vars), 32);
                let url = substitute(&button.url, &vars).trim().to_string();
                if !label.is_empty() && url.starts_with("http") {
                    buttons.push((label, url));
                }
            }
        }
    }

    let show_elapsed = block
        .show_elapsed
        .unwrap_or(cfg.presence.show_elapsed)
        && !rule.map(|r| r.no_elapsed).unwrap_or(false);

    Rendered {
        details,
        state,
        large_image,
        large_text,
        small_image,
        small_text,
        activity_type: block.activity_type.unwrap_or(cfg.presence.activity_type),
        buttons,
        start_ts: if show_elapsed {
            started.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
        } else {
            None
        },
    }
}

fn image(block_value: &str, global_value: &str, vars: &Vars) -> String {
    let raw = if !block_value.trim().is_empty() {
        block_value
    } else if !global_value.trim().is_empty() {
        global_value
    } else {
        ""
    };
    if raw.is_empty() {
        return String::new();
    }
    let value = sanitize(&substitute(raw, vars), 128);
    if value.starts_with('{') {
        String::new()
    } else {
        value
    }
}

#[allow(clippy::too_many_arguments)]
fn build_vars(
    cfg: &Config,
    label: &str,
    rule: Option<&Rule>,
    probe: Option<&Probe>,
    prepared: &Prepared,
    idle_secs: u64,
    started: SystemTime,
    now: OffsetDateTime,
) -> Vars {
    let start_unix = started
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now_unix = now.unix_timestamp().max(0) as u64;
    let elapsed = now_unix.saturating_sub(start_unix);

    let start_local = OffsetDateTime::from_unix_timestamp(start_unix as i64)
        .map(|t| t.to_offset(now.offset()))
        .unwrap_or(now);

    let filename = prepared
        .file
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_string();

    Vars {
        app: rule
            .map(|r| r.label().to_string())
            .unwrap_or_else(|| label.to_string()),
        process: probe.map(|p| p.process.clone()).unwrap_or_default(),
        pid: probe.map(|p| p.pid.to_string()).unwrap_or_default(),
        title: util::truncate(&prepared.title, 110),
        title_short: util::truncate(&prepared.title, 48),
        project: util::truncate(&prepared.project, 64),
        file: util::truncate(&prepared.file, 64),
        filename: util::truncate(&filename, 48),
        artist: util::truncate(&prepared.artist, 64),
        track: util::truncate(&prepared.track, 64),
        media: util::truncate(&prepared.media_line(), 110),
        elapsed: util::human_elapsed(elapsed),
        elapsed_short: util::human_elapsed_short(elapsed),
        elapsed_mmss: util::clock(elapsed),
        started: format!("{:02}:{:02}", start_local.hour(), start_local.minute()),
        time: format!("{:02}:{:02}", now.hour(), now.minute()),
        date: format!("{:04}-{:02}-{:02}", now.year(), now.month() as u8, now.day()),
        day: format!("{:?}", now.weekday()),
        time_of_day: util::time_of_day(now.hour()).to_string(),
        idle_for: util::human_elapsed(idle_secs),
        user: std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_default(),
        host: if cfg.privacy.hide_hostname {
            String::new()
        } else {
            std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_default()
        },
        category: rule.map(|r| r.category.clone()).unwrap_or_default(),
        private: probe
            .map(|p| if p.private { "yes" } else { "no" })
            .unwrap_or("no")
            .to_string(),
        fullscreen: probe
            .map(|p| if p.fullscreen { "yes" } else { "no" })
            .unwrap_or("no")
            .to_string(),
        instances: probe.map(|p| p.instances.to_string()).unwrap_or_default(),
    }
}

fn from_pool(pool: &[String], vars: &Vars, seed: u64) -> String {
    if pool.is_empty() {
        return String::new();
    }
    let start = (mix(seed) % pool.len() as u64) as usize;
    for offset in 0..pool.len() {
        let candidate = substitute(&pool[(start + offset) % pool.len()], vars);
        if util::meaningful(&candidate) {
            return sanitize(&candidate, 128);
        }
    }
    String::new()
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// Replace `{variables}` in a single pass. Unknown placeholders are preserved
/// so typos remain visible while editing the config.
pub fn substitute(template: &str, vars: &Vars) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('}') {
            let key = &after[..close];
            match vars.get(key) {
                Some(value) => out.push_str(value),
                None => {
                    out.push('{');
                    out.push_str(key);
                    out.push('}');
                }
            }
            rest = &after[close + 1..];
        } else {
            out.push_str(&rest[open..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn sanitize(s: &str, max: usize) -> String {
    let collapsed = util::collapse_ws(s);
    let trimmed = collapsed.trim_matches(|c: char| c.is_whitespace() || c == '·' || c == '-');
    util::truncate(trimmed, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        Vars {
            app: "VS Code".into(),
            process: "Code.exe".into(),
            pid: "1".into(),
            title: "main.rs - PulseRPC".into(),
            title_short: "main.rs".into(),
            project: "PulseRPC".into(),
            file: "main.rs".into(),
            filename: "main.rs".into(),
            artist: "A".into(),
            track: "T".into(),
            media: "A — T".into(),
            elapsed: "5m".into(),
            elapsed_short: "5m".into(),
            elapsed_mmss: "05:00".into(),
            started: "10:00".into(),
            time: "10:05".into(),
            date: "2026-10-24".into(),
            day: "Saturday".into(),
            time_of_day: "Morning".into(),
            idle_for: "0s".into(),
            user: "me".into(),
            host: String::new(),
            category: "development".into(),
            private: "no".into(),
            fullscreen: "no".into(),
            instances: "1".into(),
        }
    }

    #[test]
    fn substitutes_known_and_keeps_unknown() {
        let vars = vars();
        assert_eq!(
            substitute("Editing {filename} in {project}", &vars),
            "Editing main.rs in PulseRPC"
        );
        assert_eq!(substitute("{nope}", &vars), "{nope}");
    }

    #[test]
    fn payload_shape() {
        let rendered = Rendered {
            details: "Editing main.rs".into(),
            state: "PulseRPC".into(),
            large_image: "vscode".into(),
            large_text: "Visual Studio Code".into(),
            small_image: String::new(),
            small_text: String::new(),
            activity_type: 0,
            buttons: vec![("Website".into(), "https://code.visualstudio.com".into())],
            start_ts: Some(1_700_000_000),
        };
        let json = rendered.to_json();
        assert_eq!(json["details"], "Editing main.rs");
        assert_eq!(json["assets"]["large_image"], "vscode");
        assert_eq!(json["timestamps"]["start"], 1_700_000_000u64);
        assert_eq!(json["buttons"][0]["label"], "Website");
    }
}

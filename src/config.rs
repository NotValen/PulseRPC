//! Configuration model, loader, validation and hot-reload fingerprinting.
//!
//! Layout on disk (everything hot-reloadable):
//!   config.toml          global settings
//!   apps/*.toml          one or more `[[app]]` rule files

use crate::util;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const PLACEHOLDER_ID: &str = "000000000000000000";

// ───────────────────────────── raw TOML types ─────────────────────────────

/// Accepts either `"one"` or `["one", "two"]` in TOML.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Strings {
    One(String),
    Many(Vec<String>),
}

impl Default for Strings {
    fn default() -> Self {
        Strings::Many(Vec::new())
    }
}

impl Strings {
    pub fn list(&self) -> &[String] {
        match self {
            Strings::One(s) => std::slice::from_ref(s),
            Strings::Many(v) => v,
        }
    }

    pub fn lower(&self) -> Vec<String> {
        self.list()
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaOrder {
    ArtistFirst,
    TitleFirst,
}

impl Default for MediaOrder {
    fn default() -> Self {
        MediaOrder::ArtistFirst
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Button {
    pub label: String,
    pub url: String,
}

/// The presence fields shared by app rules, idle and fallback states.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Block {
    pub details: Strings,
    pub state: Strings,
    pub large_image: String,
    pub large_text: String,
    pub small_image: String,
    pub small_text: String,
    pub activity_type: Option<u8>,
    pub rotate_secs: Option<u64>,
    pub show_elapsed: Option<bool>,
}

impl Default for Block {
    fn default() -> Self {
        Block {
            details: Strings::default(),
            state: Strings::default(),
            large_image: String::new(),
            large_text: String::new(),
            small_image: String::new(),
            small_text: String::new(),
            activity_type: None,
            rotate_secs: None,
            show_elapsed: None,
        }
    }
}

/// Idle / fallback blocks carry an on/off switch in addition to presence data.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ConfigBlock {
    pub enabled: bool,
    #[serde(flatten)]
    pub block: Block,
}

impl Default for ConfigBlock {
    fn default() -> Self {
        ConfigBlock {
            enabled: true,
            block: Block::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Presence {
    pub client_id: String,
    pub rotate_secs: u64,
    pub refresh_secs: u64,
    pub min_update_secs: u64,
    pub switch_delay_ms: u64,
    pub resume_window_secs: u64,
    pub keep_last_secs: u64,
    pub show_elapsed: bool,
    pub activity_type: u8,
    pub buttons: bool,
    pub large_image: String,
    pub small_image: String,
}

impl Default for Presence {
    fn default() -> Self {
        Presence {
            client_id: PLACEHOLDER_ID.to_string(),
            rotate_secs: 45,
            refresh_secs: 60,
            min_update_secs: 4,
            switch_delay_ms: 1200,
            resume_window_secs: 300,
            keep_last_secs: 60,
            show_elapsed: true,
            activity_type: 0,
            buttons: true,
            large_image: String::new(),
            small_image: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Detection {
    pub poll_ms: u64,
    pub foreground_bonus: i32,
    pub fullscreen_bonus: i32,
    pub idle_after_secs: u64,
    pub process_blacklist: Strings,
    pub title_blacklist: Strings,
    pub ignore_own: bool,
}

impl Default for Detection {
    fn default() -> Self {
        Detection {
            poll_ms: 2500,
            foreground_bonus: 1000,
            fullscreen_bonus: 150,
            idle_after_secs: 600,
            process_blacklist: Strings::Many(
                [
                    "explorer.exe",
                    "searchhost.exe",
                    "startmenuexperiencehost.exe",
                    "shellexperiencehost.exe",
                    "applicationframehost.exe",
                    "textinputhost.exe",
                    "dwm.exe",
                    "csrss.exe",
                    "winlogon.exe",
                    "svchost.exe",
                    "runtimebroker.exe",
                    "backgroundtaskhost.exe",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ),
            title_blacklist: Strings::Many(
                [
                    "incognito",
                    "inprivate",
                    "private browsing",
                    "private window",
                    "privé",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ),
            ignore_own: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Logging {
    pub level: String,
    pub file: bool,
    pub max_kb: u64,
}

impl Default for Logging {
    fn default() -> Self {
        Logging {
            level: "info".to_string(),
            file: true,
            max_kb: 2048,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Privacy {
    pub hide_when_private: bool,
    pub hide_paths: bool,
    pub hide_hostname: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Privacy {
            hide_when_private: true,
            hide_paths: true,
            hide_hostname: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AppRule {
    pub id: String,
    pub name: String,
    pub category: String,
    pub priority: i32,
    pub process: Strings,
    pub title_contains: Strings,
    pub title_all: Strings,
    pub title_excludes: Strings,
    pub title_regex: String,
    pub path_contains: Strings,
    pub requires_foreground: bool,
    pub fullscreen_only: bool,
    pub min_title_len: usize,
    pub strip_prefixes: Strings,
    pub strip_suffixes: Strings,
    pub strip_regex: Strings,
    pub media: bool,
    pub media_order: MediaOrder,
    pub project_index: Option<usize>,
    pub file_index: Option<usize>,
    pub enabled: bool,
    pub no_elapsed: bool,
    pub buttons: Vec<Button>,
    #[serde(flatten)]
    pub presence: Block,
}

impl Default for AppRule {
    fn default() -> Self {
        AppRule {
            id: String::new(),
            name: String::new(),
            category: "other".to_string(),
            priority: 50,
            process: Strings::default(),
            title_contains: Strings::default(),
            title_all: Strings::default(),
            title_excludes: Strings::default(),
            title_regex: String::new(),
            path_contains: Strings::default(),
            requires_foreground: false,
            fullscreen_only: false,
            min_title_len: 0,
            strip_prefixes: Strings::default(),
            strip_suffixes: Strings::default(),
            strip_regex: Strings::default(),
            media: false,
            media_order: MediaOrder::default(),
            project_index: None,
            file_index: None,
            enabled: true,
            no_elapsed: false,
            buttons: Vec::new(),
            presence: Block::default(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AppsFile {
    app: Vec<AppRule>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    presence: Presence,
    detection: Detection,
    logging: Logging,
    privacy: Privacy,
    idle: ConfigBlock,
    fallback: ConfigBlock,
    app: Vec<AppRule>,
}

// ───────────────────────────── compiled config ─────────────────────────────

#[derive(Clone, Debug)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub category: String,
    pub priority: i32,
    pub process: Vec<String>,
    /// Original casing, for display only.
    pub process_display: Vec<String>,
    pub title_contains: Vec<String>,
    pub title_all: Vec<String>,
    pub title_excludes: Vec<String>,
    pub title_regex: Option<Regex>,
    pub path_contains: Vec<String>,
    pub requires_foreground: bool,
    pub fullscreen_only: bool,
    pub min_title_len: usize,
    pub strip_prefixes: Vec<String>,
    pub strip_suffixes: Vec<String>,
    pub strip_regex: Vec<Regex>,
    pub media: bool,
    pub media_order: MediaOrder,
    pub project_index: Option<usize>,
    pub file_index: Option<usize>,
    pub block: Block,
    pub buttons: Vec<Button>,
    pub no_elapsed: bool,
}

impl Rule {
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }

    pub fn matches_process(&self, process_lc: &str) -> bool {
        self.process
            .iter()
            .any(|p| util::glob_match(p, process_lc))
    }

    /// `title` must already be lowercase.
    pub fn title_matches(&self, title_lc: &str, title_raw: &str) -> bool {
        if !self.title_excludes.is_empty()
            && self
                .title_excludes
                .iter()
                .any(|t| title_lc.contains(t.as_str()))
        {
            return false;
        }
        if !self.title_contains.is_empty()
            && !self
                .title_contains
                .iter()
                .any(|t| title_lc.contains(t.as_str()))
        {
            return false;
        }
        if !self.title_all.is_empty()
            && !self
                .title_all
                .iter()
                .all(|t| title_lc.contains(t.as_str()))
        {
            return false;
        }
        if let Some(re) = &self.title_regex {
            if !re.is_match(title_raw) {
                return false;
            }
        }
        true
    }

    pub fn exact_process(&self, process_lc: &str) -> bool {
        self.process.iter().any(|p| p == process_lc)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub path: PathBuf,
    pub presence: Presence,
    pub detection: Detection,
    pub logging: Logging,
    pub privacy: Privacy,
    pub idle: ConfigBlock,
    pub fallback: ConfigBlock,
    pub rules: Vec<Rule>,
    pub warnings: Vec<String>,
}

// ───────────────────────────── loading ─────────────────────────────

pub fn load_dir(dir: &Path) -> Result<Config, String> {
    let path = dir.join("config.toml");
    if !path.exists() {
        ensure_defaults(dir).map_err(|e| format!("cannot create default config: {e}"))?;
    }
    let raw_text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut raw: RawConfig = toml::from_str(&raw_text)
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let mut warnings = Vec::new();
    let inline_rules = std::mem::take(&mut raw.app);
    let mut raws: Vec<(String, AppRule)> = inline_rules
        .into_iter()
        .map(|r| ("config.toml".to_string(), r))
        .collect();

    let apps_dir = dir.join("apps");
    if let Ok(entries) = std::fs::read_dir(&apps_dir) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("toml"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        for file in files {
            let name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("apps.toml")
                .to_string();
            let text = std::fs::read_to_string(&file)
                .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            let parsed: AppsFile = toml::from_str(&text)
                .map_err(|e| format!("{name}: {e}"))?;
            for rule in parsed.app {
                raws.push((name.clone(), rule));
            }
        }
    }

    let mut rules = Vec::with_capacity(raws.len());
    let mut ids: HashSet<String> = HashSet::new();
    for (source, app) in raws {
        let mut rule = compile_rule(app, &source, &mut warnings, &mut ids)?;
        rule.block.activity_type = rule
            .block
            .activity_type
            .or(Some(raw.presence.activity_type));
        rules.push(rule);
    }

    if rules.is_empty() {
        return Err("no application rules found (expected apps/*.toml with [[app]])".to_string());
    }

    validate_global(&raw, &mut warnings);

    Ok(Config {
        path,
        presence: raw.presence,
        detection: raw.detection,
        logging: raw.logging,
        privacy: raw.privacy,
        idle: raw.idle,
        fallback: raw.fallback,
        rules,
        warnings,
    })
}

fn compile_rule(
    app: AppRule,
    source: &str,
    warnings: &mut Vec<String>,
    ids: &mut HashSet<String>,
) -> Result<Rule, String> {
    let id = app.id.trim().to_string();
    if id.is_empty() {
        return Err(format!("{source}: a [[app]] entry is missing `id`"));
    }
    if !ids.insert(id.clone()) {
        return Err(format!("{source}: duplicate app id `{id}`"));
    }

    let process = app.process.lower();
    let process_display: Vec<String> = app.process.list().to_vec();
    if process.is_empty() && app.enabled {
        return Err(format!("{source}: `{id}` has no `process` patterns"));
    }

    let title_regex = compile_optional(&app.title_regex, source, &id, "title_regex")?;
    let mut strip_regex = Vec::new();
    for pattern in app.strip_regex.list() {
        if pattern.trim().is_empty() {
            continue;
        }
        if let Some(re) = compile_optional(pattern, source, &id, "strip_regex")? {
            strip_regex.push(re);
        }
    }

    let mut buttons: Vec<Button> = app
        .buttons
        .into_iter()
        .filter(|b| !b.label.trim().is_empty() && b.url.trim().starts_with("http"))
        .collect();
    if buttons.len() > 2 {
        warnings.push(format!(
            "{}: `{id}` has {} buttons; Discord allows 2 (extra ignored)",
            source,
            buttons.len()
        ));
        buttons.truncate(2);
    }
    if !app.enabled {
        // Keep it in the list but make it unmatchable; keeps ids unique for validation.
        return Ok(Rule {
            id,
            name: if app.name.is_empty() {
                String::new()
            } else {
                app.name
            },
            category: app.category,
            priority: app.priority,
            process: Vec::new(),
            process_display: Vec::new(),
            title_contains: Vec::new(),
            title_all: Vec::new(),
            title_excludes: Vec::new(),
            title_regex: None,
            path_contains: Vec::new(),
            requires_foreground: false,
            fullscreen_only: false,
            min_title_len: 0,
            strip_prefixes: Vec::new(),
            strip_suffixes: Vec::new(),
            strip_regex: Vec::new(),
            media: false,
            media_order: MediaOrder::default(),
            project_index: None,
            file_index: None,
            block: app.presence,
            buttons: Vec::new(),
                no_elapsed: true,
        });
    }

    Ok(Rule {
        id: id.clone(),
        name: if app.name.trim().is_empty() {
            String::new()
        } else {
            app.name.trim().to_string()
        },
        category: app.category.trim().to_lowercase(),
        priority: app.priority,
        process,
        process_display,
        title_contains: app.title_contains.lower(),
        title_all: app.title_all.lower(),
        title_excludes: app.title_excludes.lower(),
        title_regex,
        path_contains: app.path_contains.lower(),
        requires_foreground: app.requires_foreground,
        fullscreen_only: app.fullscreen_only,
        min_title_len: app.min_title_len,
        strip_prefixes: app.strip_prefixes.list().to_vec(),
        strip_suffixes: app.strip_suffixes.list().to_vec(),
        strip_regex,
        media: app.media,
        media_order: app.media_order,
        project_index: app.project_index,
        file_index: app.file_index,
        block: app.presence,
        buttons,
        no_elapsed: app.no_elapsed,
    })
}

fn compile_optional(
    pattern: &str,
    source: &str,
    id: &str,
    field: &str,
) -> Result<Option<Regex>, String> {
    if pattern.trim().is_empty() {
        return Ok(None);
    }
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map(Some)
        .map_err(|e| format!("{source}: `{id}` invalid `{field}`: {e}"))
}

fn validate_global(raw: &RawConfig, warnings: &mut Vec<String>) {
    if !client_id_looks_valid(&raw.presence.client_id) {
        warnings.push(format!(
            "presence.client_id is `{}` — set it to your Discord application ID (17–20 digits)",
            raw.presence.client_id
        ));
    }
    if raw.presence.rotate_secs > 0 && raw.presence.rotate_secs < 15 {
        warnings.push("presence.rotate_secs < 15 may trigger Discord rate limits".to_string());
    }
    if raw.presence.min_update_secs < 3 {
        warnings.push("presence.min_update_secs < 3 may trigger Discord rate limits".to_string());
    }
    if raw.detection.poll_ms < 500 {
        warnings.push("detection.poll_ms < 500 is wasteful; 2000–4000 is plenty".to_string());
    }
    if raw.detection.idle_after_secs > 0 && raw.detection.idle_after_secs < 60 {
        warnings.push("detection.idle_after_secs < 60 will show idle presence very eagerly".to_string());
    }
}

pub fn client_id_looks_valid(id: &str) -> bool {
    let id = id.trim();
    (17..=20).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_digit())
        && id.chars().any(|c| c != '0')
}

/// Write a Discord application id into `dir/config.toml`, creating the default
/// config first when needed. Only the `client_id` line is touched, so comments
/// and every other setting survive.
pub fn set_client_id(dir: &Path, id: &str) -> Result<PathBuf, String> {
    let id = id.trim();
    if !client_id_looks_valid(id) {
        return Err(format!(
            "`{id}` does not look like a Discord application id (17–20 digits)"
        ));
    }
    set_config_value(dir, "presence", "client_id", &format!("\"{id}\""))
}

/// Update one `key = value` inside `[section]` of `dir/config.toml`.
///
/// `value` is a raw TOML value (quote strings yourself, e.g. `"\"123\""`).
/// Comments, ordering and every other setting are preserved. The section (and
/// key) are created when missing. This is what the GUI uses to save settings,
/// so it writes through the same path users edit by hand.
pub fn set_config_value(
    dir: &Path,
    section: &str,
    key: &str,
    value: &str,
) -> Result<PathBuf, String> {
    ensure_defaults(dir).map_err(|e| format!("cannot create config: {e}"))?;

    let path = dir.join("config.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();

    let header_re = Regex::new(&format!(
        r"^[ \t]*\[[ \t]*{}[ \t]*\][ \t]*$",
        regex::escape(section)
    ))
    .unwrap();
    let key_re =
        Regex::new(&format!(r"^[ \t]*{}[ \t]*=", regex::escape(key))).unwrap();
    let is_header = |line: &str| line.trim_start().starts_with('[');

    match lines.iter().position(|line| header_re.is_match(line)) {
        Some(start) => {
            let end = lines[start + 1..]
                .iter()
                .position(|line| is_header(line))
                .map(|offset| start + 1 + offset)
                .unwrap_or(lines.len());
            match (start + 1..end).find(|&i| key_re.is_match(&lines[i])) {
                Some(index) => lines[index] = format!("{key} = {value}"),
                None => lines.insert(start + 1, format!("{key} = {value}")),
            }
        }
        None => {
            if lines.last().map(|line| !line.trim().is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push(format!("[{section}]"));
            lines.push(format!("{key} = {value}"));
        }
    }

    let mut updated = lines.join(newline);
    updated.push_str(newline);
    if updated != text {
        std::fs::write(&path, updated)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(path)
}

/// Resolve the config directory:
/// 1. explicit `--config`
/// 2. a `config/` folder next to the executable (portable / installed layout)
/// 3. the per-user config directory
pub fn resolve_dir(arg: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = arg {
        if path.is_dir() {
            return Ok(path.to_path_buf());
        }
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            if path.file_name().and_then(|n| n.to_str()) == Some("config.toml") {
                return Ok(path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf());
            }
            return Err(format!(
                "{}: point --config at the directory that contains config.toml",
                path.display()
            ));
        }
        return Err(format!("config directory not found: {}", path.display()));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let portable = dir.join("config");
            if portable.join("config.toml").exists() {
                return Ok(portable);
            }
        }
    }

    Ok(config_dir())
}

// ───────────────────────────── hot reload stamp ─────────────────────────────

pub fn watched_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = vec![dir.join("config.toml")];
    if let Ok(entries) = std::fs::read_dir(dir.join("apps")) {
        let mut extra: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("toml"))
                    .unwrap_or(false)
            })
            .collect();
        extra.sort();
        files.extend(extra);
    }
    files
}

/// Cheap fingerprint: paths + mtimes + sizes. No watcher thread needed.
pub fn stamp(dir: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for file in watched_files(dir) {
        file.hash(&mut hasher);
        if let Ok(meta) = std::fs::metadata(&file) {
            meta.len().hash(&mut hasher);
            if let Ok(modified) = meta.modified() {
                modified.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

// ───────────────────────────── paths & defaults ─────────────────────────────

pub fn base_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(dir).join("PulseRPC");
        }
        if let Some(dir) = std::env::var_os("APPDATA") {
            return PathBuf::from(dir).join("PulseRPC");
        }
        PathBuf::from("PulseRPC")
    }
    #[cfg(unix)]
    {
        if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(dir).join("pulserpc");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join("pulserpc");
        }
        PathBuf::from(".pulserpc")
    }
    #[cfg(not(any(windows, unix)))]
    {
        PathBuf::from(".pulserpc")
    }
}

pub fn config_dir() -> PathBuf {
    base_dir().join("config")
}

pub fn log_path() -> PathBuf {
    base_dir().join("logs").join("pulserpc.log")
}

pub fn lock_path() -> PathBuf {
    base_dir().join("pulserpc.lock")
}

// Embedded defaults so the binary can provision itself (installer / first run).
const DEFAULT_CONFIG: &str = include_str!("../config/config.toml");

const DEFAULT_APPS: &[(&str, &str)] = &[
    ("apps/development.toml", include_str!("../config/apps/development.toml")),
    ("apps/browsers.toml", include_str!("../config/apps/browsers.toml")),
    ("apps/media.toml", include_str!("../config/apps/media.toml")),
    ("apps/games.toml", include_str!("../config/apps/games.toml")),
    ("apps/productivity.toml", include_str!("../config/apps/productivity.toml")),
    ("apps/communication.toml", include_str!("../config/apps/communication.toml")),
    ("apps/utilities.toml", include_str!("../config/apps/utilities.toml")),
];

/// Write any missing config file. Existing user files are never touched.
pub fn ensure_defaults(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir.join("apps"))?;
    let mut written = Vec::new();
    write_if_missing(&dir.join("config.toml"), DEFAULT_CONFIG, &mut written)?;
    for (rel, content) in DEFAULT_APPS {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_if_missing(&path, content, &mut written)?;
    }
    Ok(written)
}

fn write_if_missing(path: &Path, content: &str, written: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, content)?;
    written.push(path.to_path_buf());
    Ok(())
}

/// All embedded config files are written by `ensure_defaults`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_parse() {
        let dir = std::env::temp_dir().join(format!("pulserpc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ensure_defaults(&dir).unwrap();
        let config = load_dir(&dir).unwrap();
        assert!(config.rules.len() >= 50, "got {}", config.rules.len());
        assert!(config
            .rules
            .iter()
            .any(|r| r.id == "vscode" && r.matches_process("code.exe")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn client_ids() {
        assert!(client_id_looks_valid("123456789012345678"));
        assert!(!client_id_looks_valid("abc"));
        assert!(!client_id_looks_valid(PLACEHOLDER_ID));
    }

    #[test]
    fn set_client_id_keeps_everything_else() {
        let dir = std::env::temp_dir().join(format!("pulserpc-setid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ensure_defaults(&dir).unwrap();

        let before = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let path = set_client_id(&dir, "123456789012345678").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();

        assert!(after.contains("client_id = \"123456789012345678\""));
        assert!(!after.contains(PLACEHOLDER_ID));
        assert_eq!(before.lines().count(), after.lines().count());
        assert!(set_client_id(&dir, "not-an-id").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_config_value_is_section_aware() {
        let dir = std::env::temp_dir().join(format!("pulserpc-setval-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ensure_defaults(&dir).unwrap();
        let path = dir.join("config.toml");

        let before = load_dir(&dir).unwrap();
        let text_before = std::fs::read_to_string(&path).unwrap();

        set_config_value(&dir, "detection", "idle_after_secs", "900").unwrap();
        set_config_value(&dir, "presence", "show_elapsed", "false").unwrap();
        set_config_value(&dir, "presence", "rotate_secs", "30").unwrap();

        let after = load_dir(&dir).unwrap();
        let text_after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after.detection.idle_after_secs, 900);
        assert!(!after.presence.show_elapsed);
        assert_eq!(after.presence.rotate_secs, 30);
        // Updating one section must not disturb another.
        assert_eq!(after.detection.poll_ms, before.detection.poll_ms);
        // Comments survive and no lines were lost.
        assert!(text_after.contains("# Show \"elapsed\" timers on activities."));
        assert_eq!(text_after.lines().count(), text_before.lines().count());

        // A missing key is inserted inside the right section.
        set_config_value(&dir, "detection", "test_key", "1").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let detection = text.find("[detection]").unwrap();
        let logging = text.find("[logging]").unwrap_or(text.len());
        let key = text.find("test_key = 1").unwrap();
        assert!(detection < key && key < logging);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

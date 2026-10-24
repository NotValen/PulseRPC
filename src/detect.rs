//! Process + window snapshot and rule scoring ("intelligent" detection).

use crate::config::{Config, Rule};
use crate::sys;
use crate::util;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Probe {
    pub pid: u32,
    pub process: String,
    pub process_lc: String,
    pub title: String,
    pub title_lc: String,
    pub path: Vec<String>,
    pub foreground: bool,
    pub fullscreen: bool,
    pub private: bool,
    pub instances: u32,
}

pub struct Snapshot {
    pub probes: Vec<Probe>,
    pub idle_secs: u64,
}

impl Snapshot {
    pub fn foreground(&self) -> Option<&Probe> {
        self.probes.iter().find(|p| p.foreground)
    }
}

pub struct Pick<'a> {
    pub rule: &'a Rule,
    pub probe: &'a Probe,
    pub score: i32,
}

/// Collect running processes, window titles, foreground pid and idle time.
pub fn take(cfg: &Config) -> Snapshot {
    let raw = sys::snapshot();
    let own_pid = sys::current_pid();

    let mut windows: HashMap<u32, (String, bool)> = HashMap::with_capacity(raw.windows.len());
    for window in &raw.windows {
        let entry = windows.entry(window.pid).or_insert_with(|| (String::new(), false));
        if window.title.chars().count() > entry.0.chars().count() {
            entry.0 = window.title.clone();
        }
        entry.1 |= window.fullscreen;
    }

    // Foreground handling: platforms without a reliable foreground API treat
    // every process as foreground so priority ordering still works.
    let fg_available = raw.has_foreground && raw.foreground_pid != 0;

    let mut instances: HashMap<String, u32> = HashMap::with_capacity(raw.procs.len());
    for (_, name) in &raw.procs {
        let lc = name.to_lowercase();
        *instances.entry(lc).or_insert(0) += 1;
    }

    let blacklist = cfg.detection.process_blacklist.lower();
    let privacy_terms = cfg.detection.title_blacklist.lower();
    let foreground_path = if fg_available {
        sys::process_path(raw.foreground_pid)
            .map(|p| p.to_lowercase())
    } else {
        None
    };

    let mut probes = Vec::with_capacity(raw.procs.len());
    for (pid, name) in raw.procs {
        if cfg.detection.ignore_own && pid == own_pid {
            continue;
        }
        let lc = name.to_lowercase();
        if blacklist.iter().any(|b| util::glob_match(b, &lc)) {
            continue;
        }
        let (title, fullscreen) = windows
            .get(&pid)
            .cloned()
            .unwrap_or_else(|| (String::new(), false));
        let foreground = !fg_available || pid == raw.foreground_pid;

        let mut private = false;
        let mut title = title;
        if cfg.privacy.hide_when_private {
            let lowered = title.to_lowercase();
            if privacy_terms.iter().any(|t| lowered.contains(t.as_str())) {
                private = true;
                title.clear();
            }
        }

        let path = if pid == raw.foreground_pid {
            foreground_path.iter().cloned().collect()
        } else {
            Vec::new()
        };

        probes.push(Probe {
            pid,
            process: name,
            process_lc: lc.clone(),
            title_lc: title.to_lowercase(),
            title,
            path,
            foreground,
            fullscreen,
            private,
            instances: *instances.get(&lc).unwrap_or(&1),
        });
    }

    Snapshot {
        probes,
        idle_secs: raw.idle_secs,
    }
}

/// Score a single rule against a probe. `None` means "does not match".
pub fn score(rule: &Rule, probe: &Probe, cfg: &Config) -> Option<i32> {
    if rule.process.is_empty() || !rule.matches_process(&probe.process_lc) {
        return None;
    }
    if probe.title.chars().count() < rule.min_title_len {
        return None;
    }
    if rule.requires_foreground && !probe.foreground {
        return None;
    }
    if rule.fullscreen_only && !probe.fullscreen {
        return None;
    }
    if !rule.path_contains.is_empty()
        && (probe.path.is_empty()
            || !rule
                .path_contains
                .iter()
                .any(|needle| probe.path.iter().any(|p| p.contains(needle))))
    {
        return None;
    }
    if !rule.title_matches(&probe.title_lc, &probe.title) {
        return None;
    }
    // Title-driven rules must actually have a title to look at.
    if probe.title.is_empty()
        && (!rule.title_contains.is_empty()
            || !rule.title_all.is_empty()
            || rule.title_regex.is_some())
    {
        return None;
    }

    let mut score = rule.priority;
    if probe.foreground {
        score += cfg.detection.foreground_bonus;
    }
    if probe.fullscreen {
        score += cfg.detection.fullscreen_bonus;
    }
    if rule.exact_process(&probe.process_lc) {
        score += 2;
    }
    if !rule.title_contains.is_empty() || rule.title_regex.is_some() {
        score += 5;
    }
    Some(score)
}

/// Highest-scoring rule/probe pair.
///
/// Score = priority + foreground bonus + fullscreen bonus + specificity.
/// Ties prefer the foreground window, then the earlier rule.
pub fn pick<'a>(snapshot: &'a Snapshot, cfg: &'a Config) -> Option<Pick<'a>> {
    let mut best: Option<Pick<'a>> = None;
    for rule in &cfg.rules {
        for probe in &snapshot.probes {
            let Some(score) = score(rule, probe, cfg) else {
                continue;
            };
            let better = match &best {
                None => true,
                Some(b) => {
                    score > b.score
                        || (score == b.score && probe.foreground && !b.probe.foreground)
                }
            };
            if better {
                best = Some(Pick { rule, probe, score });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    #[test]
    fn foreground_and_specificity_raise_the_score() {
        let dir = std::env::temp_dir().join(format!("pulserpc-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        config::ensure_defaults(&dir).unwrap();
        let cfg = config::load_dir(&dir).unwrap();
        let rule = cfg.rules.iter().find(|r| r.id == "brave").unwrap();

        let probe = Probe {
            pid: 42,
            process: "brave.exe".into(),
            process_lc: "brave.exe".into(),
            title: "YouTube - Brave".into(),
            title_lc: "youtube - brave".into(),
            path: Vec::new(),
            foreground: true,
            fullscreen: false,
            private: false,
            instances: 1,
        };

        let expected = rule.priority + cfg.detection.foreground_bonus + 2;
        assert_eq!(score(rule, &probe, &cfg), Some(expected));
        assert!(expected > 1000);

        // Browsers are foreground-only, so a background window must not match.
        let mut background = probe.clone();
        background.foreground = false;
        assert_eq!(score(rule, &background, &cfg), None);

        // A different executable never matches.
        let mut other = probe;
        other.process_lc = "chrome.exe".into();
        assert_eq!(score(rule, &other, &cfg), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

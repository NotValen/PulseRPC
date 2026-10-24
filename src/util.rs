//! Small dependency-free helpers used across the app.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn unix_to_system(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

/// Truncate to `max` characters, appending an ellipsis when shortened.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 90 -> "1m", 5400 -> "1h 30m", 90000 -> "1d 1h"
pub fn human_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h {}m", secs / 3_600, (secs % 3_600) / 60)
    } else {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3_600)
    }
}

/// 90 -> "1m", 5400 -> "1h"
pub fn human_elapsed_short(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// 5425 -> "1:30:25" / "30:25"
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3_600, (secs % 3_600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn time_of_day(hour: u8) -> &'static str {
    match hour {
        0..=4 => "Night",
        5..=11 => "Morning",
        12..=16 => "Afternoon",
        17..=20 => "Evening",
        _ => "Night",
    }
}

/// FNV-1a, used for stable message rotation (no RNG dependency).
pub fn hash64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// Glob matcher supporting `*` and `?`, case-sensitive (callers lowercase).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            pi += 1;
            mark = ti;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Collapse runs of whitespace (including newlines/tabs) into single spaces.
pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

/// True when the text has at least one letter or digit — used to reject
/// templates that collapsed into leftover punctuation/emoji.
pub fn meaningful(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric())
}

/// Case-insensitive `starts_with`, returning the remainder.
pub fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return None;
    }
    let tl = text.to_lowercase();
    let pl = prefix.to_lowercase();
    if !tl.starts_with(&pl) {
        return None;
    }
    let idx = prefix.len().min(text.len());
    if text.is_char_boundary(idx) {
        Some(text[idx..].trim_start())
    } else {
        None
    }
}

/// Case-insensitive `ends_with`, returning the remainder.
pub fn strip_suffix_ci<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    if suffix.is_empty() {
        return None;
    }
    let tl = text.to_lowercase();
    let sl = suffix.to_lowercase();
    if !tl.ends_with(&sl) {
        return None;
    }
    let idx = text.len().saturating_sub(suffix.len());
    if text.is_char_boundary(idx) {
        Some(text[..idx].trim_end())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("code*.exe", "code.exe"));
        assert!(glob_match("code*.exe", "code - insiders.exe"));
        assert!(!glob_match("code.exe", "code - insiders.exe"));
        assert!(glob_match("godot?.exe", "godot4.exe"));
        assert!(glob_match("*steam*", "steamwebhelper.exe"));
    }

    #[test]
    fn durations() {
        assert_eq!(human_elapsed(45), "45s");
        assert_eq!(human_elapsed(600), "10m");
        assert_eq!(human_elapsed(5_400), "1h 30m");
        assert_eq!(clock(65), "01:05");
        assert_eq!(clock(3_725), "1:02:05");
    }

    #[test]
    fn strips() {
        assert_eq!(
            strip_suffix_ci("main.rs - Visual Studio Code", " - Visual Studio Code"),
            Some("main.rs")
        );
        assert_eq!(strip_prefix_ci("● main.rs", "● "), Some("main.rs"));
        assert_eq!(truncate("abcdef", 4), "abc…");
    }
}

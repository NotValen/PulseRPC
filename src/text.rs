//! Window-title cleaning and media metadata heuristics.

use crate::config::{MediaOrder, Rule};
use crate::util;

const DEFAULT_PREFIXES: &[&str] = &["● ", "• ", "* ", "▶ ", "⏸ ", "🔊 ", "◦ ", "▪ "];
const SEPARATORS: &[&str] = &[" - ", " – ", " — ", " | ", " • ", " · ", " // "];

#[derive(Clone, Debug, Default)]
pub struct Prepared {
    /// Cleaned title (without player/app suffixes).
    pub title: String,
    pub project: String,
    pub file: String,
    pub artist: String,
    pub track: String,
}

impl Prepared {
    pub fn media_line(&self) -> String {
        match (self.artist.is_empty(), self.track.is_empty()) {
            (false, false) => format!("{} — {}", self.artist, self.track),
            (true, false) => self.track.clone(),
            (false, true) => self.artist.clone(),
            (true, true) => String::new(),
        }
    }
}

/// Compile nothing here on purpose — strip regexes live on the rule.
pub fn prepare(rule: Option<&Rule>, raw: &str) -> Prepared {
    let mut text = util::collapse_ws(raw.trim());
    if text.is_empty() {
        return Prepared::default();
    }

    if let Some(rule) = rule {
        for prefix in rule.strip_prefixes.iter().map(String::as_str).chain(DEFAULT_PREFIXES.iter().copied()) {
            if let Some(rest) = util::strip_prefix_ci(&text, prefix) {
                text = rest.to_string();
            }
        }
        // Longest suffixes first so " - X - Y" strips completely.
        let mut suffixes: Vec<&str> = rule.strip_suffixes.iter().map(String::as_str).collect();
        suffixes.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for suffix in suffixes {
            if let Some(rest) = util::strip_suffix_ci(&text, suffix) {
                text = rest.to_string();
            }
        }
        for re in &rule.strip_regex {
            if re.is_match(&text) {
                text = re.replace_all(&text, "").to_string();
            }
        }
        text = text
            .trim_matches(|c: char| c == '-' || c == '|' || c == '·' || c.is_whitespace())
            .to_string();
    }

    let segments = split_segments(&text);
    let mut prepared = Prepared {
        title: text.clone(),
        ..Prepared::default()
    };

    if rule.map(|r| r.media).unwrap_or(false) {
        let (artist, track) = match segments.len() {
            0 => (String::new(), String::new()),
            1 => (String::new(), segments[0].clone()),
            _ => match rule.map(|r| &r.media_order).unwrap_or(&MediaOrder::ArtistFirst) {
                MediaOrder::ArtistFirst => {
                    (segments[0].clone(), segments[1..].join(" - "))
                }
                MediaOrder::TitleFirst => {
                    (segments[1..].join(" - "), segments[0].clone())
                }
            },
        };
        prepared.track = track;
        prepared.artist = artist;
        prepared.title = prepared.media_line();
        prepared.project = prepared.title.clone();
        prepared.file = prepared.track.clone();
        return prepared;
    }

    let project_index = rule
        .and_then(|r| r.project_index)
        .unwrap_or(if segments.len() > 1 { 1 } else { 0 });
    let file_index = rule.and_then(|r| r.file_index).unwrap_or(0);
    prepared.project = segments
        .get(project_index)
        .cloned()
        .unwrap_or_else(|| prepared.title.clone());
    prepared.file = segments.get(file_index).cloned().unwrap_or_default();
    prepared
}

pub fn split_segments(text: &str) -> Vec<String> {
    for sep in SEPARATORS {
        if text.contains(sep) {
            return text
                .split(sep)
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
        }
    }
    if text.trim().is_empty() {
        Vec::new()
    } else {
        vec![text.trim().to_string()]
    }
}

/// The browser suffix list lives in the shipped rules; kept here only as a
/// reminder that title cleaning is rule-driven.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Block;

    fn media_rule() -> Rule {
        Rule {
            id: "spotify".into(),
            name: "Spotify".into(),
            category: "media".into(),
            priority: 50,
            process: vec!["spotify.exe".into()],
            process_display: vec!["Spotify.exe".into()],
            title_contains: vec![],
            title_all: vec![],
            title_excludes: vec![],
            title_regex: None,
            path_contains: vec![],
            requires_foreground: false,
            fullscreen_only: false,
            min_title_len: 0,
            strip_prefixes: vec![],
            strip_suffixes: vec![" | Spotify".into()],
            strip_regex: vec![],
            media: true,
            media_order: MediaOrder::ArtistFirst,
            project_index: None,
            file_index: None,
            block: Block::default(),
            buttons: vec![],
            no_elapsed: false,
        }
    }

    #[test]
    fn spotify() {
        let p = prepare(Some(&media_rule()), "Daft Punk - One More Time | Spotify");
        assert_eq!(p.artist, "Daft Punk");
        assert_eq!(p.track, "One More Time");
        assert_eq!(p.title, "Daft Punk — One More Time");
    }

    #[test]
    fn media_without_artist() {
        let mut rule = media_rule();
        rule.strip_suffixes = vec![" - VLC media player".into()];
        let p = prepare(Some(&rule), "movie.mkv - VLC media player");
        assert_eq!(p.artist, "");
        assert_eq!(p.track, "movie.mkv");
        assert_eq!(p.title, "movie.mkv");
    }

    #[test]
    fn segments() {
        assert_eq!(split_segments("main.rs - PulseRPC"), vec!["main.rs", "PulseRPC"]);
        assert_eq!(split_segments("solo"), vec!["solo"]);
    }
}

//! User preferences, stored as one readable JSON file.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    System,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [Self::Dark, Self::Light, Self::System];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "Follow system",
        }
    }
}

/// Where this computer plays audio. Spotify Connect (librespot) is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackBackend {
    #[default]
    Spotify,
    Alternate,
}

impl PlaybackBackend {
    pub const ALL: [PlaybackBackend; 2] = [Self::Spotify, Self::Alternate];

    pub fn label(self) -> &'static str {
        match self {
            Self::Spotify => "Spotify Connect",
            Self::Alternate => "Alternate local audio",
        }
    }
}

fn default_alternate_min_score() -> f32 {
    0.55
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The Spotify Connect name other devices see.
    pub device_name: String,
    /// 96, 160, or 320 kbps.
    pub bitrate: u16,
    pub normalisation: bool,
    pub autoplay: bool,
    pub gapless: bool,
    /// librespot backend name; `None` picks the platform default.
    pub audio_backend: Option<String>,
    pub audio_device: Option<String>,
    pub audio_cache: bool,
    pub audio_cache_mb: u64,
    pub theme: ThemeChoice,
    /// Tint the interface with the colour of the playing album's art.
    pub accent_from_art: bool,
    /// Last local volume, 0..=65535.
    pub volume: u16,
    pub sidebar_width: f32,
    pub search_history: Vec<String>,
    pub show_shortcut_hints: bool,
    /// A personal Spotify Web API application id, if the user registered one.
    /// `None` uses the shared public application.
    pub web_client_id: Option<String>,
    /// Local playback has been authorized at least once on this machine, so
    /// the app can resume it silently instead of prompting.
    pub playback_authorized: bool,
    /// Closing the window hides to the tray and keeps the music playing.
    pub keep_playing_in_background: bool,
    /// Ask GitHub once a day whether a newer release exists.
    pub check_for_updates: bool,
    /// Context URIs pinned to the top of the sidebar, in pin order.
    pub pinned_contexts: Vec<String>,
    /// Local audio engine. Missing from older files; defaults to Spotify Connect.
    pub playback_backend: PlaybackBackend,
    /// User-configured Piped-compatible API base URL. Empty means unused.
    pub piped_api_base: String,
    /// Optional path to a user-installed yt-dlp binary. Empty means PATH, or
    /// the official pinned build this app extracts locally.
    pub ytdlp_path: String,
    /// Minimum rank score (0..=1) required to play a third-party match.
    #[serde(default = "default_alternate_min_score")]
    pub alternate_min_score: f32,
    /// Skip to the next track when no match meets `alternate_min_score`.
    /// HTTP, stall, and decode failures still stop the current track.
    #[serde(default = "default_true")]
    pub alternate_skip_on_miss: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: "Fastpotify".to_string(),
            bitrate: 320,
            normalisation: false,
            autoplay: true,
            gapless: true,
            audio_backend: None,
            audio_device: None,
            audio_cache: true,
            audio_cache_mb: 1024,
            theme: ThemeChoice::Dark,
            accent_from_art: true,
            volume: (u16::MAX as u32 * 70 / 100) as u16,
            sidebar_width: 250.0,
            search_history: Vec::new(),
            show_shortcut_hints: true,
            web_client_id: None,
            playback_authorized: false,
            keep_playing_in_background: true,
            check_for_updates: true,
            pinned_contexts: Vec::new(),
            playback_backend: PlaybackBackend::Spotify,
            piped_api_base: String::new(),
            ytdlp_path: String::new(),
            alternate_min_score: default_alternate_min_score(),
            alternate_skip_on_miss: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut settings = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                log::warn!("settings at {} are unreadable: {error}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        settings.sanitize();
        settings
    }

    fn sanitize(&mut self) {
        if !self.alternate_min_score.is_finite() {
            self.alternate_min_score = default_alternate_min_score();
        }
        self.alternate_min_score = self.alternate_min_score.clamp(0.2, 0.95);
        self.piped_api_base = self.piped_api_base.trim().to_string();
        self.ytdlp_path = self.ytdlp_path.trim().to_string();
    }

    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = match serde_json::to_string_pretty(self) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("unable to encode settings: {error}");
                return;
            }
        };
        let temporary = path.with_extension("json.tmp");
        let written =
            std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, path));
        if let Err(error) = written {
            log::warn!("unable to save settings to {}: {error}", path.display());
        }
    }

    pub fn platform_backend(&self) -> Option<String> {
        self.audio_backend.clone().or_else(|| {
            if cfg!(target_os = "linux") {
                Some("pulseaudio".to_string())
            } else {
                None
            }
        })
    }

    pub fn remember_search(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.search_history.retain(|entry| entry != query);
        self.search_history.insert(0, query.to_string());
        self.search_history.truncate(12);
    }
}

/// Restorable UI session: what was open when the app last closed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    pub last_page: Option<String>,
    /// Context URIs most recently played, newest first.
    pub recent_contexts: Vec<String>,
    /// What was playing when the app closed, to resume from a cold start.
    pub last_context: Option<String>,
    pub last_track: Option<String>,
    pub last_position_ms: u32,
}

impl SessionState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_json_defaults_to_spotify_connect() {
        let parsed: Settings =
            serde_json::from_str(r#"{"device_name":"Desk","bitrate":160}"#).unwrap();
        assert_eq!(parsed.playback_backend, PlaybackBackend::Spotify);
        assert!(parsed.piped_api_base.is_empty());
        assert!(parsed.ytdlp_path.is_empty());
        assert!((parsed.alternate_min_score - 0.55).abs() < f32::EPSILON);
        assert!(parsed.alternate_skip_on_miss);
        assert_eq!(parsed.device_name, "Desk");
        assert_eq!(parsed.bitrate, 160);
    }

    #[test]
    fn default_settings_are_spotify_connect() {
        let settings = Settings::default();
        assert_eq!(settings.playback_backend, PlaybackBackend::Spotify);
        assert!((settings.alternate_min_score - 0.55).abs() < f32::EPSILON);
        assert!(settings.alternate_skip_on_miss);
    }

    #[test]
    fn alternate_backend_round_trips() {
        let settings = Settings {
            playback_backend: PlaybackBackend::Alternate,
            piped_api_base: "https://piped.example".into(),
            alternate_min_score: 0.7,
            alternate_skip_on_miss: false,
            ..Settings::default()
        };
        let text = serde_json::to_string(&settings).unwrap();
        let parsed: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.playback_backend, PlaybackBackend::Alternate);
        assert_eq!(parsed.piped_api_base, "https://piped.example");
        assert!((parsed.alternate_min_score - 0.7).abs() < f32::EPSILON);
        assert!(!parsed.alternate_skip_on_miss);
    }
}

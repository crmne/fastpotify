//! Opt-in alternate local playback: Spotify metadata, third-party audio.
//!
//! This is not Spotify Connect and it does not play Spotify audio. A user
//! who turns it on supplies a Piped-compatible API endpoint and/or yt-dlp
//! (an official pinned build extracted locally, or a strictly newer binary
//! they installed). Matches are ranked; low-confidence hits are never played.

mod audio;
mod buffer;
mod bundle;
mod decode;
mod engine;
mod fetch;
mod hydrate;
mod matching;
mod piped;
mod probe;
mod provider;
mod session;
mod streams;
mod ytdlp;

pub use bundle::has_bundled_ytdlp;
pub use engine::{AlternateHandle, spawn};
pub use hydrate::local_from_track;
pub use matching::{Candidate, TrackQuery, normalize_query, rank_candidates};
pub use session::Session;
pub use streams::{AudioStream, select_audio_stream};

use crate::settings::Settings;

const DEFAULT_MIN_SCORE: f32 = 0.55;

#[derive(Clone, Debug, PartialEq)]
pub struct AlternateConfig {
    pub piped_api_base: Option<String>,
    pub ytdlp_path: Option<String>,
    pub min_score: f32,
    pub skip_on_miss: bool,
    pub volume: u16,
}

impl AlternateConfig {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            piped_api_base: nonempty(settings.piped_api_base.trim()),
            ytdlp_path: nonempty(settings.ytdlp_path.trim()),
            min_score: if settings.alternate_min_score.is_finite() {
                settings.alternate_min_score.clamp(0.2, 0.95)
            } else {
                DEFAULT_MIN_SCORE
            },
            skip_on_miss: settings.alternate_skip_on_miss,
            volume: settings.volume,
        }
    }

    pub fn provider_error(&self) -> Option<String> {
        let piped_ok = self.piped_api_base.is_some();
        let ytdlp_ok = ytdlp_binary_available(self.ytdlp_path.as_deref());
        if piped_ok || ytdlp_ok {
            None
        } else {
            Some(
                "Alternate playback needs a Piped-compatible API base URL, or a yt-dlp executable on PATH (or a path you set). No public Piped instance is bundled.".into(),
            )
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(base) = &self.piped_api_base {
            validate_piped_base(base)?;
        }
        if let Some(path) = &self.ytdlp_path {
            validate_ytdlp_path(path)?;
        }
        match self.provider_error() {
            Some(message) => Err(message),
            None => Ok(()),
        }
    }

    pub fn source_summary(&self) -> String {
        match (
            self.piped_api_base.is_some(),
            ytdlp_binary_available(self.ytdlp_path.as_deref()),
        ) {
            (true, true) => "Piped, yt-dlp fallback".into(),
            (true, false) => "Piped".into(),
            (false, true) => "yt-dlp".into(),
            (false, false) => "not configured".into(),
        }
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

/// Reject credentials, non-http(s) schemes, and empty hosts.
pub fn validate_piped_base(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 500 {
        return Err("Piped API URL is too long.".into());
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|_| "Piped API URL must be a full http:// or https:// address.".to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("Piped API URL must use http or https.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Do not put usernames or passwords in the Piped API URL.".into());
    }
    if url.host_str().is_none() {
        return Err("Piped API URL needs a host name.".into());
    }
    if url.cannot_be_a_base() {
        return Err("Piped API URL is not a usable base address.".into());
    }
    Ok(Some(trimmed.trim_end_matches('/').to_string()))
}

/// Reject control characters and shell metacharacters in a binary path.
pub fn validate_ytdlp_path(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 512 {
        return Err("yt-dlp path is too long.".into());
    }
    if trimmed.chars().any(|ch| {
        ch.is_control() || matches!(ch, '|' | '&' | ';' | '<' | '>' | '$' | '`' | '\n' | '\r')
    }) {
        return Err("yt-dlp path contains characters that are not allowed.".into());
    }
    Ok(Some(trimmed.to_string()))
}

/// Syntax errors block Apply. A missing file is a warning: PATH and the bundle still work.
pub fn ytdlp_path_notice(raw: &str) -> Option<String> {
    match validate_ytdlp_path(raw) {
        Err(message) => Some(message),
        Ok(Some(path)) if !std::path::Path::new(&path).is_file() => Some(
            "That path is not a file. Fastpotify still uses yt-dlp on PATH or the official bundled build.".into(),
        ),
        _ => None,
    }
}

pub fn ytdlp_binary_available(configured: Option<&str>) -> bool {
    has_bundled_ytdlp() || bundle::user_ytdlp_present(configured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piped_url_rejects_credentials_and_bad_schemes() {
        assert!(validate_piped_base("").unwrap().is_none());
        assert!(validate_piped_base("https://piped.example/api").is_ok());
        assert!(validate_piped_base("http://127.0.0.1:8080").is_ok());
        assert!(validate_piped_base("https://user:pass@piped.example").is_err());
        assert!(validate_piped_base("file:///tmp/x").is_err());
        assert!(validate_piped_base("not a url").is_err());
    }

    #[test]
    fn ytdlp_path_rejects_shell_metacharacters() {
        assert!(validate_ytdlp_path("").unwrap().is_none());
        assert!(validate_ytdlp_path(r"C:\tools\yt-dlp.exe").is_ok());
        assert!(validate_ytdlp_path("/usr/bin/yt-dlp").is_ok());
        assert!(validate_ytdlp_path("yt-dlp; rm -rf /").is_err());
        assert!(validate_ytdlp_path("yt-dlp\n--cookies").is_err());
    }

    #[test]
    fn validate_rejects_credential_piped_url() {
        let settings = Settings {
            piped_api_base: "https://user:pass@piped.example".into(),
            ..Settings::default()
        };
        let config = AlternateConfig::from_settings(&settings);
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_from_settings_clamps_score() {
        let settings = Settings {
            alternate_min_score: 9.0,
            ..Settings::default()
        };
        let config = AlternateConfig::from_settings(&settings);
        assert!((config.min_score - 0.95).abs() < f32::EPSILON);
        assert_eq!(config.piped_api_base, None);
    }

    #[test]
    fn missing_ytdlp_path_does_not_block_piped() {
        let settings = Settings {
            piped_api_base: "https://piped.example".into(),
            ytdlp_path: "/no/such/fastpotify-ytdlp".into(),
            ..Settings::default()
        };
        let config = AlternateConfig::from_settings(&settings);
        assert!(config.validate().is_ok());
        assert!(ytdlp_path_notice(&settings.ytdlp_path).is_some());
    }

    #[test]
    fn missing_ytdlp_path_does_not_block_bundle() {
        if !has_bundled_ytdlp() {
            return;
        }
        let settings = Settings {
            ytdlp_path: "/no/such/fastpotify-ytdlp".into(),
            ..Settings::default()
        };
        let config = AlternateConfig::from_settings(&settings);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn ytdlp_syntax_still_blocks_even_with_piped() {
        let settings = Settings {
            piped_api_base: "https://piped.example".into(),
            ytdlp_path: "yt-dlp; rm -rf /".into(),
            ..Settings::default()
        };
        let config = AlternateConfig::from_settings(&settings);
        assert!(config.validate().is_err());
    }
}

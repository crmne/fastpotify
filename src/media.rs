//! Shared state and commands for desktop media controls.
//!
//! Platform modules translate this interface to MPRIS on Linux, System Media
//! Transport Controls on Windows, and Now Playing on macOS.

use std::path::PathBuf;

use crate::player::{Playback, RepeatMode};

#[derive(Clone, Debug, PartialEq)]
pub enum MediaCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    SeekBy(i64),
    SetPosition { track_uri: String, position_ms: u32 },
    SetVolume(f64),
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    OpenUri(String),
    Raise,
    Quit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaTrack {
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub art_url: Option<String>,
    /// The same artwork on disk, once the art cache holds it. Windows and
    /// macOS take this rather than `art_url`, because they load the image
    /// themselves; MPRIS only passes the URL along, so Linux keeps using
    /// `art_url`.
    pub art_file: Option<PathBuf>,
    pub duration_ms: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaState {
    pub playback: Playback,
    pub track: Option<MediaTrack>,
    pub position_ms: u32,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub can_control: bool,
}

impl Default for MediaState {
    fn default() -> Self {
        Self {
            playback: Playback::Stopped,
            track: None,
            position_ms: 0,
            volume: 1.0,
            shuffle: false,
            repeat: RepeatMode::Off,
            can_control: true,
        }
    }
}

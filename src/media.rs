//! What the desktop's own media controls say and hear.
//!
//! MPRIS on Linux, the System Media Transport Controls on Windows and Now
//! Playing on macOS answer the same questions, so the interface speaks this
//! vocabulary and each platform module translates it.

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

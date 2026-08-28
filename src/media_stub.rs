//! No-op desktop media controls for platforms with none of their own.

use crate::media::{MediaCommand, MediaState};

pub struct MediaControls;

impl MediaControls {
    pub fn spawn(_wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self
    }

    pub fn drain_commands(&self) -> Vec<MediaCommand> {
        Vec::new()
    }

    pub fn update(&mut self, _state: MediaState) {}

    pub fn seeked(&self, _position_ms: u32) {}
}

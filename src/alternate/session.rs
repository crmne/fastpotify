//! Queue and transport state for alternate local playback.

use std::collections::HashSet;
use std::time::Instant;

use rand::Rng;

use crate::player::{LocalState, LocalTrack, Playback, RepeatMode};

const PREVIOUS_RESTART_MS: u32 = 3_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    Stay,
    SeekZero,
    PlayCurrent,
    Stop,
    /// Toggle during Loading: cancel resolve, stay paused, no audio.
    CancelLoad,
}

#[derive(Clone, Debug)]
pub struct Session {
    tracks: Vec<LocalTrack>,
    order: Vec<usize>,
    index_in_order: usize,
    playback: Playback,
    position_ms: u32,
    position_at: Option<Instant>,
    volume: u16,
    shuffle: bool,
    repeat: RepeatMode,
    error: Option<String>,
    seek_sequence: u64,
    connected: bool,
    source_label: Option<String>,
    audio_ready: bool,
    manual: HashSet<usize>,
}

impl Session {
    pub fn new(volume: u16) -> Self {
        Self {
            tracks: Vec::new(),
            order: Vec::new(),
            index_in_order: 0,
            playback: Playback::Stopped,
            position_ms: 0,
            position_at: None,
            volume,
            shuffle: false,
            repeat: RepeatMode::Off,
            error: None,
            seek_sequence: 0,
            connected: true,
            source_label: None,
            audio_ready: false,
            manual: HashSet::new(),
        }
    }

    pub fn load(
        &mut self,
        tracks: Vec<LocalTrack>,
        offset: usize,
        play: bool,
        shuffle: Option<bool>,
        position_ms: u32,
    ) {
        self.tracks = tracks;
        if let Some(shuffle) = shuffle {
            self.shuffle = shuffle;
        }
        self.error = None;
        self.source_label = None;
        self.audio_ready = false;
        self.manual.clear();
        if self.tracks.is_empty() {
            self.order.clear();
            self.index_in_order = 0;
            self.playback = Playback::Stopped;
            self.position_ms = 0;
            self.position_at = None;
            return;
        }
        let current = offset.min(self.tracks.len() - 1);
        self.rebuild_order(current);
        self.position_ms = position_ms;
        self.position_at = None;
        self.playback = if play {
            Playback::Loading
        } else {
            Playback::Paused
        };
    }

    /// Replace the context list without restarting the current item.
    pub fn adopt_tracks(&mut self, tracks: Vec<LocalTrack>, offset: usize) {
        if tracks.is_empty() {
            return;
        }
        let current_uri = self.current().map(|track| track.uri.clone());
        let extras: Vec<LocalTrack> = self
            .manual
            .iter()
            .filter_map(|index| self.tracks.get(*index).cloned())
            .filter(|track| !tracks.iter().any(|known| known.uri == track.uri))
            .collect();
        let extra_count = extras.len();
        let mut tracks = tracks;
        tracks.extend(extras);
        let current = current_uri
            .and_then(|uri| tracks.iter().position(|track| track.uri == uri))
            .unwrap_or_else(|| offset.min(tracks.len() - 1));
        self.tracks = tracks;
        let n = self.tracks.len();
        self.manual = ((n - extra_count)..n).collect();
        self.rebuild_order(current);
    }

    pub fn add_to_queue(&mut self, track: LocalTrack) {
        let index = self.tracks.len();
        self.tracks.push(track);
        self.manual.insert(index);
        if self.order.is_empty() {
            self.order.push(index);
            self.index_in_order = 0;
            return;
        }
        let mut insert_at = self.index_in_order + 1;
        while insert_at < self.order.len() && self.manual.contains(&self.order[insert_at]) {
            insert_at += 1;
        }
        self.order.insert(insert_at, index);
    }

    pub fn toggle(&mut self) -> Advance {
        match self.playback {
            Playback::Playing => {
                self.sync_position();
                self.playback = Playback::Paused;
                self.position_at = None;
                Advance::Stay
            }
            Playback::Paused
                if self.current().is_some() && self.audio_ready && self.error.is_none() =>
            {
                self.playback = Playback::Playing;
                self.position_at = Some(Instant::now());
                Advance::Stay
            }
            Playback::Loading if self.current().is_some() => {
                self.playback = Playback::Paused;
                self.audio_ready = false;
                self.position_at = None;
                Advance::CancelLoad
            }
            Playback::Paused if self.current().is_some() => {
                self.playback = Playback::Loading;
                Advance::PlayCurrent
            }
            Playback::Stopped if self.current().is_some() => {
                self.playback = Playback::Loading;
                self.position_ms = 0;
                Advance::PlayCurrent
            }
            _ => Advance::Stay,
        }
    }

    pub fn skip_forward(&mut self) -> Advance {
        if self.tracks.is_empty() {
            return Advance::Stop;
        }
        if self.index_in_order + 1 < self.order.len() {
            self.index_in_order += 1;
            self.begin_current();
            Advance::PlayCurrent
        } else if self.repeat == RepeatMode::Context {
            self.index_in_order = 0;
            self.begin_current();
            Advance::PlayCurrent
        } else {
            self.playback = Playback::Stopped;
            self.position_ms = 0;
            self.position_at = None;
            self.source_label = None;
            Advance::Stop
        }
    }

    /// Skip after a resolve miss. Ignores repeat; never wraps; each item once.
    pub fn fail_next(&mut self) -> Advance {
        if self.tracks.is_empty() || self.index_in_order + 1 >= self.order.len() {
            self.playback = Playback::Paused;
            self.position_at = None;
            self.audio_ready = false;
            Advance::Stay
        } else {
            self.index_in_order += 1;
            self.begin_current();
            Advance::PlayCurrent
        }
    }

    pub fn previous(&mut self) -> Advance {
        self.sync_position();
        if self.position_ms > PREVIOUS_RESTART_MS {
            self.seek(0);
            return Advance::SeekZero;
        }
        if self.tracks.is_empty() {
            return Advance::Stop;
        }
        if self.index_in_order > 0 {
            self.index_in_order -= 1;
            self.begin_current();
            Advance::PlayCurrent
        } else if self.repeat == RepeatMode::Context && !self.order.is_empty() {
            self.index_in_order = self.order.len() - 1;
            self.begin_current();
            Advance::PlayCurrent
        } else {
            self.seek(0);
            Advance::SeekZero
        }
    }

    pub fn seek(&mut self, position_ms: u32) {
        let limit = self
            .current()
            .map(|track| track.duration_ms)
            .unwrap_or(u32::MAX);
        self.position_ms = position_ms.min(limit);
        self.position_at = (self.playback == Playback::Playing).then(Instant::now);
        self.seek_sequence = self.seek_sequence.wrapping_add(1);
    }

    pub fn set_volume(&mut self, volume: u16) {
        self.volume = volume;
    }

    pub fn set_shuffle(&mut self, enabled: bool) {
        if self.shuffle == enabled {
            return;
        }
        let current = self.current_index();
        self.shuffle = enabled;
        if let Some(current) = current {
            self.rebuild_order(current);
        }
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    pub fn set_loading(&mut self) {
        self.playback = Playback::Loading;
        self.position_at = None;
        self.error = None;
        self.audio_ready = false;
    }

    pub fn set_playing(&mut self, source_label: Option<String>) {
        self.playback = Playback::Playing;
        self.position_at = Some(Instant::now());
        self.error = None;
        self.source_label = source_label;
        self.audio_ready = true;
    }

    pub fn set_paused(&mut self) {
        self.sync_position();
        self.playback = Playback::Paused;
        self.position_at = None;
    }

    /// Stop interpolating time without changing playback. Used when PCM or
    /// the output device is gone; the user-visible position stays put.
    pub fn freeze_clock(&mut self) {
        self.sync_position();
        self.position_at = None;
    }

    pub fn resume_clock(&mut self) {
        if self.playback == Playback::Playing && self.position_at.is_none() {
            self.position_at = Some(Instant::now());
        }
    }

    pub fn clock_running(&self) -> bool {
        self.playback == Playback::Playing && self.position_at.is_some()
    }

    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
        self.playback = Playback::Paused;
        self.position_at = None;
        self.audio_ready = false;
    }

    pub fn stop(&mut self) {
        self.sync_position();
        self.playback = Playback::Stopped;
        self.position_at = None;
        self.position_ms = 0;
        self.tracks.clear();
        self.order.clear();
        self.index_in_order = 0;
        self.source_label = None;
        self.error = None;
        self.audio_ready = false;
        self.manual.clear();
    }

    pub fn on_ended(&mut self) -> Advance {
        if self.repeat == RepeatMode::Track {
            self.position_ms = 0;
            self.position_at = None;
            self.playback = Playback::Loading;
            Advance::PlayCurrent
        } else {
            self.skip_forward()
        }
    }

    pub fn current(&self) -> Option<&LocalTrack> {
        let index = *self.order.get(self.index_in_order)?;
        self.tracks.get(index)
    }

    pub fn current_index(&self) -> Option<usize> {
        self.order.get(self.index_in_order).copied()
    }

    pub fn playback(&self) -> Playback {
        self.playback
    }

    pub fn volume(&self) -> u16 {
        self.volume
    }

    pub fn position_now(&self) -> u32 {
        match (self.playback, self.position_at) {
            (Playback::Playing, Some(at)) => {
                let elapsed = at.elapsed().as_millis() as u32;
                let limit = self
                    .current()
                    .map_or(u32::MAX, |track| track.duration_ms.max(self.position_ms));
                self.position_ms.saturating_add(elapsed).min(limit)
            }
            _ => self.position_ms,
        }
    }

    pub fn snapshot(&self) -> LocalState {
        LocalState {
            playback: self.playback,
            track: self.current().cloned(),
            position_ms: self.position_now(),
            position_at: self.position_at,
            volume: self.volume,
            shuffle: self.shuffle,
            repeat: self.repeat,
            connected: self.connected,
            username: String::new(),
            active_client: String::new(),
            error: self.error.clone(),
            seek_sequence: self.seek_sequence,
            queue: self.upcoming(),
            source_label: self.source_label.clone(),
        }
    }

    fn upcoming(&self) -> Vec<LocalTrack> {
        if self.order.is_empty() {
            return Vec::new();
        }
        self.order
            .iter()
            .skip(self.index_in_order + 1)
            .filter_map(|index| self.tracks.get(*index).cloned())
            .collect()
    }

    fn begin_current(&mut self) {
        self.position_ms = 0;
        self.position_at = None;
        self.playback = Playback::Loading;
        self.error = None;
        self.source_label = None;
        self.audio_ready = false;
    }

    fn sync_position(&mut self) {
        self.position_ms = self.position_now();
        if self.playback != Playback::Playing {
            self.position_at = None;
        }
    }

    fn rebuild_order(&mut self, current: usize) {
        let n = self.tracks.len();
        if n == 0 {
            self.order.clear();
            self.index_in_order = 0;
            return;
        }
        let current = current.min(n - 1);
        if self.shuffle {
            let mut rest: Vec<usize> = (0..n).filter(|&index| index != current).collect();
            fisher_yates(&mut rest);
            let mut order = Vec::with_capacity(n);
            order.push(current);
            order.extend(rest);
            self.order = order;
            self.index_in_order = 0;
        } else {
            self.order = (0..n).collect();
            self.index_in_order = current;
        }
    }

    #[cfg(test)]
    fn force_order(&mut self, order: Vec<usize>, index_in_order: usize) {
        self.order = order;
        self.index_in_order = index_in_order;
    }

    #[cfg(test)]
    fn set_position_ms(&mut self, position_ms: u32) {
        self.position_ms = position_ms;
        self.position_at = None;
    }
}

fn fisher_yates(items: &mut [usize]) {
    if items.len() < 2 {
        return;
    }
    let mut rng = rand::rng();
    for i in (1..items.len()).rev() {
        let j = rng.random_range(0..=i);
        items.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(uri: &str, title: &str) -> LocalTrack {
        LocalTrack {
            uri: uri.into(),
            title: title.into(),
            artists: vec!["Artist".into()],
            album: "Album".into(),
            art_url: None,
            art_small_url: None,
            duration_ms: 180_000,
            is_episode: false,
        }
    }

    fn session_with(titles: &[&str], offset: usize) -> Session {
        let mut session = Session::new(1000);
        let tracks = titles
            .iter()
            .enumerate()
            .map(|(index, title)| track(&format!("spotify:track:{index}"), title))
            .collect();
        session.load(tracks, offset, true, Some(false), 0);
        session
    }

    #[test]
    fn previous_past_three_seconds_seeks_to_zero() {
        let mut session = session_with(&["a", "b", "c"], 1);
        session.set_playing(None);
        session.set_position_ms(4_000);
        assert_eq!(session.previous(), Advance::SeekZero);
        assert_eq!(session.position_now(), 0);
        assert_eq!(session.current().unwrap().title, "b");
    }

    #[test]
    fn previous_near_start_goes_back_a_track() {
        let mut session = session_with(&["a", "b", "c"], 1);
        session.set_playing(None);
        session.set_position_ms(500);
        assert_eq!(session.previous(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "a");
    }

    #[test]
    fn next_stops_at_end_when_repeat_is_off() {
        let mut session = session_with(&["a", "b"], 1);
        assert_eq!(session.skip_forward(), Advance::Stop);
        assert_eq!(session.playback(), Playback::Stopped);
    }

    #[test]
    fn next_wraps_when_repeat_context() {
        let mut session = session_with(&["a", "b"], 1);
        session.set_repeat(RepeatMode::Context);
        assert_eq!(session.skip_forward(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "a");
    }

    #[test]
    fn next_advances_even_when_repeat_track() {
        let mut session = session_with(&["a", "b"], 0);
        session.set_repeat(RepeatMode::Track);
        assert_eq!(session.skip_forward(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "b");
    }

    #[test]
    fn ended_respects_repeat_track() {
        let mut session = session_with(&["a", "b"], 0);
        session.set_repeat(RepeatMode::Track);
        assert_eq!(session.on_ended(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "a");
    }

    #[test]
    fn shuffle_keeps_current_and_contains_every_track() {
        let mut session = session_with(&["a", "b", "c", "d"], 2);
        session.set_shuffle(true);
        assert_eq!(session.current().unwrap().title, "c");
        let mut titles: Vec<_> = session
            .order
            .iter()
            .map(|index| session.tracks[*index].title.clone())
            .collect();
        titles.sort();
        assert_eq!(titles, ["a", "b", "c", "d"]);
        session.set_shuffle(false);
        assert_eq!(session.current().unwrap().title, "c");
        assert_eq!(session.order, vec![0, 1, 2, 3]);
    }

    #[test]
    fn add_to_queue_appends_upcoming() {
        let mut session = session_with(&["a"], 0);
        session.add_to_queue(track("spotify:track:z", "z"));
        let snap = session.snapshot();
        assert_eq!(snap.queue.len(), 1);
        assert_eq!(snap.queue[0].title, "z");
    }

    #[test]
    fn queued_tracks_play_before_remaining_context_fifo() {
        let mut session = session_with(&["a", "b", "c"], 0);
        session.add_to_queue(track("spotify:track:q1", "q1"));
        session.add_to_queue(track("spotify:track:q2", "q2"));
        let upcoming: Vec<_> = session
            .snapshot()
            .queue
            .into_iter()
            .map(|track| track.title)
            .collect();
        assert_eq!(upcoming, ["q1", "q2", "b", "c"]);
        assert_eq!(session.skip_forward(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "q1");
        assert_eq!(session.skip_forward(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "q2");
        assert_eq!(session.skip_forward(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "b");
    }

    #[test]
    fn pause_with_loaded_audio_resumes_without_reload() {
        let mut session = session_with(&["a", "b"], 0);
        session.set_playing(None);
        assert_eq!(session.toggle(), Advance::Stay);
        assert_eq!(session.playback(), Playback::Paused);
        assert_eq!(session.toggle(), Advance::Stay);
        assert_eq!(session.playback(), Playback::Playing);
    }

    #[test]
    fn pause_without_loaded_audio_retries() {
        let mut session = session_with(&["a"], 0);
        session.set_error("No confident match".into());
        assert_eq!(session.playback(), Playback::Paused);
        assert_eq!(session.toggle(), Advance::PlayCurrent);
    }

    #[test]
    fn toggle_while_loading_cancels_without_reload() {
        let mut session = session_with(&["a"], 0);
        assert_eq!(session.playback(), Playback::Loading);
        assert_eq!(session.toggle(), Advance::CancelLoad);
        assert_eq!(session.playback(), Playback::Paused);
        assert_eq!(session.toggle(), Advance::PlayCurrent);
    }

    #[test]
    fn fail_next_ignores_repeat_context_and_does_not_wrap() {
        let mut session = session_with(&["a", "b"], 0);
        session.set_repeat(RepeatMode::Context);
        session.set_error("miss a".into());
        assert_eq!(session.fail_next(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "b");
        session.set_error("miss b".into());
        assert_eq!(session.fail_next(), Advance::Stay);
        let snap = session.snapshot();
        assert_eq!(snap.playback, Playback::Paused);
        assert_eq!(snap.error.as_deref(), Some("miss b"));
        assert_eq!(session.current().unwrap().title, "b");
    }

    #[test]
    fn on_ended_still_wraps_with_repeat_context() {
        let mut session = session_with(&["a", "b"], 1);
        session.set_repeat(RepeatMode::Context);
        assert_eq!(session.on_ended(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "a");
    }

    #[test]
    fn shuffle_next_follows_forced_order() {
        let mut session = session_with(&["a", "b", "c"], 0);
        session.shuffle = true;
        session.force_order(vec![0, 2, 1], 0);
        assert_eq!(session.skip_forward(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "c");
        assert_eq!(session.skip_forward(), Advance::PlayCurrent);
        assert_eq!(session.current().unwrap().title, "b");
    }

    #[test]
    fn adopt_tracks_keeps_current_and_fills_queue() {
        let mut session = session_with(&["a"], 0);
        session.set_playing(None);
        session.adopt_tracks(
            vec![
                track("spotify:track:0", "a"),
                track("spotify:track:1", "b"),
                track("spotify:track:2", "c"),
            ],
            0,
        );
        assert_eq!(session.current().unwrap().title, "a");
        assert_eq!(session.playback(), Playback::Playing);
        let queue: Vec<_> = session
            .snapshot()
            .queue
            .into_iter()
            .map(|track| track.title)
            .collect();
        assert_eq!(queue, ["b", "c"]);
    }

    #[test]
    fn adopt_tracks_keeps_manual_queue_items() {
        let mut session = session_with(&["a"], 0);
        session.add_to_queue(track("spotify:track:z", "z"));
        session.adopt_tracks(
            vec![track("spotify:track:0", "a"), track("spotify:track:1", "b")],
            0,
        );
        let queue: Vec<_> = session
            .snapshot()
            .queue
            .into_iter()
            .map(|track| track.title)
            .collect();
        assert!(queue.contains(&"b".to_string()));
        assert!(queue.contains(&"z".to_string()));
    }

    #[test]
    fn snapshot_maps_queue_and_source() {
        let mut session = session_with(&["a", "b"], 0);
        session.set_playing(Some("Piped match".into()));
        let snap = session.snapshot();
        assert_eq!(snap.playback, Playback::Playing);
        assert_eq!(snap.source_label.as_deref(), Some("Piped match"));
        assert_eq!(snap.queue[0].title, "b");
        assert!(snap.connected);
    }

    #[test]
    fn freeze_clock_holds_position_until_resume() {
        let mut session = session_with(&["a"], 0);
        session.set_playing(None);
        session.set_position_ms(1_000);
        session.resume_clock();
        std::thread::sleep(std::time::Duration::from_millis(30));
        session.freeze_clock();
        let frozen = session.position_now();
        assert!(frozen >= 1_000);
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(session.position_now(), frozen);
        assert!(!session.clock_running());
        session.resume_clock();
        assert!(session.clock_running());
    }
}

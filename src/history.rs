//! What you actually listened to, kept here because Spotify does not keep
//! it for us.
//!
//! Spotify's own recently-played list is filled in by its official
//! clients reporting what they played. librespot, which is what plays
//! music here, reports nothing: it has no telemetry of any kind, and
//! Spotify offers no way for a client like this one to say "I played
//! that". So `/me/player/recently-played` knows every device you own
//! except this one, and a Recently played tab built only on it shows a
//! stranger's afternoon: whatever you last put on in the car.
//!
//! The two blind spots are exact opposites, though. Spotify knows the
//! other devices and not this one; this file knows this one and not the
//! others. Merged, the list is whole, which is why both are kept rather
//! than one replacing the other, and why nothing in the interface has to
//! apologise for it.
//!
//! A play is written down once the song has really been listened to
//! rather than passed over, so that skimming twenty songs does not bury
//! the one that was played.

use std::collections::HashMap;
use std::path::Path;

use crate::api::models::{Album, Image, PlayHistory, Track};

/// How long a song must play before it counts, or half its length if it
/// is shorter than a minute. This is the rule every scrobbler settles on:
/// long enough that skipping through a playlist writes nothing down,
/// short enough that a short song still counts.
const COUNTS_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// How many plays are kept. Enough to page back through a few weeks
/// without the file becoming something anyone has to think about.
const KEPT: usize = 500;

/// Two plays of the same song this close together are the same play seen
/// twice, not a song played twice. Only matters where the local history
/// and Spotify's overlap, which they should not, but clocks differ and a
/// duplicated row is worse than a missing one.
const SAME_PLAY: i64 = 60;

/// When a song has been listened to long enough to count.
pub fn counts_after(duration_ms: u32) -> std::time::Duration {
    let half = std::time::Duration::from_millis(u64::from(duration_ms) / 2);
    COUNTS_AFTER
        .min(half)
        .max(std::time::Duration::from_secs(1))
}

/// The plays made here, newest first.
#[derive(Default)]
pub struct History {
    plays: Vec<PlayHistory>,
    /// Set when the list has changed and the file has not caught up.
    dirty: bool,
}

impl History {
    /// Reads the history, or starts an empty one. A file that cannot be
    /// read is not worth a word to the listener: the history rebuilds
    /// itself by being used.
    pub fn load(path: &Path) -> Self {
        let plays = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<PlayHistory>>(&text).ok())
            .unwrap_or_default();
        Self {
            plays,
            dirty: false,
        }
    }

    pub fn plays(&self) -> &[PlayHistory] {
        &self.plays
    }

    pub fn is_empty(&self) -> bool {
        self.plays.is_empty()
    }

    /// Writes the history down if it has changed since it was last
    /// written.
    pub fn save(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string(&self.plays) {
            Ok(text) => {
                if let Err(error) = std::fs::write(path, text) {
                    log::warn!("could not write the play history: {error}");
                }
            }
            Err(error) => log::warn!("could not write the play history: {error}"),
        }
    }

    /// Writes down that `track` was played at `at`, newest first.
    pub fn record(&mut self, track: Track, at: jiff::Timestamp) {
        self.plays.insert(
            0,
            PlayHistory {
                track,
                played_at: Some(at.to_string()),
                context: None,
            },
        );
        self.plays.truncate(KEPT);
        self.dirty = true;
    }

    pub fn clear(&mut self) {
        if self.plays.is_empty() {
            return;
        }
        self.plays.clear();
        self.dirty = true;
    }
}

/// What the playing song looks like as something to write down.
///
/// The interface holds a song ready to draw rather than the record it
/// came from, so this puts back the parts a history row shows: the name,
/// who made it, the album it is from, and its cover.
pub fn played_track(now: &crate::app::NowPlaying) -> Track {
    Track {
        id: now.id.clone(),
        name: now.title.clone(),
        uri: now.uri.clone(),
        duration_ms: now.duration_ms,
        artists: now.artists.clone(),
        album: Some(Album {
            id: now.album_id.clone().unwrap_or_default(),
            name: now.album_name.clone(),
            images: now
                .art_url
                .as_deref()
                .or(now.art_small.as_deref())
                .map(|url| {
                    vec![Image {
                        url: url.to_string(),
                        width: None,
                        height: None,
                    }]
                })
                .unwrap_or_default(),
            ..Album::default()
        }),
        ..Track::default()
    }
}

/// The two histories as one list, newest first.
///
/// `local` is what was played here and `remote` is what Spotify knows,
/// which is every other device. They should not overlap at all, since
/// Spotify never hears about a play made here, but a song played on two
/// devices within a minute is written down once rather than twice.
///
/// A play with no time on it cannot be placed, so it sorts to the end
/// rather than to the top, where an unknown time would look like now.
pub fn merged(local: &[PlayHistory], remote: &[PlayHistory]) -> Vec<PlayHistory> {
    let mut seen: HashMap<String, Vec<i64>> = HashMap::new();
    let mut out: Vec<(Option<i64>, PlayHistory)> = Vec::new();
    for play in local.iter().chain(remote) {
        let at = play
            .played_at
            .as_deref()
            .and_then(|at| at.parse::<jiff::Timestamp>().ok())
            .map(|at| at.as_second());
        if let Some(at) = at {
            let times = seen.entry(play.track.uri.clone()).or_default();
            if times.iter().any(|held| (held - at).abs() <= SAME_PLAY) {
                continue;
            }
            times.push(at);
        }
        out.push((at, play.clone()));
    }
    out.sort_by(|a, b| match (a.0, b.0) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    out.into_iter().map(|(_, play)| play).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(uri: &str, at: &str) -> PlayHistory {
        PlayHistory {
            track: Track {
                uri: uri.to_string(),
                ..Track::default()
            },
            played_at: Some(at.to_string()),
            context: None,
        }
    }

    /// Rule: half a minute of listening counts, and half the song when
    /// the song is shorter than a minute, so a short song is reachable.
    #[test]
    fn a_song_counts_after_half_a_minute_or_half_of_it() {
        assert_eq!(counts_after(240_000).as_secs(), 30, "a four minute song");
        assert_eq!(counts_after(40_000).as_secs(), 20, "a forty second song");
        // Nothing counts instantly, however short the song claims to be.
        assert!(counts_after(0) >= std::time::Duration::from_secs(1));
    }

    /// Rule: a song already written down is never written down twice,
    /// however long it keeps playing. This guards the shape of the
    /// bookkeeping in `App::note_listening`, where an "already counted"
    /// marker made of a very large span used to overflow and panic on
    /// the next frame.
    #[test]
    fn counting_never_overflows_however_long_a_song_runs() {
        let threshold = counts_after(240_000);
        let mut listened = std::time::Duration::ZERO;
        let mut recorded = 0;
        // An hour of frames, well past the threshold, on a song nobody
        // stopped.
        for _ in 0..3_600 {
            listened += std::time::Duration::from_secs(1);
            if recorded == 0 && listened >= threshold {
                recorded += 1;
            }
        }
        assert_eq!(recorded, 1, "written down once, and it did not panic");
    }

    /// Rule: both histories, newest first, whichever they came from.
    #[test]
    fn the_two_histories_interleave_by_time() {
        let local = vec![
            play("spotify:track:here-late", "2026-09-01T15:00:00Z"),
            play("spotify:track:here-early", "2026-09-01T09:00:00Z"),
        ];
        let remote = vec![play("spotify:track:phone", "2026-09-01T12:00:00Z")];
        let rows = merged(&local, &remote);
        let uris: Vec<&str> = rows.iter().map(|play| play.track.uri.as_str()).collect();
        assert_eq!(
            uris,
            vec![
                "spotify:track:here-late",
                "spotify:track:phone",
                "spotify:track:here-early"
            ]
        );
    }

    /// Rule: the same song played twice really is two rows. A history
    /// that collapses repeats is not a history.
    #[test]
    fn the_same_song_played_twice_is_two_rows() {
        let local = vec![
            play("spotify:track:a", "2026-09-01T15:00:00Z"),
            play("spotify:track:a", "2026-09-01T09:00:00Z"),
        ];
        assert_eq!(merged(&local, &[]).len(), 2);
    }

    /// Rule: the same play seen from both sides is one row. It should
    /// never happen, since Spotify never hears about a play made here,
    /// but clocks differ and a doubled row is worse than a missing one.
    #[test]
    fn one_play_seen_twice_is_one_row() {
        let local = vec![play("spotify:track:a", "2026-09-01T15:00:00Z")];
        let remote = vec![play("spotify:track:a", "2026-09-01T15:00:20Z")];
        assert_eq!(merged(&local, &remote).len(), 1, "twenty seconds apart");
        let distant = vec![play("spotify:track:a", "2026-09-01T15:05:00Z")];
        assert_eq!(merged(&local, &distant).len(), 2, "five minutes apart");
    }

    /// Rule: a play with no time on it goes to the end. At the top an
    /// unknown time would read as "just now", which it is not.
    #[test]
    fn a_play_with_no_time_sinks_to_the_end() {
        let mut timeless = play("spotify:track:timeless", "");
        timeless.played_at = None;
        let local = vec![timeless, play("spotify:track:a", "2026-09-01T09:00:00Z")];
        let rows = merged(&local, &[]);
        let uris: Vec<&str> = rows.iter().map(|play| play.track.uri.as_str()).collect();
        assert_eq!(uris, vec!["spotify:track:a", "spotify:track:timeless"]);
    }

    /// Rule: the newest play is first, and the list does not grow for
    /// ever.
    #[test]
    fn the_newest_play_is_first_and_the_list_is_capped() {
        let mut history = History::default();
        let at: jiff::Timestamp = "2026-09-01T09:00:00Z".parse().unwrap();
        for index in 0..KEPT + 10 {
            history.record(
                Track {
                    uri: format!("spotify:track:{index}"),
                    ..Track::default()
                },
                at,
            );
        }
        assert_eq!(history.plays().len(), KEPT, "the oldest fall off the end");
        assert_eq!(
            history.plays()[0].track.uri,
            format!("spotify:track:{}", KEPT + 9),
            "the newest is first"
        );
    }
}

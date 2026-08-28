//! macOS media controls: Now Playing and the remote command centre.
//!
//! This is what puts Fastpotify on the media keys, in Control Centre, on the lock
//! screen, and behind the AirPods pinch. The command centre keeps its handler
//! blocks for the life of the process, so they post into a shared queue that
//! the interface drains on its own thread.
//!
//! Cover art arrives over the network. The fetch runs on a worker thread and
//! leaves bytes behind; the `NSImage` is built on the next update, which is
//! always the main thread.

use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::ClassType;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSImage;
use objc2_foundation::{CGSize, NSData, NSDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle,
    MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
    MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommandCenter,
    MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};

use crate::media::{MediaCommand, MediaState};
use crate::player::Playback;

/// How often the elapsed time is restated. macOS extrapolates between
/// updates from the playback rate, so this only has to correct drift.
const POSITION_REFRESH: Duration = Duration::from_secs(5);
const SKIP_INTERVAL_SECONDS: f64 = 10.0;
/// The square the artwork is published at. Control Centre and the lock
/// screen both scale down from it.
const ARTWORK_SIZE: f64 = 600.0;

static COMMANDS: Mutex<Vec<MediaCommand>> = Mutex::new(Vec::new());
static CURRENT_URI: Mutex<Option<String>> = Mutex::new(None);
#[allow(clippy::type_complexity)]
static WAKER: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

fn push(command: MediaCommand) {
    if let Ok(mut queue) = COMMANDS.lock() {
        queue.push(command);
    }
    let wake = WAKER.lock().ok().and_then(|waker| waker.clone());
    if let Some(wake) = wake {
        wake();
    }
}

/// Wraps `command` around the position the event carries, when there is a
/// track for it to apply to.
fn seek_command(position_seconds: f64) -> Option<MediaCommand> {
    let track_uri = CURRENT_URI.lock().ok()?.clone()?;
    Some(MediaCommand::SetPosition {
        track_uri,
        position_ms: (position_seconds.max(0.0) * 1000.0) as u32,
    })
}

/// The bytes of one cover, and the cover they were asked for.
#[derive(Default)]
struct Artwork {
    /// The cover `bytes` belong to, once a download has succeeded.
    url: Option<String>,
    bytes: Option<Vec<u8>>,
    /// The cover a download has been started for, whether it is still
    /// running, succeeded, or failed. One attempt per cover: a cover that
    /// will not download must not be retried on every frame.
    attempted: Option<String>,
}

pub struct MediaControls {
    published: Option<MediaState>,
    published_art: Option<String>,
    last_position_update: Instant,
    artwork: Arc<Mutex<Artwork>>,
}

impl MediaControls {
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        if let Ok(mut slot) = WAKER.lock() {
            *slot = Some(Arc::new(wake));
        }
        register_commands();
        Self {
            published: None,
            published_art: None,
            last_position_update: Instant::now() - POSITION_REFRESH,
            artwork: Arc::new(Mutex::new(Artwork::default())),
        }
    }

    pub fn drain_commands(&self) -> Vec<MediaCommand> {
        COMMANDS
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }

    pub fn update(&mut self, state: MediaState) {
        let art_url = state.track.as_ref().and_then(|track| track.art_url.clone());
        self.request_artwork(art_url.as_deref());

        let art_ready = self.ready_artwork_url();
        let position_due = self.last_position_update.elapsed() >= POSITION_REFRESH;
        let changed = match &self.published {
            Some(published) => !same_except_position(published, &state),
            None => true,
        };
        if !changed && !position_due && art_ready == self.published_art {
            return;
        }

        if let Ok(mut slot) = CURRENT_URI.lock() {
            *slot = state.track.as_ref().map(|track| track.uri.clone());
        }
        self.apply(&state, art_ready.is_some());
        self.published_art = art_ready;
        self.published = Some(state);
        self.last_position_update = Instant::now();
    }

    pub fn seeked(&self, _position_ms: u32) {
        // The next update carries the position; forcing one here would race
        // the state the interface has not applied yet.
    }

    fn apply(&self, state: &MediaState, with_artwork: bool) {
        let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
        let Some(track) = state.track.as_ref() else {
            unsafe {
                center.setNowPlayingInfo(None);
                center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
            }
            return;
        };

        let rate = if state.playback == Playback::Playing {
            1.0
        } else {
            0.0
        };
        let mut keys: Vec<&NSString> = vec![
            unsafe { MPMediaItemPropertyTitle },
            unsafe { MPMediaItemPropertyArtist },
            unsafe { MPMediaItemPropertyAlbumTitle },
            unsafe { MPMediaItemPropertyPlaybackDuration },
            unsafe { MPNowPlayingInfoPropertyElapsedPlaybackTime },
            unsafe { MPNowPlayingInfoPropertyPlaybackRate },
        ];
        let mut values: Vec<Retained<AnyObject>> = vec![
            into_object(NSString::from_str(&track.title)),
            into_object(NSString::from_str(&track.artists.join(", "))),
            into_object(NSString::from_str(&track.album)),
            into_object(NSNumber::new_f64(f64::from(track.duration_ms) / 1000.0)),
            into_object(NSNumber::new_f64(f64::from(state.position_ms) / 1000.0)),
            into_object(NSNumber::new_f64(rate)),
        ];
        if with_artwork && let Some(artwork) = self.build_artwork() {
            keys.push(unsafe { MPMediaItemPropertyArtwork });
            values.push(into_object(artwork));
        }

        let info = NSDictionary::from_vec(&keys, values);
        unsafe {
            center.setNowPlayingInfo(Some(&info));
            center.setPlaybackState(match state.playback {
                Playback::Playing | Playback::Loading => MPNowPlayingPlaybackState::Playing,
                Playback::Paused => MPNowPlayingPlaybackState::Paused,
                Playback::Stopped => MPNowPlayingPlaybackState::Stopped,
            });
        }
    }

    /// The URL whose bytes are downloaded and ready to publish.
    fn ready_artwork_url(&self) -> Option<String> {
        let artwork = self.artwork.lock().ok()?;
        artwork.bytes.as_ref()?;
        artwork.url.clone()
    }

    fn build_artwork(&self) -> Option<Retained<MPMediaItemArtwork>> {
        let bytes = self.artwork.lock().ok()?.bytes.clone()?;
        let data = NSData::with_bytes(&bytes);
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        let size = CGSize::new(ARTWORK_SIZE, ARTWORK_SIZE);
        // The handler is called back for whatever size the system wants; one
        // image answers every request.
        let handler = RcBlock::new(move |_: CGSize| NonNull::from(&*image));
        Some(unsafe {
            MPMediaItemArtwork::initWithBoundsSize_requestHandler(
                MPMediaItemArtwork::alloc(),
                size,
                &handler,
            )
        })
    }

    /// Starts a download when the cover changed, and forgets the old bytes.
    fn request_artwork(&self, url: Option<&str>) {
        let Ok(mut artwork) = self.artwork.lock() else {
            return;
        };
        let wanted = url.filter(|url| url.starts_with("http"));
        if artwork.attempted.as_deref() == wanted {
            return;
        }
        let Some(wanted) = wanted else {
            *artwork = Artwork::default();
            return;
        };
        artwork.bytes = None;
        artwork.url = None;
        artwork.attempted = Some(wanted.to_string());
        drop(artwork);

        let slot = Arc::clone(&self.artwork);
        let url = wanted.to_string();
        let spawned = std::thread::Builder::new()
            .name("fastpotify-now-playing-art".to_string())
            .spawn(move || {
                let bytes = reqwest::blocking::get(&url)
                    .and_then(|response| response.error_for_status())
                    .and_then(|response| response.bytes());
                let Ok(mut artwork) = slot.lock() else {
                    return;
                };
                if artwork.attempted.as_deref() != Some(url.as_str()) {
                    return;
                }
                match bytes {
                    Ok(bytes) => {
                        artwork.bytes = Some(bytes.to_vec());
                        artwork.url = Some(url);
                    }
                    Err(error) => log::debug!("no Now Playing artwork for {url}: {error}"),
                }
                drop(artwork);
                let wake = WAKER.lock().ok().and_then(|waker| waker.clone());
                if let Some(wake) = wake {
                    wake();
                }
            });
        if let Err(error) = spawned {
            log::warn!("unable to fetch Now Playing artwork: {error}");
        }
    }
}

fn into_object<T: objc2::Message>(value: Retained<T>) -> Retained<AnyObject> {
    unsafe { Retained::cast(value) }
}

fn same_except_position(left: &MediaState, right: &MediaState) -> bool {
    left.playback == right.playback
        && left.track == right.track
        && left.can_control == right.can_control
}

fn register_commands() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static REGISTERED: AtomicBool = AtomicBool::new(false);
    if REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }

    let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
    let simple: [(Retained<objc2_media_player::MPRemoteCommand>, MediaCommand); 6] = unsafe {
        [
            (center.playCommand(), MediaCommand::Play),
            (center.pauseCommand(), MediaCommand::Pause),
            (center.togglePlayPauseCommand(), MediaCommand::PlayPause),
            (center.stopCommand(), MediaCommand::Stop),
            (center.nextTrackCommand(), MediaCommand::Next),
            (center.previousTrackCommand(), MediaCommand::Previous),
        ]
    };
    for (command, message) in simple {
        let handler = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
            push(message.clone());
            MPRemoteCommandHandlerStatus::Success
        });
        unsafe {
            command.setEnabled(true);
            command.addTargetWithHandler(&handler);
        }
    }

    let skip_ms = (SKIP_INTERVAL_SECONDS * 1000.0) as i64;
    let intervals =
        objc2_foundation::NSArray::from_vec(vec![NSNumber::new_f64(SKIP_INTERVAL_SECONDS)]);
    for (command, offset) in unsafe {
        [
            (center.skipForwardCommand(), skip_ms),
            (center.skipBackwardCommand(), -skip_ms),
        ]
    } {
        let handler = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
            push(MediaCommand::SeekBy(offset));
            MPRemoteCommandHandlerStatus::Success
        });
        unsafe {
            command.setPreferredIntervals(&intervals);
            command.setEnabled(true);
            command.addTargetWithHandler(&handler);
        }
    }

    let scrub = unsafe { center.changePlaybackPositionCommand() };
    let handler = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
        let seconds = unsafe {
            let event: &MPChangePlaybackPositionCommandEvent = event.cast().as_ref();
            event.positionTime()
        };
        match seek_command(seconds) {
            Some(command) => {
                push(command);
                MPRemoteCommandHandlerStatus::Success
            }
            None => MPRemoteCommandHandlerStatus::NoActionableNowPlayingItem,
        }
    });
    unsafe {
        scrub.setEnabled(true);
        scrub.addTargetWithHandler(&handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::MediaTrack;

    fn controls() -> MediaControls {
        MediaControls {
            published: None,
            published_art: None,
            last_position_update: Instant::now(),
            artwork: Arc::new(Mutex::new(Artwork::default())),
        }
    }

    #[test]
    fn a_track_reaches_the_now_playing_centre_and_stopping_clears_it() {
        let state = MediaState {
            playback: Playback::Playing,
            track: Some(MediaTrack {
                uri: "spotify:track:abc".into(),
                title: "Blue Monday".into(),
                artists: vec!["New Order".into()],
                album: "Power, Corruption & Lies".into(),
                art_url: None,
                duration_ms: 448_000,
            }),
            position_ms: 12_000,
            ..MediaState::default()
        };
        controls().apply(&state, false);

        let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
        let info = unsafe { center.nowPlayingInfo() }.expect("the centre took the info");
        assert_eq!(
            text(&info, unsafe { MPMediaItemPropertyTitle }),
            Some("Blue Monday".to_string())
        );
        assert_eq!(
            text(&info, unsafe { MPMediaItemPropertyArtist }),
            Some("New Order".to_string())
        );
        assert_eq!(
            number(&info, unsafe { MPMediaItemPropertyPlaybackDuration }),
            Some(448.0)
        );
        assert_eq!(
            number(&info, unsafe { MPNowPlayingInfoPropertyPlaybackRate }),
            Some(1.0)
        );
        assert_eq!(
            unsafe { center.playbackState() },
            MPNowPlayingPlaybackState::Playing
        );

        // Same test: the centre is process-wide, so clearing it cannot be a
        // second one running alongside this.
        controls().apply(&MediaState::default(), false);
        assert!(unsafe { center.nowPlayingInfo() }.is_none());
        assert_eq!(
            unsafe { center.playbackState() },
            MPNowPlayingPlaybackState::Stopped
        );
    }

    fn text(info: &NSDictionary<NSString, AnyObject>, key: &NSString) -> Option<String> {
        let value = info.get(key)?;
        let value: &NSString = unsafe { &*(value as *const AnyObject).cast() };
        Some(value.to_string())
    }

    fn number(info: &NSDictionary<NSString, AnyObject>, key: &NSString) -> Option<f64> {
        let value = info.get(key)?;
        let value: &NSNumber = unsafe { &*(value as *const AnyObject).cast() };
        Some(value.as_f64())
    }
}

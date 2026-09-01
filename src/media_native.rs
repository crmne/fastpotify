//! Desktop media controls on Windows and macOS: the keyboard's media keys,
//! the system's now-playing panel, a headset's buttons. The same surface
//! `mpris.rs` gives Linux, through the platforms' own APIs (the System Media
//! Transport Controls; MPNowPlayingInfoCenter with MPRemoteCommandCenter) by
//! way of souvlaki.
//!
//! Windows ties the controls to a window, so they get a hidden one of their
//! own, on a thread with a message loop, and survive the app's real window
//! closing to the tray. macOS needs none of that: its handlers run on the
//! main thread, which the headless loop in `main` keeps pumping.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

use crate::media::{MediaCommand, MediaState};
use crate::player::Playback;

type Wake = Arc<dyn Fn() + Send + Sync>;

/// How far a seek button without an amount moves.
const SEEK_STEP_MS: i64 = 10_000;

#[cfg(any(target_os = "macos", test))]
fn should_claim_now_playing(claimed: bool, state: &MediaState) -> bool {
    !claimed && state.track.is_some() && state.playback != Playback::Playing
}

fn command_for(event: MediaControlEvent, track_uri: &str) -> Option<MediaCommand> {
    let step = |direction: SeekDirection, ms: i64| match direction {
        SeekDirection::Forward => MediaCommand::SeekBy(ms),
        SeekDirection::Backward => MediaCommand::SeekBy(-ms),
    };
    Some(match event {
        MediaControlEvent::Play => MediaCommand::Play,
        MediaControlEvent::Pause => MediaCommand::Pause,
        MediaControlEvent::Toggle => MediaCommand::PlayPause,
        MediaControlEvent::Next => MediaCommand::Next,
        MediaControlEvent::Previous => MediaCommand::Previous,
        MediaControlEvent::Stop => MediaCommand::Stop,
        MediaControlEvent::Seek(direction) => step(direction, SEEK_STEP_MS),
        MediaControlEvent::SeekBy(direction, amount) => {
            step(direction, amount.as_millis().min(i64::MAX as u128) as i64)
        }
        MediaControlEvent::SetPosition(position) => MediaCommand::SetPosition {
            track_uri: track_uri.to_string(),
            position_ms: position.0.as_millis().min(u32::MAX as u128) as u32,
        },
        MediaControlEvent::SetVolume(volume) => MediaCommand::SetVolume(volume),
        MediaControlEvent::OpenUri(uri) => MediaCommand::OpenUri(uri),
        MediaControlEvent::Raise => MediaCommand::Raise,
        MediaControlEvent::Quit => MediaCommand::Quit,
    })
}

/// The controls, and what they were last told, so only changes are sent.
struct Bridge {
    controls: MediaControls,
    last: MediaState,
    /// The playing track, for a "set position" request to name.
    track_uri: Arc<std::sync::Mutex<String>>,
    /// macOS routes media keys to the last active Now Playing owner. Merely
    /// attaching handlers does not make an app that owner.
    claimed: bool,
}

impl Bridge {
    fn new(
        hwnd: Option<*mut std::ffi::c_void>,
        sender: Sender<MediaCommand>,
        wake: Wake,
    ) -> Result<Self, String> {
        let mut controls = MediaControls::new(PlatformConfig {
            display_name: "Fastpotify",
            dbus_name: "fastpotify",
            hwnd,
        })
        .map_err(|error| error.to_string())?;
        let track_uri: Arc<std::sync::Mutex<String>> = Arc::default();
        let current = Arc::clone(&track_uri);
        controls
            .attach(move |event| {
                let uri = current.lock().unwrap_or_else(|p| p.into_inner()).clone();
                if let Some(command) = command_for(event, &uri)
                    && sender.send(command).is_ok()
                {
                    wake();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            controls,
            last: MediaState::default(),
            track_uri,
            claimed: false,
        })
    }

    fn apply(&mut self, state: MediaState) {
        let track_changed = state.track != self.last.track;
        if track_changed {
            let artist = state
                .track
                .as_ref()
                .map(|track| track.artists.join(", "))
                .unwrap_or_default();
            *self.track_uri.lock().unwrap_or_else(|p| p.into_inner()) = state
                .track
                .as_ref()
                .map(|track| track.uri.clone())
                .unwrap_or_default();
            let metadata = match &state.track {
                Some(track) => MediaMetadata {
                    title: Some(track.title.as_str()),
                    album: Some(track.album.as_str()),
                    artist: Some(artist.as_str()),
                    cover_url: track.art_url.as_deref(),
                    duration: Some(Duration::from_millis(u64::from(track.duration_ms))),
                },
                None => MediaMetadata::default(),
            };
            if let Err(error) = self.controls.set_metadata(metadata) {
                log::debug!("media controls refused the metadata: {error}");
            }
        }
        // A paused remembered track is useful: Play resumes it. macOS does
        // not route the keyboard to newly attached handlers until their Now
        // Playing centre has been active once, so establish ownership when
        // that track arrives, then immediately publish its truthful state.
        #[cfg(target_os = "macos")]
        if should_claim_now_playing(self.claimed, &state) {
            self.set_playback(Playback::Playing, state.position_ms);
            self.claimed = true;
        }
        if track_changed || state.playback != self.last.playback {
            self.set_playback(state.playback, state.position_ms);
        }
        if state.playback == Playback::Playing {
            self.claimed = true;
        }
        self.last = state;
    }

    fn seeked(&mut self, position_ms: u32) {
        self.last.position_ms = position_ms;
        self.set_playback(self.last.playback, position_ms);
    }

    fn set_playback(&mut self, playback: Playback, position_ms: u32) {
        let progress = Some(MediaPosition(Duration::from_millis(u64::from(position_ms))));
        let playback = match playback {
            Playback::Playing => MediaPlayback::Playing { progress },
            Playback::Paused | Playback::Loading => MediaPlayback::Paused { progress },
            Playback::Stopped => MediaPlayback::Stopped,
        };
        if let Err(error) = self.controls.set_playback(playback) {
            log::debug!("media controls refused the playback state: {error}");
        }
    }
}

/// What the app sends to the controls' thread.
#[cfg(windows)]
enum Update {
    State(MediaState),
    Seeked(u32),
}

#[cfg(windows)]
mod host {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, PostThreadMessageW,
        RegisterClassW, TranslateMessage, WM_APP, WNDCLASSW, WS_OVERLAPPED,
    };

    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A window that is never shown, for the controls to belong to.
    fn create_hidden_window() -> Result<HWND, String> {
        let class_name = wide("FastpotifyMediaControls");
        let title = wide("Fastpotify");
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(DefWindowProcW),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err("cannot register a window class".to_string());
        }
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            Err("cannot create a window".to_string())
        } else {
            Ok(hwnd)
        }
    }

    /// Runs the controls on their own thread. Answers with the thread's id
    /// once they exist, or with why they could not be made.
    pub fn start(
        sender: Sender<MediaCommand>,
        wake: Wake,
        updates: Receiver<Update>,
    ) -> Result<u32, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("fastpotify-media".to_owned())
            .spawn(move || {
                // The controls are WinRT objects, which want COM on the thread
                // that makes them; apartment-threaded, so their callbacks
                // arrive through the message loop below.
                unsafe {
                    CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
                }
                let mut bridge = match create_hidden_window()
                    .and_then(|hwnd| Bridge::new(Some(hwnd), sender, wake))
                {
                    Ok(bridge) => bridge,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
                let mut message: MSG = unsafe { std::mem::zeroed() };
                while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
                    if message.message == WM_APP {
                        while let Ok(update) = updates.try_recv() {
                            match update {
                                Update::State(state) => bridge.apply(state),
                                Update::Seeked(position_ms) => bridge.seeked(position_ms),
                            }
                        }
                        continue;
                    }
                    unsafe {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            });
        if let Err(error) = spawned {
            return Err(error.to_string());
        }
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "the media controls thread did not answer".to_string())?
    }

    /// Wakes the thread's message loop to read what was sent to it.
    pub fn poke(thread_id: u32) {
        unsafe {
            PostThreadMessageW(thread_id, WM_APP, 0, 0);
        }
    }
}

#[cfg(windows)]
pub struct MediaService {
    commands: Receiver<MediaCommand>,
    /// Where updates go, and the thread to wake for them; `None` when the
    /// controls could not be made.
    updates: Option<(Sender<Update>, u32)>,
}

#[cfg(windows)]
impl MediaService {
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (sender, commands) = std::sync::mpsc::channel();
        let (update_tx, update_rx) = std::sync::mpsc::channel();
        let updates = match host::start(sender, Arc::new(wake), update_rx) {
            Ok(thread_id) => Some((update_tx, thread_id)),
            Err(error) => {
                log::warn!("no media controls: {error}");
                None
            }
        };
        Self { commands, updates }
    }

    pub fn drain_commands(&self) -> Vec<MediaCommand> {
        self.commands.try_iter().collect()
    }

    pub fn update(&mut self, state: MediaState) {
        self.send(Update::State(state));
    }

    pub fn seeked(&self, position_ms: u32) {
        self.send(Update::Seeked(position_ms));
    }

    fn send(&self, update: Update) {
        if let Some((updates, thread_id)) = &self.updates
            && updates.send(update).is_ok()
        {
            host::poke(*thread_id);
        }
    }
}

#[cfg(target_os = "macos")]
pub struct MediaService {
    commands: Receiver<MediaCommand>,
    bridge: std::sync::Mutex<Option<Bridge>>,
}

#[cfg(target_os = "macos")]
impl MediaService {
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (sender, commands) = std::sync::mpsc::channel();
        let bridge = match Bridge::new(None, sender, Arc::new(wake)) {
            Ok(bridge) => Some(bridge),
            Err(error) => {
                log::warn!("no media controls: {error}");
                None
            }
        };
        Self {
            commands,
            bridge: std::sync::Mutex::new(bridge),
        }
    }

    pub fn drain_commands(&self) -> Vec<MediaCommand> {
        self.commands.try_iter().collect()
    }

    /// Makes a saved track the macOS Now Playing owner before its metadata
    /// has made a network round trip. The same service stays alive through
    /// normal, Winamp, tray-only, and MilkDrop-only window states.
    pub fn claim_resume(&mut self, track_uri: &str, position_ms: u32) {
        self.with_bridge(|bridge| {
            if bridge.claimed {
                return;
            }
            *bridge.track_uri.lock().unwrap_or_else(|p| p.into_inner()) = track_uri.to_owned();
            bridge.set_playback(Playback::Playing, position_ms);
            bridge.set_playback(Playback::Paused, position_ms);
            bridge.claimed = true;
        });
    }

    pub fn update(&mut self, state: MediaState) {
        self.with_bridge(|bridge| bridge.apply(state));
    }

    pub fn seeked(&self, position_ms: u32) {
        self.with_bridge(|bridge| bridge.seeked(position_ms));
    }

    fn with_bridge(&self, act: impl FnOnce(&mut Bridge)) {
        if let Some(bridge) = self
            .bridge
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            act(bridge);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remembered_paused_track_claims_the_media_keys_once() {
        let mut state = MediaState {
            track: Some(crate::media::MediaTrack {
                uri: "spotify:track:remembered".into(),
                ..Default::default()
            }),
            playback: Playback::Paused,
            ..Default::default()
        };
        assert!(should_claim_now_playing(false, &state));
        assert!(!should_claim_now_playing(true, &state));
        state.track = None;
        assert!(!should_claim_now_playing(false, &state));
    }
}

//! The application: state, event handling, and the actions views ask for.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use egui::Color32;

use crate::api::PlayRequest;
use crate::api::models::{
    ArtistRef, Device, PlayableItem, PlaybackState, Playlist, Queue, Track, User, pick_image,
};
use crate::backend::{
    ApiRequest, ApiResponse, AuthStatus, Backend, Command, Event, LocalPlayback, RemoteAction,
    Waker,
};
use crate::model::*;
use crate::mpris::{MprisCommand, MprisService, MprisState, MprisTrack};
use crate::paths::AppDirs;
use crate::player::{EngineConfig, LoadSpec, LocalState, Playback, PlayerCommand, RepeatMode};
use crate::settings::{SessionState, Settings, ThemeChoice};
use crate::theme::{self, Palette};
use crate::tray::{TrayCommand, TrayService};
use crate::util;

const REMOTE_POLL_ACTIVE: Duration = Duration::from_secs(4);
const REMOTE_POLL_IDLE: Duration = Duration::from_secs(20);
const REMOTE_FRESH: Duration = Duration::from_secs(45);
const DEVICES_FRESH: Duration = Duration::from_secs(12);
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(280);
const TOAST_LIFETIME: Duration = Duration::from_millis(3200);
const OPTIMISTIC_HOLD: Duration = Duration::from_millis(2500);
/// How long the interface trusts its own play/pause over a polled state that
/// has not caught up yet. Spotify can take a moment to report a command it
/// has already carried out, and a button that springs back looks broken.
const PLAYBACK_HOLD: Duration = Duration::from_secs(6);
/// A second look after a command, so the button settles quickly rather than
/// waiting for the ordinary poll.
const REMOTE_RECHECK: Duration = Duration::from_millis(1200);
const CONTAINS_BATCH: usize = 50;

pub struct RemoteSnapshot {
    pub state: PlaybackState,
    pub received_at: Instant,
}

/// The playing item as the interface sees it, whichever device plays it.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub local: bool,
    pub device_name: Option<String>,
    pub uri: String,
    pub id: Option<String>,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub subtitle: String,
    pub album_name: String,
    pub album_id: Option<String>,
    pub show_id: Option<String>,
    pub art_url: Option<String>,
    pub art_small: Option<String>,
    pub duration_ms: u32,
    pub position_ms: u32,
    pub playing: bool,
    pub loading: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub volume_percent: u8,
    pub can_control: bool,
    pub is_episode: bool,
    /// Streaming quality of the local player, e.g. "320 kbps OGG".
    pub quality: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Local,
    Remote(Option<String>),
}

/// How the application is being started.
#[derive(Clone, Copy, Debug)]
pub struct AppOptions {
    /// Register the MPRIS media-control service (Linux).
    pub mpris: bool,
    /// Register the system-tray item (Linux).
    pub tray: bool,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            mpris: true,
            tray: true,
        }
    }
}

pub struct App {
    pub dirs: AppDirs,
    pub settings: Settings,
    settings_dirty: bool,
    last_settings_save: Instant,
    pub backend: Backend,
    mpris: Option<MprisService>,
    tray: Option<TrayService>,
    pub window_hidden: bool,
    /// The window should close but the process should stay in the tray.
    pub hide_intent: bool,
    /// A hidden app was asked to show itself; the outer loop recreates the
    /// window.
    pub wants_show: bool,
    /// Set by a second launch that wants this window brought forward, on the
    /// platforms where that request does not arrive through MPRIS.
    show_requests: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Sample data is loaded and nothing is asked of Spotify.
    pub offline: bool,
    pub palette: Palette,
    applied_dark: Option<bool>,

    pub auth: AuthStatus,
    pub user: Option<User>,
    pub local_device_id: Option<String>,
    /// Local playback is authorized and the engine is connected.
    pub local_ready: bool,
    pub local_playback: LocalPlayback,
    pub local: LocalState,
    pub remote: Option<RemoteSnapshot>,
    remote_polled_at: Instant,
    remote_poll_pending: bool,
    pub devices: Vec<Device>,
    /// Receivers seen on the local network. Spotify lists a receiver only
    /// once it has an account, so these are the ones it cannot see yet.
    pub receivers: Vec<crate::zeroconf::Receiver>,
    /// The receiver currently being handed the account, by name.
    pub activating_receiver: Option<String>,
    pub devices_loading: bool,
    devices_fetched_at: Option<Instant>,
    pub selected_device: Option<String>,
    pub queue: Loadable<Queue>,
    queue_fetched_at: Option<Instant>,

    pub library: Library,
    pub home: HomeData,
    pub search: SearchState,
    pub playlist_pages: HashMap<String, PlaylistPage>,
    pub album_pages: HashMap<String, AlbumPage>,
    pub artist_pages: HashMap<String, ArtistPage>,
    pub show_pages: HashMap<String, ShowPage>,
    pub track_cache: HashMap<String, Track>,
    track_requests: HashSet<String>,

    pub history: Vec<Page>,
    pub history_index: usize,

    pub saved: HashMap<String, bool>,
    saved_pending: HashSet<String>,
    pub accents: HashMap<String, Color32>,
    accent_pending: HashSet<String>,

    pub dialog: Option<Dialog>,
    pub show_queue_panel: bool,
    pub show_devices: bool,
    /// A single-line now-playing strip instead of the full interface,
    /// opened and closed with a shortcut or a button in the player bar.
    pub mini_player: bool,
    pub toasts: Vec<Toast>,
    pub actions: Vec<Action>,
    volume_before_mute: Option<u8>,
    /// What was just asked to play, until Spotify visibly reacts: the keys
    /// (context and track URIs) whose play buttons show a spinner.
    pending_play_keys: Vec<String>,
    pending_play_at: Option<Instant>,
    /// A play request made while the local engine was still connecting; it
    /// starts the moment the engine reports ready.
    queued_play: Option<PlayRequest>,
    /// A receiver just activated, waiting for Spotify to list it so playback
    /// can move there.
    pending_transfer_to: Option<(String, Instant)>,
    /// When to take a confirming look at remote playback after a command.
    remote_recheck_at: Option<Instant>,
    pub seek_preview: Option<f32>,
    pub volume_preview: Option<f32>,
    /// When playback should pause automatically, or `None` without a timer.
    pub sleep_timer_end: Option<Instant>,
    /// A volume change just happened through the keyboard; show an overlay
    /// with the new level until `Instant` passes.
    pub volume_osd: Option<(u8, Instant)>,
    /// Restored window geometry to apply on the next attach, taken from the
    /// session file.
    session_window_size: Option<[f32; 2]>,
    session_window_pos: Option<[f32; 2]>,
    /// The album-art tint currently on screen, eased toward the latest
    /// requested colour so changing tracks or pages crossfades instead of
    /// snapping.
    pub tint_current: Option<Color32>,
    /// Last observed window geometry, saved to the session on close.
    last_window_size: Option<[f32; 2]>,
    last_window_pos: Option<[f32; 2]>,
    last_eviction: Instant,
    pub sign_in_url: Option<String>,
    pending_remote_position: Option<(u32, Instant)>,
    pending_remote_volume: Option<(u8, Instant)>,
    optimistic_playing: Option<(bool, Instant)>,
    last_now_playing_uri: Option<String>,
    pub playlist_busy: bool,
    pub quit_requested: bool,
}

impl App {
    pub fn new(waker: &Waker, dirs: AppDirs, settings: Settings, options: AppOptions) -> Self {
        let engine_config = engine_config(&dirs, &settings);
        let backend = Backend::spawn(
            dirs.clone(),
            engine_config,
            settings.web_client_id.clone(),
            waker.clone(),
        );
        let wake = waker.clone();
        let mpris = options
            .mpris
            .then(|| MprisService::spawn(move || wake.wake()));
        let wake = waker.clone();
        let tray = options
            .tray
            .then(|| TrayService::spawn(move || wake.wake()))
            .flatten();

        let session = SessionState::load(&dirs.session_file());
        let first_page = session
            .last_page
            .as_deref()
            .and_then(Page::decode)
            .filter(|page| !matches!(page, Page::Settings | Page::Queue))
            .unwrap_or(Page::Home);
        let session_window_size = session.window_size;
        let session_window_pos = session.window_pos;
        let session_queue_panel = session.show_queue_panel;

        let mut app = Self {
            dirs,
            settings,
            settings_dirty: false,
            last_settings_save: Instant::now(),
            backend,
            mpris,
            tray,
            window_hidden: false,
            hide_intent: false,
            wants_show: false,
            show_requests: None,
            offline: false,
            palette: Palette::dark(),
            applied_dark: None,
            auth: AuthStatus::Starting,
            user: None,
            local_device_id: None,
            local_ready: false,
            local_playback: LocalPlayback::Unavailable,
            local: LocalState::default(),
            remote: None,
            remote_polled_at: Instant::now() - REMOTE_POLL_IDLE,
            remote_poll_pending: false,
            devices: Vec::new(),
            receivers: Vec::new(),
            activating_receiver: None,
            devices_loading: false,
            devices_fetched_at: None,
            selected_device: None,
            queue: Loadable::NotLoaded,
            queue_fetched_at: None,
            library: Library::default(),
            home: HomeData::default(),
            search: SearchState::default(),
            playlist_pages: HashMap::new(),
            album_pages: HashMap::new(),
            artist_pages: HashMap::new(),
            show_pages: HashMap::new(),
            track_cache: HashMap::new(),
            track_requests: HashSet::new(),
            history: vec![first_page],
            history_index: 0,
            saved: HashMap::new(),
            saved_pending: HashSet::new(),
            accents: HashMap::new(),
            accent_pending: HashSet::new(),
            dialog: None,
            show_queue_panel: session_queue_panel,
            show_devices: false,
            mini_player: false,
            toasts: Vec::new(),
            actions: Vec::new(),
            volume_before_mute: None,
            pending_play_keys: Vec::new(),
            pending_play_at: None,
            queued_play: None,
            pending_transfer_to: None,
            remote_recheck_at: None,
            seek_preview: None,
            volume_preview: None,
            sleep_timer_end: None,
            volume_osd: None,
            session_window_size: None,
            session_window_pos: None,
            tint_current: None,
            last_window_size: None,
            last_window_pos: None,
            last_eviction: Instant::now(),
            sign_in_url: None,
            pending_remote_position: None,
            pending_remote_volume: None,
            optimistic_playing: None,
            last_now_playing_uri: None,
            playlist_busy: false,
            quit_requested: false,
        };
        app.local.volume = app.settings.volume;
        app.session_window_size = session_window_size;
        app.session_window_pos = session_window_pos;
        app
    }

    /// Watches the flag a second launch sets to ask for this window.
    pub fn set_show_requests(&mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.show_requests = Some(flag);
    }

    /// Per-window setup: fonts, icons, loaders, theme. Called every time a
    /// window is (re)created around this long-lived application state.
    pub fn attach(&mut self, ctx: &egui::Context) {
        theme::install(ctx);
        ctx.add_bytes_loader(std::sync::Arc::new(self.backend.art().clone()));
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::System => egui::ThemePreference::System,
        });
        self.applied_dark = None;
        self.window_hidden = false;
        self.hide_intent = false;
        self.wants_show = false;
        if let Some(size) = self.session_window_size.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(size[0], size[1])));
        }
        if let Some(pos) = self.session_window_pos.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(pos[0], pos[1])));
        }
    }

    // ---- derived state -----------------------------------------------------

    pub fn page(&self) -> &Page {
        &self.history[self.history_index]
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.auth, AuthStatus::Connected { .. })
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user.as_ref().map(|user| user.id.as_str())
    }

    pub fn is_saved(&self, uri: &str) -> Option<bool> {
        self.saved.get(uri).copied()
    }

    fn remote_fresh(&self) -> Option<&RemoteSnapshot> {
        self.remote
            .as_ref()
            .filter(|remote| remote.received_at.elapsed() < REMOTE_FRESH)
    }

    /// Where playback commands go: this computer's player or a remote device.
    pub fn target(&self) -> Target {
        if self.local_ready && self.local.is_active() {
            return Target::Local;
        }
        if let Some(selected) = &self.selected_device
            && Some(selected.as_str()) != self.local_device_id.as_deref()
        {
            return Target::Remote(Some(selected.clone()));
        }
        if let Some(remote) = self.remote_fresh() {
            let device = remote.state.device.as_ref();
            let is_local = device
                .and_then(|device| device.id.as_deref())
                .is_some_and(|id| Some(id) == self.local_device_id.as_deref());
            if !is_local && (remote.state.is_playing || remote.state.item.is_some()) {
                return Target::Remote(device.and_then(|device| device.id.clone()));
            }
        }
        if self.local_ready {
            Target::Local
        } else {
            Target::Remote(None)
        }
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        if self.local.is_active() {
            let track = self.local.track.as_ref()?;
            let cached = track
                .uri
                .rsplit(':')
                .next()
                .and_then(|id| self.track_cache.get(id));
            let artists = cached
                .map(|cached| cached.artists.clone())
                .unwrap_or_else(|| {
                    track
                        .artists
                        .iter()
                        .map(|name| ArtistRef {
                            id: None,
                            name: name.clone(),
                            uri: None,
                        })
                        .collect()
                });
            let playing = match self.optimistic_playing {
                Some((playing, at)) if at.elapsed() < PLAYBACK_HOLD => playing,
                _ => self.local.playback == Playback::Playing,
            };
            return Some(NowPlaying {
                local: true,
                device_name: None,
                uri: track.uri.clone(),
                id: util::uri_id(&track.uri).map(str::to_string),
                title: track.title.clone(),
                subtitle: track.artist_names(),
                artists,
                album_name: track.album.clone(),
                album_id: cached
                    .and_then(|cached| cached.album.as_ref())
                    .map(|album| album.id.clone()),
                show_id: None,
                art_url: track.art_url.clone(),
                art_small: track
                    .art_small_url
                    .clone()
                    .or_else(|| track.art_url.clone()),
                duration_ms: track.duration_ms,
                position_ms: self.local.position_now(),
                playing,
                loading: self.local.playback == Playback::Loading,
                shuffle: self.local.shuffle,
                repeat: self.local.repeat,
                volume_percent: volume_to_percent(self.local.volume),
                can_control: true,
                is_episode: track.is_episode,
                quality: track
                    .stream_format
                    .map(crate::player::format_quality_label),
            });
        }
        let remote = self.remote_fresh()?;
        let item = remote.state.item.as_ref()?;
        let device = remote.state.device.as_ref();
        let playing = match self.optimistic_playing {
            Some((playing, at)) if at.elapsed() < PLAYBACK_HOLD => playing,
            _ => remote.state.is_playing,
        };
        let position = match self.pending_remote_position {
            Some((position, at)) if at.elapsed() < OPTIMISTIC_HOLD => position,
            _ => {
                let base = remote.state.progress_ms.unwrap_or(0);
                if remote.state.is_playing {
                    (base as u64 + remote.received_at.elapsed().as_millis() as u64)
                        .min(item.duration_ms() as u64) as u32
                } else {
                    base
                }
            }
        };
        let volume = match self.pending_remote_volume {
            Some((volume, at)) if at.elapsed() < OPTIMISTIC_HOLD => volume,
            _ => device
                .and_then(|device| device.volume_percent)
                .unwrap_or(50),
        };
        let (artists, album_name, album_id, show_id, is_episode) = match item {
            PlayableItem::Track(track) => (
                track.artists.clone(),
                track
                    .album
                    .as_ref()
                    .map(|album| album.name.clone())
                    .unwrap_or_default(),
                track.album.as_ref().map(|album| album.id.clone()),
                None,
                false,
            ),
            PlayableItem::Episode(episode) => (
                Vec::new(),
                episode
                    .show
                    .as_ref()
                    .map(|show| show.name.clone())
                    .unwrap_or_default(),
                None,
                episode.show.as_ref().map(|show| show.id.clone()),
                true,
            ),
        };
        Some(NowPlaying {
            local: false,
            device_name: device.map(|device| device.name.clone()),
            uri: item.uri().to_string(),
            id: item.id().map(str::to_string),
            title: item.name().to_string(),
            subtitle: item.subtitle(),
            artists,
            album_name,
            album_id,
            show_id,
            art_url: item.image(640).map(str::to_string),
            art_small: item.image(64).map(str::to_string),
            duration_ms: item.duration_ms(),
            position_ms: position,
            playing,
            loading: false,
            shuffle: remote.state.shuffle_state,
            repeat: RepeatMode::from_api(&remote.state.repeat_state),
            volume_percent: volume,
            can_control: device.is_none_or(|device| !device.is_restricted),
            is_episode,
            quality: None,
        })
    }

    /// The play request for `key` (a context or track URI) is still waiting
    /// for Spotify to react.
    pub fn play_pending(&self, key: &str) -> bool {
        self.pending_fresh() && self.pending_play_keys.iter().any(|k| k == key)
    }

    pub fn any_play_pending(&self) -> bool {
        self.pending_fresh() && !self.pending_play_keys.is_empty()
    }

    fn pending_fresh(&self) -> bool {
        // A request queued behind a connecting engine stays pending for as
        // long as the engine may take; an ordinary request times out fast.
        self.queued_play.is_some()
            || self
                .pending_play_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(8))
    }

    fn set_play_pending(&mut self, keys: Vec<String>) {
        self.pending_play_keys = keys;
        self.pending_play_at = Some(Instant::now());
    }

    fn clear_play_pending(&mut self) {
        self.pending_play_keys.clear();
        self.pending_play_at = None;
    }

    /// The colour to tint the interface with, from the playing art.
    pub fn now_playing_tint(&self) -> Option<Color32> {
        if !self.settings.accent_from_art {
            return None;
        }
        let now = self.now_playing()?;
        let url = now.art_small.or(now.art_url)?;
        self.accents.get(&url).copied()
    }

    pub fn tint_for(&mut self, url: Option<&str>) -> Option<Color32> {
        let url = url?;
        if let Some(color) = self.accents.get(url) {
            return Some(*color);
        }
        if self.accent_pending.insert(url.to_string()) {
            self.backend.send(Command::Accent {
                url: url.to_string(),
            });
        }
        None
    }

    /// Arms the sleep timer for `minutes` from now. A paused track stays
    /// paused; the timer pauses whatever is playing when it expires.
    pub fn set_sleep_timer(&mut self, minutes: u64) {
        self.sleep_timer_end = Some(Instant::now() + Duration::from_secs(minutes * 60));
    }

    pub fn cancel_sleep_timer(&mut self) {
        self.sleep_timer_end = None;
    }

    /// Toggles the compact single-line player. When it opens, the window is
    /// resized to just fit it; leaving restores the previous size.
    pub fn toggle_mini_player(&mut self, ctx: &egui::Context) {
        self.mini_player = !self.mini_player;
        let size = if self.mini_player {
            egui::vec2(520.0, 96.0)
        } else {
            egui::vec2(1240.0, 800.0)
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    }

    /// Remaining sleep-timer time, or `None` when no timer is armed.
    pub fn sleep_timer_left(&self) -> Option<Duration> {
        self.sleep_timer_end
            .map(|end| end.saturating_duration_since(Instant::now()))
    }

    /// Pauses playback when the sleep timer expires. Called once per frame.
    pub fn tick_sleep_timer(&mut self) {
        if self.sleep_timer_end.is_none() {
            return;
        }
        let expired = self
            .sleep_timer_end
            .is_some_and(|end| Instant::now() >= end);
        if !expired {
            return;
        }
        self.sleep_timer_end = None;
        let was_playing = self.now_playing().is_some_and(|now| now.playing);
        if was_playing {
            self.actions.push(Action::TogglePlay);
        }
        self.toast("Sleep timer: playback paused.");
    }

    /// Eases `tint_current` toward the tint the interface wants this frame.
    /// Returns what should be drawn, crossfading over a few frames so a
    /// track change or page switch never snaps the background colour.
    pub fn eased_tint(&mut self, ctx: &egui::Context, wanted: Option<Color32>) -> Option<Color32> {
        let Some(wanted) = wanted else {
            self.tint_current = None;
            return None;
        };
        let current = self.tint_current;
        match (current, wanted) {
            (None, wanted) => {
                self.tint_current = Some(wanted);
                Some(wanted)
            }
            (Some(current), wanted) if current == wanted => Some(current),
            (Some(current), wanted) => {
                let t = ctx.animate_value_with_time(
                    egui::Id::new("tint-crossfade"),
                    (current != wanted) as u8 as f32,
                    0.3,
                );
                if t <= 0.0 {
                    self.tint_current = Some(wanted);
                    Some(wanted)
                } else {
                    let eased = crate::theme::ease_out(t);
                    let mixed = crate::ui::blend(current, wanted, eased);
                    self.tint_current = Some(mixed);
                    Some(mixed)
                }
            }
        }
    }

    // ---- frame ---------------------------------------------------------------

    fn handle_events(&mut self) {
        for event in self.backend.poll() {
            if self.offline {
                continue;
            }
            match event {
                Event::Auth(status) => self.handle_auth(status),
                Event::Playback(status) => self.handle_playback(status),
                Event::Receivers(receivers) => self.receivers = receivers,
                Event::ReceiverActivated { name, result } => {
                    self.activating_receiver = None;
                    match result {
                        Ok(()) => {
                            self.toast(format!("{name} is ready"));
                            // It takes a moment to appear in the device list.
                            self.pending_transfer_to = Some((name, Instant::now()));
                            self.devices_fetched_at = None;
                            self.refresh_devices();
                        }
                        Err(error) => self.toast_error(format!("{name}: {error}")),
                    }
                }
                Event::Local(state) => self.handle_local(*state),
                Event::Api(response) => self.handle_api(*response),
                Event::Accent { url, color } => {
                    self.accent_pending.remove(&url);
                    let tint = self.palette.tint_from_art(color);
                    self.accents.insert(url, tint);
                }
                Event::Error(message) => self.toast_error(message),
            }
        }
    }

    fn handle_auth(&mut self, status: AuthStatus) {
        match &status {
            AuthStatus::Connected { .. } => {
                self.sign_in_url = None;
                self.reset_data();
                self.load_playlists();
                self.ensure_loaded(self.page().clone());
                self.poll_remote(true);
            }
            AuthStatus::WaitingForBrowser { url } => self.sign_in_url = Some(url.clone()),
            AuthStatus::SignedOut => {
                self.sign_in_url = None;
                self.user = None;
                self.local = LocalState::default();
                self.local_ready = false;
                self.local_device_id = None;
                self.local_playback = LocalPlayback::Unavailable;
                self.remote = None;
                self.reset_data();
            }
            AuthStatus::Failed(message) => {
                self.sign_in_url = None;
                self.toast_error(message.clone());
            }
            _ => {}
        }
        self.auth = status;
    }

    fn handle_playback(&mut self, status: LocalPlayback) {
        match &status {
            LocalPlayback::Ready { device_id } => {
                self.local_device_id = Some(device_id.clone());
                self.local_ready = true;
                if let Some(request) = self.queued_play.take() {
                    self.play_request(request);
                }
            }
            LocalPlayback::Unavailable => {
                self.local_ready = false;
                self.local_device_id = None;
            }
            LocalPlayback::Failed(message) => {
                self.local_ready = false;
                if self.queued_play.take().is_some() {
                    self.clear_play_pending();
                }
                self.toast_error(format!("Local playback: {message}"));
            }
            LocalPlayback::Authorizing | LocalPlayback::Connecting => {}
        }
        self.local_playback = status;
    }

    fn reset_data(&mut self) {
        self.library = Library::default();
        self.home = HomeData::default();
        self.playlist_pages.clear();
        self.album_pages.clear();
        self.artist_pages.clear();
        self.show_pages.clear();
        self.saved.clear();
        self.saved_pending.clear();
        self.queue = Loadable::NotLoaded;
        self.devices.clear();
        self.devices_fetched_at = None;
        self.search.results = Loadable::NotLoaded;
        self.search.committed.clear();
    }

    fn handle_local(&mut self, state: LocalState) {
        let track_changed = state.track != self.local.track;
        if state.playback != self.local.playback {
            self.optimistic_playing = None;
            if matches!(state.playback, Playback::Playing | Playback::Loading) {
                self.clear_play_pending();
            }
        }
        if state.track != self.local.track {
            self.clear_play_pending();
        }
        if state.volume != self.local.volume {
            self.settings.volume = state.volume;
            self.settings_dirty = true;
        }
        if state.seek_sequence != self.local.seek_sequence
            && let Some(mpris) = &self.mpris
        {
            mpris.seeked(state.position_ms);
        }
        if let Some(error) = &state.error
            && self.local.error.as_deref() != Some(error.as_str())
        {
            self.toast_error(error.clone());
        }
        self.local = state;
        if track_changed {
            self.on_now_playing_changed();
        }
    }

    fn on_now_playing_changed(&mut self) {
        let Some(now) = self.now_playing() else {
            return;
        };
        if self.last_now_playing_uri.as_deref() == Some(now.uri.as_str()) {
            return;
        }
        self.last_now_playing_uri = Some(now.uri.clone());
        if now.local
            && !now.is_episode
            && let Some(id) = &now.id
            && !self.track_cache.contains_key(id)
            && self.track_requests.insert(id.clone())
        {
            self.backend.api(ApiRequest::Track { id: id.clone() });
        }
        self.request_contains(vec![now.uri.clone()]);
        if let Some(url) = now.art_small.or(now.art_url) {
            self.tint_for(Some(&url));
        }
        if matches!(self.page(), Page::Queue) || self.show_queue_panel {
            self.refresh_queue(true);
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts
            .retain(|toast| toast.created.elapsed() < TOAST_LIFETIME);

        if self.is_connected() && !self.offline {
            let interval = match self.target() {
                Target::Local if self.local.is_active() => REMOTE_POLL_IDLE,
                _ => REMOTE_POLL_ACTIVE,
            };
            if !self.remote_poll_pending && self.remote_polled_at.elapsed() >= interval {
                self.poll_remote(false);
            }
            if let Some(due) = self.remote_recheck_at
                && Instant::now() >= due
            {
                self.remote_recheck_at = None;
                self.poll_remote(true);
            }
            if self.show_devices
                && !self.devices_loading
                && self
                    .devices_fetched_at
                    .is_none_or(|at| at.elapsed() > DEVICES_FRESH)
            {
                self.refresh_devices();
            }
            if (self.show_queue_panel || matches!(self.page(), Page::Queue))
                && !self.queue.is_loading()
                && self
                    .queue_fetched_at
                    .is_none_or(|at| at.elapsed() > Duration::from_secs(20))
            {
                self.refresh_queue(false);
            }
        }

        if let Some(typed) = self.search.typed_at {
            if typed.elapsed() >= SEARCH_DEBOUNCE {
                self.search.typed_at = None;
                let query = self.search.query.trim().to_string();
                self.run_search(query);
            } else {
                ctx.request_repaint_after(SEARCH_DEBOUNCE - typed.elapsed());
            }
        }

        if self.last_eviction.elapsed() > Duration::from_secs(20) {
            self.last_eviction = now;
            self.backend.art().evict_stale(ctx);
        }
        if self.settings_dirty && self.last_settings_save.elapsed() > Duration::from_secs(2) {
            self.save_settings();
        }
    }

    fn save_settings(&mut self) {
        self.settings_dirty = false;
        self.last_settings_save = Instant::now();
        if self.offline {
            // Demo data must never overwrite the person's real preferences.
            return;
        }
        self.settings.save(&self.dirs.settings_file());
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let dark = ctx.theme() == egui::Theme::Dark;
        if self.applied_dark != Some(dark) {
            self.palette = if dark {
                Palette::dark()
            } else {
                Palette::light()
            };
            theme::apply(ctx, &self.palette);
            self.applied_dark = Some(dark);
            self.accents.clear();
            self.accent_pending.clear();
        }
    }

    fn handle_tray(&mut self) {
        let Some(commands) = self.tray.as_ref().map(TrayService::drain_commands) else {
            return;
        };
        for command in commands {
            match command {
                TrayCommand::ShowHide => self.actions.push(if self.window_hidden {
                    Action::ShowWindow
                } else {
                    Action::HideWindow
                }),
                TrayCommand::PlayPause => self.actions.push(Action::TogglePlay),
                TrayCommand::Next => self.actions.push(Action::Next),
                TrayCommand::Previous => self.actions.push(Action::Previous),
                TrayCommand::Quit => self.actions.push(Action::Quit),
            }
        }
    }

    fn handle_mpris(&mut self) {
        let Some(commands) = self.mpris.as_ref().map(MprisService::drain_commands) else {
            return;
        };
        for command in commands {
            let playing = self.now_playing().is_some_and(|now| now.playing);
            let action = match command {
                MprisCommand::Play => (!playing).then_some(Action::TogglePlay),
                MprisCommand::Pause | MprisCommand::Stop => playing.then_some(Action::TogglePlay),
                MprisCommand::PlayPause => Some(Action::TogglePlay),
                MprisCommand::Next => Some(Action::Next),
                MprisCommand::Previous => Some(Action::Previous),
                MprisCommand::SeekBy(offset) => Some(Action::SeekBy(offset)),
                MprisCommand::SetPosition {
                    track_uri,
                    position_ms,
                } => self
                    .now_playing()
                    .filter(|now| now.uri == track_uri)
                    .map(|_| Action::Seek(position_ms)),
                MprisCommand::SetVolume(volume) => Some(Action::SetVolume(
                    (volume.clamp(0.0, 1.0) * 100.0).round() as u8,
                )),
                MprisCommand::SetShuffle(shuffle) => Some(Action::SetShuffle(shuffle)),
                MprisCommand::SetRepeat(mode) => Some(Action::SetRepeat(mode)),
                MprisCommand::OpenUri(uri) => Some(Action::PlayContext {
                    uri,
                    offset_uri: None,
                    offset_index: None,
                }),
                MprisCommand::Raise => Some(Action::ShowWindow),
                MprisCommand::Quit => Some(Action::Quit),
            };
            if let Some(action) = action {
                self.actions.push(action);
            }
        }
    }

    fn sync_mpris(&mut self) {
        let state = match self.now_playing() {
            Some(now) => MprisState {
                playback: if now.playing {
                    Playback::Playing
                } else if now.loading {
                    Playback::Loading
                } else {
                    Playback::Paused
                },
                track: Some(MprisTrack {
                    uri: now.uri.clone(),
                    title: now.title.clone(),
                    artists: now
                        .artists
                        .iter()
                        .map(|artist| artist.name.clone())
                        .collect(),
                    album: now.album_name.clone(),
                    art_url: now.art_url.clone(),
                    duration_ms: now.duration_ms,
                }),
                position_ms: now.position_ms,
                volume: f64::from(now.volume_percent) / 100.0,
                shuffle: now.shuffle,
                repeat: now.repeat,
                can_control: now.can_control,
            },
            None => MprisState::default(),
        };
        if let Some(mpris) = &mut self.mpris {
            mpris.update(state);
        }
        let playing = self.now_playing().is_some_and(|now| now.playing);
        if let Some(tray) = &mut self.tray {
            tray.set_playing(playing);
        }
    }

    // ---- loading ---------------------------------------------------------------

    fn load_playlists(&mut self) {
        if self.library.playlists.is_loading() {
            return;
        }
        self.library.playlists = Loadable::Loading;
        self.library.playlists_next = None;
        self.backend.api(ApiRequest::MyPlaylists { offset: 0 });
    }

    pub fn ensure_loaded(&mut self, page: Page) {
        if !self.is_connected() {
            return;
        }
        match page {
            Page::Home => self.load_home(false),
            Page::Search => {}
            Page::LikedSongs => {
                if !self.library.liked.loaded_once {
                    self.load_more(Page::LikedSongs);
                }
            }
            Page::Albums => {
                if !self.library.albums.loaded_once {
                    self.load_more(Page::Albums);
                }
            }
            Page::Artists => {
                if !self.library.artists.loaded_once {
                    self.load_more(Page::Artists);
                }
            }
            Page::Podcasts => {
                if !self.library.shows.loaded_once {
                    self.load_more(Page::Podcasts);
                }
            }
            Page::Episodes => {
                if !self.library.episodes.loaded_once {
                    self.load_more(Page::Episodes);
                }
            }
            Page::Playlist(id) => {
                let page = self.playlist_pages.entry(id.clone()).or_default();
                if page.playlist.needs_load() {
                    page.playlist = Loadable::Loading;
                    self.backend.api(ApiRequest::Playlist { id: id.clone() });
                }
                if !page.items.loaded_once && page.items.can_load_more() {
                    page.items.loading = true;
                    self.backend.api(ApiRequest::PlaylistItems {
                        id: id.clone(),
                        offset: 0,
                    });
                }
                self.request_contains(vec![format!("spotify:playlist:{id}")]);
            }
            Page::Album(id) => {
                let page = self.album_pages.entry(id.clone()).or_default();
                if page.album.needs_load() {
                    page.album = Loadable::Loading;
                    self.backend.api(ApiRequest::Album { id: id.clone() });
                }
                self.request_contains(vec![format!("spotify:album:{id}")]);
            }
            Page::Artist(id) => {
                let page = self.artist_pages.entry(id.clone()).or_default();
                if page.artist.needs_load() {
                    page.artist = Loadable::Loading;
                    self.backend.api(ApiRequest::Artist { id: id.clone() });
                }
                let filter = page.filter;
                self.load_artist_albums(&id, filter);
                if page_related_needs_load(&self.artist_pages, &id) {
                    if let Some(page) = self.artist_pages.get_mut(&id) {
                        page.related = Loadable::Loading;
                    }
                    self.backend
                        .api(ApiRequest::RelatedArtists { id: id.clone() });
                }
                self.request_contains(vec![format!("spotify:artist:{id}")]);
            }
            Page::Show(id) => {
                let page = self.show_pages.entry(id.clone()).or_default();
                if page.show.needs_load() {
                    page.show = Loadable::Loading;
                    self.backend.api(ApiRequest::Show { id: id.clone() });
                }
                self.request_contains(vec![format!("spotify:show:{id}")]);
            }
            Page::Queue => self.refresh_queue(true),
            Page::Settings => {}
        }
    }

    fn load_artist_albums(&mut self, id: &str, filter: DiscographyFilter) {
        let Some(page) = self.artist_pages.get_mut(id) else {
            return;
        };
        let list = page.albums.entry(filter.groups().to_string()).or_default();
        if !list.loaded_once && list.can_load_more() {
            list.loading = true;
            self.backend.api(ApiRequest::ArtistAlbums {
                id: id.to_string(),
                groups: filter.groups().to_string(),
                offset: 0,
            });
        }
    }

    fn load_home(&mut self, force: bool) {
        if self.home.requested
            && !force
            && self
                .home
                .loaded_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(600))
        {
            return;
        }
        self.home.requested = true;
        self.home.loaded_at = Some(Instant::now());
        self.home.recently_played = Loadable::Loading;
        self.home.top_artists = Loadable::Loading;
        self.home.top_tracks = Loadable::Loading;
        self.backend.api(ApiRequest::RecentlyPlayed);
        self.backend.api(ApiRequest::TopArtists);
        self.backend.api(ApiRequest::TopTracks);
        for term in DISCOVER_TERMS {
            self.home
                .discover
                .insert((*term).to_string(), Loadable::Loading);
            self.backend.api(ApiRequest::Discover {
                term: (*term).to_string(),
            });
        }
    }

    pub fn load_more(&mut self, page: Page) {
        match page {
            Page::LikedSongs => {
                let list = &mut self.library.liked;
                if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                    list.loading = true;
                    self.backend.api(ApiRequest::SavedTracks { offset });
                }
            }
            Page::Albums => {
                let list = &mut self.library.albums;
                if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                    list.loading = true;
                    self.backend.api(ApiRequest::SavedAlbums { offset });
                }
            }
            Page::Artists => {
                let list = &mut self.library.artists;
                if list.can_load_more() {
                    list.loading = true;
                    self.backend.api(ApiRequest::FollowedArtists {
                        after: list.after.clone(),
                    });
                }
            }
            Page::Podcasts => {
                let list = &mut self.library.shows;
                if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                    list.loading = true;
                    self.backend.api(ApiRequest::SavedShows { offset });
                }
            }
            Page::Episodes => {
                let list = &mut self.library.episodes;
                if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                    list.loading = true;
                    self.backend.api(ApiRequest::SavedEpisodes { offset });
                }
            }
            Page::Playlist(id) => {
                if let Some(page) = self.playlist_pages.get_mut(&id) {
                    let list = &mut page.items;
                    if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                        list.loading = true;
                        self.backend.api(ApiRequest::PlaylistItems { id, offset });
                    }
                }
            }
            Page::Album(id) => {
                if let Some(page) = self.album_pages.get_mut(&id) {
                    let list = &mut page.tracks;
                    if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                        list.loading = true;
                        self.backend.api(ApiRequest::AlbumTracks { id, offset });
                    }
                }
            }
            Page::Show(id) => {
                if let Some(page) = self.show_pages.get_mut(&id) {
                    let list = &mut page.episodes;
                    if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                        list.loading = true;
                        self.backend.api(ApiRequest::ShowEpisodes { id, offset });
                    }
                }
            }
            Page::Home => {
                if let Some(offset) = self.library.playlists_next.take() {
                    self.backend.api(ApiRequest::MyPlaylists { offset });
                }
            }
            _ => {}
        }
    }

    fn reload(&mut self, page: Page) {
        match &page {
            Page::Home => self.load_home(true),
            Page::LikedSongs => self.library.liked.reset(),
            Page::Albums => self.library.albums.reset(),
            Page::Artists => self.library.artists.reset(),
            Page::Podcasts => self.library.shows.reset(),
            Page::Episodes => self.library.episodes.reset(),
            Page::Playlist(id) => {
                self.playlist_pages.remove(id);
            }
            Page::Album(id) => {
                self.album_pages.remove(id);
            }
            Page::Artist(id) => {
                self.artist_pages.remove(id);
            }
            Page::Show(id) => {
                self.show_pages.remove(id);
            }
            Page::Queue => self.queue = Loadable::NotLoaded,
            _ => {}
        }
        self.ensure_loaded(page);
    }

    fn poll_remote(&mut self, _immediate: bool) {
        if !self.is_connected() {
            return;
        }
        self.remote_poll_pending = true;
        self.remote_polled_at = Instant::now();
        self.backend.api(ApiRequest::PlaybackState);
    }

    fn refresh_devices(&mut self) {
        if !self.is_connected() || self.devices_loading {
            return;
        }
        self.devices_loading = true;
        self.backend.api(ApiRequest::Devices);
    }

    fn refresh_queue(&mut self, force: bool) {
        if !self.is_connected() {
            return;
        }
        if self.queue.is_loading() && !force {
            return;
        }
        if !matches!(self.queue, Loadable::Loaded(_)) {
            self.queue = Loadable::Loading;
        }
        self.queue_fetched_at = Some(Instant::now());
        self.backend.api(ApiRequest::Queue);
    }

    fn run_search(&mut self, query: String) {
        if query.is_empty() {
            self.search.results = Loadable::NotLoaded;
            self.search.committed.clear();
            return;
        }
        if query == self.search.committed && !self.search.results.needs_load() {
            return;
        }
        self.search.serial += 1;
        self.search.committed = query.clone();
        self.search.results = Loadable::Loading;
        self.backend.api(ApiRequest::Search {
            query,
            serial: self.search.serial,
        });
    }

    /// Asks Spotify whether these items are in the library, in batches.
    pub fn request_contains(&mut self, uris: Vec<String>) {
        let Some(user_id) = self.user_id().map(str::to_string) else {
            return;
        };
        let mut batch = Vec::new();
        for uri in uris {
            if uri.is_empty()
                || self.saved.contains_key(&uri)
                || self.saved_pending.contains(&uri)
                || uri.starts_with("spotify:local")
            {
                continue;
            }
            self.saved_pending.insert(uri.clone());
            batch.push(uri);
            if batch.len() == CONTAINS_BATCH {
                self.backend.api(ApiRequest::Contains {
                    uris: std::mem::take(&mut batch),
                    user_id: user_id.clone(),
                });
            }
        }
        if !batch.is_empty() {
            self.backend.api(ApiRequest::Contains {
                uris: batch,
                user_id,
            });
        }
    }

    // ---- api responses -------------------------------------------------------

    fn handle_api(&mut self, response: ApiResponse) {
        match response {
            ApiResponse::Me(result) => match result {
                Ok(user) => {
                    self.user = Some(user);
                    let page = self.page().clone();
                    self.ensure_loaded(page);
                    if let Some(now) = self.now_playing() {
                        self.request_contains(vec![now.uri]);
                    }
                }
                Err(error) => {
                    if matches!(error, crate::api::ApiError::SignInExpired(_)) {
                        self.auth = AuthStatus::Failed(
                            "Your Spotify sign-in expired. Please sign in again.".into(),
                        );
                        self.backend.send(Command::SignOut);
                    } else {
                        self.toast_error(format!("Couldn't load your profile: {error}"));
                    }
                }
            },
            ApiResponse::Devices(result) => {
                self.devices_loading = false;
                self.devices_fetched_at = Some(Instant::now());
                match result {
                    Ok(devices) => {
                        self.devices = devices;
                        if let Some((name, since)) = self.pending_transfer_to.clone() {
                            let matching = self
                                .devices
                                .iter()
                                .find(|device| device.name == name)
                                .and_then(|device| device.id.clone());
                            if let Some(id) = matching {
                                self.pending_transfer_to = None;
                                self.transfer(id);
                            } else if since.elapsed() > Duration::from_secs(20) {
                                self.pending_transfer_to = None;
                            } else {
                                self.devices_fetched_at = None;
                            }
                        }
                        if let Some(selected) = &self.selected_device
                            && !self
                                .devices
                                .iter()
                                .any(|device| device.id.as_deref() == Some(selected.as_str()))
                        {
                            self.selected_device = None;
                        }
                    }
                    Err(error) => self.toast_error(format!("Couldn't list devices: {error}")),
                }
            }
            ApiResponse::PlaybackState(result) => {
                self.remote_poll_pending = false;
                match result {
                    Ok(state) => {
                        let previous_uri = self.remote.as_ref().and_then(|remote| {
                            remote
                                .state
                                .item
                                .as_ref()
                                .map(|item| item.uri().to_string())
                        });
                        self.remote = state.map(|state| RemoteSnapshot {
                            state,
                            received_at: Instant::now(),
                        });
                        let uri = self.remote.as_ref().and_then(|remote| {
                            remote
                                .state
                                .item
                                .as_ref()
                                .map(|item| item.uri().to_string())
                        });
                        if let Some(remote) = &self.remote
                            && let Some(device) = &remote.state.device
                            && device.id.is_some()
                            && let Some(known) =
                                self.devices.iter_mut().find(|known| known.id == device.id)
                        {
                            known.is_active = true;
                            known.volume_percent = device.volume_percent;
                        }
                        if let Some((wanted, _)) = self.optimistic_playing
                            && self
                                .remote
                                .as_ref()
                                .is_some_and(|remote| remote.state.is_playing == wanted)
                        {
                            self.optimistic_playing = None;
                        }
                        if uri != previous_uri {
                            self.on_now_playing_changed();
                        }
                    }
                    Err(error) => log::debug!("playback state unavailable: {error}"),
                }
            }
            ApiResponse::Queue(result) => {
                self.queue = Loadable::from_result(result);
                if let Some(queue) = self.queue.get() {
                    let uris: Vec<String> = queue
                        .queue
                        .iter()
                        .map(|item| item.uri().to_string())
                        .collect();
                    self.request_contains(uris);
                }
            }
            ApiResponse::RecentlyPlayed(result) => {
                self.home.recently_played = Loadable::from_result(result);
            }
            ApiResponse::TopTracks(result) => {
                if let Ok(tracks) = &result {
                    let seeds: Vec<String> = tracks
                        .iter()
                        .filter_map(|track| track.id.clone())
                        .take(5)
                        .collect();
                    if !seeds.is_empty() && self.home.recommendations.needs_load() {
                        self.home.recommendations = Loadable::Loading;
                        self.backend.api(ApiRequest::Recommendations {
                            seed_tracks: seeds,
                            seed_artists: Vec::new(),
                        });
                    }
                    let uris: Vec<String> = tracks.iter().map(|track| track.uri.clone()).collect();
                    self.request_contains(uris);
                }
                self.home.top_tracks = Loadable::from_result(result);
            }
            ApiResponse::TopArtists(result) => {
                self.home.top_artists = Loadable::from_result(result);
            }
            ApiResponse::Recommendations(result) => {
                if let Ok(tracks) = &result {
                    let uris: Vec<String> = tracks.iter().map(|track| track.uri.clone()).collect();
                    self.request_contains(uris);
                }
                self.home.recommendations = Loadable::from_result(result);
            }
            ApiResponse::Discover { term, result } => {
                let filtered = result.map(|playlists| {
                    let needle = term.to_lowercase();
                    let mut seen = std::collections::HashSet::new();
                    let mut matching: Vec<Playlist> = playlists
                        .into_iter()
                        .filter(|playlist| {
                            let owner = playlist.owner.id.as_deref().unwrap_or("");
                            playlist.name.to_lowercase().contains(&needle)
                                && (owner == "spotify" || playlist.owner_name() == "Spotify")
                                && seen.insert(playlist.name.to_lowercase())
                        })
                        .collect();
                    matching.truncate(6);
                    matching
                });
                self.home
                    .discover
                    .insert(term, Loadable::from_result(filtered));
            }
            ApiResponse::MyPlaylists { offset, result } => match result {
                Ok(page) => {
                    let has_more = page.next.is_some() && !page.items.is_empty();
                    let received = page.items.len() as u32;
                    match &mut self.library.playlists {
                        Loadable::Loaded(existing) if offset > 0 => existing.extend(page.items),
                        slot => *slot = Loadable::Loaded(page.items),
                    }
                    self.library.playlists_next = has_more.then_some(offset + received);
                    if has_more {
                        self.load_more(Page::Home);
                    }
                    if let Some(playlists) = self.library.playlists.get() {
                        for playlist in playlists {
                            self.saved.insert(playlist.uri.clone(), true);
                        }
                    }
                }
                Err(error) => {
                    if offset == 0 {
                        self.library.playlists = Loadable::Failed(error.to_string());
                    } else {
                        self.toast_error(format!("Couldn't load more playlists: {error}"));
                    }
                }
            },
            ApiResponse::Playlist { id, result } => {
                if let Ok(playlist) = &result
                    && let Some(image) = pick_image(&playlist.images, 300)
                {
                    self.tint_for(Some(image));
                }
                if let Some(page) = self.playlist_pages.get_mut(&id) {
                    page.playlist = Loadable::from_result(result);
                }
            }
            ApiResponse::PlaylistItems { id, offset, result } => {
                let mut uris = Vec::new();
                if let Some(page) = self.playlist_pages.get_mut(&id) {
                    match result {
                        Ok(items) => {
                            uris = items
                                .items
                                .iter()
                                .filter_map(|item| item.playable())
                                .map(|item| item.uri().to_string())
                                .collect();
                            page.items.absorb(offset, items);
                        }
                        Err(error) => page.items.fail(friendly_page_error(&error)),
                    }
                }
                self.request_contains(uris);
            }
            ApiResponse::PlaylistCreated(result) => {
                self.playlist_busy = false;
                match result {
                    Ok(playlist) => {
                        self.toast(format!("Created {}", playlist.name));
                        if let Some(playlists) = self.library.playlists.get_mut() {
                            playlists.insert(0, playlist.clone());
                        }
                        self.saved.insert(playlist.uri.clone(), true);
                        if let Some(Dialog::CreatePlaylist { add_uris, .. }) = self.dialog.take()
                            && !add_uris.is_empty()
                        {
                            self.backend.api(ApiRequest::AddToPlaylist {
                                playlist_id: playlist.id.clone(),
                                playlist_name: playlist.name.clone(),
                                uris: add_uris,
                            });
                        }
                        self.open(Page::Playlist(playlist.id));
                    }
                    Err(error) => {
                        self.toast_error(format!("Couldn't create the playlist: {error}"))
                    }
                }
            }
            ApiResponse::PlaylistUpdated { id, result } => {
                self.playlist_busy = false;
                match result {
                    Ok(()) => {
                        self.toast("Playlist updated");
                        self.playlist_pages.remove(&id);
                        self.load_playlists();
                        if matches!(self.page(), Page::Playlist(current) if *current == id) {
                            self.ensure_loaded(Page::Playlist(id));
                        }
                    }
                    Err(error) => {
                        self.toast_error(format!("Couldn't update the playlist: {error}"))
                    }
                }
            }
            ApiResponse::PlaylistItemsChanged {
                id,
                message,
                result,
            } => {
                self.playlist_busy = false;
                match result {
                    Ok(snapshot) => {
                        if !message.is_empty() {
                            self.toast(message);
                        }
                        if let Some(page) = self.playlist_pages.get_mut(&id) {
                            if let Some(playlist) = page.playlist.get_mut()
                                && snapshot.is_some()
                            {
                                playlist.snapshot_id = snapshot;
                            }
                            page.items.reset();
                        }
                        if matches!(self.page(), Page::Playlist(current) if *current == id) {
                            self.ensure_loaded(Page::Playlist(id.clone()));
                        }
                        if let Some(playlists) = self.library.playlists.get_mut() {
                            for playlist in playlists.iter_mut().filter(|p| p.id == id) {
                                playlist.snapshot_id = None;
                            }
                        }
                        self.load_playlists();
                    }
                    Err(error) => {
                        self.toast_error(format!("Playlist change failed: {error}"));
                        if let Some(page) = self.playlist_pages.get_mut(&id) {
                            page.items.reset();
                        }
                        self.ensure_loaded(Page::Playlist(id));
                    }
                }
            }
            ApiResponse::PlaylistFollowChanged {
                id,
                followed,
                result,
            } => match result {
                Ok(()) => {
                    self.saved
                        .insert(format!("spotify:playlist:{id}"), followed);
                    self.toast(if followed {
                        "Added to Your Library"
                    } else {
                        "Removed from Your Library"
                    });
                    self.load_playlists();
                    if !followed && matches!(self.page(), Page::Playlist(current) if *current == id)
                    {
                        self.open(Page::Home);
                    }
                }
                Err(error) => {
                    self.saved
                        .insert(format!("spotify:playlist:{id}"), !followed);
                    self.toast_error(format!("Couldn't update the playlist: {error}"));
                }
            },
            ApiResponse::SavedTracks { offset, result } => match result {
                Ok(page) => {
                    for item in &page.items {
                        self.saved.insert(item.track.uri.clone(), true);
                    }
                    self.library.liked.absorb(offset, page);
                }
                Err(error) => self.library.liked.fail(error.to_string()),
            },
            ApiResponse::SavedAlbums { offset, result } => match result {
                Ok(page) => {
                    for item in &page.items {
                        self.saved.insert(item.album.uri.clone(), true);
                    }
                    self.library.albums.absorb(offset, page);
                }
                Err(error) => self.library.albums.fail(error.to_string()),
            },
            ApiResponse::FollowedArtists { after, result } => {
                let list = &mut self.library.artists;
                list.loading = false;
                list.loaded_once = true;
                match result {
                    Ok(page) => {
                        if after.is_none() {
                            list.items.clear();
                        }
                        let received = page.items.len();
                        for artist in &page.items {
                            self.saved.insert(artist.uri.clone(), true);
                        }
                        list.items.extend(page.items);
                        let next = page.cursors.and_then(|cursors| cursors.after);
                        list.complete = next.is_none() || received == 0;
                        list.after = next;
                        list.error = None;
                    }
                    Err(error) => list.error = Some(error.to_string()),
                }
            }
            ApiResponse::SavedShows { offset, result } => match result {
                Ok(page) => {
                    for item in &page.items {
                        self.saved.insert(item.show.uri.clone(), true);
                    }
                    self.library.shows.absorb(offset, page);
                }
                Err(error) => self.library.shows.fail(error.to_string()),
            },
            ApiResponse::SavedEpisodes { offset, result } => match result {
                Ok(page) => {
                    for item in &page.items {
                        self.saved.insert(item.episode.uri.clone(), true);
                    }
                    self.library.episodes.absorb(offset, page);
                }
                Err(error) => self.library.episodes.fail(error.to_string()),
            },
            ApiResponse::SavedChanged {
                uris,
                saved,
                result,
            } => match result {
                Ok(()) => {
                    for uri in &uris {
                        self.saved.insert(uri.clone(), saved);
                        match util::uri_kind(uri) {
                            Some("track") => {
                                if self.library.liked.loaded_once {
                                    if saved {
                                        self.library.liked.reset();
                                        if matches!(self.page(), Page::LikedSongs) {
                                            self.load_more(Page::LikedSongs);
                                        }
                                    } else {
                                        self.library
                                            .liked
                                            .items
                                            .retain(|item| item.track.uri != *uri);
                                        if let Some(total) = self.library.liked.total.as_mut() {
                                            *total = total.saturating_sub(1);
                                        }
                                    }
                                }
                            }
                            Some("album") => self.library.albums.reset(),
                            Some("artist") => self.library.artists.reset(),
                            Some("show") => self.library.shows.reset(),
                            Some("episode") => self.library.episodes.reset(),
                            _ => {}
                        }
                    }
                    let message = match (uris.first().and_then(|uri| util::uri_kind(uri)), saved) {
                        (Some("track"), true) => "Added to Liked Songs",
                        (Some("track"), false) => "Removed from Liked Songs",
                        (Some("artist"), true) => "Following artist",
                        (Some("artist"), false) => "Unfollowed artist",
                        (_, true) => "Saved to Your Library",
                        (_, false) => "Removed from Your Library",
                    };
                    self.toast(message);
                }
                Err(error) => {
                    for uri in &uris {
                        self.saved.insert(uri.clone(), !saved);
                    }
                    self.toast_error(format!("Couldn't update your library: {error}"));
                }
            },
            ApiResponse::Contains { uris, result } => {
                for uri in &uris {
                    self.saved_pending.remove(uri);
                }
                if let Ok(flags) = result {
                    for (uri, flag) in uris.into_iter().zip(flags) {
                        self.saved.insert(uri, flag);
                    }
                }
            }
            ApiResponse::Search {
                query,
                serial,
                result,
            } => {
                if serial != self.search.serial || query != self.search.committed {
                    return;
                }
                if let Ok(results) = &result {
                    let uris: Vec<String> = results
                        .tracks
                        .iter()
                        .flat_map(|page| page.items.iter())
                        .map(|track| track.uri.clone())
                        .collect();
                    self.request_contains(uris);
                    self.settings.remember_search(&query);
                    self.settings_dirty = true;
                }
                self.search.results = Loadable::from_result(result);
            }
            ApiResponse::Artist { id, result } => {
                if let Ok(artist) = &result {
                    if let Some(image) = pick_image(&artist.images, 300) {
                        self.tint_for(Some(image));
                    }
                    let name = artist.name.clone();
                    if let Some(page) = self.artist_pages.get_mut(&id)
                        && page.top_tracks.needs_load()
                    {
                        page.top_tracks = Loadable::Loading;
                        self.backend.api(ApiRequest::ArtistTopTracks {
                            id: id.clone(),
                            name,
                        });
                    }
                }
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    page.artist = Loadable::from_result(result);
                }
            }
            ApiResponse::ArtistTopTracks { id, result } => {
                if let Ok(tracks) = &result {
                    let uris: Vec<String> = tracks.iter().map(|track| track.uri.clone()).collect();
                    self.request_contains(uris);
                }
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    page.top_tracks = Loadable::from_result(result);
                }
            }
            ApiResponse::ArtistAlbums {
                id,
                groups,
                offset,
                result,
            } => {
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    let list = page.albums.entry(groups).or_default();
                    match result {
                        Ok(albums) => list.absorb(offset, albums),
                        Err(error) => list.fail(error.to_string()),
                    }
                }
            }
            ApiResponse::RelatedArtists { id, result } => {
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    page.related = Loadable::from_result(result);
                }
            }
            ApiResponse::Album { id, result } => {
                let mut uris = Vec::new();
                if let Ok(album) = &result
                    && let Some(image) = pick_image(&album.images, 300)
                {
                    self.tint_for(Some(image));
                }
                if let Some(page) = self.album_pages.get_mut(&id) {
                    match result {
                        Ok(mut album) => {
                            if let Some(tracks) = album.tracks.take() {
                                uris = tracks.items.iter().map(|track| track.uri.clone()).collect();
                                page.tracks.absorb(0, tracks);
                            }
                            page.album = Loadable::Loaded(album);
                            if !page.tracks.loaded_once {
                                page.tracks.loading = true;
                                self.backend.api(ApiRequest::AlbumTracks { id, offset: 0 });
                            }
                        }
                        Err(error) => page.album = Loadable::Failed(error.to_string()),
                    }
                }
                self.request_contains(uris);
            }
            ApiResponse::AlbumTracks { id, offset, result } => {
                let mut uris = Vec::new();
                if let Some(page) = self.album_pages.get_mut(&id) {
                    match result {
                        Ok(tracks) => {
                            uris = tracks.items.iter().map(|track| track.uri.clone()).collect();
                            page.tracks.absorb(offset, tracks);
                        }
                        Err(error) => page.tracks.fail(error.to_string()),
                    }
                }
                self.request_contains(uris);
            }
            ApiResponse::Show { id, result } => {
                if let Ok(show) = &result
                    && let Some(image) = pick_image(&show.images, 300)
                {
                    self.tint_for(Some(image));
                }
                if let Some(page) = self.show_pages.get_mut(&id) {
                    match result {
                        Ok(mut show) => {
                            if let Some(episodes) = show.episodes.take() {
                                page.episodes.absorb(0, episodes);
                            }
                            page.show = Loadable::Loaded(show);
                            if !page.episodes.loaded_once {
                                page.episodes.loading = true;
                                self.backend.api(ApiRequest::ShowEpisodes { id, offset: 0 });
                            }
                        }
                        Err(error) => page.show = Loadable::Failed(error.to_string()),
                    }
                }
            }
            ApiResponse::ShowEpisodes { id, offset, result } => {
                if let Some(page) = self.show_pages.get_mut(&id) {
                    match result {
                        Ok(episodes) => page.episodes.absorb(offset, episodes),
                        Err(error) => page.episodes.fail(error.to_string()),
                    }
                }
            }
            ApiResponse::Track { id, result } => {
                self.track_requests.remove(&id);
                if let Ok(track) = result {
                    self.track_cache.insert(id, track);
                }
            }
            ApiResponse::Remote { action, result } => {
                if matches!(action, RemoteAction::Play | RemoteAction::Pause) {
                    self.clear_play_pending();
                }
                match result {
                    Ok(()) => {
                        self.remote_recheck_at = Some(Instant::now() + REMOTE_RECHECK);
                    }
                    Err(error) => {
                        self.optimistic_playing = None;
                        self.pending_remote_position = None;
                        self.pending_remote_volume = None;
                        let hint = if error.status() == Some(404) {
                            " Choose a device from the devices menu first."
                        } else {
                            ""
                        };
                        self.toast_error(format!(
                            "{}: {error}.{hint}",
                            remote_action_label(action)
                        ));
                    }
                }
                self.poll_remote_soon();
            }
            ApiResponse::Transferred { device_id, result } => match result {
                Ok(()) => {
                    self.selected_device = Some(device_id);
                    self.show_devices = false;
                    self.poll_remote_soon();
                    self.refresh_devices();
                }
                Err(error) => self.toast_error(format!("Couldn't switch device: {error}")),
            },
            ApiResponse::QueueAdded { label, result } => match result {
                Ok(()) => {
                    self.toast(format!("Added {label} to queue"));
                    self.refresh_queue(true);
                }
                Err(error) => self.toast_error(format!("Couldn't add to queue: {error}")),
            },
        }
    }

    fn poll_remote_soon(&mut self) {
        self.remote_polled_at = Instant::now() - REMOTE_POLL_IDLE + Duration::from_millis(700);
    }

    // ---- navigation ------------------------------------------------------------

    pub fn open(&mut self, page: Page) {
        if *self.page() == page {
            self.ensure_loaded(page);
            return;
        }
        self.history.truncate(self.history_index + 1);
        self.history.push(page.clone());
        if self.history.len() > 60 {
            self.history.remove(0);
        }
        self.history_index = self.history.len() - 1;
        self.show_devices = false;
        self.ensure_loaded(page);
    }

    pub fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    // ---- playback --------------------------------------------------------------

    fn remote(&mut self, action: RemoteAction, device_id: Option<String>) {
        if device_id.is_none() && self.remote_fresh().is_none() {
            // Spotify would only answer "no active device found".
            self.clear_play_pending();
            self.toast("Nothing is playing. Pick something first");
            return;
        }
        self.backend.api(ApiRequest::Remote {
            action,
            device_id,
            play: None,
            position_ms: 0,
            percent: 0,
            flag: false,
            repeat: String::new(),
        });
    }

    fn play_request(&mut self, request: PlayRequest) {
        let mut keys: Vec<String> = Vec::new();
        if let Some(context) = &request.context_uri {
            keys.push(context.clone());
        }
        if let Some(offset) = &request.offset_uri {
            keys.push(offset.clone());
        }
        if let Some(first) = request.uris.first() {
            keys.push(first.clone());
        }
        if let Some(position) = request.offset_position
            && let Some(uri) = request.uris.get(position as usize)
        {
            keys.push(uri.clone());
        }
        self.set_play_pending(keys);
        match self.target() {
            Target::Local => {
                self.queued_play = None;
                self.backend.player(PlayerCommand::Load(LoadSpec {
                    context_uri: request.context_uri.clone(),
                    uris: request.uris.clone(),
                    offset_uri: request.offset_uri.clone(),
                    offset_index: request.offset_position,
                    position_ms: request.position_ms,
                    play: true,
                    shuffle: None,
                }));
                self.optimistic_playing = Some((true, Instant::now()));
            }
            Target::Remote(Some(device_id)) => {
                self.queued_play = None;
                self.backend.api(ApiRequest::Remote {
                    action: RemoteAction::Play,
                    device_id: Some(device_id),
                    play: Some(request),
                    position_ms: 0,
                    percent: 0,
                    flag: false,
                    repeat: String::new(),
                });
                self.optimistic_playing = Some((true, Instant::now()));
            }
            Target::Remote(None) => {
                // No remote device is active, and this computer's player is
                // not ready. Never ask Spotify to play "nowhere": either
                // wait for the connecting engine or ask for a device.
                if matches!(
                    self.local_playback,
                    LocalPlayback::Connecting | LocalPlayback::Authorizing
                ) {
                    self.queued_play = Some(request);
                } else {
                    self.clear_play_pending();
                    self.queued_play = None;
                    self.toast("Choose a device, or enable playback on this computer");
                    self.show_devices = true;
                    self.refresh_devices();
                }
            }
        }
    }

    fn toggle_play(&mut self) {
        let playing = self.now_playing().map(|now| now.playing);
        match self.target() {
            Target::Local => {
                if self.local.is_active() {
                    self.backend.player(PlayerCommand::Toggle);
                } else if let Some(remote) = self.remote_fresh() {
                    // Nothing is playing locally: resume on this computer.
                    let uri = remote
                        .state
                        .item
                        .as_ref()
                        .map(|item| item.uri().to_string());
                    let position = remote.state.progress_ms.unwrap_or(0);
                    if let Some(uri) = uri {
                        let mut request = match &remote.state.context {
                            Some(context) if !context.uri.is_empty() => {
                                PlayRequest::context(context.uri.clone()).starting_at_uri(uri)
                            }
                            _ => PlayRequest::tracks(vec![uri]),
                        };
                        request.position_ms = position;
                        self.play_request(request);
                        return;
                    }
                    self.toast("Pick something to play");
                    return;
                } else {
                    self.toast("Pick something to play");
                    return;
                }
            }
            Target::Remote(device_id) => {
                if device_id.is_none() && self.remote_fresh().is_none() {
                    self.toast("Pick a song, album, or playlist");
                    return;
                }
                self.set_play_pending(vec!["::toggle".into()]);
                if playing == Some(true) {
                    self.remote(RemoteAction::Pause, device_id);
                } else {
                    self.remote(RemoteAction::Play, device_id);
                }
            }
        }
        if let Some(playing) = playing {
            self.optimistic_playing = Some((!playing, Instant::now()));
        }
    }

    fn seek(&mut self, position_ms: u32) {
        match self.target() {
            Target::Local => self.backend.player(PlayerCommand::Seek(position_ms)),
            Target::Remote(device_id) => {
                self.pending_remote_position = Some((position_ms, Instant::now()));
                self.backend.api(ApiRequest::Remote {
                    action: RemoteAction::Seek,
                    device_id,
                    play: None,
                    position_ms,
                    percent: 0,
                    flag: false,
                    repeat: String::new(),
                });
            }
        }
    }

    fn set_volume(&mut self, percent: u8) {
        let percent = percent.min(100);
        match self.target() {
            Target::Local => {
                let volume = percent_to_volume(percent);
                self.local.volume = volume;
                self.backend.player(PlayerCommand::Volume(volume));
            }
            Target::Remote(device_id) => {
                self.pending_remote_volume = Some((percent, Instant::now()));
                self.backend.api(ApiRequest::Remote {
                    action: RemoteAction::Volume,
                    device_id,
                    play: None,
                    position_ms: 0,
                    percent,
                    flag: false,
                    repeat: String::new(),
                });
            }
        }
    }

    fn set_shuffle(&mut self, shuffle: bool) {
        match self.target() {
            Target::Local => {
                self.local.shuffle = shuffle;
                self.backend.player(PlayerCommand::Shuffle(shuffle));
            }
            Target::Remote(device_id) => {
                if let Some(remote) = self.remote.as_mut() {
                    remote.state.shuffle_state = shuffle;
                }
                self.backend.api(ApiRequest::Remote {
                    action: RemoteAction::Shuffle,
                    device_id,
                    play: None,
                    position_ms: 0,
                    percent: 0,
                    flag: shuffle,
                    repeat: String::new(),
                });
            }
        }
    }

    fn set_repeat(&mut self, mode: RepeatMode) {
        match self.target() {
            Target::Local => {
                self.local.repeat = mode;
                self.backend.player(PlayerCommand::Repeat(mode));
            }
            Target::Remote(device_id) => {
                if let Some(remote) = self.remote.as_mut() {
                    remote.state.repeat_state = mode.api_name().to_string();
                }
                self.backend.api(ApiRequest::Remote {
                    action: RemoteAction::Repeat,
                    device_id,
                    play: None,
                    position_ms: 0,
                    percent: 0,
                    flag: false,
                    repeat: mode.api_name().to_string(),
                });
            }
        }
    }

    fn transfer(&mut self, device_id: String) {
        if Some(device_id.as_str()) == self.local_device_id.as_deref() {
            self.selected_device = None;
            self.show_devices = false;
            let was_playing = self.now_playing().is_some_and(|now| now.playing);
            self.backend.player(PlayerCommand::Activate);
            if let Some(remote) = self.remote_fresh()
                && let Some(item) = &remote.state.item
            {
                let uri = item.uri().to_string();
                let position = {
                    let base = remote.state.progress_ms.unwrap_or(0);
                    if remote.state.is_playing {
                        base + remote.received_at.elapsed().as_millis() as u32
                    } else {
                        base
                    }
                };
                let mut request = match &remote.state.context {
                    Some(context) if !context.uri.is_empty() => {
                        PlayRequest::context(context.uri.clone()).starting_at_uri(uri)
                    }
                    _ => PlayRequest::tracks(vec![uri]),
                };
                request.position_ms = position;
                self.backend.player(PlayerCommand::Load(LoadSpec {
                    context_uri: request.context_uri,
                    uris: request.uris,
                    offset_uri: request.offset_uri,
                    offset_index: None,
                    position_ms: request.position_ms,
                    play: was_playing,
                    shuffle: None,
                }));
            }
            self.poll_remote_soon();
            return;
        }
        let play = self.now_playing().is_some_and(|now| now.playing);
        self.selected_device = Some(device_id.clone());
        self.backend.api(ApiRequest::Transfer { device_id, play });
    }

    fn add_to_queue(&mut self, uri: String, label: String) {
        let device_id = match self.target() {
            Target::Local => self.local_device_id.clone(),
            Target::Remote(device_id) => device_id,
        };
        self.backend.api(ApiRequest::AddToQueue {
            uri,
            device_id,
            label,
        });
    }

    fn set_saved(&mut self, uri: String, saved: bool) {
        self.saved.insert(uri.clone(), saved);
        if uri.starts_with("spotify:playlist:") {
            let id = util::uri_id(&uri).unwrap_or_default().to_string();
            self.backend
                .api(ApiRequest::FollowPlaylist { id, follow: saved });
            return;
        }
        self.backend.api(ApiRequest::SetSaved {
            uris: vec![uri],
            saved,
        });
    }

    // ---- actions -----------------------------------------------------------------

    fn apply_actions(&mut self, ctx: &egui::Context) {
        let mut actions = std::mem::take(&mut self.actions);
        while !actions.is_empty() {
            for action in actions.drain(..) {
                self.apply(action, ctx);
            }
            actions = std::mem::take(&mut self.actions);
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open(page) => self.open(page),
            Action::OpenUri(uri) => {
                if let Some(page) = Page::from_uri(&uri) {
                    self.open(page);
                }
            }
            Action::Back => {
                if self.can_go_back() {
                    self.history_index -= 1;
                    let page = self.page().clone();
                    self.ensure_loaded(page);
                }
            }
            Action::Forward => {
                if self.can_go_forward() {
                    self.history_index += 1;
                    let page = self.page().clone();
                    self.ensure_loaded(page);
                }
            }
            Action::PlayContext {
                uri,
                offset_uri,
                offset_index,
            } => {
                let mut request = PlayRequest::context(uri);
                request.offset_uri = offset_uri;
                request.offset_position = offset_index;
                self.play_request(request);
            }
            Action::PlayUris { uris, index } => {
                if uris.is_empty() {
                    return;
                }
                let request = PlayRequest::tracks(uris).starting_at_index(index);
                self.play_request(request);
            }
            Action::PlayFromRow {
                context,
                uri,
                index,
            } => match context {
                RowContext::Context {
                    uri: context_uri, ..
                } => {
                    let request = PlayRequest::context(context_uri).starting_at_uri(uri);
                    self.play_request(request);
                }
                RowContext::Uris(uris) => {
                    let request = PlayRequest::tracks(uris).starting_at_index(index);
                    self.play_request(request);
                }
            },
            Action::ShufflePlay(uri) => match self.target() {
                Target::Local => {
                    self.set_play_pending(vec![uri.clone()]);
                    self.backend.player(PlayerCommand::Load(LoadSpec {
                        context_uri: Some(uri),
                        uris: Vec::new(),
                        offset_uri: None,
                        offset_index: None,
                        position_ms: 0,
                        play: true,
                        shuffle: Some(true),
                    }));
                    self.optimistic_playing = Some((true, Instant::now()));
                }
                Target::Remote(device_id) => {
                    self.backend.api(ApiRequest::Remote {
                        action: RemoteAction::Shuffle,
                        device_id: device_id.clone(),
                        play: None,
                        position_ms: 0,
                        percent: 0,
                        flag: true,
                        repeat: String::new(),
                    });
                    self.play_request(PlayRequest::context(uri));
                }
            },
            Action::TogglePlay => self.toggle_play(),
            Action::Next => match self.target() {
                Target::Local => self.backend.player(PlayerCommand::Next),
                Target::Remote(device_id) => self.remote(RemoteAction::Next, device_id),
            },
            Action::Previous => match self.target() {
                Target::Local => self.backend.player(PlayerCommand::Previous),
                Target::Remote(device_id) => self.remote(RemoteAction::Previous, device_id),
            },
            Action::Seek(position_ms) => self.seek(position_ms),
            Action::SeekBy(offset) => {
                if let Some(now) = self.now_playing() {
                    let target = (i64::from(now.position_ms) + offset)
                        .clamp(0, i64::from(now.duration_ms))
                        as u32;
                    self.seek(target);
                }
            }
            Action::SetVolume(percent) => {
                self.volume_before_mute = None;
                self.set_volume(percent);
            }
            Action::VolumeBy(delta) => {
                if let Some(now) = self.now_playing() {
                    let next =
                        (i16::from(now.volume_percent) + i16::from(delta)).clamp(0, 100) as u8;
                    self.volume_before_mute = None;
                    self.set_volume(next);
                    self.volume_osd = Some((next, Instant::now()));
                } else if self.is_connected() {
                    let current = volume_to_percent(self.local.volume);
                    let next = (i16::from(current) + i16::from(delta)).clamp(0, 100) as u8;
                    self.set_volume(next);
                    self.volume_osd = Some((next, Instant::now()));
                }
            }
            Action::ToggleMute => {
                let current = self
                    .now_playing()
                    .map(|now| now.volume_percent)
                    .unwrap_or_else(|| volume_to_percent(self.local.volume));
                if current == 0 {
                    let restore = self.volume_before_mute.take().unwrap_or(50).max(5);
                    self.set_volume(restore);
                } else {
                    self.volume_before_mute = Some(current);
                    self.set_volume(0);
                }
            }
            Action::ToggleShuffle => {
                let shuffle = self.now_playing().is_some_and(|now| now.shuffle);
                self.set_shuffle(!shuffle);
            }
            Action::SetShuffle(shuffle) => self.set_shuffle(shuffle),
            Action::CycleRepeat => {
                let mode = self.now_playing().map(|now| now.repeat).unwrap_or_default();
                self.set_repeat(mode.next());
            }
            Action::SetRepeat(mode) => self.set_repeat(mode),
            Action::AddToQueue { uri, label } => self.add_to_queue(uri, label),
            Action::ToggleSaved(uri) => {
                let saved = self.saved.get(&uri).copied().unwrap_or(false);
                self.set_saved(uri, !saved);
            }
            Action::AddToPlaylist {
                playlist_id,
                playlist_name,
                uris,
            } => {
                self.playlist_busy = true;
                self.backend.api(ApiRequest::AddToPlaylist {
                    playlist_id,
                    playlist_name,
                    uris,
                });
            }
            Action::RemoveFromPlaylist { playlist_id, uris } => {
                let snapshot_id = self
                    .playlist_pages
                    .get(&playlist_id)
                    .and_then(|page| page.playlist.get())
                    .and_then(|playlist| playlist.snapshot_id.clone());
                if let Some(page) = self.playlist_pages.get_mut(&playlist_id) {
                    page.items.items.retain(|item| {
                        item.playable()
                            .is_none_or(|playable| !uris.iter().any(|uri| uri == playable.uri()))
                    });
                }
                self.playlist_busy = true;
                self.backend.api(ApiRequest::RemoveFromPlaylist {
                    playlist_id,
                    uris,
                    snapshot_id,
                });
            }
            Action::MoveInPlaylist {
                playlist_id,
                from,
                to,
            } => {
                let snapshot_id = self
                    .playlist_pages
                    .get(&playlist_id)
                    .and_then(|page| page.playlist.get())
                    .and_then(|playlist| playlist.snapshot_id.clone());
                if let Some(page) = self.playlist_pages.get_mut(&playlist_id) {
                    let items = &mut page.items.items;
                    if (from as usize) < items.len() && (to as usize) <= items.len() {
                        let item = items.remove(from as usize);
                        let insert_at = if to > from { to - 1 } else { to } as usize;
                        items.insert(insert_at.min(items.len()), item);
                    }
                }
                self.playlist_busy = true;
                self.backend.api(ApiRequest::ReorderPlaylist {
                    playlist_id,
                    range_start: from,
                    insert_before: to,
                    snapshot_id,
                });
            }
            Action::ShowDialog(dialog) => self.dialog = Some(dialog),
            Action::CloseDialog => self.dialog = None,
            Action::CreatePlaylist {
                name,
                public,
                add_uris,
            } => {
                let Some(user_id) = self.user_id().map(str::to_string) else {
                    return;
                };
                self.playlist_busy = true;
                self.dialog = Some(Dialog::CreatePlaylist {
                    name: name.clone(),
                    public,
                    add_uris,
                });
                self.backend.api(ApiRequest::CreatePlaylist {
                    user_id,
                    name,
                    public,
                    description: String::new(),
                });
            }
            Action::UpdatePlaylist {
                id,
                name,
                description,
                public,
            } => {
                self.dialog = None;
                self.playlist_busy = true;
                self.backend.api(ApiRequest::UpdatePlaylist {
                    id,
                    name: Some(name),
                    description: Some(description),
                    public: Some(public),
                });
            }
            Action::DeletePlaylist(id) => {
                self.dialog = None;
                self.saved.insert(format!("spotify:playlist:{id}"), false);
                if let Some(playlists) = self.library.playlists.get_mut() {
                    playlists.retain(|playlist| playlist.id != id);
                }
                self.backend
                    .api(ApiRequest::FollowPlaylist { id, follow: false });
            }
            Action::Transfer(device_id) => self.transfer(device_id),
            Action::ActivateReceiver(receiver) => {
                if self.activating_receiver.is_none() {
                    self.activating_receiver = Some(receiver.name.clone());
                    self.backend.send(Command::ActivateReceiver(receiver));
                }
            }
            Action::RefreshDevices => {
                self.devices_fetched_at = None;
                self.refresh_devices();
                self.backend.send(Command::DiscoverReceivers);
            }
            Action::RefreshQueue => self.refresh_queue(true),
            Action::CopyLink(uri) => {
                if let Some(url) = util::open_spotify_url(&uri) {
                    ctx.copy_text(url);
                    self.toast("Link copied");
                }
            }
            Action::OpenInSpotify(uri) => {
                if let Some(url) = util::open_spotify_url(&uri) {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
            }
            Action::Search(query) => {
                self.search.query = query.clone();
                self.search.typed_at = None;
                self.open(Page::Search);
                self.run_search(query.trim().to_string());
            }
            Action::SetSearchFilter(filter) => self.search.filter = filter,
            Action::FocusSearch => {
                self.search.focus_requested = true;
                if !matches!(self.page(), Page::Search) {
                    self.open(Page::Search);
                }
            }
            Action::LoadMore(page) => self.load_more(page),
            Action::LoadMoreArtistAlbums(id) => {
                let Some(page) = self.artist_pages.get_mut(&id) else {
                    return;
                };
                let groups = page.filter.groups().to_string();
                let list = page.albums.entry(groups.clone()).or_default();
                if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                    list.loading = true;
                    self.backend
                        .api(ApiRequest::ArtistAlbums { id, groups, offset });
                }
            }
            Action::SetDiscographyFilter { artist_id, filter } => {
                if let Some(page) = self.artist_pages.get_mut(&artist_id) {
                    page.filter = filter;
                }
                self.load_artist_albums(&artist_id, filter);
            }
            Action::ToggleShowAllTop(id) => {
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    page.show_all_top = !page.show_all_top;
                }
            }
            Action::Reload(page) => self.reload(page),
            Action::SignIn => self.backend.send(Command::SignIn),
            Action::CancelSignIn => {
                self.backend.send(Command::CancelSignIn);
                self.sign_in_url = None;
                self.auth = AuthStatus::SignedOut;
            }
            Action::SignOut => {
                self.backend.send(Command::SignOut);
                self.history = vec![Page::Home];
                self.history_index = 0;
            }
            Action::ToggleQueuePanel => {
                self.show_queue_panel = !self.show_queue_panel;
                if self.show_queue_panel {
                    self.refresh_queue(true);
                }
            }
            Action::ToggleDevicesPopup => {
                self.show_devices = !self.show_devices;
                if self.show_devices {
                    self.refresh_devices();
                    // Receivers waiting on the network are invisible to the
                    // Web API, so look for them ourselves.
                    self.backend.send(Command::DiscoverReceivers);
                }
            }
            Action::ToggleMiniPlayer => self.toggle_mini_player(ctx),
            Action::SettingsChanged => {
                self.settings_dirty = true;
                ctx.set_theme(match self.settings.theme {
                    ThemeChoice::Dark => egui::ThemePreference::Dark,
                    ThemeChoice::Light => egui::ThemePreference::Light,
                    ThemeChoice::System => egui::ThemePreference::System,
                });
            }
            Action::RestartEngine => {
                self.save_settings();
                let config = engine_config(&self.dirs, &self.settings);
                self.backend.send(Command::RestartEngine(config));
                if self.local_ready {
                    self.toast("Restarting local playback");
                }
            }
            Action::ShowWindow => {
                if self.window_hidden {
                    // No window exists; the outer loop creates one.
                    self.wants_show = true;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            Action::HideWindow => {
                if self.tray.is_some() {
                    self.hide_intent = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Action::EnablePlayback => {
                if !self.local_ready
                    && !matches!(
                        self.local_playback,
                        LocalPlayback::Authorizing | LocalPlayback::Connecting
                    )
                {
                    self.settings.playback_authorized = true;
                    self.settings_dirty = true;
                    self.backend.send(Command::AuthorizePlayback);
                    self.toast("Opening Spotify to enable playback here");
                }
            }
            Action::ClearArtCache => match self.backend.art().clear_disk_cache() {
                Ok(bytes) => {
                    ctx.forget_all_images();
                    self.toast(format!(
                        "Cleared {:.1} MB of artwork",
                        bytes as f64 / 1_048_576.0
                    ));
                }
                Err(error) => self.toast_error(format!("Couldn't clear artwork: {error}")),
            },
            Action::Quit => {
                self.quit_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub fn toast(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast {
            message: message.into(),
            kind: ToastKind::Info,
            created: Instant::now(),
        });
        self.toasts.truncate(4);
    }

    pub fn toast_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        log::warn!("{message}");
        self.toasts.push(Toast {
            message,
            kind: ToastKind::Error,
            created: Instant::now(),
        });
    }

    /// Playlists the signed-in user can add to.
    pub fn editable_playlists(&self) -> Vec<(String, String)> {
        let Some(user_id) = self.user_id() else {
            return Vec::new();
        };
        self.library
            .playlists
            .get()
            .map(|playlists| {
                playlists
                    .iter()
                    .filter(|playlist| playlist.owned_by(user_id) || playlist.collaborative)
                    .map(|playlist| (playlist.id.clone(), playlist.name.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl App {
    /// Everything that must keep happening whether or not a window exists:
    /// backend events, MPRIS, the tray, polling, and pending actions. The
    /// headless loop in `main` drives this with a windowless context while
    /// the app lives in the tray.
    pub fn background_frame(&mut self, ctx: &egui::Context) {
        if let Some(flag) = &self.show_requests
            && flag.swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.actions.push(Action::ShowWindow);
        }
        self.handle_events();
        self.handle_mpris();
        self.handle_tray();
        self.tick(ctx);
        self.apply_actions(ctx);
        self.sync_mpris();
    }

    pub fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.apply_theme(ctx);
        crate::ui::show(self, ui);
        self.apply_actions(ctx);
        self.sync_mpris();
        self.tick_sleep_timer();
        if let Some(rect) = ctx.input(|input| input.viewport().inner_rect) {
            self.last_window_size = Some([rect.width(), rect.height()]);
        }
        if let Some(rect) = ctx.input(|input| input.viewport().outer_rect) {
            self.last_window_pos = Some([rect.min.x, rect.min.y]);
        }

        let playing = self.now_playing().is_some_and(|now| now.playing);
        if playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.sleep_timer_end.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.any_play_pending() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.is_connected() {
            ctx.request_repaint_after(REMOTE_POLL_ACTIVE);
        }
        if ctx.input(|input| input.viewport().close_requested())
            && !self.quit_requested
            && self.settings.keep_playing_in_background
            && self.tray.is_some()
        {
            // The window genuinely closes; the process stays in the tray and
            // the outer loop recreates a window on demand. No compositor
            // tricks: this works the same on every desktop.
            self.hide_intent = true;
        }
    }

    /// Persist state when a window closes (to the tray or for good).
    pub fn save_state(&mut self) {
        self.save_settings();
        if !self.offline {
            SessionState {
                last_page: Some(self.page().encode()),
                window_size: self.last_window_size,
                window_pos: self.last_window_pos,
                show_queue_panel: self.show_queue_panel,
            }
            .save(&self.dirs.session_file());
        }
    }

    /// Final teardown at real quit.
    pub fn shutdown(&mut self) {
        self.save_state();
        self.backend.shutdown();
    }
}

pub fn engine_config(dirs: &AppDirs, settings: &Settings) -> EngineConfig {
    EngineConfig {
        device_name: settings.device_name.trim().to_string(),
        bitrate_kbps: settings.bitrate,
        normalisation: settings.normalisation,
        normalisation_pregain_db: settings.normalisation_pregain_db,
        autoplay: settings.autoplay,
        gapless: settings.gapless,
        backend: settings.platform_backend(),
        audio_device: settings
            .audio_device
            .clone()
            .filter(|device| !device.trim().is_empty()),
        initial_volume: settings.volume,
        credentials_dir: dirs.credentials_dir(),
        volume_dir: dirs.volume_dir(),
        audio_cache_dir: settings.audio_cache.then(|| dirs.audio_cache_dir()),
        audio_cache_limit: Some(settings.audio_cache_mb.max(64) * 1024 * 1024),
    }
}

pub fn volume_to_percent(volume: u16) -> u8 {
    ((u32::from(volume) * 100 + u32::from(u16::MAX) / 2) / u32::from(u16::MAX)) as u8
}

pub fn percent_to_volume(percent: u8) -> u16 {
    ((u32::from(percent.min(100)) * u32::from(u16::MAX)) / 100) as u16
}

fn page_related_needs_load(pages: &HashMap<String, ArtistPage>, id: &str) -> bool {
    pages.get(id).is_some_and(|page| page.related.needs_load())
}

fn remote_action_label(action: RemoteAction) -> &'static str {
    match action {
        RemoteAction::Play => "Couldn't start playback",
        RemoteAction::Pause => "Couldn't pause",
        RemoteAction::Next => "Couldn't skip",
        RemoteAction::Previous => "Couldn't go back",
        RemoteAction::Seek => "Couldn't seek",
        RemoteAction::Volume => "Couldn't change the volume",
        RemoteAction::Shuffle => "Couldn't change shuffle",
        RemoteAction::Repeat => "Couldn't change repeat",
    }
}

fn friendly_page_error(error: &crate::api::ApiError) -> String {
    match error.status() {
        Some(403) | Some(404) => {
            "Spotify doesn't make this playlist's songs available to third-party apps.".to_string()
        }
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_conversions_round_trip() {
        assert_eq!(volume_to_percent(u16::MAX), 100);
        assert_eq!(volume_to_percent(0), 0);
        assert_eq!(volume_to_percent(percent_to_volume(70)), 70);
        assert_eq!(percent_to_volume(200), u16::MAX);
    }
}

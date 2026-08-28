//! The application: state, event handling, and the actions views ask for.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use egui::Color32;

use crate::alternate::{AlternateConfig, local_from_track};
use crate::api::PlayRequest;
use crate::api::models::{
    ArtistRef, Device, PlayableItem, PlaybackState, Playlist, Queue, Track, User, pick_image,
};
use crate::backend::{
    ApiRequest, ApiResponse, AuthStatus, Backend, Command, Event, LocalPlayback, LyricsRequest,
    RemoteAction, Waker,
};
use crate::media::{MediaCommand, MediaState, MediaTrack};
use crate::media_controls::MediaService;
use crate::model::*;
use crate::paths::AppDirs;
use crate::player::{
    EngineConfig, LoadSpec, LocalState, LocalTrack, Playback, PlayerCommand, RepeatMode,
};
use crate::settings::{PlaybackBackend, SessionState, Settings, ThemeChoice};
use crate::single_instance::ControlCommand;
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

/// How long a context the app just started is shown as playing while
/// Spotify's state catches up. During a local takeover the cluster can
/// report the old context, then the new, then the old again; the whole
/// dance settles well inside this window, so no early hand-back.
const ASSUMED_CONTEXT_HOLD: Duration = Duration::from_secs(8);
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

/// A context the interface asked Spotify to play and shows as playing
/// before any state says so.
struct AssumedContext {
    uri: String,
    /// `Some` when the play asked for shuffle too.
    shuffle: Option<bool>,
    at: Instant,
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
    pub source_label: Option<String>,
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
    pub media_controls: bool,
    /// Register the system-tray item (Linux).
    pub tray: bool,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            media_controls: true,
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
    media_controls: Option<MediaService>,
    tray: Option<TrayService>,
    pub window_hidden: bool,
    /// The window should close but the process should stay in the tray.
    pub hide_intent: bool,
    /// A hidden app was asked to show itself; the outer loop recreates the
    /// window.
    pub wants_show: bool,
    /// Commands from control clients (a second `fastpotify <verb>` launch,
    /// a Raycast script), on the platforms where they do not arrive through
    /// MPRIS. Drained every frame.
    control_commands: Option<std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>>>,
    /// Where the now-playing snapshot goes for the control channel's
    /// `nowplaying` verb to answer from.
    control_now_playing: Option<std::sync::Arc<std::sync::Mutex<String>>>,
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
    /// Serial of the newest playback poll sent; older answers are stale.
    remote_poll_seq: u64,
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
    pub show_lyrics_panel: bool,
    /// The track the lyrics below are for.
    pub lyrics_uri: Option<String>,
    /// `Loaded(None)` when nobody has transcribed the track.
    pub lyrics: Loadable<Option<crate::lyrics::Lyrics>>,
    /// Whether the panel scrolls to the line being sung. Off once the
    /// reader scrolls by hand, on again with the Follow button or a new
    /// track.
    pub lyrics_following: bool,
    /// The line the panel last positioned itself for (`Some(None)` before
    /// the first line), so it moves once per change; `None` until it has
    /// positioned itself at all for this track.
    pub lyrics_line_shown: Option<Option<usize>>,
    pub show_devices: bool,
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
    last_eviction: Instant,
    pub sign_in_url: Option<String>,
    /// The Web API application the current sign-in belongs to, so Settings
    /// can say whether the one named there is in use yet.
    pub web_app: Option<String>,
    pending_remote_position: Option<(u32, Instant)>,
    pending_remote_volume: Option<(u8, Instant)>,
    /// A local volume set here that the engine has not echoed back yet. It
    /// reports `VolumeChanged` asynchronously while position snapshots land
    /// every second, so a snapshot must not undo the change on its way past.
    pending_local_volume: Option<(u16, Instant)>,
    optimistic_playing: Option<(bool, Instant)>,
    /// The context the interface just started, shown as playing until
    /// Spotify's own state says the same thing.
    assumed_context: Option<AssumedContext>,
    last_now_playing_uri: Option<String>,
    pub playlist_busy: bool,
    pub quit_requested: bool,
    /// The axis a scroll gesture settled on, and when it last moved.
    scroll_lock: Option<(ScrollAxis, Instant)>,
    /// Whether the current scroll gesture comes from a trackpad.
    scroll_from_trackpad: bool,
    /// Recent scroll positions, to read the gesture's speed when it ends.
    scroll_history: egui::util::History<egui::Vec2>,
    /// Where the gesture has scrolled to so far, for the history.
    scroll_accum: egui::Vec2,
    /// The speed still carrying the page after the fingers lifted.
    glide: Option<egui::Vec2>,
    /// When the last scroll event arrived, for lifts nobody announces.
    scroll_last_event: Option<Instant>,
    /// How each table is sorted, per page, for as long as the app runs.
    pub table_sorts: HashMap<Page, TableSort>,
    /// User ids resolved to display names; `None` while unknown, so an id
    /// is asked about only once per run.
    pub user_names: HashMap<String, Option<String>>,
    /// Context URIs most recently played, newest first: the sidebar's order.
    pub recent_contexts: Vec<String>,
    /// What was playing when the app last closed, to resume from cold.
    resume_context: Option<String>,
    resume_track: Option<String>,
    resume_position_ms: u32,
    /// A newer release than this build, once GitHub has said so.
    pub update: Option<crate::updates::Release>,
    last_update_check: Option<Instant>,
    /// Free-account (or Premium-required) switch to Alternate already applied.
    free_alternate_applied: bool,
    /// Local engine started once after `/me` selected the effective mode.
    profile_playback_applied: bool,
    /// One quiet toast while `/me` has not arrived yet.
    loading_account_toasted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollAxis {
    Horizontal,
    Vertical,
}

/// A trackpad gesture that pauses this long has ended; the next movement
/// picks its axis afresh.
const SCROLL_GESTURE_GAP: Duration = Duration::from_millis(150);

/// How far short Linux trackpad deltas land of what other players scroll.
const TRACKPAD_SCALE: f32 = 1.8;

/// The glide's exponential decay time, in seconds; the speed below which a
/// lift starts no glide; and the speed at which a glide stops, points per
/// second.
const GLIDE_DECAY: f32 = 0.35;
const GLIDE_START: f32 = 120.0;
const GLIDE_STOP: f32 = 40.0;

impl App {
    pub fn new(waker: &Waker, dirs: AppDirs, settings: Settings, options: AppOptions) -> Self {
        let engine_config = engine_config(&dirs, &settings);
        let backend = Backend::spawn(
            dirs.clone(),
            engine_config,
            settings.web_client_id.clone(),
            waker.clone(),
            settings.playback_backend,
            AlternateConfig::from_settings(&settings),
        );
        let wake = waker.clone();
        let media_controls = options
            .media_controls
            .then(|| MediaService::spawn(move || wake.wake()));
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

        let mut app = Self {
            dirs,
            settings,
            settings_dirty: false,
            last_settings_save: Instant::now(),
            backend,
            media_controls,
            tray,
            window_hidden: false,
            hide_intent: false,
            wants_show: false,
            control_commands: None,
            control_now_playing: None,
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
            remote_poll_seq: 0,
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
            show_queue_panel: false,
            show_lyrics_panel: false,
            lyrics_uri: None,
            lyrics: Loadable::NotLoaded,
            lyrics_following: true,
            lyrics_line_shown: None,
            show_devices: false,
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
            last_eviction: Instant::now(),
            sign_in_url: None,
            web_app: None,
            pending_remote_position: None,
            pending_remote_volume: None,
            pending_local_volume: None,
            optimistic_playing: None,
            assumed_context: None,
            last_now_playing_uri: None,
            playlist_busy: false,
            quit_requested: false,
            scroll_lock: None,
            scroll_from_trackpad: false,
            scroll_history: egui::util::History::new(2..16, 0.1),
            scroll_accum: egui::Vec2::ZERO,
            glide: None,
            scroll_last_event: None,
            table_sorts: HashMap::new(),
            user_names: HashMap::new(),
            recent_contexts: session.recent_contexts.clone(),
            resume_context: session.last_context.clone(),
            resume_track: session.last_track.clone(),
            resume_position_ms: session.last_position_ms,
            update: None,
            last_update_check: None,
            free_alternate_applied: false,
            profile_playback_applied: false,
            loading_account_toasted: false,
        };
        app.local.volume = app.settings.volume;
        app
    }

    /// Watches the queue control clients fill and keeps their now-playing
    /// snapshot fresh.
    pub fn set_remote_control(&mut self, guard: &crate::single_instance::Guard) {
        self.control_commands = Some(guard.commands());
        self.control_now_playing = Some(guard.now_playing_slot());
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
        if let Some(tray) = &mut self.tray {
            tray.attach();
        }
        // egui's consensus wheel speed is 40 points per line, about a third
        // of what every other player scrolls per notch; trackpads report
        // pixels and are unaffected (#32).
        ctx.options_mut(|options| options.input_options.line_scroll_speed = 120.0);
    }

    /// The window is gone but the process stays: audio, the tray, and the
    /// media controls keep running until Show or Quit.
    pub fn window_gone(&mut self) {
        self.window_hidden = true;
        self.hide_intent = false;
        self.wants_show = false;
        if let Some(tray) = &mut self.tray {
            tray.hidden();
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

    pub fn is_free_account(&self) -> bool {
        is_free_spotify_product(self.user.as_ref().and_then(|user| user.product.as_deref()))
    }

    pub fn is_confirmed_premium(&self) -> bool {
        is_confirmed_premium_product(self.user.as_ref().and_then(|user| user.product.as_deref()))
    }

    pub fn alternate_local(&self) -> bool {
        uses_local_transport(
            self.settings.playback_backend,
            self.user.as_ref().and_then(|user| user.product.as_deref()),
            &self.local_playback,
            self.user.is_some(),
        )
    }

    fn allows_spotify_player_api(&self) -> bool {
        self.is_connected() && self.is_confirmed_premium() && !self.alternate_local()
    }

    /// Local Alternate, or a loaded profile that can legally use player APIs.
    pub fn transport_ready(&self) -> bool {
        self.alternate_local() || self.user.is_some()
    }

    /// Where playback commands go: this computer's player or a remote device.
    pub fn target(&self) -> Target {
        if self.alternate_local() || self.user.is_none() {
            return Target::Local;
        }
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

    /// Whether the Web API sign-in belongs to an app of the user's own
    /// rather than the shared one.
    pub fn own_web_app(&self) -> bool {
        self.web_app
            .as_deref()
            .is_some_and(|id| id != crate::auth::DEFAULT_WEB_CLIENT_ID)
    }

    /// The context playing as the interface should show it: the one just
    /// asked for until Spotify's state confirms it, then Spotify's own.
    pub fn playing_context_uri(&self) -> Option<String> {
        if let Some(assumed) = &self.assumed_context
            && assumed.at.elapsed() < ASSUMED_CONTEXT_HOLD
        {
            return Some(assumed.uri.clone());
        }
        self.remote
            .as_ref()
            .and_then(|remote| remote.state.context.as_ref())
            .map(|context| context.uri.clone())
    }

    /// Whether the playing context shuffles, honouring a shuffle the
    /// interface just asked for ahead of Spotify's state.
    /// Whether something plays, as the interface should show it: what it
    /// just asked for, before any state reports back.
    pub fn believed_playing(&self) -> bool {
        if let Some((playing, at)) = self.optimistic_playing
            && at.elapsed() < PLAYBACK_HOLD
        {
            return playing;
        }
        self.now_playing().is_some_and(|now| now.playing)
    }

    pub fn playing_context_shuffle(&self) -> bool {
        if let Some(assumed) = &self.assumed_context
            && assumed.at.elapsed() < ASSUMED_CONTEXT_HOLD
            && let Some(shuffle) = assumed.shuffle
        {
            return shuffle;
        }
        self.now_playing().is_some_and(|now| now.shuffle)
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
                source_label: self.local.source_label.clone(),
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
            source_label: None,
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
                Event::Lyrics { uri, result } => {
                    if self.lyrics_uri.as_deref() == Some(uri.as_str()) {
                        self.lyrics = match result {
                            Ok(found) => Loadable::Loaded(found),
                            Err(error) => Loadable::Failed(error),
                        };
                    }
                }
                Event::PlaylistCache {
                    id,
                    snapshot,
                    items,
                } => {
                    if let Some(page) = self.playlist_pages.get_mut(&id) {
                        page.pending_cache = Some((snapshot, items));
                    }
                    self.try_adopt_playlist_cache(&id);
                }
                Event::UserName { id, name } => {
                    self.user_names.insert(id, name);
                }
                Event::WebApp { client_id } => self.web_app = Some(client_id),
                Event::UpdateAvailable { version, url } => {
                    let notice = crate::updates::Release { version, url };
                    if self.update.as_ref() != Some(&notice) {
                        self.toast(format!("Fastpotify {} is out", notice.version));
                    }
                    self.update = Some(notice);
                }
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
            }
            AuthStatus::WaitingForBrowser { url } => self.sign_in_url = Some(url.clone()),
            AuthStatus::SignedOut => {
                self.sign_in_url = None;
                self.web_app = None;
                self.user = None;
                self.free_alternate_applied = false;
                self.profile_playback_applied = false;
                self.loading_account_toasted = false;
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
                    self.play_request(request, false);
                }
            }
            LocalPlayback::AlternateReady { .. } => {
                self.local_device_id = None;
                self.local_ready = true;
                if let Some(request) = self.queued_play.take() {
                    self.play_request(request, false);
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
                if crate::backend::connect_error_is_entitlement(message) {
                    self.apply_free_alternate_mode();
                } else {
                    self.toast_error(format!("Local playback: {message}"));
                }
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
        let held_volume = self.held_local_volume(state.volume);
        if held_volume.is_none() && state.volume != self.settings.volume {
            self.settings.volume = state.volume;
            self.settings_dirty = true;
        }
        if state.seek_sequence != self.local.seek_sequence
            && let Some(controls) = &self.media_controls
        {
            controls.seeked(state.position_ms);
        }
        if let Some(error) = &state.error
            && self.local.error.as_deref() != Some(error.as_str())
        {
            self.toast_error(error.clone());
        }
        self.local = state;
        if let Some(volume) = held_volume {
            self.local.volume = volume;
        }
        if matches!(self.local_playback, LocalPlayback::AlternateReady { .. }) {
            self.queue = Loadable::Loaded(queue_from_local(&self.local, &self.track_cache));
            self.queue_fetched_at = Some(Instant::now());
        }
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
        if self.show_lyrics_panel {
            self.request_lyrics();
        }
    }

    /// Asks for the playing track's lyrics unless they are here or on the
    /// way. Podcasts have no lyrics to ask for.
    pub fn request_lyrics(&mut self) {
        let Some(now) = self.now_playing() else {
            return;
        };
        if self.lyrics_uri.as_deref() == Some(now.uri.as_str())
            && !matches!(self.lyrics, Loadable::NotLoaded | Loadable::Failed(_))
        {
            return;
        }
        self.lyrics_uri = Some(now.uri.clone());
        self.lyrics_following = true;
        self.lyrics_line_shown = None;
        if now.is_episode || self.offline {
            self.lyrics = Loadable::Loaded(None);
            return;
        }
        self.lyrics = Loadable::Loading;
        self.backend.send(Command::Lyrics(Box::new(LyricsRequest {
            uri: now.uri,
            query: crate::lyrics::Query {
                artist: now
                    .artists
                    .first()
                    .map(|artist| artist.name.clone())
                    .unwrap_or_default(),
                title: now.title,
                album: now.album_name,
                duration_ms: now.duration_ms,
            },
        })));
    }

    pub fn remember_track(&mut self, track: &Track) {
        if let Some(id) = track.id.as_deref().filter(|id| !id.is_empty()) {
            self.track_cache.insert(id.to_string(), track.clone());
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts
            .retain(|toast| toast.created.elapsed() < TOAST_LIFETIME);

        if self.settings.check_for_updates
            && !self.offline
            && self
                .last_update_check
                .is_none_or(|at| at.elapsed() >= crate::updates::CHECK_INTERVAL)
        {
            self.last_update_check = Some(now);
            self.backend.send(Command::CheckForUpdates);
        }

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

    /// Note that a setting changed, so the file is saved shortly.
    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
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

    fn handle_control_commands(&mut self) {
        let Some(queue) = &self.control_commands else {
            return;
        };
        let commands: Vec<ControlCommand> =
            std::mem::take(&mut *queue.lock().unwrap_or_else(|p| p.into_inner()));
        for command in commands {
            let playing = self.now_playing().is_some_and(|now| now.playing);
            let action = match command {
                ControlCommand::Show => Some(Action::ShowWindow),
                ControlCommand::PlayPause => Some(Action::TogglePlay),
                ControlCommand::Play => (!playing).then_some(Action::TogglePlay),
                ControlCommand::Pause => playing.then_some(Action::TogglePlay),
                ControlCommand::Next => Some(Action::Next),
                ControlCommand::Previous => Some(Action::Previous),
                ControlCommand::SeekBy(offset) => Some(Action::SeekBy(offset)),
                ControlCommand::VolumeBy(delta) => Some(Action::VolumeBy(delta)),
                ControlCommand::SetVolume(volume) => Some(Action::SetVolume(volume.min(100))),
                ControlCommand::ToggleMute => Some(Action::ToggleMute),
                ControlCommand::ToggleShuffle => Some(Action::ToggleShuffle),
                ControlCommand::CycleRepeat => Some(Action::CycleRepeat),
            };
            if let Some(action) = action {
                self.actions.push(action);
            }
        }
    }

    fn handle_media_commands(&mut self) {
        let Some(commands) = self
            .media_controls
            .as_ref()
            .map(MediaService::drain_commands)
        else {
            return;
        };
        for command in commands {
            let playing = self.now_playing().is_some_and(|now| now.playing);
            let action = match command {
                MediaCommand::Play => (!playing).then_some(Action::TogglePlay),
                MediaCommand::Pause | MediaCommand::Stop => playing.then_some(Action::TogglePlay),
                MediaCommand::PlayPause => Some(Action::TogglePlay),
                MediaCommand::Next => Some(Action::Next),
                MediaCommand::Previous => Some(Action::Previous),
                MediaCommand::SeekBy(offset) => Some(Action::SeekBy(offset)),
                MediaCommand::SetPosition {
                    track_uri,
                    position_ms,
                } => self
                    .now_playing()
                    .filter(|now| now.uri == track_uri)
                    .map(|_| Action::Seek(position_ms)),
                MediaCommand::SetVolume(volume) => Some(Action::SetVolume(
                    (volume.clamp(0.0, 1.0) * 100.0).round() as u8,
                )),
                MediaCommand::SetShuffle(shuffle) => Some(Action::SetShuffle(shuffle)),
                MediaCommand::SetRepeat(mode) => Some(Action::SetRepeat(mode)),
                MediaCommand::OpenUri(uri) => Some(Action::PlayContext {
                    uri,
                    offset_uri: None,
                    offset_index: None,
                }),
                MediaCommand::Raise => Some(Action::ShowWindow),
                MediaCommand::Quit => Some(Action::Quit),
            };
            if let Some(action) = action {
                self.actions.push(action);
            }
        }
    }

    fn sync_media_controls(&mut self) {
        let state = match self.now_playing() {
            Some(now) => MediaState {
                playback: if now.playing {
                    Playback::Playing
                } else if now.loading {
                    Playback::Loading
                } else {
                    Playback::Paused
                },
                track: Some(MediaTrack {
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
            None => MediaState::default(),
        };
        if let Some(controls) = &mut self.media_controls {
            controls.update(state);
        }
        let playing = self.now_playing().is_some_and(|now| now.playing);
        if let Some(tray) = &mut self.tray {
            tray.set_playing(playing);
        }
        if let Some(slot) = &self.control_now_playing {
            let snapshot = self.control_snapshot();
            *slot.lock().unwrap_or_else(|p| p.into_inner()) = snapshot;
        }
    }

    /// One line for the control channel's `nowplaying` verb: tab-separated
    /// `state, title, artists, album, position_ms, duration_ms, volume,
    /// shuffle, repeat`, or [`crate::single_instance::NOTHING_PLAYING`].
    fn control_snapshot(&self) -> String {
        let Some(now) = self.now_playing() else {
            return crate::single_instance::NOTHING_PLAYING.to_owned();
        };
        let state = if now.playing { "playing" } else { "paused" };
        let repeat = match now.repeat {
            RepeatMode::Off => "off",
            RepeatMode::Context => "context",
            RepeatMode::Track => "track",
        };
        // Tabs separate the fields, so a tab inside one would shift the rest.
        let clean = |text: &str| text.replace('\t', " ");
        format!(
            "{state}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{repeat}",
            clean(&now.title),
            clean(&now.subtitle),
            clean(&now.album_name),
            now.position_ms,
            now.duration_ms,
            now.volume_percent,
            if now.shuffle { "on" } else { "off" },
        )
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
            Page::TopSongs => self.load_top_songs(false),
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
                    // The disk may hold the whole list already; it is
                    // adopted only if Spotify's snapshot still matches.
                    self.backend
                        .send(Command::LoadPlaylistCache { id: id.clone() });
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
        self.backend.api(ApiRequest::TopTracks {
            offset: 0,
            full: false,
        });
        for term in DISCOVER_TERMS {
            self.home
                .discover
                .insert((*term).to_string(), Loadable::Loading);
            self.backend.api(ApiRequest::Discover {
                term: (*term).to_string(),
            });
        }
    }

    fn load_top_songs(&mut self, force: bool) {
        if self.home.top_songs_loading || (!force && self.home.top_songs_complete) {
            return;
        }
        self.home.top_songs = Loadable::Loading;
        self.home.top_songs_loading = true;
        self.home.top_songs_complete = false;
        self.backend.api(ApiRequest::TopTracks {
            offset: 0,
            full: true,
        });
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
            Page::TopSongs => self.load_top_songs(true),
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
        if !self.allows_spotify_player_api() {
            return;
        }
        self.remote_poll_pending = true;
        self.remote_polled_at = Instant::now();
        self.remote_poll_seq += 1;
        self.backend.api(ApiRequest::PlaybackState {
            seq: self.remote_poll_seq,
        });
    }

    fn refresh_devices(&mut self) {
        if !self.allows_spotify_player_api() || self.devices_loading {
            return;
        }
        self.devices_loading = true;
        self.backend.api(ApiRequest::Devices);
    }

    fn refresh_queue(&mut self, force: bool) {
        if self.alternate_local() {
            self.queue = Loadable::Loaded(queue_from_local(&self.local, &self.track_cache));
            self.queue_fetched_at = Some(Instant::now());
            return;
        }
        if !self.allows_spotify_player_api() {
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
    /// Resolve adder ids that have no known name yet.
    pub fn request_user_names(&mut self, ids: Vec<String>) {
        let unknown: Vec<String> = ids
            .into_iter()
            .filter(|id| !self.user_names.contains_key(id))
            .collect();
        if unknown.is_empty() {
            return;
        }
        for id in &unknown {
            self.user_names.insert(id.clone(), None);
        }
        self.backend.send(Command::UserNames(unknown));
    }

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
        let own_app = self.own_web_app();
        match response {
            ApiResponse::Me(result) => match result {
                Ok(user) => {
                    self.user = Some(user);
                    self.loading_account_toasted = false;
                    self.start_playback_after_profile();
                    if self.allows_spotify_player_api() {
                        self.poll_remote(true);
                    }
                    let page = self.page().clone();
                    self.ensure_loaded(page);
                    if let Some(now) = self.now_playing() {
                        self.request_contains(vec![now.uri]);
                    }
                    if let Some(request) = self.queued_play.take() {
                        self.play_request(request, false);
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
                    Err(error) if error.is_premium_required() => {
                        self.apply_free_alternate_mode();
                    }
                    Err(error) => self.toast_error(format!("Couldn't list devices: {error}")),
                }
            }
            ApiResponse::PlaybackState { seq, result } => {
                if seq != self.remote_poll_seq {
                    // An older poll finishing late describes the past.
                    return;
                }
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
                        if let Some(context) = self
                            .remote
                            .as_ref()
                            .and_then(|remote| remote.state.context.as_ref())
                            .map(|context| context.uri.clone())
                        {
                            // Mid-takeover the cluster still names the old
                            // context; noting that would dance the sidebar
                            // back and forth.
                            let stale = self.assumed_context.as_ref().is_some_and(|assumed| {
                                assumed.at.elapsed() < ASSUMED_CONTEXT_HOLD
                                    && assumed.uri != context
                            });
                            if !stale {
                                self.note_recent_context(&context);
                            }
                        }
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
                            && !self.local.is_active()
                            && self
                                .remote
                                .as_ref()
                                .is_some_and(|remote| remote.state.is_playing == wanted)
                        {
                            self.optimistic_playing = None;
                        }
                        if uri != previous_uri && !self.local.is_active() {
                            self.on_now_playing_changed();
                        }
                    }
                    Err(error) if error.is_premium_required() => {
                        self.apply_free_alternate_mode();
                    }
                    Err(error) => log::debug!("playback state unavailable: {error}"),
                }
            }
            ApiResponse::Queue(result) => {
                if let Err(error) = &result
                    && error.is_premium_required()
                {
                    self.apply_free_alternate_mode();
                    self.queue = Loadable::Loaded(queue_from_local(&self.local, &self.track_cache));
                    self.queue_fetched_at = Some(Instant::now());
                    return;
                }
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
                if let Ok(history) = &result {
                    // Oldest first, so the newest ends up at the front.
                    let contexts: Vec<String> = history
                        .iter()
                        .rev()
                        .filter_map(|play| play.context.as_ref().map(|context| context.uri.clone()))
                        .collect();
                    for context in contexts {
                        self.note_recent_context(&context);
                    }
                }
                self.home.recently_played = Loadable::from_result(result);
            }
            ApiResponse::TopTracks {
                offset,
                full,
                result,
            } => {
                if full {
                    match result {
                        Ok(page) => {
                            let received = page.items.len() as u32;
                            let tracks = page.items;
                            let uris: Vec<String> =
                                tracks.iter().map(|track| track.uri.clone()).collect();
                            self.request_contains(uris);
                            if offset == 0 {
                                self.home.top_songs = Loadable::Loaded(tracks);
                            } else if let Some(current) = self.home.top_songs.get_mut() {
                                current.extend(tracks);
                            }
                            if page.next.is_some() && received > 0 && offset + received < 100 {
                                self.backend.api(ApiRequest::TopTracks {
                                    offset: offset + received,
                                    full: true,
                                });
                            } else {
                                self.home.top_songs_loading = false;
                                self.home.top_songs_complete = true;
                            }
                        }
                        Err(error) => {
                            self.home.top_songs = Loadable::Failed(error.to_string());
                            self.home.top_songs_loading = false;
                        }
                    }
                } else if let Ok(page) = result {
                    let tracks = page.items;
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
                    self.home.top_tracks = Loadable::Loaded(tracks);
                } else if offset == 0
                    && let Err(error) = result
                {
                    self.home.top_tracks = Loadable::Failed(error.to_string());
                }
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
                self.try_adopt_playlist_cache(&id);
            }
            ApiResponse::PlaylistItems { id, offset, result } => {
                let mut uris = Vec::new();
                let mut adders: Vec<String> = Vec::new();
                if let Some(page) = self.playlist_pages.get_mut(&id) {
                    match result {
                        Ok(_) if page.cache_complete => {
                            // A page in flight from before the cache
                            // adopted; the list is already whole.
                        }
                        Ok(items) => {
                            uris = items
                                .items
                                .iter()
                                .filter_map(|item| item.playable())
                                .map(|item| item.uri().to_string())
                                .collect();
                            adders = items
                                .items
                                .iter()
                                .filter_map(|item| item.added_by.as_ref()?.id.clone())
                                .collect();
                            page.contributors.extend(adders.iter().cloned());
                            page.items.absorb(offset, items);
                            // The rows load from the top, and songs a friend
                            // added often sit at the end; look there once.
                            if !page.tail_checked {
                                page.tail_checked = true;
                                let loaded = page.items.items.len() as u32;
                                if let Some(total) =
                                    page.items.total.filter(|total| *total > loaded)
                                {
                                    self.backend.api(ApiRequest::PlaylistSample {
                                        id: id.clone(),
                                        offset: total.saturating_sub(100),
                                    });
                                }
                            }
                        }
                        Err(error) => page.items.fail(friendly_page_error(&error, own_app)),
                    }
                }
                self.request_contains(uris);
                self.request_user_names(adders);
                // The whole list is here; remember it under its snapshot.
                if let Some(page) = self.playlist_pages.get(&id)
                    && page.items.is_complete()
                    && !page.cache_complete
                    && let Some(snapshot) = page
                        .playlist
                        .get()
                        .and_then(|playlist| playlist.snapshot_id.clone())
                {
                    self.backend.send(Command::StorePlaylistCache {
                        id: id.clone(),
                        snapshot,
                        items: page.items.items.clone(),
                    });
                }
                // A sorted table means the whole list, not the loaded part.
                if self.table_sorts.contains_key(&Page::Playlist(id.clone())) {
                    self.load_more(Page::Playlist(id));
                }
            }
            ApiResponse::PlaylistSample { id, result } => {
                let mut adders: Vec<String> = Vec::new();
                if let Ok(items) = result
                    && let Some(page) = self.playlist_pages.get_mut(&id)
                {
                    adders = items
                        .items
                        .iter()
                        .filter_map(|item| item.added_by.as_ref()?.id.clone())
                        .collect();
                    page.contributors.extend(adders.iter().cloned());
                }
                self.request_user_names(adders);
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
                            page.contributors.clear();
                            page.tail_checked = false;
                            page.cache_complete = false;
                            page.pending_cache = None;
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
                            page.contributors.clear();
                            page.tail_checked = false;
                            page.cache_complete = false;
                            page.pending_cache = None;
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
            ApiResponse::SavedTracks { offset, result } => {
                match result {
                    Ok(page) => {
                        for item in &page.items {
                            self.saved.insert(item.track.uri.clone(), true);
                        }
                        self.library.liked.absorb(offset, page);
                    }
                    Err(error) => self.library.liked.fail(error.to_string()),
                }
                // A sorted table means the whole list, not the loaded part.
                if self.table_sorts.contains_key(&Page::LikedSongs) {
                    self.load_more(Page::LikedSongs);
                }
            }
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
                // A sorted table means the whole list, not the loaded part.
                if self.table_sorts.contains_key(&Page::Album(id.clone())) {
                    self.load_more(Page::Album(id));
                }
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
                        self.poll_remote_soon();
                    }
                    Err(error) if error.is_premium_required() => {
                        let retry_volume = if action == RemoteAction::Volume {
                            self.pending_remote_volume.map(|(percent, _)| percent)
                        } else {
                            None
                        };
                        self.apply_free_alternate_mode();
                        if let Some(percent) = retry_volume {
                            self.set_volume(percent, true);
                        }
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
                        self.poll_remote_soon();
                    }
                }
            }
            ApiResponse::Transferred { device_id, result } => match result {
                Ok(()) => {
                    self.selected_device = Some(device_id);
                    self.show_devices = false;
                    self.poll_remote_soon();
                    self.refresh_devices();
                }
                Err(error) if error.is_premium_required() => {
                    self.apply_free_alternate_mode();
                }
                Err(error) => self.toast_error(format!("Couldn't switch device: {error}")),
            },
            ApiResponse::QueueAdded { label, result } => match result {
                Ok(()) => {
                    self.toast(format!("Added {label} to queue"));
                    self.refresh_queue(true);
                }
                Err(error) if error.is_premium_required() => {
                    self.apply_free_alternate_mode();
                }
                Err(error) => self.toast_error(format!("Couldn't add to queue: {error}")),
            },
        }
    }

    fn poll_remote_soon(&mut self) {
        self.remote_polled_at = Instant::now() - REMOTE_POLL_IDLE + Duration::from_millis(700);
    }

    fn clear_remote_control_state(&mut self) {
        self.selected_device = None;
        self.remote = None;
        self.remote_poll_pending = false;
        self.remote_recheck_at = None;
        self.pending_remote_position = None;
        self.pending_remote_volume = None;
        self.pending_transfer_to = None;
        self.devices_loading = false;
        self.activating_receiver = None;
        self.devices.clear();
        self.receivers.clear();
    }

    fn apply_free_alternate_mode(&mut self) {
        let product = self.user.as_ref().and_then(|user| user.product.clone());
        let needs_switch = self.settings.playback_backend != PlaybackBackend::Alternate;
        let first = !self.free_alternate_applied;
        let start = !self.profile_playback_applied || needs_switch;
        self.settings.playback_backend = PlaybackBackend::Alternate;
        self.free_alternate_applied = true;
        self.profile_playback_applied = true;
        self.clear_remote_control_state();
        if needs_switch {
            self.settings_dirty = true;
            self.save_settings();
        }
        if start {
            self.backend.send(Command::RestartEngine {
                config: engine_config(&self.dirs, &self.settings),
                mode: PlaybackBackend::Alternate,
                alternate: AlternateConfig::from_settings(&self.settings),
            });
        }
        if first && needs_switch {
            self.toast(if is_free_spotify_product(product.as_deref()) {
                FREE_ALTERNATE_TOAST
            } else {
                UNCONFIRMED_ALTERNATE_TOAST
            });
        }
    }

    fn start_playback_after_profile(&mut self) {
        let product = self.user.as_ref().and_then(|user| user.product.clone());
        let product = product.as_deref();
        log::info!(
            "Spotify product classification: {}",
            if is_confirmed_premium_product(product) {
                "premium"
            } else {
                "unconfirmed"
            }
        );
        let plan = profile_playback_plan(
            product,
            self.settings.playback_backend,
            self.profile_playback_applied,
        );
        if requires_alternate_product(product) {
            self.apply_free_alternate_mode();
            return;
        }
        if !plan.start_engine {
            return;
        }
        self.profile_playback_applied = true;
        self.backend.send(Command::RestartEngine {
            config: engine_config(&self.dirs, &self.settings),
            mode: plan.mode,
            alternate: AlternateConfig::from_settings(&self.settings),
        });
    }

    fn notify_loading_account(&mut self) {
        if self.loading_account_toasted {
            return;
        }
        self.loading_account_toasted = true;
        self.toast("Loading account...");
    }

    fn require_transport(&mut self) -> bool {
        if self.transport_ready() {
            true
        } else {
            self.notify_loading_account();
            false
        }
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
        if !self.allows_spotify_player_api() {
            return;
        }
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

    /// Remembers `uri` as the most recently played context, for the sidebar's order.
    fn note_recent_context(&mut self, uri: &str) {
        if !uri.contains(":playlist:") && !uri.contains(":album:") && !uri.contains(":collection") {
            return;
        }
        self.recent_contexts.retain(|held| held != uri);
        self.recent_contexts.insert(0, uri.to_string());
        self.recent_contexts.truncate(60);
    }

    fn load_spec(&self, request: PlayRequest, play: bool, shuffle: Option<bool>) -> LoadSpec {
        let known_tracks = if self.alternate_local() {
            self.known_local_tracks(&request)
        } else {
            Vec::new()
        };
        LoadSpec {
            context_uri: request.context_uri,
            uris: request.uris,
            offset_uri: request.offset_uri,
            offset_index: request.offset_position,
            position_ms: request.position_ms,
            play,
            shuffle,
            known_tracks,
        }
    }

    fn known_local_tracks(&self, request: &PlayRequest) -> Vec<LocalTrack> {
        let mut uris = Vec::new();
        if let Some(uri) = &request.offset_uri {
            uris.push(uri.clone());
        }
        uris.extend(request.uris.iter().cloned());
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for uri in uris {
            if !seen.insert(uri.clone()) {
                continue;
            }
            let Some(track) = util::uri_id(&uri).and_then(|id| self.track_cache.get(id)) else {
                continue;
            };
            let local = local_from_track(track);
            if local.has_search_metadata() {
                out.push(local);
            }
        }
        out
    }

    /// With `shuffle_first`, shuffle is turned on before playback starts in one ordered exchange.
    fn play_request(&mut self, request: PlayRequest, shuffle_first: bool) {
        let mut keys: Vec<String> = Vec::new();
        if let Some(context) = &request.context_uri {
            keys.push(context.clone());
        }
        if let Some(offset) = &request.offset_uri {
            keys.push(offset.clone());
        }
        match request.offset_position {
            // The play starts at a chosen row; only that row is starting.
            Some(position) => {
                if let Some(uri) = request.uris.get(position as usize) {
                    keys.push(uri.clone());
                }
            }
            // No chosen row: the list starts at its first song.
            None if request.offset_uri.is_none() => {
                if let Some(first) = request.uris.first() {
                    keys.push(first.clone());
                }
            }
            None => {}
        }
        self.set_play_pending(keys);
        if let Some(context) = request.context_uri.clone() {
            self.note_recent_context(&context);
            self.assumed_context = Some(AssumedContext {
                uri: context,
                shuffle: shuffle_first.then_some(true),
                at: Instant::now(),
            });
        }
        if !self.transport_ready() {
            self.queued_play = Some(request);
            return;
        }
        match self.target() {
            Target::Local => {
                if !self.local_ready {
                    self.queued_play = Some(request);
                    return;
                }
                self.queued_play = None;
                self.backend.player(PlayerCommand::Load(self.load_spec(
                    request,
                    true,
                    shuffle_first.then_some(true),
                )));
                self.optimistic_playing = Some((true, Instant::now()));
            }
            Target::Remote(Some(device_id)) => {
                self.queued_play = None;
                if shuffle_first {
                    self.backend.api(ApiRequest::ShufflePlay {
                        device_id: Some(device_id),
                        play: request,
                    });
                } else {
                    self.backend.api(ApiRequest::Remote {
                        action: RemoteAction::Play,
                        device_id: Some(device_id),
                        play: Some(request),
                        position_ms: 0,
                        percent: 0,
                        flag: false,
                        repeat: String::new(),
                    });
                }
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

    /// Adopt a playlist's disk cache once both it and the live playlist
    /// are here and Spotify's snapshot still matches; a stale cache is
    /// discarded, never shown.
    fn try_adopt_playlist_cache(&mut self, id: &str) {
        let mut uris = Vec::new();
        let mut adders: Vec<String> = Vec::new();
        if let Some(page) = self.playlist_pages.get_mut(id) {
            let Some(snapshot_now) = page
                .playlist
                .get()
                .and_then(|playlist| playlist.snapshot_id.clone())
            else {
                return;
            };
            match &page.pending_cache {
                Some((held, _)) if *held == snapshot_now => {}
                Some(_) => {
                    // The playlist changed since; the cache is history.
                    page.pending_cache = None;
                    return;
                }
                None => return,
            }
            if page.items.is_complete() || page.cache_complete {
                page.pending_cache = None;
                return;
            }
            let Some((_, items)) = page.pending_cache.take() else {
                return;
            };
            uris = items
                .iter()
                .filter_map(|item| item.playable())
                .map(|item| item.uri().to_string())
                .collect();
            adders = items
                .iter()
                .filter_map(|item| item.added_by.as_ref()?.id.clone())
                .collect();
            page.contributors.extend(adders.iter().cloned());
            page.items.total = Some(items.len() as u32);
            page.items.items = items;
            page.items.next_offset = None;
            page.items.loading = false;
            page.items.loaded_once = true;
            page.items.error = None;
            page.cache_complete = true;
        }
        self.request_contains(uris);
        self.request_user_names(adders);
    }

    /// Play what was playing when the app last closed. `false` when
    /// nothing is known to resume.
    fn resume_last(&mut self) -> bool {
        let Some(track) = self.resume_track.clone() else {
            return false;
        };
        let mut request = match self.resume_context.clone() {
            Some(context) => PlayRequest::context(context).starting_at_uri(track),
            None => PlayRequest::tracks(vec![track]),
        };
        request.position_ms = self.resume_position_ms;
        self.play_request(request, false);
        true
    }

    fn toggle_play(&mut self) {
        if !self.require_transport() {
            return;
        }
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
                        self.play_request(request, false);
                        return;
                    }
                    if !self.resume_last() {
                        self.toast("Pick something to play");
                    }
                    return;
                } else {
                    if !self.resume_last() {
                        self.toast("Pick something to play");
                    }
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
        if !self.require_transport() {
            return;
        }
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

    /// The volume this side set that the engine has yet to confirm, if the
    /// hold is still good. Clears itself once the engine agrees or it expires.
    fn held_local_volume(&mut self, reported: u16) -> Option<u16> {
        match self.pending_local_volume {
            Some((volume, at)) if volume != reported && at.elapsed() < OPTIMISTIC_HOLD => {
                Some(volume)
            }
            _ => {
                self.pending_local_volume = None;
                None
            }
        }
    }

    /// `settle` is false while the slider is still moving: the level is heard
    /// at once, and Spotify is told where it ended up on release.
    fn set_volume(&mut self, percent: u8, settle: bool) {
        let percent = percent.min(100);
        if !self.transport_ready() {
            self.local.volume = percent_to_volume(percent);
            return;
        }
        match self.target() {
            Target::Local => {
                let volume = percent_to_volume(percent);
                self.local.volume = volume;
                self.pending_local_volume = Some((volume, Instant::now()));
                // The engine echoes `VolumeChanged` only while this device
                // holds the Connect session, so the snapshot that would
                // otherwise persist this may never arrive.
                if self.settings.volume != volume {
                    self.settings.volume = volume;
                    self.settings_dirty = true;
                }
                self.backend.player(if settle {
                    PlayerCommand::Volume(volume)
                } else {
                    PlayerCommand::VolumePreview(volume)
                });
            }
            Target::Remote(_) if !settle => {}
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
        if !self.require_transport() {
            return;
        }
        if let Some(assumed) = &mut self.assumed_context {
            assumed.shuffle = Some(shuffle);
        }
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
        if !self.require_transport() {
            return;
        }
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
        if !self.require_transport() {
            return;
        }
        if self.alternate_local() {
            self.select_this_computer();
            return;
        }
        if matches!(self.local_playback, LocalPlayback::AlternateReady { .. })
            && self.local.is_active()
            && Some(device_id.as_str()) != self.local_device_id.as_deref()
        {
            self.backend.player(PlayerCommand::Stop);
        }
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
                self.backend.player(PlayerCommand::Load(self.load_spec(
                    request,
                    was_playing,
                    None,
                )));
            }
            self.poll_remote_soon();
            return;
        }
        let play = self.now_playing().is_some_and(|now| now.playing);
        self.selected_device = Some(device_id.clone());
        self.backend.api(ApiRequest::Transfer { device_id, play });
    }

    fn add_to_queue(&mut self, uri: String, label: String) {
        if !self.transport_ready() {
            self.notify_loading_account();
            return;
        }
        if self.alternate_local() {
            if uri.contains(":episode:") || uri.contains(":show:") {
                self.toast_error("Podcasts are not supported in alternate playback");
                return;
            }
            let track = util::uri_id(&uri)
                .and_then(|id| self.track_cache.get(id))
                .map(local_from_track)
                .unwrap_or(LocalTrack {
                    uri: uri.clone(),
                    title: label.clone(),
                    ..LocalTrack::default()
                });
            self.backend.player(PlayerCommand::AddToQueue(track));
            self.toast(format!("Added {label} to queue"));
            self.refresh_queue(true);
            return;
        }
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

    fn select_this_computer(&mut self) {
        self.selected_device = None;
        self.show_devices = false;
        if matches!(self.local_playback, LocalPlayback::AlternateReady { .. }) {
            return;
        }
        if self.local_ready {
            self.backend.player(PlayerCommand::Activate);
        }
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
                self.play_request(request, false);
            }
            Action::PlayUris { uris, index } => {
                if uris.is_empty() {
                    return;
                }
                let (uris, index) = cap_uris(uris, index);
                let request = PlayRequest::tracks(uris).starting_at_index(index);
                self.play_request(request, false);
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
                    self.play_request(request, false);
                }
                RowContext::Uris(uris) => {
                    let (uris, index) = cap_uris(uris, index);
                    let request = PlayRequest::tracks(uris).starting_at_index(index);
                    self.play_request(request, false);
                }
            },
            Action::ShufflePlay(uri) => {
                self.play_request(PlayRequest::context(uri), true);
            }
            Action::TogglePlay => self.toggle_play(),
            Action::Next => {
                if !self.require_transport() {
                    return;
                }
                match self.target() {
                    Target::Local => self.backend.player(PlayerCommand::Next),
                    Target::Remote(device_id) => self.remote(RemoteAction::Next, device_id),
                }
            }
            Action::Previous => {
                if !self.require_transport() {
                    return;
                }
                match self.target() {
                    Target::Local => self.backend.player(PlayerCommand::Previous),
                    Target::Remote(device_id) => self.remote(RemoteAction::Previous, device_id),
                }
            }
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
                self.set_volume(percent, true);
            }
            Action::PreviewVolume(percent) => self.set_volume(percent, false),
            Action::VolumeBy(delta) => {
                if let Some(now) = self.now_playing() {
                    let next =
                        (i16::from(now.volume_percent) + i16::from(delta)).clamp(0, 100) as u8;
                    self.volume_before_mute = None;
                    self.set_volume(next, true);
                } else if self.is_connected() {
                    let current = volume_to_percent(self.local.volume);
                    let next = (i16::from(current) + i16::from(delta)).clamp(0, 100) as u8;
                    self.set_volume(next, true);
                }
            }
            Action::ToggleMute => {
                let current = self
                    .now_playing()
                    .map(|now| now.volume_percent)
                    .unwrap_or_else(|| volume_to_percent(self.local.volume));
                if current == 0 {
                    let restore = self.volume_before_mute.take().unwrap_or(50).max(5);
                    self.set_volume(restore, true);
                } else {
                    self.volume_before_mute = Some(current);
                    self.set_volume(0, true);
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
                if !self.require_transport() || self.alternate_local() {
                    return;
                }
                if self.activating_receiver.is_none() {
                    self.activating_receiver = Some(receiver.name.clone());
                    self.backend.send(Command::ActivateReceiver(receiver));
                }
            }
            Action::RefreshDevices => {
                if !self.allows_spotify_player_api() {
                    return;
                }
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
            Action::SwitchWebApp => {
                self.save_settings();
                self.backend
                    .send(Command::SwitchWebApp(self.settings.web_client_id.clone()));
            }
            Action::SignOut => {
                self.backend.send(Command::SignOut);
                self.history = vec![Page::Home];
                self.history_index = 0;
            }
            Action::ToggleQueuePanel => {
                self.show_queue_panel = !self.show_queue_panel;
                if self.show_queue_panel {
                    self.show_lyrics_panel = false;
                    self.refresh_queue(true);
                }
            }
            Action::ToggleLyricsPanel => {
                self.show_lyrics_panel = !self.show_lyrics_panel;
                if self.show_lyrics_panel {
                    self.show_queue_panel = false;
                    self.lyrics_following = true;
                    self.request_lyrics();
                }
            }
            Action::ToggleDevicesPopup => {
                self.show_devices = !self.show_devices;
                if self.show_devices && self.allows_spotify_player_api() {
                    self.refresh_devices();
                    // Receivers waiting on the network are invisible to the
                    // Web API, so look for them ourselves.
                    self.backend.send(Command::DiscoverReceivers);
                }
            }
            Action::SettingsChanged => {
                self.settings_dirty = true;
                ctx.set_theme(match self.settings.theme {
                    ThemeChoice::Dark => egui::ThemePreference::Dark,
                    ThemeChoice::Light => egui::ThemePreference::Light,
                    ThemeChoice::System => egui::ThemePreference::System,
                });
            }
            Action::RestartEngine => {
                if self.user.is_none() {
                    self.notify_loading_account();
                    return;
                }
                if !self.is_confirmed_premium() {
                    self.settings.playback_backend = PlaybackBackend::Alternate;
                }
                self.save_settings();
                let config = engine_config(&self.dirs, &self.settings);
                self.backend.send(Command::RestartEngine {
                    config,
                    mode: self.settings.playback_backend,
                    alternate: AlternateConfig::from_settings(&self.settings),
                });
                if self.local_ready || self.settings.playback_backend == PlaybackBackend::Alternate
                {
                    self.toast("Restarting local playback");
                }
            }
            Action::SelectLocalPlayback => self.select_this_computer(),
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
                if self.user.is_none() {
                    self.notify_loading_account();
                    return;
                }
                if !self.is_confirmed_premium() {
                    self.apply_free_alternate_mode();
                    return;
                }
                if self.settings.playback_backend == PlaybackBackend::Alternate {
                    self.open(Page::Settings);
                    self.toast(if crate::alternate::has_bundled_ytdlp() {
                        "Apply playback settings to start alternate local audio"
                    } else {
                        "Set a Piped API URL or yt-dlp path, then apply playback settings"
                    });
                    return;
                }
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
            Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
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
        self.handle_control_commands();
        self.handle_events();
        self.handle_media_commands();
        self.handle_tray();
        self.tick(ctx);
        self.apply_actions(ctx);
        self.sync_media_controls();
    }

    pub fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.apply_theme(ctx);
        self.lock_scroll_axis(ctx);
        crate::ui::show(self, ui);
        self.apply_actions(ctx);
        self.sync_media_controls();

        let playing = self.now_playing().is_some_and(|now| now.playing);
        if playing {
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

    /// Keeps a scroll gesture on one axis.
    ///
    /// A trackpad reports a little of the other axis during a one-axis
    /// gesture, so a page whose rows scroll sideways drifted diagonally. The
    /// axis is chosen from the first movement of a gesture and held until it
    /// pauses, the way the platforms' own scrolling behaves.
    fn lock_scroll_axis(&mut self, ctx: &egui::Context) {
        let (raw, from_trackpad, ended) = ctx.input(|input| {
            let mut sum = egui::Vec2::ZERO;
            let mut pointish = false;
            let mut ended = false;
            for event in &input.events {
                if let egui::Event::MouseWheel {
                    unit, delta, phase, ..
                } = event
                {
                    sum += *delta;
                    pointish |= *unit == egui::MouseWheelUnit::Point;
                    ended |= matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel);
                }
            }
            (sum, pointish, ended)
        });
        let now = Instant::now();
        if raw != egui::Vec2::ZERO {
            self.scroll_from_trackpad = from_trackpad;
        }
        // Linux compositors hand touchpad deltas through unscaled and they
        // land well short of what other players scroll; wheels arrive as
        // lines and are scaled already. macOS feels right as delivered.
        let trackpad_here = cfg!(target_os = "linux") && self.scroll_from_trackpad;
        if trackpad_here {
            ctx.input_mut(|input| input.smooth_scroll_delta *= TRACKPAD_SCALE);
        }
        // macOS glides after the fingers lift; Linux hands over raw deltas
        // that stop dead. While the fingers move, remember where the gesture
        // has been; the frame it ends, carry its speed of the last tenth of
        // a second on, decaying, the way native scroll views here feel.
        if trackpad_here && raw != egui::Vec2::ZERO {
            self.glide = None;
            self.scroll_accum += raw * TRACKPAD_SCALE;
            self.scroll_history
                .add(ctx.input(|input| input.time), self.scroll_accum);
            self.scroll_last_event = Some(now);
            // Wayland announces the lift; where nothing does, the quiet-gap
            // check below needs a frame to run in.
            ctx.request_repaint_after(Duration::from_millis(60));
        } else if raw != egui::Vec2::ZERO || ctx.input(|input| input.pointer.any_down()) {
            // A wheel takes over, or a press catches the page.
            self.glide = None;
            self.scroll_history.clear();
            self.scroll_last_event = None;
        }
        let quiet = self
            .scroll_last_event
            .is_some_and(|at| now.duration_since(at).as_secs_f32() > 0.15);
        if ended || quiet {
            let mut velocity = self.scroll_history.velocity().unwrap_or(egui::Vec2::ZERO);
            if let Some((axis, _)) = self.scroll_lock {
                match axis {
                    ScrollAxis::Horizontal => velocity.y = 0.0,
                    ScrollAxis::Vertical => velocity.x = 0.0,
                }
            }
            self.glide = (velocity.length() > GLIDE_START).then_some(velocity);
            self.scroll_history.clear();
            self.scroll_accum = egui::Vec2::ZERO;
            self.scroll_last_event = None;
        }
        if let Some(velocity) = self.glide {
            if raw == egui::Vec2::ZERO {
                let dt = ctx.input(|input| input.stable_dt).clamp(0.001, 0.05);
                ctx.input_mut(|input| input.smooth_scroll_delta += velocity * dt);
                let slower = velocity * (-dt / GLIDE_DECAY).exp();
                self.glide = (slower.length() > GLIDE_STOP).then_some(slower);
            }
            ctx.request_repaint();
        }
        let held = self
            .scroll_lock
            .filter(|(_, at)| now.duration_since(*at) < SCROLL_GESTURE_GAP)
            .map(|(axis, _)| axis);
        let moved = raw != egui::Vec2::ZERO;
        let axis = match held {
            Some(axis) => axis,
            None if moved && raw.x.abs() > raw.y.abs() * 1.2 => ScrollAxis::Horizontal,
            None if moved => ScrollAxis::Vertical,
            None => {
                self.scroll_lock = None;
                return;
            }
        };
        if moved {
            self.scroll_lock = Some((axis, now));
        }
        ctx.input_mut(|input| match axis {
            ScrollAxis::Horizontal => input.smooth_scroll_delta.y = 0.0,
            ScrollAxis::Vertical => input.smooth_scroll_delta.x = 0.0,
        });
    }

    /// Persist state when a window closes (to the tray or for good).
    pub fn save_state(&mut self) {
        self.save_settings();
        if let Some(now) = self.now_playing() {
            self.resume_context = self.playing_context_uri();
            self.resume_track = Some(now.uri.clone());
            self.resume_position_ms = now.position_ms;
        }
        if !self.offline {
            SessionState {
                last_page: Some(self.page().encode()),
                recent_contexts: self.recent_contexts.clone(),
                last_context: self.resume_context.clone(),
                last_track: self.resume_track.clone(),
                last_position_ms: self.resume_position_ms,
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

const FREE_ALTERNATE_TOAST: &str =
    "Spotify Free uses bundled alternate local audio; not Spotify audio/Connect.";
const UNCONFIRMED_ALTERNATE_TOAST: &str =
    "Spotify playback is not confirmed Premium; using bundled alternate audio";

fn normalized_product(product: Option<&str>) -> Option<&str> {
    product.map(str::trim).filter(|value| !value.is_empty())
}

/// Only an explicit Premium product may use Spotify Connect / librespot.
pub fn is_confirmed_premium_product(product: Option<&str>) -> bool {
    normalized_product(product).is_some_and(|value| value.eq_ignore_ascii_case("premium"))
}

/// Free, open, empty, missing, and unknown products use Alternate.
pub fn requires_alternate_product(product: Option<&str>) -> bool {
    !is_confirmed_premium_product(product)
}

/// Spotify `product` values that are known Free-tier labels.
pub fn is_free_spotify_product(product: Option<&str>) -> bool {
    normalized_product(product).is_some_and(|value| {
        value.eq_ignore_ascii_case("free") || value.eq_ignore_ascii_case("open")
    })
}

/// Persist Alternate when the account is not confirmed Premium and settings still say Connect.
pub fn should_force_alternate(product: Option<&str>, current: PlaybackBackend) -> bool {
    requires_alternate_product(product) && current != PlaybackBackend::Alternate
}

/// What to do the first time `/me` arrives. Local engines stay down until this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfilePlaybackPlan {
    pub mode: PlaybackBackend,
    pub start_engine: bool,
    pub persist_alternate: bool,
    pub toast_free: bool,
    pub toast_unconfirmed: bool,
}

/// Map account product + stored mode to one engine start. Repeated loads are no-ops.
pub fn profile_playback_plan(
    product: Option<&str>,
    stored: PlaybackBackend,
    already_applied: bool,
) -> ProfilePlaybackPlan {
    let mode = if is_confirmed_premium_product(product) {
        stored
    } else {
        PlaybackBackend::Alternate
    };
    if already_applied {
        return ProfilePlaybackPlan {
            mode,
            start_engine: false,
            persist_alternate: false,
            toast_free: false,
            toast_unconfirmed: false,
        };
    }
    let persist_alternate = should_force_alternate(product, stored);
    ProfilePlaybackPlan {
        mode,
        start_engine: true,
        persist_alternate,
        toast_free: persist_alternate && is_free_spotify_product(product),
        toast_unconfirmed: persist_alternate && !is_free_spotify_product(product),
    }
}

/// Volume and other transport controls stay on this computer.
pub fn uses_local_transport(
    playback_backend: PlaybackBackend,
    product: Option<&str>,
    local_playback: &LocalPlayback,
    profile_loaded: bool,
) -> bool {
    playback_backend == PlaybackBackend::Alternate
        || matches!(local_playback, LocalPlayback::AlternateReady { .. })
        || (profile_loaded && requires_alternate_product(product))
}

fn queue_from_local(
    local: &LocalState,
    cache: &HashMap<String, Track>,
) -> crate::api::models::Queue {
    let currently_playing = local
        .track
        .as_ref()
        .map(|track| playable_from_local(track, cache));
    let queue = local
        .queue
        .iter()
        .map(|track| playable_from_local(track, cache))
        .collect();
    crate::api::models::Queue {
        currently_playing,
        queue,
    }
}

fn playable_from_local(
    track: &LocalTrack,
    cache: &HashMap<String, Track>,
) -> crate::api::models::PlayableItem {
    if let Some(cached) = util::uri_id(&track.uri).and_then(|id| cache.get(id)) {
        return crate::api::models::PlayableItem::Track(cached.clone());
    }
    crate::api::models::PlayableItem::Track(Track {
        id: util::uri_id(&track.uri).map(str::to_string),
        name: track.title.clone(),
        uri: track.uri.clone(),
        duration_ms: track.duration_ms,
        artists: track
            .artists
            .iter()
            .map(|name| crate::api::models::ArtistRef {
                id: None,
                name: name.clone(),
                uri: None,
            })
            .collect(),
        album: Some(crate::api::models::Album {
            name: track.album.clone(),
            images: track
                .art_url
                .as_ref()
                .map(|url| {
                    vec![crate::api::models::Image {
                        url: url.clone(),
                        width: None,
                        height: None,
                    }]
                })
                .unwrap_or_default(),
            ..crate::api::models::Album::default()
        }),
        ..Track::default()
    })
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

/// Since February 2026 a personal app (Development Mode) may read only the
/// playlists its user owns or collaborates on; the shared app predates
/// that and reads anything public.
fn friendly_page_error(error: &crate::api::ApiError, own_app: bool) -> String {
    match error.status() {
        Some(403) | Some(404) if own_app => {
            "Spotify lets a personal app open only the playlists you own or collaborate on. Switch back to the shared app in Settings to open this one.".to_string()
        }
        Some(403) | Some(404) => {
            "Spotify doesn't make this playlist's songs available to third-party apps.".to_string()
        }
        _ => error.to_string(),
    }
}

/// Spotify balks at gigantic track lists, so a play that starts deep in
/// one keeps the five hundred songs from its start onward.
fn cap_uris(uris: Vec<String>, index: u32) -> (Vec<String>, u32) {
    const MAX: usize = 500;
    if uris.len() <= MAX {
        return (uris, index);
    }
    let start = (index as usize).min(uris.len() - 1);
    let end = (start + MAX).min(uris.len());
    (uris[start..end].to_vec(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiError;
    use crate::paths::AppDirs;

    fn test_app() -> App {
        let root = std::env::temp_dir().join(format!(
            "fastpotify-free-route-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let waker = crate::backend::Waker::default();
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.backend.set_offline(true);
        app.auth = AuthStatus::Connected {
            username: "test".into(),
        };
        app
    }

    fn user_with_product(product: &str) -> User {
        User {
            id: "user".into(),
            display_name: Some("Test".into()),
            product: Some(product.into()),
            ..User::default()
        }
    }

    fn premium_required() -> ApiError {
        ApiError::Status {
            status: 403,
            message: "Player command failed: Premium required".into(),
        }
    }

    #[test]
    fn volume_conversions_round_trip() {
        assert_eq!(volume_to_percent(u16::MAX), 100);
        assert_eq!(volume_to_percent(0), 0);
        assert_eq!(volume_to_percent(percent_to_volume(70)), 70);
        assert_eq!(percent_to_volume(200), u16::MAX);
    }

    #[test]
    fn free_product_detection_is_case_insensitive() {
        for product in ["free", "FREE", "Free", " free ", "open", "OPEN", "Open"] {
            assert!(is_free_spotify_product(Some(product)), "{product}");
            assert!(requires_alternate_product(Some(product)), "{product}");
            assert!(!is_confirmed_premium_product(Some(product)), "{product}");
        }
        assert!(!is_free_spotify_product(Some("premium")));
        assert!(!is_free_spotify_product(Some("PREMIUM")));
        assert!(!is_free_spotify_product(Some("Premium")));
        assert!(!is_free_spotify_product(None));
        assert!(!is_free_spotify_product(Some("")));
        assert!(is_confirmed_premium_product(Some("premium")));
        assert!(is_confirmed_premium_product(Some("PREMIUM")));
        assert!(is_confirmed_premium_product(Some(" Premium ")));
        for product in [None, Some(""), Some("  "), Some("unknown"), Some("student")] {
            assert!(requires_alternate_product(product), "{product:?}");
            assert!(!is_confirmed_premium_product(product), "{product:?}");
        }
    }

    #[test]
    fn free_profile_forces_alternate_premium_does_not() {
        assert!(should_force_alternate(
            Some("free"),
            PlaybackBackend::Spotify
        ));
        assert!(should_force_alternate(
            Some("FREE"),
            PlaybackBackend::Spotify
        ));
        assert!(should_force_alternate(
            Some("open"),
            PlaybackBackend::Spotify
        ));
        assert!(should_force_alternate(None, PlaybackBackend::Spotify));
        assert!(should_force_alternate(Some(""), PlaybackBackend::Spotify));
        assert!(should_force_alternate(
            Some("unknown"),
            PlaybackBackend::Spotify
        ));
        assert!(!should_force_alternate(
            Some("free"),
            PlaybackBackend::Alternate
        ));
        assert!(!should_force_alternate(
            Some("premium"),
            PlaybackBackend::Spotify
        ));
        assert!(!should_force_alternate(
            Some("PREMIUM"),
            PlaybackBackend::Alternate
        ));
    }

    #[test]
    fn profile_playback_plan_maps_once_per_account() {
        let free_from_spotify =
            profile_playback_plan(Some("FREE"), PlaybackBackend::Spotify, false);
        assert_eq!(free_from_spotify.mode, PlaybackBackend::Alternate);
        assert!(free_from_spotify.start_engine);
        assert!(free_from_spotify.persist_alternate);
        assert!(free_from_spotify.toast_free);

        let open_from_spotify =
            profile_playback_plan(Some("open"), PlaybackBackend::Spotify, false);
        assert_eq!(open_from_spotify.mode, PlaybackBackend::Alternate);
        assert!(open_from_spotify.start_engine);

        let free_already_alternate =
            profile_playback_plan(Some("free"), PlaybackBackend::Alternate, false);
        assert_eq!(free_already_alternate.mode, PlaybackBackend::Alternate);
        assert!(free_already_alternate.start_engine);
        assert!(!free_already_alternate.persist_alternate);
        assert!(!free_already_alternate.toast_free);

        let premium_spotify =
            profile_playback_plan(Some("premium"), PlaybackBackend::Spotify, false);
        assert_eq!(premium_spotify.mode, PlaybackBackend::Spotify);
        assert!(premium_spotify.start_engine);
        assert!(!premium_spotify.persist_alternate);
        assert!(!premium_spotify.toast_free);

        let premium_alternate =
            profile_playback_plan(Some("premium"), PlaybackBackend::Alternate, false);
        assert_eq!(premium_alternate.mode, PlaybackBackend::Alternate);
        assert!(premium_alternate.start_engine);
        assert!(!premium_alternate.persist_alternate);

        let again = profile_playback_plan(Some("free"), PlaybackBackend::Alternate, true);
        assert!(!again.start_engine);
        assert!(!again.persist_alternate);
        let premium_again = profile_playback_plan(Some("premium"), PlaybackBackend::Spotify, true);
        assert_eq!(premium_again.mode, PlaybackBackend::Spotify);
        assert!(!premium_again.start_engine);

        for product in [None, Some(""), Some("unknown")] {
            let plan = profile_playback_plan(product, PlaybackBackend::Spotify, false);
            assert_eq!(plan.mode, PlaybackBackend::Alternate, "{product:?}");
            assert!(plan.start_engine, "{product:?}");
            assert!(plan.persist_alternate, "{product:?}");
            assert!(!plan.toast_free, "{product:?}");
            assert!(plan.toast_unconfirmed, "{product:?}");
            let once = profile_playback_plan(product, PlaybackBackend::Alternate, true);
            assert!(!once.start_engine, "{product:?}");
        }
    }

    fn assert_local_transport_only(app: &mut App) {
        assert!(app.alternate_local() || app.transport_ready());
        assert_eq!(app.target(), Target::Local);
        assert!(!app.allows_spotify_player_api());
        app.set_volume(40, true);
        assert_eq!(app.local.volume, percent_to_volume(40));
        assert!(app.pending_remote_volume.is_none());
        app.seek(1_000);
        assert!(app.pending_remote_position.is_none());
        app.set_shuffle(true);
        app.set_repeat(RepeatMode::Context);
        app.play_request(PlayRequest::tracks(vec!["spotify:track:x".into()]), false);
        assert!(app.pending_remote_volume.is_none());
        app.add_to_queue("spotify:track:x".into(), "X".into());
        app.transfer("speaker".into());
        assert!(app.selected_device.is_none() || app.alternate_local());
        app.poll_remote(true);
        app.refresh_devices();
        app.refresh_queue(true);
        assert!(!app.remote_poll_pending);
        assert!(!app.devices_loading);
        assert!(!matches!(app.queue, Loadable::Loading));
    }

    #[test]
    fn local_transport_guard_covers_free_open_and_alternate() {
        for product in [
            Some("free"),
            Some("FREE"),
            Some("open"),
            Some("Open"),
            None,
            Some(""),
            Some("unknown"),
        ] {
            assert!(
                uses_local_transport(
                    PlaybackBackend::Spotify,
                    product,
                    &LocalPlayback::Unavailable,
                    true,
                ),
                "{product:?}"
            );
        }
        assert!(uses_local_transport(
            PlaybackBackend::Alternate,
            Some("premium"),
            &LocalPlayback::Connecting,
            true,
        ));
        assert!(uses_local_transport(
            PlaybackBackend::Alternate,
            None,
            &LocalPlayback::Failed("nope".into()),
            false,
        ));
        assert!(!uses_local_transport(
            PlaybackBackend::Spotify,
            Some("premium"),
            &LocalPlayback::Unavailable,
            true,
        ));
        assert!(!uses_local_transport(
            PlaybackBackend::Spotify,
            None,
            &LocalPlayback::Unavailable,
            false,
        ));

        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.settings.playback_authorized = true;
        app.user = Some(user_with_product("free"));
        app.local_ready = false;
        app.local_playback = LocalPlayback::Unavailable;
        app.selected_device = Some("speaker".into());
        assert_local_transport_only(&mut app);

        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.user = Some(user_with_product("open"));
        app.local_ready = false;
        app.local_playback = LocalPlayback::Unavailable;
        app.selected_device = Some("speaker".into());
        assert_local_transport_only(&mut app);

        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Alternate;
        app.user = None;
        app.local_ready = false;
        app.local_playback = LocalPlayback::Connecting;
        app.selected_device = Some("speaker".into());
        assert_local_transport_only(&mut app);

        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.user = Some(User {
            id: "user".into(),
            product: None,
            ..User::default()
        });
        app.local_ready = false;
        app.local_playback = LocalPlayback::Unavailable;
        app.selected_device = Some("speaker".into());
        assert_local_transport_only(&mut app);
    }

    #[test]
    fn volume_routing_premium_with_stored_spotify_can_be_remote() {
        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.user = Some(user_with_product("premium"));
        app.local_ready = false;
        app.local_playback = LocalPlayback::Unavailable;
        app.selected_device = Some("speaker".into());
        assert!(!app.alternate_local());
        assert_eq!(app.target(), Target::Remote(Some("speaker".into())));
        app.set_volume(25, true);
        assert_eq!(
            app.pending_remote_volume.map(|(percent, _)| percent),
            Some(25)
        );
    }

    #[test]
    fn free_profile_forces_alternate_persists_and_restarts() {
        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.settings.playback_authorized = true;
        app.selected_device = Some("speaker".into());
        app.remote_poll_pending = true;
        app.handle_api(ApiResponse::Me(Ok(user_with_product("FREE"))));
        assert_eq!(app.settings.playback_backend, PlaybackBackend::Alternate);
        assert!(app.free_alternate_applied);
        assert!(app.profile_playback_applied);
        assert!(app.selected_device.is_none());
        assert!(app.remote.is_none());
        assert_eq!(app.target(), Target::Local);
        assert!(!app.allows_spotify_player_api());
        assert!(app.settings.playback_authorized);
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message == FREE_ALTERNATE_TOAST)
        );
        let loaded = Settings::load(&app.dirs.settings_file());
        assert_eq!(loaded.playback_backend, PlaybackBackend::Alternate);

        let toast_count = app.toasts.len();
        app.handle_api(ApiResponse::Me(Ok(user_with_product("free"))));
        assert_eq!(app.toasts.len(), toast_count);
        assert!(app.profile_playback_applied);
    }

    #[test]
    fn premium_profile_keeps_spotify_backend() {
        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.handle_api(ApiResponse::Me(Ok(user_with_product("premium"))));
        assert_eq!(app.settings.playback_backend, PlaybackBackend::Spotify);
        assert!(!app.free_alternate_applied);
        assert!(app.profile_playback_applied);
        assert!(
            app.toasts
                .iter()
                .all(|toast| toast.message != FREE_ALTERNATE_TOAST)
        );
        app.handle_api(ApiResponse::Me(Ok(user_with_product("premium"))));
        assert_eq!(app.settings.playback_backend, PlaybackBackend::Spotify);
        assert!(app.profile_playback_applied);
    }

    #[test]
    fn premium_profile_keeps_user_selected_alternate() {
        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Alternate;
        app.handle_api(ApiResponse::Me(Ok(user_with_product("premium"))));
        assert_eq!(app.settings.playback_backend, PlaybackBackend::Alternate);
        assert!(!app.free_alternate_applied);
        assert!(app.profile_playback_applied);
        assert_eq!(app.target(), Target::Local);
    }

    #[test]
    fn unknown_profile_forces_alternate_and_blocks_player_api() {
        for product in [None, Some(""), Some("unknown")] {
            let mut app = test_app();
            app.settings.playback_backend = PlaybackBackend::Spotify;
            app.settings.playback_authorized = true;
            app.selected_device = Some("speaker".into());
            let user = User {
                id: "user".into(),
                display_name: Some("Test".into()),
                product: product.map(str::to_string),
                ..User::default()
            };
            app.handle_api(ApiResponse::Me(Ok(user)));
            assert_eq!(app.settings.playback_backend, PlaybackBackend::Alternate);
            assert!(app.profile_playback_applied);
            assert!(app.free_alternate_applied);
            assert!(!app.is_confirmed_premium());
            assert!(!app.allows_spotify_player_api());
            assert_eq!(app.target(), Target::Local);
            assert!(
                app.toasts
                    .iter()
                    .any(|toast| toast.message == UNCONFIRMED_ALTERNATE_TOAST),
                "{product:?}"
            );
            let loaded = Settings::load(&app.dirs.settings_file());
            assert_eq!(loaded.playback_backend, PlaybackBackend::Alternate);
            app.poll_remote(true);
            assert!(!app.remote_poll_pending);
        }
    }

    #[test]
    fn me_failure_does_not_start_playback() {
        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.handle_api(ApiResponse::Me(Err(ApiError::Status {
            status: 500,
            message: "try again".into(),
        })));
        assert!(app.user.is_none());
        assert!(!app.profile_playback_applied);
        assert_eq!(app.settings.playback_backend, PlaybackBackend::Spotify);
        assert!(!app.transport_ready());
    }

    #[test]
    fn alternate_connecting_or_failed_never_routes_remote() {
        for status in [
            LocalPlayback::Unavailable,
            LocalPlayback::Connecting,
            LocalPlayback::Authorizing,
            LocalPlayback::Failed("nope".into()),
        ] {
            let mut app = test_app();
            app.settings.playback_backend = PlaybackBackend::Alternate;
            app.local_ready = false;
            app.local_playback = status;
            app.selected_device = Some("speaker".into());
            assert!(app.alternate_local());
            assert_eq!(app.target(), Target::Local);
        }
    }

    #[test]
    fn remote_polling_is_skipped_for_free_and_alternate() {
        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Alternate;
        app.poll_remote(true);
        app.refresh_devices();
        assert!(!app.remote_poll_pending);
        assert!(!app.devices_loading);

        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.user = Some(user_with_product("free"));
        app.poll_remote(true);
        app.refresh_devices();
        assert!(!app.remote_poll_pending);
        assert!(!app.devices_loading);

        app.user = Some(user_with_product("premium"));
        app.poll_remote(true);
        app.refresh_devices();
        assert!(app.remote_poll_pending);
        assert!(app.devices_loading);
    }

    #[test]
    fn connected_without_profile_cannot_poll_or_route_remote() {
        let mut app = test_app();
        app.user = None;
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.settings.playback_authorized = true;
        app.local_ready = false;
        app.local_playback = LocalPlayback::Unavailable;
        app.selected_device = Some("speaker".into());
        app.handle_auth(AuthStatus::Connected {
            username: "x".into(),
        });
        assert!(app.user.is_none());
        assert!(!app.transport_ready());
        assert!(!app.allows_spotify_player_api());
        assert_eq!(app.target(), Target::Local);
        assert!(!app.remote_poll_pending);

        app.poll_remote(true);
        app.refresh_devices();
        app.refresh_queue(true);
        assert!(!app.remote_poll_pending);
        assert!(!app.devices_loading);
        assert!(!matches!(app.queue, Loadable::Loading));

        app.set_volume(33, true);
        assert_eq!(app.local.volume, percent_to_volume(33));
        assert!(app.pending_remote_volume.is_none());

        app.seek(2_000);
        assert!(app.pending_remote_position.is_none());

        app.play_request(PlayRequest::tracks(vec!["spotify:track:x".into()]), false);
        assert!(app.queued_play.is_some());

        let selected = app.selected_device.clone();
        app.transfer("speaker".into());
        assert_eq!(app.selected_device, selected);
        app.add_to_queue("spotify:track:x".into(), "X".into());
        assert!(!matches!(app.queue, Loadable::Loading));
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message == "Loading account...")
        );
    }

    #[test]
    fn premium_required_volume_switches_once_without_error_storm() {
        let mut app = test_app();
        app.settings.playback_backend = PlaybackBackend::Spotify;
        app.pending_remote_volume = Some((40, Instant::now()));
        app.handle_api(ApiResponse::Remote {
            action: RemoteAction::Volume,
            result: Err(premium_required()),
        });
        assert_eq!(app.settings.playback_backend, PlaybackBackend::Alternate);
        assert_eq!(app.target(), Target::Local);
        assert_eq!(app.local.volume, percent_to_volume(40));
        assert!(app.pending_remote_volume.is_none());
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message == UNCONFIRMED_ALTERNATE_TOAST)
        );
        assert!(
            !app.toasts
                .iter()
                .any(|toast| toast.kind == ToastKind::Error)
        );

        app.handle_api(ApiResponse::Remote {
            action: RemoteAction::Volume,
            result: Err(premium_required()),
        });
        assert_eq!(
            app.toasts
                .iter()
                .filter(|toast| toast.message == UNCONFIRMED_ALTERNATE_TOAST)
                .count(),
            1
        );
        assert!(
            !app.toasts
                .iter()
                .any(|toast| toast.kind == ToastKind::Error)
        );
    }

    fn headless_app() -> App {
        let root =
            std::env::temp_dir().join(format!("fastpotify-volume-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let mut app = App::new(
            &Waker::default(),
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.user = Some(user_with_product("premium"));
        app.profile_playback_applied = true;
        app.local_ready = true;
        app
    }

    fn snapshot_at(percent: u8) -> LocalState {
        LocalState {
            volume: percent_to_volume(percent),
            ..LocalState::default()
        }
    }

    #[test]
    fn a_volume_set_here_is_saved_immediately() {
        let mut app = headless_app();
        app.set_volume(80, true);
        assert_eq!(volume_to_percent(app.settings.volume), 80);
        assert!(app.settings_dirty);
    }

    #[test]
    fn a_stale_engine_snapshot_does_not_pull_the_volume_back() {
        let mut app = headless_app();
        app.set_volume(80, true);

        // The engine reports `VolumeChanged` asynchronously, so its next
        // snapshot still carries the volume from before the change.
        app.handle_local(snapshot_at(20));
        assert_eq!(volume_to_percent(app.local.volume), 80);
        assert_eq!(volume_to_percent(app.settings.volume), 80);

        // Once it has caught up, its snapshots are trusted again.
        app.handle_local(snapshot_at(80));
        assert_eq!(volume_to_percent(app.local.volume), 80);
    }

    #[test]
    fn a_volume_changed_outside_the_app_is_adopted() {
        let mut app = headless_app();
        app.handle_local(snapshot_at(35));
        assert_eq!(volume_to_percent(app.local.volume), 35);
        assert_eq!(volume_to_percent(app.settings.volume), 35);
    }

    /// What a Raycast script sends becomes the same action a menu pick or a
    /// media key would produce.
    #[test]
    fn a_control_command_becomes_the_action_it_names() {
        // #given
        let mut app = headless_app();
        let queue: std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>> = Default::default();
        app.control_commands = Some(std::sync::Arc::clone(&queue));

        // #when
        queue.lock().expect("the queue").extend([
            ControlCommand::Next,
            ControlCommand::Previous,
            ControlCommand::SeekBy(-15_000),
            ControlCommand::VolumeBy(10),
            ControlCommand::SetVolume(240),
            ControlCommand::ToggleShuffle,
            ControlCommand::Show,
        ]);
        app.handle_control_commands();

        // #then
        assert!(
            matches!(
                app.actions.as_slice(),
                [
                    Action::Next,
                    Action::Previous,
                    Action::SeekBy(-15_000),
                    Action::VolumeBy(10),
                    // A percentage above the scale is clamped, not wrapped.
                    Action::SetVolume(100),
                    Action::ToggleShuffle,
                    Action::ShowWindow,
                ]
            ),
            "{:?}",
            app.actions
        );
        assert!(queue.lock().expect("the queue").is_empty());
    }

    /// `play` and `pause` say what state to end in, so the one that would
    /// undo the current state does nothing.
    #[test]
    fn play_and_pause_do_not_toggle_the_wrong_way() {
        let mut app = headless_app();
        let queue: std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>> = Default::default();
        app.control_commands = Some(std::sync::Arc::clone(&queue));

        // Nothing is playing in a headless app, so `pause` has nothing to do
        // and `play` asks for the toggle.
        queue
            .lock()
            .expect("the queue")
            .extend([ControlCommand::Pause, ControlCommand::Play]);
        app.handle_control_commands();

        assert!(
            matches!(app.actions.as_slice(), [Action::TogglePlay]),
            "{:?}",
            app.actions
        );
    }
}

//! The application: state, event handling, and the actions views ask for.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use egui::Color32;

use crate::api::PlayRequest;
use crate::api::models::{
    ArtistRef, Device, PlayableItem, PlaybackState, Playlist, Queue, Track, User, pick_image,
};
use crate::backend::{
    ApiRequest, ApiResponse, AuthStatus, Backend, Command, Event, LocalPlayback, LyricsRequest,
    PLAYLIST_PAGE_SIZE, RemoteAction, Waker,
};
use crate::media::{MediaCommand, MediaState, MediaTrack};
use crate::media_controls::MediaService;
use crate::model::*;
use crate::paths::AppDirs;
use crate::player::{EngineConfig, LoadSpec, LocalState, Playback, PlayerCommand, RepeatMode};
use crate::settings::{SessionState, Settings, ThemeChoice};
use crate::single_instance::ControlCommand;
use crate::theme::{self, Palette};
use crate::tray::{TrayCommand, TrayService};
use crate::util;

const REMOTE_POLL_ACTIVE: Duration = Duration::from_secs(4);
const REMOTE_POLL_IDLE: Duration = Duration::from_secs(20);
const REMOTE_FRESH: Duration = Duration::from_secs(45);
const DEVICES_FRESH: Duration = Duration::from_secs(12);
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(280);
/// How far into a song Previous restarts it rather than stepping back,
/// matching what librespot does during playback.
const RESTART_BEFORE_PREVIOUS: u32 = 3_000;

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
/// A second look at the queue after a change made here, because Spotify's
/// queue endpoint can answer with a snapshot from before the change.
const QUEUE_RECHECK: Duration = Duration::from_millis(700);
/// How many stale queue answers are asked again before Spotify's version
/// of events wins anyway.
const QUEUE_STALE_RETRIES: u8 = 6;
/// Within this window a second Play next of the same song is the same
/// click; beyond it, it is a second wish and queues a second row.
const QUEUE_ADD_DEBOUNCE: Duration = Duration::from_millis(1500);
const CONTAINS_BATCH: usize = 40;

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
    /// The remembered song from the last session, shown paused before a
    /// first press. Nothing is playing yet.
    pub resuming: bool,
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
    /// The window should close and reopen at once as the other kind: the
    /// big window or the Winamp mini player.
    pub switch_intent: bool,
    /// Commands from control clients (a second `fastpotify <verb>` launch,
    /// a Raycast script), on the platforms where they do not arrive through
    /// MPRIS. Drained every frame.
    control_commands: Option<std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>>>,
    /// Where the now-playing snapshot goes for the control channel's
    /// `nowplaying` verb to answer from.
    control_now_playing: Option<std::sync::Arc<std::sync::Mutex<String>>>,
    /// The same, for its `devices` verb.
    control_devices: Option<std::sync::Arc<std::sync::Mutex<String>>>,
    /// Whether that device slot still matches [`Self::devices`]. The
    /// now-playing snapshot is rebuilt every frame because its position
    /// moves every frame; a device list changes when Spotify answers, which
    /// is seconds apart, so it is written when it changes instead.
    control_devices_stale: bool,
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
    /// The restorable session (sorts, recents, resume point) changed and
    /// should be written shortly, not only at exit.
    pub session_dirty: bool,
    last_session_save: Instant,
    /// The saved zoom has been applied to the context once.
    zoom_applied: bool,
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
    /// The latest queue request sent; an answer to an older one is a
    /// story already overtaken and is dropped unread.
    queue_seq: u64,
    /// When to look at the queue again because the last answer told a
    /// story from before the user's latest change.
    queue_recheck_at: Option<Instant>,
    queue_stale_retries: u8,
    /// What a Clear queue just removed, so a fetched queue that still
    /// carries those rows is recognised as stale.
    queue_cleared: Option<(std::collections::HashSet<String>, Instant)>,
    /// What the window's title bar says, as last set.
    window_title: String,

    pub library: Library,
    pub home: HomeData,
    pub search: SearchState,
    pub playlist_pages: HashMap<String, PlaylistPage>,
    load_generation: u64,
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
    /// The plain list of songs local playback was last given, so autoplay
    /// can carry on from its last one when it ends: librespot only
    /// continues from a context with a URI, and a list has none.
    local_list: Option<Vec<String>>,
    /// A receiver just activated, waiting for Spotify to list it so playback
    /// can move there.
    pending_transfer_to: Option<(String, Instant)>,
    /// When to take a confirming look at remote playback after a command.
    remote_recheck_at: Option<Instant>,
    pub seek_preview: Option<f32>,
    pub volume_preview: Option<f32>,
    /// Window geometry to restore on next attach, from the session file.
    session_window_size: Option<[f32; 2]>,
    session_window_pos: Option<[f32; 2]>,
    /// Last observed window geometry, updated each frame for saving.
    last_window_size: Option<[f32; 2]>,
    last_window_pos: Option<[f32; 2]>,
    /// Where the MilkDrop window last was, as it reported, for restoring it.
    pub milkdrop_pos: Option<[f32; 2]>,
    /// The MilkDrop child process; `None` until it is first opened. Its
    /// `Drop` stops the child when the app does.
    #[cfg(feature = "milkdrop")]
    milkdrop_host: Option<crate::milkdrop::host::Host>,
    last_eviction: Instant,
    pub sign_in_url: Option<String>,
    /// The verified personal Web API application, when acceleration is ready.
    pub web_app: Option<String>,
    pending_remote_position: Option<(u32, Instant)>,
    pending_remote_volume: Option<(u8, Instant)>,
    /// A local volume set here that the engine has not echoed back yet. It
    /// reports `VolumeChanged` asynchronously while position snapshots land
    /// every second, so a snapshot must not undo the change on its way past.
    pending_local_volume: Option<(u16, Instant)>,
    optimistic_playing: Option<(bool, Instant)>,
    /// The track a play just asked for, marked current at once; the
    /// engine's report (or time) hands back to the reported state.
    intent_track: Option<(String, Instant)>,
    /// Shuffle as the listener set it: a mode, not a property of one
    /// context. Every play of a context applies it until turned off.
    shuffle_wanted: bool,
    /// When the listener last set shuffle here, so an echo of that same
    /// change from the engine is not mistaken for another client's toggle.
    shuffle_set_at: Option<Instant>,
    /// When tracks recently came up unavailable, to spot a key-service
    /// cascade and reconnect once instead of skipping through an album.
    unavailable_at: Vec<Instant>,
    last_unavailable_reconnect: Option<Instant>,
    /// The Premium notice has been shown for this sign-in.
    premium_notice_shown: bool,
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
    pub user_names_revision: u64,
    /// Context URIs most recently played, newest first: the sidebar's
    /// order. Kept with the session, so it survives a restart.
    pub recent_contexts: Vec<String>,
    /// What was playing when the app last closed, to resume from cold.
    pub resume_context: Option<String>,
    pub resume_track: Option<String>,
    pub resume_position_ms: u32,
    /// The songs that were queued by hand, queued again when it resumes.
    pub resume_queue: Vec<String>,
    /// What the listener queued by hand this session, oldest first; the
    /// context's own upcoming songs never belong here.
    pub manual_queue: Vec<String>,
    /// Adds shown in the queue before Spotify confirms them: the uri and
    /// when it was asked, so a slow answer cannot erase or double them.
    pending_queue_adds: Vec<(String, Instant)>,
    /// The account's playlist tree from Spotify, folders and all; empty
    /// until the session answers.
    pub rootlist: Vec<crate::player::RootlistEntry>,
    /// Sidebar folders rolled up, by their rootlist ids.
    pub collapsed_folders: Vec<String>,
    /// A newer release than this build, once GitHub has said so.
    pub update: Option<crate::updates::Release>,
    last_update_check: Option<Instant>,
    /// The Winamp window and the skin it wears.
    pub winamp: crate::winamp::WinampState,
    /// The WMP skin the skin window wears, read like the Winamp skin.
    pub wmp: crate::wmp::WmpState,
    /// Watching a window we minimized come back, because the interface
    /// freezes until egui is told it did. A field a painter may write
    /// while a skin is borrowed.
    pub(crate) restore_watch: RestoreWatch,
}

/// A window we told to minimize: egui is told on macOS only from our
/// own command, never again from the window itself, so a window
/// restored by hand still reads as minimized and no pass runs. The
/// watch says the window is back once it has a place again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RestoreWatch {
    #[default]
    None,
    /// The command went out; the window has not been seen gone yet.
    Sent,
    /// The window has been seen without a place; waiting for it back.
    Gone,
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
        let tap = crate::vis::AudioTap::new();
        let eq = crate::eq::shared();
        if let Ok(mut shared) = eq.lock() {
            *shared = eq_settings(&settings);
        }
        let engine_config = engine_config(
            &dirs,
            &settings,
            std::sync::Arc::clone(&tap),
            std::sync::Arc::clone(&eq),
        );
        let backend = Backend::spawn(
            dirs.clone(),
            engine_config,
            settings.web_client_id.clone(),
            waker.clone(),
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
            switch_intent: false,
            control_commands: None,
            control_now_playing: None,
            control_devices: None,
            control_devices_stale: true,
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
            session_dirty: false,
            last_session_save: Instant::now(),
            zoom_applied: false,
            devices: Vec::new(),
            receivers: Vec::new(),
            activating_receiver: None,
            devices_loading: false,
            devices_fetched_at: None,
            selected_device: None,
            queue: if session.last_track.is_some() && !session.last_queue_rows.is_empty() {
                // The queue as it was at close, shown until something
                // plays; then the live queue takes over.
                Loadable::Loaded(Queue {
                    currently_playing: None,
                    queue: session.last_queue_rows.clone(),
                })
            } else {
                Loadable::NotLoaded
            },
            queue_fetched_at: None,
            queue_seq: 0,
            queue_recheck_at: None,
            queue_stale_retries: 0,
            queue_cleared: None,
            window_title: String::new(),
            library: Library::default(),
            home: HomeData::default(),
            search: SearchState::default(),
            playlist_pages: HashMap::new(),
            load_generation: 0,
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
            show_queue_panel: session.queue_open.unwrap_or(false),
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
            local_list: None,
            pending_transfer_to: None,
            remote_recheck_at: None,
            seek_preview: None,
            volume_preview: None,
            session_window_size: session.window_size,
            session_window_pos: session.window_pos,
            last_window_size: None,
            last_window_pos: None,
            milkdrop_pos: session.milkdrop_pos,
            #[cfg(feature = "milkdrop")]
            milkdrop_host: None,
            last_eviction: Instant::now(),
            sign_in_url: None,
            web_app: None,
            pending_remote_position: None,
            pending_remote_volume: None,
            pending_local_volume: None,
            optimistic_playing: None,
            intent_track: None,
            shuffle_wanted: session.shuffle_on,
            shuffle_set_at: None,
            unavailable_at: Vec::new(),
            last_unavailable_reconnect: None,
            premium_notice_shown: false,
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
            table_sorts: session
                .sorts
                .iter()
                .filter_map(|(page, sort)| Some((Page::decode(page)?, *sort)))
                .collect(),
            user_names: HashMap::new(),
            user_names_revision: 0,
            recent_contexts: session.recent_contexts.clone(),
            resume_context: session.last_context.clone(),
            resume_track: session.last_track.clone(),
            resume_position_ms: session.last_position_ms,
            resume_queue: session.last_added_queue.clone(),
            manual_queue: Vec::new(),
            pending_queue_adds: Vec::new(),
            rootlist: Vec::new(),
            collapsed_folders: session.collapsed_folders.clone(),
            update: None,
            last_update_check: None,
            winamp: crate::winamp::WinampState::new(session.winamp_pos, tap, eq),
            wmp: crate::wmp::WmpState::default(),
            restore_watch: RestoreWatch::None,
        };
        app.local.volume = app.settings.volume;
        app.wmp.restore_pos = session.wmp_pos;
        app
    }

    /// Watches the queue control clients fill and keeps the snapshots they
    /// read -- now playing, and the device list -- fresh.
    pub fn set_remote_control(&mut self, guard: &crate::single_instance::Guard) {
        self.control_commands = Some(guard.commands());
        self.control_now_playing = Some(guard.now_playing_slot());
        self.control_devices = Some(guard.devices_slot());
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
        self.winamp.forget_textures();
        self.wmp.forget_textures();
        self.window_hidden = false;
        self.hide_intent = false;
        self.wants_show = false;
        self.switch_intent = false;
        if let Some(tray) = &mut self.tray {
            tray.attach();
        }
        if self.settings.winamp_window {
            // The mini player sizes itself; the big window's geometry
            // waits here for its return.
            return;
        }
        if let Some(size) = self.session_window_size.take() {
            // Clamp to a sane range so a stale session never creates an
            // unusable window; the OS will further clamp to the monitor.
            if (400.0..=3000.0).contains(&size[0]) && (300.0..=2000.0).contains(&size[1]) {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
        }
        if let Some(pos) = self.session_window_pos.take() {
            // On Wayland this is a no-op. Validate against a large virtual
            // desktop so a window that was on a now-disconnected monitor
            // doesn't open off-screen.
            if (-1000.0..=5000.0).contains(&pos[0]) && (-1000.0..=5000.0).contains(&pos[1]) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
            }
        }
        // egui's consensus wheel speed is 40 points per line, about a third
        // of what every other player scrolls per notch; trackpads report
        // pixels and are unaffected (#32).
        ctx.options_mut(|options| options.input_options.line_scroll_speed = 120.0);
    }

    /// The window is gone but the process stays: audio, the tray, and the
    /// media controls keep running until Show or Quit.
    pub fn window_gone(&mut self) {
        // The Winamp window went with it; it comes back where it was.
        self.winamp.remember_position();
        self.winamp.forget_textures();
        // The skin window went with it too; it comes back where it was.
        self.wmp.remember_position();
        self.window_hidden = true;
        self.hide_intent = false;
        self.wants_show = false;
        if let Some(tray) = &mut self.tray {
            tray.hidden();
        }
    }

    /// Whether closing the window keeps the app in the tray rather than
    /// quitting.
    pub fn hides_to_tray(&self) -> bool {
        self.tray.is_some() && self.settings.keep_playing_in_background
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

    /// The context playing as the interface should show it: the one just
    /// asked for until Spotify's state confirms it, then Spotify's own.
    pub fn playing_context_uri(&self) -> Option<String> {
        let remote = self
            .remote
            .as_ref()
            .and_then(|remote| remote.state.context.as_ref())
            .map(|context| context.uri.clone());
        if let Some(assumed) = &self.assumed_context {
            let held = assumed.at.elapsed() < ASSUMED_CONTEXT_HOLD;
            // A view of a context plays as plain tracks, so no state will
            // ever name the context: the assumption stands while nothing
            // names another one and something is believed to be playing.
            let contradicted = remote.as_deref().is_some_and(|uri| uri != assumed.uri);
            if held || (!contradicted && self.believed_playing()) {
                return Some(assumed.uri.clone());
            }
        }
        remote
    }

    /// Whether the playing context shuffles, honouring a shuffle the
    /// interface just asked for ahead of Spotify's state.
    /// The track the interface should mark as current: the one a click
    /// just asked for, until a report names it or the moment passes.
    pub fn current_track_uri(&self) -> Option<String> {
        if let Some((uri, at)) = &self.intent_track
            && at.elapsed() < PLAYBACK_HOLD
        {
            return Some(uri.clone());
        }
        self.now_playing().map(|now| now.uri)
    }

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
        self.shuffle_wanted
    }

    /// The playing thing as a playable item, for menus that act on it:
    /// the cached full track when known, a minimal one otherwise.
    pub fn now_playing_item(&self) -> Option<PlayableItem> {
        let now = self.now_playing()?;
        if now.is_episode {
            return None;
        }
        if let Some(track) = now.id.as_deref().and_then(|id| self.track_cache.get(id)) {
            return Some(PlayableItem::Track(track.clone()));
        }
        Some(PlayableItem::Track(Track {
            id: now.id.clone(),
            uri: now.uri.clone(),
            name: now.title.clone(),
            artists: now.artists.clone(),
            duration_ms: now.duration_ms,
            ..Track::default()
        }))
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        self.now_playing_live().or_else(|| self.resume_preview())
    }

    /// What a device is actually playing, here or elsewhere.
    fn now_playing_live(&self) -> Option<NowPlaying> {
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
                shuffle: self.shuffle_wanted,
                repeat: self.local.repeat,
                volume_percent: volume_to_percent(self.local.volume),
                can_control: true,
                is_episode: track.is_episode,
                resuming: false,
            });
        }
        let remote = self.remote_fresh()?;
        // When the snapshot names this very computer as its device, the
        // local engine is the truth, and it just said it has nothing: a
        // poll from before a stop must not say otherwise.
        if self.local_device_id.is_some()
            && remote
                .state
                .device
                .as_ref()
                .and_then(|device| device.id.as_deref())
                == self.local_device_id.as_deref()
        {
            return None;
        }
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
            shuffle: self.shuffle_wanted,
            repeat: RepeatMode::from_api(&remote.state.repeat_state),
            volume_percent: volume,
            can_control: device.is_none_or(|device| !device.is_restricted),
            is_episode,
            resuming: false,
        })
    }

    /// The song the last session ended on, drawn paused at the position it
    /// stopped at so the listener can see what a press of play will resume.
    /// Only ever offered when no device is playing anything.
    fn resume_preview(&self) -> Option<NowPlaying> {
        let uri = self.resume_track.as_deref()?;
        let track = self.track_cache.get(util::uri_id(uri)?)?;
        Some(NowPlaying {
            local: true,
            device_name: None,
            uri: uri.to_string(),
            id: track.id.clone(),
            title: track.name.clone(),
            subtitle: track.artist_names(),
            artists: track.artists.clone(),
            album_name: track
                .album
                .as_ref()
                .map(|album| album.name.clone())
                .unwrap_or_default(),
            album_id: track.album.as_ref().map(|album| album.id.clone()),
            show_id: None,
            art_url: track.image(640).map(str::to_string),
            art_small: track.image(64).map(str::to_string),
            duration_ms: track.duration_ms,
            position_ms: self.resume_position_ms.min(track.duration_ms),
            playing: false,
            loading: false,
            shuffle: self.shuffle_wanted,
            repeat: RepeatMode::Off,
            volume_percent: volume_to_percent(self.local.volume),
            can_control: true,
            is_episode: false,
            resuming: true,
        })
    }

    /// The play request for `key` (a context or track URI) is still waiting
    /// for Spotify to react.
    pub fn play_pending(&self, key: &str) -> bool {
        self.pending_fresh() && self.pending_play_keys.iter().any(|k| k == key)
    }

    pub fn set_user_name(&mut self, id: String, name: Option<String>) {
        if self.user_names.get(&id) != Some(&name) {
            self.user_names.insert(id, name);
            self.user_names_revision = self.user_names_revision.wrapping_add(1);
        }
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
                Event::Rootlist { result } => match result {
                    Ok(entries) => self.rootlist = entries,
                    Err(error) => log::warn!("rootlist unavailable: {error}"),
                },
                Event::Lyrics { uri, result } => {
                    if self.lyrics_uri.as_deref() == Some(uri.as_str()) {
                        self.lyrics = match result {
                            Ok(found) => Loadable::Loaded(found),
                            Err(error) => Loadable::Failed(error),
                        };
                    }
                }
                Event::PlaylistCache {
                    account_id,
                    id,
                    snapshot,
                    items,
                } => {
                    if self.user_id() != Some(account_id.as_str()) {
                        continue;
                    }
                    if let Some(page) = self.playlist_pages.get_mut(&id) {
                        page.pending_cache = Some((snapshot, items));
                    }
                    self.try_adopt_playlist_cache(&id);
                }
                Event::UserName { id, name } => {
                    self.set_user_name(id, name);
                }
                Event::WebApp { client_id } => self.web_app = client_id,
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
                self.poll_remote(true);
            }
            AuthStatus::WaitingForBrowser { url } => self.sign_in_url = Some(url.clone()),
            AuthStatus::SignedOut => {
                self.sign_in_url = None;
                self.web_app = None;
                self.user = None;
                self.premium_notice_shown = false;
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
        self.control_devices_stale = true;
        self.devices_fetched_at = None;
        self.search.results = Loadable::NotLoaded;
        self.search.committed.clear();
    }

    fn handle_local(&mut self, state: LocalState) {
        let track_changed = state.track != self.local.track;
        let reconnected = state.connected && !self.local.connected;
        if state.shuffle != self.local.shuffle
            && self
                .shuffle_set_at
                .is_none_or(|at| at.elapsed() > Duration::from_secs(5))
        {
            // Another client toggled it; that is the listener too.
            self.shuffle_wanted = state.shuffle;
        }
        if state.playback != self.local.playback {
            self.optimistic_playing = None;
            if matches!(state.playback, Playback::Playing | Playback::Loading) {
                self.clear_play_pending();
            }
        }
        if state.track != self.local.track {
            self.clear_play_pending();
            if let (Some(track), Some((intent, _))) = (&state.track, &self.intent_track)
                && track.uri == *intent
            {
                // The engine reports the very track that was asked for.
                self.intent_track = None;
            }
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
            // One unavailable track is Spotify's catalogue; several in a
            // row is the session's audio-key service gone bad, which
            // leaves librespot feeding the decoder encrypted bytes and
            // skipping through the whole album. A fresh session cures it.
            if error.starts_with("This item isn't available") {
                let now = Instant::now();
                self.unavailable_at
                    .retain(|at| now.duration_since(*at) < Duration::from_secs(20));
                self.unavailable_at.push(now);
                if self.unavailable_at.len() >= 3
                    && self
                        .last_unavailable_reconnect
                        .is_none_or(|at| at.elapsed() > Duration::from_secs(60))
                {
                    self.unavailable_at.clear();
                    self.last_unavailable_reconnect = Some(now);
                    self.backend.send(Command::Reconnect);
                    self.toast("Spotify's audio service faltered; reconnecting local playback");
                }
            }
        }
        if let Some(seed) = autoplay_seed(
            self.local_list.as_deref(),
            self.settings.autoplay,
            &self.local,
            &state,
        ) {
            log::info!("the list ended; playing what Spotify follows {seed} with");
            self.local_list = None;
            self.backend.player(PlayerCommand::Load(LoadSpec {
                context_uri: Some(seed),
                play: true,
                autoplay: true,
                ..LoadSpec::default()
            }));
        }
        self.local = state;
        if let Some(volume) = held_volume {
            self.local.volume = volume;
        }
        if track_changed {
            self.on_now_playing_changed();
        }
        if reconnected {
            if let Some(request) = self.queued_play.take() {
                self.play_request(request, false);
            }
            // Names asked about before the session existed never got an
            // answer and showed as bare ids; ask again now that someone
            // can answer.
            let unresolved: Vec<String> = self
                .user_names
                .iter()
                .filter(|(_, name)| name.is_none())
                .map(|(id, _)| id.clone())
                .collect();
            for id in &unresolved {
                self.user_names.remove(id);
            }
            self.request_user_names(unresolved);
        }
    }

    /// Fetch the remembered song's details so the player bar can show it
    /// before anything plays. Asked for once, and only while nothing is
    /// playing: a live song needs no preview.
    fn request_resume_track(&mut self) {
        if self.now_playing_live().is_some() {
            return;
        }
        let Some(uri) = self.resume_track.clone() else {
            return;
        };
        // Episodes are not in the track endpoint; the preview skips them.
        if !uri.starts_with("spotify:track:") {
            return;
        }
        let Some(id) = util::uri_id(&uri).map(str::to_string) else {
            return;
        };
        if self.track_cache.contains_key(&id) || !self.track_requests.insert(id.clone()) {
            return;
        }
        self.backend.api(ApiRequest::Track { id });
    }

    /// True while the player bar is showing the remembered song and no
    /// device is playing anything: the song is loaded and current, but not
    /// playing, and the transport acts on it here rather than on an engine
    /// that has nothing to skip.
    fn resume_only(&self) -> bool {
        self.resume_track.is_some() && self.now_playing_live().is_none()
    }

    /// Load the rows of the context the last session was left in, so the
    /// remembered song knows its neighbours before anything plays. The
    /// playlist cache on disk usually answers this without a request.
    fn ensure_resume_context_loaded(&mut self) {
        if !self.resume_only() {
            return;
        }
        let Some(context) = self.resume_context.clone() else {
            return;
        };
        if self.context_track_uris(&context).is_some() {
            return;
        }
        if let Some(page) = Page::decode(&Self::context_page(&context)) {
            self.ensure_loaded(page);
        }
    }

    /// The page that shows a context, in the encoded form `Page::decode`
    /// takes.
    fn context_page(context_uri: &str) -> String {
        if context_uri.ends_with(":collection") {
            return "liked".to_owned();
        }
        match (util::uri_kind(context_uri), util::uri_id(context_uri)) {
            (Some(kind), Some(id)) => format!("{kind}:{id}"),
            _ => String::new(),
        }
    }

    /// Step the remembered song to its neighbour in the context, without
    /// playing anything: the song stays loaded and paused, at its start.
    /// `false` when there is no list to step through.
    fn step_resume(&mut self, forward: bool) -> bool {
        let Some(context) = self.resume_context.clone() else {
            return false;
        };
        let Some(uris) = self.context_track_uris(&context) else {
            return false;
        };
        let current = self.resume_track.clone().unwrap_or_default();
        let next = if self.shuffle_wanted && forward {
            // Shuffle is the listener's mode, and it outlives a close: a
            // skip under it lands somewhere else in the context, as a skip
            // during playback would.
            let choices: Vec<&String> = uris.iter().filter(|uri| **uri != current).collect();
            if choices.is_empty() {
                return false;
            }
            choices[rand::random_range(0..choices.len())].clone()
        } else {
            let Some(index) = uris.iter().position(|uri| *uri == current) else {
                return false;
            };
            let last = uris.len() - 1;
            let target = match (forward, index) {
                (true, i) if i == last => 0,
                (true, i) => i + 1,
                (false, 0) => last,
                (false, i) => i - 1,
            };
            uris[target].clone()
        };
        self.cache_track_from_context(&context, &next);
        self.resume_track = Some(next);
        self.resume_position_ms = 0;
        self.session_dirty = true;
        true
    }

    /// Take a song's details from the context's own rows, so a skip shows
    /// the new song at once instead of blanking the bar until a request
    /// for it comes back.
    fn cache_track_from_context(&mut self, context_uri: &str, uri: &str) {
        let Some(id) = util::uri_id(uri) else {
            return;
        };
        if self.track_cache.contains_key(id) {
            return;
        }
        let found = if let Some(pid) = context_uri.strip_prefix("spotify:playlist:") {
            self.playlist_pages.get(pid).and_then(|page| {
                page.items
                    .items
                    .iter()
                    .find_map(|item| match item.playable() {
                        Some(PlayableItem::Track(track)) if track.uri == uri => Some(track.clone()),
                        _ => None,
                    })
            })
        } else if let Some(aid) = context_uri.strip_prefix("spotify:album:") {
            self.album_pages.get(aid).and_then(|page| {
                page.tracks
                    .items
                    .iter()
                    .find(|track| track.uri == uri)
                    .cloned()
            })
        } else if context_uri.ends_with(":collection") {
            self.library
                .liked
                .items
                .iter()
                .find(|item| item.track.uri == uri)
                .map(|item| item.track.clone())
        } else {
            None
        };
        if let Some(track) = found {
            self.track_cache.insert(id.to_owned(), track);
        }
    }

    fn on_now_playing_changed(&mut self) {
        let Some(now) = self.now_playing() else {
            return;
        };
        // The remembered song is not a song that started: taking it for one
        // would rewind the very position it is there to show.
        if now.resuming {
            return;
        }
        if self.last_now_playing_uri.as_deref() == Some(now.uri.as_str()) {
            return;
        }
        // The queue saved with the remembered song comes back when that song
        // resumes; starting anything else instead lets it go, the way
        // Spotify's own queue does not follow a fresh start.
        if !self.resume_queue.is_empty() {
            let queued = std::mem::take(&mut self.resume_queue);
            self.session_dirty = true;
            if now.local && self.resume_track.as_deref() == Some(now.uri.as_str()) {
                for uri in queued {
                    self.manual_queue.push(uri.clone());
                    self.backend.api(ApiRequest::AddToQueue {
                        uri,
                        device_id: self.local_device_id.clone(),
                        label: String::new(),
                    });
                }
            }
        }
        // A hand-queued song that starts has been consumed.
        if self.manual_queue.first().map(String::as_str) == Some(now.uri.as_str()) {
            self.manual_queue.remove(0);
            self.session_dirty = true;
        }
        // The queue view follows at once: the song that just started stops
        // being next up without waiting for the Web API, whose answer lags
        // this moment by a round trip or more.
        if let Loadable::Loaded(queue) = &mut self.queue {
            let accounted = queue
                .currently_playing
                .as_ref()
                .is_some_and(|item| item.uri() == now.uri);
            if !accounted
                && queue
                    .queue
                    .first()
                    .is_some_and(|item| item.uri() == now.uri)
            {
                let item = queue.queue.remove(0);
                queue.currently_playing = Some(item);
            }
        }
        self.last_now_playing_uri = Some(now.uri.clone());
        self.resume_context = self.playing_context_uri();
        self.resume_track = Some(now.uri.clone());
        self.resume_position_ms = 0;
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
        if matches!(self.page(), Page::Queue)
            || self.show_queue_panel
            || (self.settings.winamp_window && self.settings.playlist_open)
        {
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

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if !self.zoom_applied {
            self.zoom_applied = true;
            let zoom = self.settings.zoom.clamp(0.5, 2.5);
            if (zoom - 1.0).abs() > 0.001 {
                ctx.set_zoom_factor(zoom);
            }
        } else {
            let zoom = ctx.zoom_factor();
            if (zoom - self.settings.zoom).abs() > 0.001 {
                // Ctrl+plus/minus zoomed; remembered for the next start.
                self.settings.zoom = zoom;
                self.settings_dirty = true;
            }
        }
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
            self.request_resume_track();
            self.ensure_resume_context_loaded();
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
            let playlist_open = self.settings.winamp_window && self.settings.playlist_open;
            if (self.show_queue_panel || matches!(self.page(), Page::Queue) || playlist_open)
                && !self.queue.is_loading()
                && self
                    .queue_fetched_at
                    .is_none_or(|at| at.elapsed() > Duration::from_secs(20))
            {
                self.refresh_queue(false);
            }
            if let Some(due) = self.queue_recheck_at {
                if Instant::now() >= due {
                    self.queue_recheck_at = None;
                    self.refresh_queue(true);
                } else {
                    ctx.request_repaint_after(
                        (due - Instant::now()).max(Duration::from_millis(50)),
                    );
                }
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
        self.sync_skin(ctx);
        if self.settings_dirty && self.last_settings_save.elapsed() > Duration::from_secs(2) {
            self.save_settings();
        }
        if self.session_dirty && self.last_session_save.elapsed() > Duration::from_secs(2) {
            self.save_session();
        }
    }

    /// Keeps the WMP skin window wearing the skin the settings name,
    /// the way the Winamp window does.
    fn sync_wmp_skin(&mut self, ctx: &egui::Context) {
        if self.settings.wmp_window
            && !self.wmp.is_loading()
            && self.wmp.worn != self.settings.wmp_skin
        {
            match self.settings.wmp_skin.clone() {
                None => self.wmp.skin = None,
                Some(name) => self.wmp.load(name, &self.dirs.skins_dir(), ctx),
            }
        }
    }

    /// Keeps the Winamp window wearing the skin the settings name: starts
    /// reading a newly chosen one, and puts it on once it is read. A skin
    /// that cannot be read is announced and the setting goes back to the
    /// one still on, so nothing is retried forever.
    fn sync_skin(&mut self, ctx: &egui::Context) {
        if self.settings.winamp_window
            && !self.winamp.is_loading()
            && self.winamp.worn != self.settings.skin
        {
            match self.settings.skin.clone() {
                None => self.winamp.wear(None, crate::skin::Skin::builtin()),
                Some(name) => self.winamp.load(name, &self.dirs.skins_dir(), ctx),
            }
        }
        if let Some(loaded) = self.winamp.poll() {
            self.skin_loaded(loaded);
        }
        self.sync_wmp_skin(ctx);
        if let Some(loaded) = self.wmp.poll() {
            self.wmp_loaded(ctx, loaded);
        }
        let fetched = self.winamp.presets.poll();
        if let Some(fetched) = fetched {
            match fetched {
                Ok(count) => {
                    self.toast(format!("Added {count} MilkDrop presets"));
                    // The window lists presets when it starts; a running one
                    // is bounced so the new arrivals play. It reopens on the
                    // next frame, since the setting still says open.
                    #[cfg(feature = "milkdrop")]
                    if let Some(host) = self.milkdrop_host.as_mut()
                        && host.is_running()
                    {
                        host.close();
                    }
                }
                Err(error) => self.toast_error(format!("Couldn't fetch presets: {error}")),
            }
        }
    }

    /// Keeps the MilkDrop child process in step with the settings, and takes
    /// back where its window sits and whether it was closed from there.
    #[cfg(feature = "milkdrop")]
    fn sync_milkdrop(&mut self, ctx: &egui::Context) {
        let presets = self.dirs.milkdrop_dir();
        let open = self.settings.milkdrop_open;
        let size = self.settings.milkdrop_size;
        let pos = self.milkdrop_pos;
        let fullscreen = self.settings.milkdrop_fullscreen;
        let fps = self.settings.milkdrop_fps;
        let seconds = self.settings.milkdrop_seconds;
        let scale = self.settings.milkdrop_scale.max(1);
        // The playing song, for the window to overlay when it changes.
        let song = self.now_playing().filter(|now| !now.resuming).map(|now| {
            let mut lines = vec![now.title.clone()];
            let mut second = now.subtitle.clone();
            if !now.album_name.is_empty() {
                second = if second.is_empty() {
                    now.album_name.clone()
                } else {
                    format!("{second} \u{2014} {}", now.album_name)
                };
            }
            if !second.is_empty() {
                lines.push(second);
            }
            lines
        });
        if self.milkdrop_host.is_none() {
            let tap = std::sync::Arc::clone(&self.winamp.tap);
            self.milkdrop_host = Some(crate::milkdrop::host::Host::new(tap));
        }
        let poll = {
            let host = self.milkdrop_host.as_mut().expect("the host was just made");
            if open {
                if !host.is_running() {
                    host.open(&presets, size, pos, fullscreen, fps, seconds, scale);
                }
                host.update(fps, seconds, scale);
                host.song(song);
            } else if host.is_running() {
                host.close();
            }
            host.poll()
        };
        if poll.closed {
            self.settings.milkdrop_open = false;
            self.mark_settings_dirty();
        }
        if let Some(size) = poll.size
            && self.settings.milkdrop_size != size
        {
            self.settings.milkdrop_size = size;
            self.mark_settings_dirty();
        }
        if let Some(pos) = poll.pos {
            self.milkdrop_pos = Some(pos);
        }
        // Look in on the child now and then, so its close or move is noticed
        // while the app is otherwise idle.
        if self.settings.milkdrop_open {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
    }

    /// Shows a folder of the config directory in the desktop's file
    /// manager, making it first if need be.
    fn open_folder(&mut self, folder: std::path::PathBuf) {
        let opened = std::fs::create_dir_all(&folder).and_then(|()| open::that(&folder));
        if let Err(error) = opened {
            self.toast_error(format!("Couldn't open {}: {error}", folder.display()));
        }
    }

    /// A skin has been read. A dropped file becomes the choice; a chosen
    /// name is already the choice, and if the choice moved on while this
    /// one was read, the next tick reads that one.
    fn skin_loaded(&mut self, loaded: crate::winamp::Loaded) {
        match loaded.result {
            Ok(skin) => {
                self.winamp
                    .wear(Some(loaded.name.clone()), std::sync::Arc::new(skin));
                if loaded.installed {
                    self.toast(format!(
                        "Added the {} skin",
                        crate::winamp::label(&loaded.name)
                    ));
                    self.winamp.list_choices(&self.dirs.skins_dir());
                    self.settings.skin = Some(loaded.name);
                    self.settings_dirty = true;
                }
            }
            Err(error) => {
                self.toast_error(format!("{}: {error}", crate::winamp::label(&loaded.name)));
                if !loaded.installed {
                    self.settings.skin = self.winamp.worn.clone();
                    self.settings_dirty = true;
                }
            }
        }
    }

    /// Puts a freshly read WMP skin on. The first one arrives while the
    /// big window is up — the skin window's size is only known from the
    /// skin — so the loop opens the small one.
    fn wmp_loaded(&mut self, ctx: &egui::Context, loaded: crate::wmp::Loaded) {
        match loaded.result {
            Ok(document) => {
                let was_none = self.wmp.skin.is_none();
                self.wmp.wear(
                    Some(loaded.name.clone()),
                    crate::wmp::WmpSkin {
                        document: std::sync::Arc::new(document),
                        render: crate::ui::wmp::Render::default(),
                    },
                );
                if was_none && self.settings.wmp_window {
                    // The window that opened before the skin was read was
                    // the plain one, since a skin window needs the skin's
                    // size to open. Stand the skin window now: the loop
                    // in `main` closes this and reopens the other kind.
                    self.switch_intent = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Err(error) => {
                self.toast_error(format!("{}: {error}", crate::wmp::label(&loaded.name)));
                self.settings.wmp_skin = self.wmp.worn.clone();
                // Nothing worn to fall back on: the skin window is over,
                // and the plain player is what the person has again. A
                // window showing now is the plain one already — the
                // skin window only stands where a skin was worn — so
                // the setting alone comes back, with no window dance.
                if self.settings.wmp_window && self.wmp.skin.is_none() {
                    self.settings.wmp_window = false;
                }
                self.settings_dirty = true;
            }
        }
    }

    /// Hands the equalizer settings to the player's thread and marks them
    /// for saving.
    fn push_eq(&mut self) {
        if let Ok(mut shared) = self.winamp.eq.lock() {
            *shared = eq_settings(&self.settings);
        }
        self.settings_dirty = true;
    }

    /// Note that a setting changed, so the file is saved shortly.
    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    /// Note that the restorable session changed, so it is saved shortly.
    pub fn note_session_change(&mut self) {
        self.session_dirty = true;
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
                TrayCommand::Show => self.actions.push(Action::ShowWindow),
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
                ControlCommand::SetShuffle(shuffle) => Some(Action::SetShuffle(shuffle)),
                ControlCommand::SetRepeat(mode) => Some(Action::SetRepeat(mode)),
                ControlCommand::SeekTo(position) => Some(Action::Seek(position)),
                // Nothing playing is nothing to save, so the verb is a
                // no-op rather than an error the client has to handle.
                ControlCommand::ToggleSaved => {
                    self.now_playing().map(|now| Action::ToggleSaved(now.uri))
                }
                ControlCommand::PlayUri(uri) => Some(Action::PlayContext {
                    uri,
                    offset_uri: None,
                    offset_index: None,
                }),
                ControlCommand::Transfer(device_id) => Some(Action::Transfer(device_id)),
                ControlCommand::RefreshDevices => Some(Action::RefreshDevices),
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
        if self.control_devices_stale
            && let Some(slot) = self.control_devices.clone()
        {
            let snapshot = self.control_devices_snapshot();
            *slot.lock().unwrap_or_else(|p| p.into_inner()) = snapshot;
            self.control_devices_stale = false;
        }
    }

    /// One line for the control channel's `nowplaying` verb: tab-separated
    /// `state, title, artists, album, position_ms, duration_ms, volume,
    /// shuffle, repeat, art_url, saved, device`, or
    /// [`crate::single_instance::NOTHING_PLAYING`].
    ///
    /// The last three are what a Stream Deck key needs and a media key does
    /// not: something to draw, whether the heart is filled, and where the
    /// sound is coming out. They are appended rather than woven in, so a
    /// script written against the older nine fields still reads correctly.
    fn control_snapshot(&self) -> String {
        let Some(now) = self.now_playing() else {
            return crate::single_instance::NOTHING_PLAYING.to_owned();
        };
        let state = if now.playing { "playing" } else { "paused" };
        // Not every track has been looked up yet; say so rather than
        // claiming an unsaved track the client would draw as a hollow heart
        // and then watch fill in a moment later.
        let saved = match self.is_saved(&now.uri) {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        };
        // Local playback is this computer, which Spotify has not named in
        // the snapshot because it is not a remote device.
        let device = match (&now.device_name, now.local) {
            (Some(name), _) => name.as_str(),
            (None, true) => self.settings.device_name.as_str(),
            (None, false) => "",
        };
        // Tabs separate the fields, so a tab inside one would shift the rest.
        // This runs every frame, and titles almost never contain one, so the
        // usual answer borrows rather than allocating a copy per field.
        fn clean(text: &str) -> std::borrow::Cow<'_, str> {
            match text.contains('\t') {
                true => std::borrow::Cow::Owned(text.replace('\t', " ")),
                false => std::borrow::Cow::Borrowed(text),
            }
        }
        format!(
            "{state}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{saved}\t{}",
            clean(&now.title),
            clean(&now.subtitle),
            clean(&now.album_name),
            now.position_ms,
            now.duration_ms,
            now.volume_percent,
            if now.shuffle { "on" } else { "off" },
            now.repeat.api_name(),
            clean(now.art_url.as_deref().unwrap_or_default()),
            clean(device),
        )
    }

    /// One line for the control channel's `devices` verb: the Spotify
    /// Connect devices the app last saw, as JSON.
    ///
    /// JSON rather than the tab-separated shape `nowplaying` uses because
    /// there are several of them and a device carries a name its owner
    /// chose, which is exactly the kind of free text a hand-rolled
    /// separator gets wrong.
    fn control_devices_snapshot(&self) -> String {
        let devices: Vec<_> = self
            .devices
            .iter()
            .filter_map(|device| {
                Some(serde_json::json!({
                    "id": device.id.as_deref()?,
                    "name": device.name,
                    "kind": device.kind,
                    "active": device.is_active,
                }))
            })
            .collect();
        serde_json::to_string(&devices)
            .unwrap_or_else(|_| crate::single_instance::NO_DEVICES.to_owned())
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
                let needs_generation = self
                    .playlist_pages
                    .get(&id)
                    .is_none_or(|page| page.generation == 0);
                if needs_generation {
                    self.load_generation += 1;
                    self.playlist_pages
                        .entry(id.clone())
                        .or_default()
                        .generation = self.load_generation;
                }
                let page = self.playlist_pages.entry(id.clone()).or_default();
                let generation = page.generation;
                if page.playlist.needs_load() {
                    page.playlist = Loadable::Loading;
                    self.backend.api(ApiRequest::Playlist {
                        id: id.clone(),
                        generation,
                    });
                }
                if !page.items.loaded_once && page.items.can_load_more() {
                    page.items.loading = true;
                    self.backend.api(ApiRequest::PlaylistItems {
                        id: id.clone(),
                        offset: 0,
                        generation,
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
        self.home.generation += 1;
        let generation = self.home.generation;
        if self.home.recently_played.get().is_none() {
            self.home.recently_played = Loadable::Loading;
        }
        if self.home.top_artists.get().is_none() {
            self.home.top_artists = Loadable::Loading;
        }
        if self.home.top_tracks.get().is_none() {
            self.home.top_tracks = Loadable::Loading;
        }
        self.backend.api(ApiRequest::RecentlyPlayed { generation });
        self.backend.api(ApiRequest::TopArtists { generation });
        self.backend.api(ApiRequest::TopTracks {
            offset: 0,
            full: false,
            generation,
        });
        self.home.discover_pending.clear();
        for term in DISCOVER_TERMS {
            self.home
                .discover_pending
                .insert((*term).to_string(), Loadable::Loading);
            if !self.home.discover.contains_key(*term) {
                self.home
                    .discover
                    .insert((*term).to_string(), Loadable::Loading);
            }
            self.backend.api(ApiRequest::Discover {
                term: (*term).to_string(),
                generation,
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
        self.home.top_songs_generation += 1;
        self.backend.api(ApiRequest::TopTracks {
            offset: 0,
            full: true,
            generation: self.home.top_songs_generation,
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
                        self.backend.api(ApiRequest::PlaylistItems {
                            id,
                            offset,
                            generation: page.generation,
                        });
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
                if let Some(playlist) = self.playlist_pages.get_mut(id) {
                    self.load_generation += 1;
                    playlist.generation = self.load_generation;
                    playlist.items.loading = true;
                    playlist.cache_complete = false;
                    playlist.pending_cache = None;
                    self.backend.api(ApiRequest::Playlist {
                        id: id.clone(),
                        generation: playlist.generation,
                    });
                    self.backend.api(ApiRequest::PlaylistItems {
                        id: id.clone(),
                        offset: 0,
                        generation: playlist.generation,
                    });
                    return;
                }
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
        self.remote_poll_seq += 1;
        self.backend.api(ApiRequest::PlaybackState {
            seq: self.remote_poll_seq,
        });
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
        if self.resume_only() && matches!(self.queue, Loadable::Loaded(_)) {
            // Nothing is playing anywhere and the queue on show is the
            // remembered one; a fetch could only replace it with less.
            return;
        }
        if self.queue.is_loading() && !force {
            return;
        }
        if !matches!(self.queue, Loadable::Loaded(_)) {
            self.queue = Loadable::Loading;
        }
        self.queue_fetched_at = Some(Instant::now());
        self.queue_seq += 1;
        self.backend.api(ApiRequest::Queue {
            seq: self.queue_seq,
        });
    }

    /// A chosen row of Next up plays at once, and the rows above it go
    /// with it: skips consume the queue, so the playing context and the
    /// songs queued after the chosen one stay intact. Loading the queue's
    /// rows as a fresh list instead used to take seconds, threw the
    /// context away, and left Spotify's copy of the queue to reappear.
    fn play_queue_item(&mut self, index: usize, uri: String) {
        if self.resume_only() {
            // Nothing is playing anywhere, so there is no live queue to
            // consume: play the shown rows as a plain list.
            let uris: Vec<String> = self
                .queue
                .get()
                .map(|queue| {
                    queue
                        .queue
                        .iter()
                        .map(|item| item.uri().to_string())
                        .collect()
                })
                .unwrap_or_default();
            if uris.is_empty() {
                return;
            }
            let (uris, index) = cap_uris(uris, index as u32);
            self.play_request(PlayRequest::tracks(uris).starting_at_index(index), false);
            return;
        }
        let mut skips = index + 1;
        let mut consumed: Vec<String> = Vec::new();
        if let Loadable::Loaded(queue) = &mut self.queue {
            // The click names a song; if the rows shifted under the
            // pointer, the song wins over the row number.
            let position = match queue.queue.get(index) {
                Some(item) if item.uri() == uri => Some(index),
                _ => queue.queue.iter().position(|item| item.uri() == uri),
            };
            let Some(position) = position else {
                self.refresh_queue(true);
                return;
            };
            skips = position + 1;
            let mut items: Vec<_> = queue.queue.drain(..=position).collect();
            let chosen = items.pop().expect("the chosen row was just drained");
            consumed = items.iter().map(|item| item.uri().to_string()).collect();
            consumed.push(chosen.uri().to_string());
            queue.currently_playing = Some(chosen);
        }
        for gone in &consumed {
            if let Some(at) = self.manual_queue.iter().position(|queued| queued == gone) {
                self.manual_queue.remove(at);
                self.session_dirty = true;
            }
            self.pending_queue_adds
                .retain(|(pending, _)| pending != gone);
        }
        self.intent_track = Some((uri.clone(), Instant::now()));
        self.set_play_pending(vec![uri]);
        self.optimistic_playing = Some((true, Instant::now()));
        match self.target() {
            Target::Local => {
                for _ in 0..skips {
                    self.backend.player(PlayerCommand::Next);
                }
            }
            Target::Remote(device_id) => {
                // With nothing to act on, one call earns the "pick
                // something first" toast; a skip per row would repeat it.
                if device_id.is_none() && self.remote_fresh().is_none() {
                    self.remote(RemoteAction::Next, None);
                    return;
                }
                for _ in 0..skips {
                    self.remote(RemoteAction::Next, device_id.clone());
                }
            }
        }
    }

    /// How many leading rows of Next up are songs the user queued here,
    /// so the view can give them their own section.
    pub fn queued_rows_len(&self) -> usize {
        // Before anything resumes, the remembered hand-queued songs say
        // where the user's section of the restored queue ends.
        let manual = if self.manual_queue.is_empty() && self.resume_only() {
            &self.resume_queue
        } else {
            &self.manual_queue
        };
        match &self.queue {
            Loadable::Loaded(queue) => Self::end_of_queued_rows(&queue.queue, manual),
            _ => 0,
        }
    }

    /// Whether Clear queue can truly clear: the queue has rows and this
    /// computer's engine is the playing device, the only device whose
    /// queue any client is allowed to drop.
    pub fn can_clear_queue(&self) -> bool {
        self.local.is_active()
            && matches!(self.target(), Target::Local)
            && self.queued_rows_len() > 0
    }

    /// The hand-queued rows leave Next up at once, and the engine drops
    /// its queued tracks behind them. The context's own upcoming songs
    /// stay: that is what Spotify's own Clear queue keeps too.
    fn clear_queue(&mut self) {
        if !matches!(self.target(), Target::Local) {
            return;
        }
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for uri in self
            .manual_queue
            .iter()
            .chain(self.pending_queue_adds.iter().map(|(uri, _)| uri))
        {
            *counts.entry(uri.clone()).or_insert(0) += 1;
        }
        if let Loadable::Loaded(queue) = &mut self.queue {
            // Each record removes one row, front first: a song queued once
            // that the context also carries keeps its context row.
            queue.queue.retain(|item| match counts.get_mut(item.uri()) {
                Some(left) if *left > 0 => {
                    *left -= 1;
                    false
                }
                _ => true,
            });
        }
        let cleared: std::collections::HashSet<String> = counts.into_keys().collect();
        if !self.manual_queue.is_empty() {
            self.session_dirty = true;
        }
        self.manual_queue.clear();
        self.pending_queue_adds.clear();
        if !cleared.is_empty() {
            self.queue_cleared = Some((cleared, Instant::now()));
        }
        self.backend.player(PlayerCommand::ClearQueue);
        // The engine also drops queued tracks this app never saw added;
        // the fetch behind this recheck sweeps their rows away.
        self.queue_recheck_at = Some(Instant::now() + QUEUE_RECHECK);
        self.toast("Queue cleared");
    }

    /// The playing song and every row after it, each song once, in the
    /// order they play: what saving the queue writes down.
    pub fn queue_playlist_uris(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut uris = Vec::new();
        let queued = self.queue.get();
        let rows = self.now_playing().map(|now| now.uri).into_iter().chain(
            queued
                .iter()
                .flat_map(|queue| queue.queue.iter().map(|item| item.uri().to_string())),
        );
        for uri in rows {
            if seen.insert(uri.clone()) {
                uris.push(uri);
            }
        }
        uris
    }

    /// What a saved queue is called: the station's own name when a song
    /// radio plays, otherwise the queue and the day.
    pub fn queue_playlist_name(&self) -> String {
        if let Some(context) = self.playing_context_uri()
            && let Some(id) = context.strip_prefix("spotify:station:track:")
            && let Some(track) = self.track_cache.get(id)
        {
            return format!("{} Radio", track.name);
        }
        let today = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
        format!("Queue {today}")
    }

    /// The queue, made permanent as a new playlist of the user's.
    fn save_queue_as_playlist(&mut self) {
        let uris = self.queue_playlist_uris();
        if uris.is_empty() {
            return;
        }
        let name = self.queue_playlist_name();
        self.actions.push(Action::CreatePlaylist {
            name,
            public: false,
            add_uris: uris,
        });
    }

    /// The head of Next up becomes the playing row at once; the claim is
    /// held the way a clicked row's is, until a report confirms it.
    fn pop_queue_head(&mut self) {
        let Loadable::Loaded(queue) = &mut self.queue else {
            return;
        };
        if queue.queue.is_empty() {
            return;
        }
        let item = queue.queue.remove(0);
        self.intent_track = Some((item.uri().to_string(), Instant::now()));
        queue.currently_playing = Some(item);
    }

    /// Whether a fetched queue predates the user's latest change here.
    /// Spotify's queue endpoint can lag a skip or an add by seconds, and a
    /// lagging answer must not undo what the interface already shows.
    fn queue_fetch_is_stale(&self, fetched: &Queue) -> bool {
        if self.queue_stale_retries >= QUEUE_STALE_RETRIES {
            return false;
        }
        // A row was just chosen or popped: the fetch has to name it as
        // playing before it is believed.
        if let Some((uri, at)) = &self.intent_track
            && at.elapsed() < PLAYBACK_HOLD
            && fetched
                .currently_playing
                .as_ref()
                .is_none_or(|item| item.uri() != uri)
        {
            return true;
        }
        // The local engine is the truth for this computer: an answer that
        // names another song as playing is an old one. Songs advance on
        // their own, so this holds with or without a recent click.
        if self.local.is_active()
            && let Some(track) = &self.local.track
            && fetched
                .currently_playing
                .as_ref()
                .is_some_and(|item| item.uri() != track.uri)
        {
            return true;
        }
        // A cleared row still on top means the clear has not landed yet.
        if let Some((cleared, at)) = &self.queue_cleared
            && at.elapsed() < PLAYBACK_HOLD
            && fetched
                .queue
                .first()
                .is_some_and(|item| cleared.contains(item.uri()))
        {
            return true;
        }
        false
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
        if self.search.results.get().is_none() {
            self.search.results = Loadable::Loading;
        }
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
                });
            }
        }
        if !batch.is_empty() {
            self.backend.api(ApiRequest::Contains { uris: batch });
        }
    }

    // ---- api responses -------------------------------------------------------

    fn handle_api(&mut self, response: ApiResponse) {
        match response {
            ApiResponse::Me(result) => match result {
                Ok(user) => {
                    // Spotify only takes playback commands from Premium
                    // accounts, here or on any device, so a Free account
                    // is told once rather than left pressing play.
                    let free = user
                        .product
                        .as_deref()
                        .is_some_and(|product| product != "premium");
                    if free && !self.premium_notice_shown {
                        self.premium_notice_shown = true;
                        self.dialog = Some(Dialog::PremiumNeeded);
                    }
                    self.user = Some(user);
                    let page = self.page().clone();
                    self.ensure_loaded(page);
                    if let Some(now) = self.now_playing() {
                        self.request_contains(vec![now.uri]);
                    }
                }
                Err(error) => {
                    if matches!(error, crate::api::ApiError::SignInExpired { .. }) {
                        self.auth = AuthStatus::Failed(
                            "Your Spotify sign-in expired. Please sign in again.".into(),
                        );
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
                        self.control_devices_stale = true;
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
                        let previous_shuffle = self
                            .remote
                            .as_ref()
                            .map(|remote| remote.state.shuffle_state);
                        self.remote = state.map(|state| RemoteSnapshot {
                            state,
                            received_at: Instant::now(),
                        });
                        if let (Some(previous), Some(current)) = (
                            previous_shuffle,
                            self.remote
                                .as_ref()
                                .map(|remote| remote.state.shuffle_state),
                        ) && previous != current
                            && self
                                .shuffle_set_at
                                .is_none_or(|at| at.elapsed() > Duration::from_secs(5))
                        {
                            // Toggled on another device; that is the
                            // listener's setting too.
                            self.shuffle_wanted = current;
                        }
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
            ApiResponse::Queue { seq, result } => {
                if seq != self.queue_seq {
                    // Overtaken: a newer request is out. Answers must not
                    // land out of order, or an old snapshot could erase a
                    // row the newer answer had already confirmed.
                    return;
                }
                if let Ok(fetched) = &result
                    && self.queue_fetch_is_stale(fetched)
                {
                    // A snapshot from before the user's last change here:
                    // showing it would undo what they just did. Keep the
                    // optimistic queue and ask again shortly; if Spotify
                    // keeps telling the old story, it eventually wins.
                    self.queue_stale_retries += 1;
                    self.queue_recheck_at = Some(Instant::now() + QUEUE_RECHECK);
                    return;
                }
                self.queue_stale_retries = 0;
                if result.is_ok() {
                    self.queue_cleared = None;
                }
                self.queue = Loadable::from_result(result);
                self.reconcile_pending_queue();
                if let Some(queue) = self.queue.get() {
                    let uris: Vec<String> = queue
                        .queue
                        .iter()
                        .map(|item| item.uri().to_string())
                        .collect();
                    self.request_contains(uris);
                }
            }
            ApiResponse::RecentlyPlayed { generation, result } => {
                if generation != self.home.generation {
                    return;
                }
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
                self.home.recently_played.refresh(result);
            }
            ApiResponse::TopTracks {
                offset,
                full,
                generation,
                result,
            } => {
                if full {
                    if generation != self.home.top_songs_generation {
                        return;
                    }
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
                                    generation,
                                });
                            } else {
                                self.home.top_songs_loading = false;
                                self.home.top_songs_complete = true;
                            }
                        }
                        Err(error) => {
                            self.home.top_songs.refresh(Err::<Vec<Track>, _>(error));
                            self.home.top_songs_loading = false;
                        }
                    }
                } else if generation == self.home.generation
                    && let Ok(page) = result
                {
                    let tracks = page.items;
                    let seeds: Vec<String> = tracks
                        .iter()
                        .filter_map(|track| track.id.clone())
                        .take(5)
                        .collect();
                    if !seeds.is_empty() {
                        if self.home.recommendations.get().is_none() {
                            self.home.recommendations = Loadable::Loading;
                        }
                        self.backend.api(ApiRequest::Recommendations {
                            seed_tracks: seeds,
                            seed_artists: Vec::new(),
                            generation,
                        });
                    }
                    let uris: Vec<String> = tracks.iter().map(|track| track.uri.clone()).collect();
                    self.request_contains(uris);
                    self.home.top_tracks = Loadable::Loaded(tracks);
                } else if generation == self.home.generation
                    && offset == 0
                    && let Err(error) = result
                    && self.home.top_tracks.get().is_none()
                {
                    self.home.top_tracks = Loadable::Failed(error.to_string());
                }
            }
            ApiResponse::TopArtists { generation, result } => {
                if generation != self.home.generation {
                    return;
                }
                self.home.top_artists.refresh(result);
            }
            ApiResponse::Recommendations { generation, result } => {
                if generation != self.home.generation {
                    return;
                }
                if let Ok(tracks) = &result {
                    let uris: Vec<String> = tracks.iter().map(|track| track.uri.clone()).collect();
                    self.request_contains(uris);
                }
                self.home.recommendations.refresh(result);
            }
            ApiResponse::Discover {
                term,
                generation,
                result,
            } => {
                if generation != self.home.generation {
                    return;
                }
                let filtered = result.map(|playlists| {
                    let mut seen = std::collections::HashSet::new();
                    let mut matching: Vec<Playlist> = playlists
                        .into_iter()
                        .filter(|playlist| {
                            let owner = playlist.owner.id.as_deref().unwrap_or("");
                            is_made_for_you(&playlist.name, &term)
                                && (owner == "spotify" || playlist.owner_name() == "Spotify")
                                && seen.insert(playlist.name.to_lowercase())
                        })
                        .collect();
                    matching.truncate(6);
                    matching
                });
                self.home
                    .discover_pending
                    .insert(term, Loadable::from_result(filtered));
                let complete = DISCOVER_TERMS.iter().all(|term| {
                    self.home
                        .discover_pending
                        .get(*term)
                        .is_some_and(|result| !result.is_loading())
                });
                if complete {
                    self.home.discover = std::mem::take(&mut self.home.discover_pending);
                }
            }
            ApiResponse::MyPlaylists { offset, result } => match result {
                Ok(page) => {
                    let next_offset = page.next_offset();
                    match &mut self.library.playlists {
                        Loadable::Loaded(existing) if offset > 0 => existing.extend(page.items),
                        slot => *slot = Loadable::Loaded(page.items),
                    }
                    self.library.playlists_next = next_offset;
                    if next_offset.is_some() {
                        self.load_more(Page::Home);
                    } else {
                        // The list is whole; ask the session how the
                        // listener arranged it into folders.
                        self.backend.send(Command::Rootlist);
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
            ApiResponse::Playlist {
                id,
                generation,
                result,
            } => {
                if self
                    .playlist_pages
                    .get(&id)
                    .is_none_or(|page| page.generation != generation)
                {
                    return;
                }
                if let Ok(playlist) = &result
                    && let Some(image) = pick_image(&playlist.images, 300)
                {
                    self.tint_for(Some(image));
                }
                if let Some(page) = self.playlist_pages.get_mut(&id) {
                    page.playlist.refresh(result);
                }
                self.try_adopt_playlist_cache(&id);
            }
            ApiResponse::PlaylistItems {
                id,
                offset,
                generation,
                result,
            } => {
                if self
                    .playlist_pages
                    .get(&id)
                    .is_none_or(|page| page.generation != generation)
                {
                    return;
                }
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
                                .filter(|id| !id.is_empty())
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
                                        offset: total.saturating_sub(PLAYLIST_PAGE_SIZE),
                                        generation,
                                    });
                                }
                            }
                        }
                        Err(error) => page.items.fail(friendly_page_error(&error)),
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
            ApiResponse::PlaylistSample {
                id,
                generation,
                result,
            } => {
                if self
                    .playlist_pages
                    .get(&id)
                    .is_none_or(|page| page.generation != generation)
                {
                    return;
                }
                let mut adders: Vec<String> = Vec::new();
                if let Ok(items) = result
                    && let Some(page) = self.playlist_pages.get_mut(&id)
                {
                    adders = items
                        .items
                        .iter()
                        .filter_map(|item| item.added_by.as_ref()?.id.clone())
                        .filter(|id| !id.is_empty())
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
                                        let total = self
                                            .library
                                            .liked
                                            .total
                                            .map(|total| total.saturating_add(1));
                                        self.library.liked.reset();
                                        self.library.liked.total = total;
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
                self.search.results.refresh(result);
            }
            ApiResponse::Artist { id, result } => {
                if let Ok(artist) = &result {
                    if let Some(image) = pick_image(&artist.images, 300) {
                        self.tint_for(Some(image));
                    }
                    if let Some(page) = self.artist_pages.get_mut(&id)
                        && page.top_tracks.needs_load()
                    {
                        page.top_tracks = Loadable::Loading;
                        self.backend
                            .api(ApiRequest::ArtistTopTracks { id: id.clone() });
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
            ApiResponse::QueueAdded { label: _, result } => match result {
                Ok(()) => {
                    // The toast came at the click; here the view catches up.
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

    /// Remembers `uri` as the most recently played context, for the
    /// sidebar's order.
    fn note_recent_context(&mut self, uri: &str) {
        self.session_dirty = true;
        if !uri.contains(":playlist:") && !uri.contains(":album:") && !uri.contains(":collection") {
            return;
        }
        self.recent_contexts.retain(|held| held != uri);
        self.recent_contexts.insert(0, uri.to_string());
        self.recent_contexts.truncate(60);
    }

    /// A random playable track of a context the app has rows for: the
    /// start of a shuffle play. `None` when no rows are at hand.
    /// The songs a context holds, in the order they are shown, from the
    /// rows already loaded. `None` when those rows are not here yet.
    fn context_track_uris(&self, context_uri: &str) -> Option<Vec<String>> {
        let uris: Vec<String> = if let Some(id) = context_uri.strip_prefix("spotify:playlist:") {
            self.playlist_pages
                .get(id)?
                .items
                .items
                .iter()
                .filter_map(|item| item.playable())
                .map(|item| item.uri().to_string())
                .collect()
        } else if let Some(id) = context_uri.strip_prefix("spotify:album:") {
            self.album_pages
                .get(id)?
                .tracks
                .items
                .iter()
                .map(|track| track.uri.clone())
                .collect()
        } else if context_uri.ends_with(":collection") {
            self.library
                .liked
                .items
                .iter()
                .map(|item| item.track.uri.clone())
                .collect()
        } else {
            return None;
        };
        (!uris.is_empty()).then_some(uris)
    }

    fn random_track_in(&self, context_uri: &str) -> Option<String> {
        let uris = self.context_track_uris(context_uri)?;
        Some(uris[rand::random_range(0..uris.len())].clone())
    }

    /// How many songs Spotify last said a context holds, from the library
    /// alone: enough to start a shuffle play at a random position when the
    /// rows are not loaded, which is every play begun from the sidebar, a
    /// card, or a context menu. `None` for anything unsaved, whose length
    /// nobody here knows.
    fn context_len(&self, context_uri: &str) -> Option<u32> {
        if context_uri.ends_with(":collection") {
            return self.library.liked.total;
        }
        match util::uri_kind(context_uri)? {
            "playlist" => self
                .library
                .playlists
                .get()?
                .iter()
                .find(|playlist| playlist.uri == context_uri)?
                .tracks
                .as_ref()
                .map(|tracks| tracks.total),
            "album" => {
                self.library
                    .albums
                    .items
                    .iter()
                    .find(|saved| saved.album.uri == context_uri)?
                    .album
                    .total_tracks
            }
            "show" => {
                self.library
                    .shows
                    .items
                    .iter()
                    .find(|saved| saved.show.uri == context_uri)?
                    .show
                    .total_episodes
            }
            _ => None,
        }
    }

    /// Where a shuffled play of `context_uri` begins, as an offset by
    /// track URI or by position: a random one of the rows the app holds,
    /// or, when it holds none, a random position within the length the
    /// library knows. Neither, and the play carries no offset at all:
    /// librespot then picks its own random track, and only the Web API,
    /// which would start at track one, needs telling where to go.
    fn shuffle_start(&self, context_uri: &str) -> (Option<String>, Option<u32>) {
        if let Some(uri) = self.random_track_in(context_uri) {
            return (Some(uri), None);
        }
        if matches!(self.target(), Target::Remote(Some(_)))
            && let Some(len) = self.context_len(context_uri)
            && len > 0
        {
            return (None, Some(rand::random_range(0..len)));
        }
        (None, None)
    }

    /// With `shuffle_first`, shuffle is turned on before playback starts,
    /// in one ordered exchange: two independent requests race, and shuffle
    /// sometimes lost.
    fn play_request(&mut self, request: PlayRequest, shuffle_first: bool) {
        // Shuffle is a mode the listener sets, not a property of one
        // context: once on, every play shuffles until it is turned off,
        // whichever playlist it starts. A chosen row still starts the
        // play; without one, a random song does.
        let mut request = request;
        if shuffle_first {
            self.shuffle_wanted = true;
            self.shuffle_set_at = Some(Instant::now());
        }
        let shuffle = shuffle_first || self.shuffle_wanted;
        if shuffle
            && request.offset_uri.is_none()
            && request.offset_position.is_none()
            && request.uris.is_empty()
            && let Some(context) = request.context_uri.clone()
        {
            (request.offset_uri, request.offset_position) = self.shuffle_start(&context);
        }
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
        self.intent_track = keys
            .iter()
            .find(|key| key.contains(":track:"))
            .cloned()
            .map(|uri| (uri, Instant::now()));
        self.set_play_pending(keys);
        if let Some(context) = request.context_uri.clone() {
            self.note_recent_context(&context);
            // Light the page and the sidebar up at once; Spotify's own
            // state takes a poll or two to say the same thing.
            self.assumed_context = Some(AssumedContext {
                uri: context,
                shuffle: shuffle.then_some(true),
                at: Instant::now(),
            });
        }
        match self.target() {
            Target::Local if !self.local.connected => {
                // The engine's session dropped (a sleep, a network change)
                // and is reconnecting on its own; hold the request and play
                // the moment it is back. The spinner keeps spinning.
                self.queued_play = Some(request);
            }
            Target::Local => {
                self.queued_play = None;
                let load = local_load(&request, shuffle);
                self.local_list = load.context_uri.is_none().then(|| load.uris.clone());
                let shuffle_after = shuffle && load.shuffle.is_none() && !load.uris.is_empty();
                self.backend.player(PlayerCommand::Load(load));
                if shuffle_after {
                    self.backend.player(PlayerCommand::Shuffle(true));
                }
                self.optimistic_playing = Some((true, Instant::now()));
            }
            Target::Remote(Some(device_id)) => {
                self.queued_play = None;
                if shuffle {
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
                .filter(|id| !id.is_empty())
                .collect();
            page.contributors.extend(adders.iter().cloned());
            page.items.set_cached(items);
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
                    // Nothing is known to be playing anywhere, which is how
                    // a fresh start looks before the local engine is ready:
                    // pick up where the last run left off, the way the
                    // local branch does. The engine plays it once it is up.
                    if !self.resume_last() {
                        self.toast("Pick a song, album, or playlist");
                    }
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
        // Dragging the bar under the remembered song moves the point a press
        // of play will resume from; there is no stream to seek yet.
        if self.now_playing_live().is_none() && self.resume_track.is_some() {
            self.resume_position_ms = position_ms;
            self.session_dirty = true;
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
        self.shuffle_wanted = shuffle;
        self.shuffle_set_at = Some(Instant::now());
        self.session_dirty = true;
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
                    autoplay: false,
                }));
            }
            self.poll_remote_soon();
            return;
        }
        let play = self.now_playing().is_some_and(|now| now.playing);
        self.selected_device = Some(device_id.clone());
        self.backend.api(ApiRequest::Transfer { device_id, play });
    }

    /// Play next: the row appears at once, after the songs already
    /// queued and before the playing context's own, and the backend makes
    /// it true behind it.
    fn add_to_queue(&mut self, uri: String, label: String) {
        // A double-click is one wish; a deliberate second ask is a second
        // row, the way Spotify queues the same song twice.
        self.expire_pending_queue_adds();
        if self
            .pending_queue_adds
            .iter()
            .any(|(pending, at)| *pending == uri && at.elapsed() < QUEUE_ADD_DEBOUNCE)
        {
            return;
        }
        self.pending_queue_adds.push((uri.clone(), Instant::now()));
        let item = self.optimistic_queue_item(&uri, &label);
        if let Loadable::Loaded(queue) = &self.queue {
            let at = Self::end_of_queued_rows(&queue.queue, &self.manual_queue);
            if let Loadable::Loaded(queue) = &mut self.queue {
                queue.queue.insert(at, item);
            }
        }
        self.manual_queue.push(uri.clone());
        if self.manual_queue.len() > 100 {
            self.manual_queue.remove(0);
        }
        self.session_dirty = true;
        self.toast(format!("{label} will play next"));
        // This computer's playing engine queues directly: no round trip
        // through the Web API, no device for it to fail to find. Anything
        // else, and any album, goes the long way.
        let track_like = uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:");
        if track_like && self.local.is_active() && matches!(self.target(), Target::Local) {
            self.backend.player(PlayerCommand::AddToQueue(uri));
            self.queue_recheck_at = Some(Instant::now() + QUEUE_RECHECK);
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

    /// Where a newly queued row goes: after the leading rows that are
    /// hand-queued songs, before the playing context's own.
    fn end_of_queued_rows(rows: &[PlayableItem], manual: &[String]) -> usize {
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for uri in manual {
            *counts.entry(uri.as_str()).or_insert(0) += 1;
        }
        let mut at = 0;
        for item in rows {
            match counts.get_mut(item.uri()) {
                Some(left) if *left > 0 => {
                    *left -= 1;
                    at += 1;
                }
                _ => break,
            }
        }
        at
    }

    /// The queued row as it can be shown right now: the cached track, or
    /// its name alone until the details arrive.
    fn optimistic_queue_item(&self, uri: &str, label: &str) -> PlayableItem {
        let cached = util::uri_id(uri)
            .and_then(|id| self.track_cache.get(id))
            .cloned();
        PlayableItem::Track(cached.unwrap_or_else(|| crate::api::models::Track {
            uri: uri.to_string(),
            name: label.to_string(),
            ..Default::default()
        }))
    }

    fn expire_pending_queue_adds(&mut self) {
        self.pending_queue_adds
            .retain(|(_, at)| at.elapsed() < Duration::from_secs(30));
    }

    /// A fetched queue that has not caught up yet gets the pending adds
    /// put back on top, so a slow answer cannot erase them; ones it now
    /// carries stop being pending.
    fn reconcile_pending_queue(&mut self) {
        self.expire_pending_queue_adds();
        if self.pending_queue_adds.is_empty() {
            return;
        }
        let Loadable::Loaded(queue) = &mut self.queue else {
            return;
        };
        let fetched: std::collections::HashSet<String> = queue
            .queue
            .iter()
            .map(|item| item.uri().to_string())
            .collect();
        // An add the fetch carries has landed; one that is playing has been
        // consumed. Without the second check, skipping into a just-queued
        // song would put its row back on top for as long as the add stayed
        // pending.
        let current = self.current_track_uri();
        self.pending_queue_adds
            .retain(|(uri, _)| !fetched.contains(uri) && current.as_deref() != Some(uri.as_str()));
        let missing: Vec<PlayableItem> = self
            .pending_queue_adds
            .iter()
            .map(|(uri, _)| (uri.clone(), String::new()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(uri, label)| self.optimistic_queue_item(&uri, &label))
            .collect();
        let at = match &self.queue {
            Loadable::Loaded(queue) => Self::end_of_queued_rows(&queue.queue, &self.manual_queue),
            _ => 0,
        };
        if let Loadable::Loaded(queue) = &mut self.queue {
            for item in missing.into_iter().rev() {
                queue.queue.insert(at, item);
            }
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

    /// Arms the watch when a window is minimized by a command, and says
    /// the window is back when it has a place again. egui-winit learns
    /// "minimized" on macOS only from our command, so a window restored
    /// by hand still reads as minimized and the interface freezes until
    /// it is corrected. winit answers the command with the truth of the
    /// window, and one already shown is left alone.
    pub fn watch_window_restore(&mut self, ctx: &egui::Context) {
        let positioned = ctx.input(|input| input.viewport().inner_rect.is_some());
        match self.restore_watch {
            RestoreWatch::Sent if !positioned => self.restore_watch = RestoreWatch::Gone,
            RestoreWatch::Gone if positioned => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.request_repaint();
                self.restore_watch = RestoreWatch::None;
            }
            _ => {}
        }
    }

    /// The window was minimized by a command; watch for it coming back.
    pub fn arm_restore_watch(&mut self) {
        self.restore_watch = RestoreWatch::Sent;
    }

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
            Action::PlayTrackRadio(uri) => {
                // The song's station, loaded as an ordinary context: the
                // station uri resolves to a 50-song context (see
                // examples/station_probe.rs), while asking the engine for
                // autoplay of a bare track dies inside the load and left
                // silence and an emptied queue. The Web API cannot start
                // a station on another device, so it plays here.
                let id = util::uri_id(&uri).unwrap_or_default();
                let station = format!("spotify:station:track:{id}");
                self.local_list = None;
                self.backend.player(PlayerCommand::Load(LoadSpec {
                    context_uri: Some(station.clone()),
                    play: true,
                    autoplay: false,
                    ..LoadSpec::default()
                }));
                self.optimistic_playing = Some((true, Instant::now()));
                // The station is the queue, so show it: fifty songs
                // appearing there is the whole visible result.
                self.assumed_context = Some(AssumedContext {
                    uri: station,
                    shuffle: None,
                    at: Instant::now(),
                });
                if !matches!(self.page(), Page::Queue) && !self.show_queue_panel {
                    self.show_queue_panel = true;
                    self.show_lyrics_panel = false;
                }
                self.refresh_queue(true);
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
                RowContext::Queue => self.play_queue_item(index as usize, uri),
                RowContext::View { uris, context_uri } => {
                    let (uris, index) = cap_uris(uris, index);
                    let request = PlayRequest::tracks(uris).starting_at_index(index);
                    self.play_request(request, false);
                    self.note_recent_context(&context_uri);
                    self.assumed_context = Some(AssumedContext {
                        uri: context_uri,
                        shuffle: None,
                        at: Instant::now(),
                    });
                }
            },
            Action::ShufflePlay(uri) => {
                // The random starting song is picked in `play_request`,
                // which every shuffled play goes through.
                self.play_request(PlayRequest::context(uri), true);
            }
            Action::TogglePlay => self.toggle_play(),
            Action::Next if self.resume_only() => {
                self.step_resume(true);
            }
            Action::Next => {
                // Next is a pop: the head of Next up becomes the playing
                // row the moment the button is pressed, and Spotify's
                // answers catch up behind it. No pop when the press can
                // only earn the "pick something first" toast.
                let target = self.target();
                if !matches!(target, Target::Remote(None)) || self.remote_fresh().is_some() {
                    self.pop_queue_head();
                }
                match target {
                    Target::Local => self.backend.player(PlayerCommand::Next),
                    Target::Remote(device_id) => self.remote(RemoteAction::Next, device_id),
                }
            }
            // Previous restarts the song when it is far enough in and steps
            // back otherwise, which is what librespot's own prev does; the
            // remembered song answers it the same way, from a standstill.
            Action::Previous if self.resume_only() => {
                if self.resume_position_ms > RESTART_BEFORE_PREVIOUS {
                    self.resume_position_ms = 0;
                    self.session_dirty = true;
                } else {
                    self.step_resume(false);
                }
            }
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
                    page.items.retain(|item| {
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
                    page.items.reorder(from as usize, to as usize);
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
                self.playlist_busy = true;
                self.dialog = Some(Dialog::CreatePlaylist {
                    name: name.clone(),
                    public,
                    add_uris,
                });
                self.backend.api(ApiRequest::CreatePlaylist {
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
            Action::ClearQueue => self.clear_queue(),
            Action::SaveQueueAsPlaylist => self.save_queue_as_playlist(),
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
            Action::ConfigurePersonalWebApp => {
                self.save_settings();
                self.backend.send(Command::ConfigurePersonalWebApp(
                    self.settings.web_client_id.clone(),
                ));
            }
            Action::SignOut => {
                self.backend.send(Command::SignOut);
                self.history = vec![Page::Home];
                self.history_index = 0;
            }
            Action::ToggleSidebar => {
                self.settings.sidebar_visible = !self.settings.sidebar_visible;
                self.settings_dirty = true;
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
                if self.show_devices {
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
                self.save_settings();
                let config = engine_config(
                    &self.dirs,
                    &self.settings,
                    std::sync::Arc::clone(&self.winamp.tap),
                    std::sync::Arc::clone(&self.winamp.eq),
                );
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
                let free = self
                    .user
                    .as_ref()
                    .and_then(|user| user.product.as_deref())
                    .is_some_and(|product| product != "premium");
                if free {
                    self.toast_error("Local playback needs Spotify Premium");
                } else if !self.local_ready
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
            Action::OpenUrl(url) => {
                // The door the sign-in uses, off the interface thread. egui's
                // own link opening runs through a chain of features and a
                // guessing launcher, and a click that does nothing is worse
                // than no link.
                std::thread::spawn(move || {
                    if let Err(error) = open::that(&url) {
                        log::warn!("unable to open {url}: {error}");
                    }
                });
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
            Action::ToggleWinampWindow => {
                // One window at a time: this one closes and the loop in
                // `main` opens the other kind where each was last.
                if self.settings.winamp_window {
                    self.winamp.remember_position();
                } else {
                    self.session_window_size = self.last_window_size.or(self.session_window_size);
                    self.session_window_pos = self.last_window_pos.or(self.session_window_pos);
                }
                self.settings.winamp_window = !self.settings.winamp_window;
                self.settings_dirty = true;
                self.switch_intent = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Action::SetSkin(name) => {
                self.settings.skin = name;
                self.settings_dirty = true;
            }
            Action::InstallSkin(path) => {
                self.winamp.install(path, &self.dirs.skins_dir(), ctx);
            }
            Action::SetSkinScale(scale) => {
                self.settings.skin_scale = Some(scale);
                self.settings_dirty = true;
            }
            Action::ToggleWinampOnTop => {
                self.settings.winamp_on_top = !self.settings.winamp_on_top;
                self.settings_dirty = true;
            }
            Action::ToggleWmpWindow => {
                // One window at a time, like the Winamp one: this one
                // closes and the loop in `main` opens the other kind.
                // The skin window comes back where it was.
                if self.settings.wmp_window {
                    self.wmp.remember_position();
                } else {
                    self.session_window_size = self.last_window_size.or(self.session_window_size);
                    self.session_window_pos = self.last_window_pos.or(self.session_window_pos);
                }
                self.settings.wmp_window = !self.settings.wmp_window;
                self.settings_dirty = true;
                self.switch_intent = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Action::SetWmpSkin(name) => {
                self.settings.wmp_skin = name;
                self.settings_dirty = true;
            }
            Action::SetWmpScale(scale) => {
                self.settings.wmp_scale = Some(scale);
                self.settings_dirty = true;
            }
            Action::ToggleWinampPlaylist => {
                self.settings.playlist_open = !self.settings.playlist_open;
                self.settings_dirty = true;
                if self.settings.playlist_open {
                    self.refresh_queue(false);
                }
            }
            Action::SetPlaylistHeight(height) => {
                self.settings.playlist_height = height.clamp(
                    crate::skin::layout::PLAYLIST_MIN_HEIGHT,
                    crate::skin::layout::PLAYLIST_MAX_HEIGHT,
                );
                self.settings_dirty = true;
            }
            Action::ToggleWinampEq => {
                self.settings.eq_open = !self.settings.eq_open;
                self.settings_dirty = true;
            }
            Action::ToggleEq => {
                self.settings.eq_on = !self.settings.eq_on;
                self.push_eq();
            }
            Action::SetEqBand(band, gain_db) => {
                if let Some(slot) = self.settings.eq_bands_db.get_mut(band) {
                    *slot = gain_db.clamp(-crate::eq::RANGE_DB, crate::eq::RANGE_DB);
                    self.push_eq();
                }
            }
            Action::SetEqPreamp(gain_db) => {
                self.settings.eq_preamp_db =
                    gain_db.clamp(-crate::eq::RANGE_DB, crate::eq::RANGE_DB);
                self.push_eq();
            }
            Action::ApplyEqPreset(index) => {
                if let Some(preset) = crate::eq::PRESETS.get(index) {
                    self.settings.eq_bands_db = preset.bands_db;
                    self.settings.eq_on = true;
                    self.push_eq();
                }
            }
            Action::SetBalance(balance) => {
                self.settings.balance = balance.clamp(-1.0, 1.0);
                self.push_eq();
            }
            Action::ToggleMono => {
                self.settings.mono = !self.settings.mono;
                self.push_eq();
            }
            Action::ToggleWinampShade => {
                self.settings.winamp_shaded = !self.settings.winamp_shaded;
                self.settings_dirty = true;
            }
            Action::ToggleWinampPlaylistShade => {
                self.settings.playlist_shaded = !self.settings.playlist_shaded;
                self.settings_dirty = true;
            }
            Action::ToggleWinampEqShade => {
                self.settings.eq_shaded = !self.settings.eq_shaded;
                self.settings_dirty = true;
            }
            // The same request the window's own close button makes, so the
            // close-to-tray setting decides what follows.
            Action::CloseWindow => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::CycleVisualiser => {
                self.settings.vis = self.settings.vis.next();
                self.settings_dirty = true;
                self.winamp.analyser.reset();
            }
            Action::SetVisualiser(mode) => {
                if self.settings.vis != mode {
                    self.settings.vis = mode;
                    self.settings_dirty = true;
                    self.winamp.analyser.reset();
                }
            }
            Action::OpenSkinsFolder => self.open_folder(self.dirs.skins_dir()),
            Action::ToggleWinampMilkdrop => {
                self.settings.milkdrop_open = !self.settings.milkdrop_open;
                self.settings_dirty = true;
                #[cfg(feature = "milkdrop")]
                if self.settings.milkdrop_open {
                    // A first open has nothing to draw but the idle preset,
                    // which hardly answers the music; fetch the packs in the
                    // background and the window fills up on its own.
                    let folder = self.dirs.milkdrop_dir();
                    self.winamp.presets.refresh(&folder);
                    if self.winamp.presets.count() == 0
                        && self.winamp.presets.downloading().is_none()
                    {
                        self.winamp.presets.download_missing(folder, ctx.clone());
                        self.toast("Fetching MilkDrop's preset packs in the background");
                    }
                }
            }
            Action::SetMilkdropSeconds(seconds) => {
                self.settings.milkdrop_seconds = seconds.clamp(1, 3600);
                self.settings_dirty = true;
            }
            Action::SetMilkdropFps(fps) => {
                self.settings.milkdrop_fps = fps.min(240);
                self.settings_dirty = true;
            }
            Action::SetMilkdropScale(scale) => {
                self.settings.milkdrop_scale = scale.clamp(1, 4);
                self.settings_dirty = true;
            }
            Action::OpenMilkdropFolder => self.open_folder(self.dirs.milkdrop_dir()),
            Action::DownloadMilkdropPack(index) => {
                if let Some(pack) = crate::milkdrop::PACKS.get(index) {
                    self.winamp
                        .presets
                        .download(pack, self.dirs.milkdrop_dir(), ctx.clone());
                    self.toast(format!("Fetching {} presets", pack.name));
                }
            }
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
        self.sync_window_title(ctx);
    }

    /// The title bar carries the playing song, the way Spotify's does, so
    /// an ungrouped taskbar button says what is on (#94).
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let title = match self.now_playing().filter(|now| now.playing) {
            Some(now) if now.subtitle.is_empty() => format!("{} - Fastpotify", now.title),
            Some(now) => format!("{} - {}", now.subtitle, now.title),
            None => "Fastpotify".to_string(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    pub fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.apply_theme(ctx);
        self.lock_scroll_axis(ctx);
        // The mini player has no sign-in screen; someone who needs one gets
        // the big window.
        let needs_sign_in = !(self.is_connected() && self.user.is_some())
            && !matches!(self.auth, AuthStatus::Connecting | AuthStatus::Starting)
            && !(self.is_connected() && self.user.is_none());
        if needs_sign_in && !self.switch_intent {
            // The skin windows want a signed-in player to answer to; the
            // sign-in screen comes back first.
            if self.settings.winamp_window {
                self.actions.push(Action::ToggleWinampWindow);
            } else if self.settings.wmp_window && self.wmp.skin.is_some() {
                self.actions.push(Action::ToggleWmpWindow);
            }
        }
        // The skin is the window: no sign-in screen, no Spotify chrome,
        // only the view the skin defines.
        if self.settings.wmp_window && self.wmp.skin.is_some() {
            crate::ui::wmp::show_window(self, ui);
        } else if self.settings.winamp_window {
            crate::ui::winamp::show(self, ui);
        } else {
            crate::ui::show(self, ui);
        }
        // MilkDrop is a window of its own, in a child process; the app opens,
        // updates, and hears back from it here.
        #[cfg(feature = "milkdrop")]
        self.sync_milkdrop(ctx);
        self.apply_actions(ctx);
        self.sync_media_controls();

        if !self.settings.winamp_window {
            if let Some(rect) = ctx.input(|input| input.viewport().inner_rect) {
                self.last_window_size = Some([rect.width(), rect.height()]);
            }
            if let Some(rect) = ctx.input(|input| input.viewport().outer_rect) {
                self.last_window_pos = Some([rect.min.x, rect.min.y]);
            }
        }

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
            && !self.switch_intent
            && self.hides_to_tray()
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
        self.save_session();
    }

    /// Write the restorable session: page, recents, resume point, sorts.
    fn save_session(&mut self) {
        self.session_dirty = false;
        self.last_session_save = Instant::now();
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
                collapsed_folders: self.collapsed_folders.clone(),
                last_added_queue: if self.resume_queue.is_empty() {
                    self.manual_queue.clone()
                } else {
                    // Never resumed this session; the owed queue carries over.
                    self.resume_queue.clone()
                },
                last_queue_rows: self
                    .queue
                    .get()
                    .map(|queue| queue.queue.iter().take(30).cloned().collect())
                    .unwrap_or_default(),
                shuffle_on: self.shuffle_wanted,
                sorts: self
                    .table_sorts
                    .iter()
                    .map(|(page, sort)| (page.encode(), *sort))
                    .collect(),
                window_size: self.last_window_size.or(self.session_window_size),
                window_pos: self.last_window_pos.or(self.session_window_pos),
                queue_open: Some(self.show_queue_panel),
                winamp_pos: self.winamp.last_pos.or(self.winamp.restore_pos),
                wmp_pos: self.wmp.last_pos.or(self.wmp.restore_pos),
                milkdrop_pos: self.milkdrop_pos,
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

pub fn engine_config(
    dirs: &AppDirs,
    settings: &Settings,
    tap: std::sync::Arc<crate::vis::AudioTap>,
    eq: crate::eq::SharedEq,
) -> EngineConfig {
    EngineConfig {
        tap,
        eq,
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

/// The equalizer as the settings describe it.
pub fn eq_settings(settings: &Settings) -> crate::eq::EqSettings {
    crate::eq::EqSettings {
        on: settings.eq_on,
        preamp_db: settings.eq_preamp_db,
        bands_db: settings.eq_bands_db,
        balance: settings.balance,
        mono: settings.mono,
    }
    .clamped()
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

/// Whether a Spotify-owned playlist named `name` is the personal one the
/// Made for you shelf looks for under `term`. The name has to be the term
/// itself, or "Daily Mix" with a number: Spotify also makes "<Artist> Mix",
/// "This Is <Artist>", and "<Artist> Radio" for every artist, and an artist
/// called "Discover Weekly" put those on the shelf (#89).
fn is_made_for_you(name: &str, term: &str) -> bool {
    let name = name.trim().to_lowercase();
    let term = term.to_lowercase();
    if name == term {
        return true;
    }
    term == "daily mix"
        && name
            .strip_prefix("daily mix ")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// What the engine is told to play. A single song goes as a context of
/// its own rather than a list of one: Spotify resolves a track's URI as a
/// context, and a context with a URI is what librespot's autoplay carries
/// on from when it ends, the way one song from a search does in Spotify.
fn local_load(request: &PlayRequest, shuffle: bool) -> LoadSpec {
    let single_song = request.context_uri.is_none()
        && request.uris.len() == 1
        && request.uris[0].contains(":track:");
    if single_song {
        return LoadSpec {
            context_uri: Some(request.uris[0].clone()),
            position_ms: request.position_ms,
            play: true,
            ..LoadSpec::default()
        };
    }
    // A plain track list must not load shuffled when a row was chosen:
    // librespot shuffles the list first and then cannot find the chosen
    // row in it, falls back to nowhere, and replays what was on. The list
    // loads straight and shuffle is switched on right after the load,
    // which also matches Spotify: the chosen song plays, the rest shuffle.
    let chosen = request.offset_uri.is_some() || request.offset_position.is_some();
    let list = request.context_uri.is_none();
    LoadSpec {
        context_uri: request.context_uri.clone(),
        uris: request.uris.clone(),
        offset_uri: request.offset_uri.clone(),
        offset_index: request.offset_position,
        position_ms: request.position_ms,
        play: true,
        shuffle: (shuffle && !(list && chosen)).then_some(true),
        autoplay: false,
    }
}

/// The song to seed autoplay with when local playback stops at the end of
/// a plain list: the list's last one, provided the stop is the list's end
/// rather than the listener's, the session still stands, and autoplay is
/// on.
fn autoplay_seed(
    list: Option<&[String]>,
    autoplay: bool,
    before: &LocalState,
    after: &LocalState,
) -> Option<String> {
    if !autoplay
        || !after.connected
        || after.playback != Playback::Stopped
        || before.playback != Playback::Playing
    {
        return None;
    }
    let track = before.track.as_ref()?;
    if list?.last() != Some(&track.uri) || track.duration_ms == 0 {
        return None;
    }
    let near_end = before.position_now() + 3_000 >= track.duration_ms;
    near_end.then(|| track.uri.clone())
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

    #[test]
    fn volume_conversions_round_trip() {
        assert_eq!(volume_to_percent(u16::MAX), 100);
        assert_eq!(volume_to_percent(0), 0);
        assert_eq!(volume_to_percent(percent_to_volume(70)), 70);
        assert_eq!(percent_to_volume(200), u16::MAX);
    }

    /// The song the last session ended on is shown, paused, at the position
    /// it stopped at, so a cold start does not look like an empty player.
    #[test]
    fn the_remembered_song_is_shown_paused_at_its_position() {
        use crate::api::models::{Album, ArtistRef, Track};
        let mut app = headless_app();
        app.resume_track = Some("spotify:track:abc".into());
        app.resume_position_ms = 19_566;
        assert!(
            app.now_playing().is_none(),
            "nothing to show until the song's details arrive"
        );
        app.track_cache.insert(
            "abc".into(),
            Track {
                id: Some("abc".into()),
                uri: "spotify:track:abc".into(),
                name: "Karma Police".into(),
                artists: vec![ArtistRef {
                    id: None,
                    name: "Radiohead".into(),
                    uri: None,
                }],
                album: Some(Album {
                    id: "ok".into(),
                    name: "OK Computer".into(),
                    ..Default::default()
                }),
                duration_ms: 264_000,
                ..Default::default()
            },
        );
        let now = app.now_playing().expect("the remembered song is shown");
        assert!(now.resuming);
        assert!(!now.playing, "it is shown paused, not played");
        assert_eq!(now.title, "Karma Police");
        assert_eq!(now.subtitle, "Radiohead");
        assert_eq!(now.album_name, "OK Computer");
        assert_eq!(now.duration_ms, 264_000);
        assert_eq!(
            now.position_ms, 19_566,
            "the bar sits where the listener left it, not at zero"
        );
    }

    /// Drawing the remembered song must not be mistaken for a song that
    /// started: that would rewind the very position it exists to show.
    #[test]
    fn showing_the_remembered_song_keeps_its_position() {
        use crate::api::models::Track;
        let mut app = headless_app();
        app.resume_track = Some("spotify:track:abc".into());
        app.resume_position_ms = 19_566;
        app.track_cache.insert(
            "abc".into(),
            Track {
                id: Some("abc".into()),
                uri: "spotify:track:abc".into(),
                duration_ms: 264_000,
                ..Default::default()
            },
        );
        app.on_now_playing_changed();
        assert_eq!(app.resume_position_ms, 19_566);
        app.save_session();
        assert_eq!(
            app.resume_position_ms, 19_566,
            "closing again must not lose the position"
        );
    }

    /// Dragging the bar before pressing play moves where play will land.
    #[test]
    fn seeking_the_remembered_song_moves_the_resume_point() {
        let mut app = headless_app();
        app.resume_track = Some("spotify:track:abc".into());
        app.resume_position_ms = 19_566;
        app.seek(90_000);
        assert_eq!(app.resume_position_ms, 90_000);
    }

    /// The point of the whole thing: play, pressed on a cold start, picks
    /// the song up where it was left rather than at its beginning.
    #[test]
    fn pressing_play_on_a_cold_start_does_not_restart_the_song() {
        let mut app = headless_app();
        app.resume_context = Some("spotify:playlist:pl1".into());
        app.resume_track = Some("spotify:track:abc".into());
        app.resume_position_ms = 19_566;
        app.toggle_play();
        let request = app
            .queued_play
            .as_ref()
            .expect("the resumed play is held for the engine");
        assert_eq!(
            request.context_uri.as_deref(),
            Some("spotify:playlist:pl1"),
            "it resumes inside the playlist it was left in"
        );
        assert_eq!(request.offset_uri.as_deref(), Some("spotify:track:abc"));
        assert_eq!(
            request.position_ms, 19_566,
            "the song resumes where it stopped, not at zero"
        );
    }

    /// A restored song is loaded and current, not playing. The transport
    /// must act on it from that standstill: skipping moves through the
    /// playlist it was left in without ever starting the restored song.
    #[test]
    fn the_transport_works_on_a_restored_song_without_playing_it() {
        use crate::api::models::{PlayableItem, PlaylistItem, Track};
        use crate::model::PagedList;
        let row = |uri: &str| PlaylistItem {
            item: Some(PlayableItem::Track(Track {
                id: Some(uri.rsplit(':').next().unwrap().into()),
                uri: uri.into(),
                name: uri.into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.playlist_pages.insert(
            "pl1".into(),
            PlaylistPage {
                items: PagedList {
                    items: vec![
                        row("spotify:track:one"),
                        row("spotify:track:two"),
                        row("spotify:track:three"),
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        app.resume_context = Some("spotify:playlist:pl1".into());
        app.resume_track = Some("spotify:track:two".into());
        app.resume_position_ms = 19_566;
        assert!(app.resume_only(), "loaded and current, but not playing");

        // Next steps to the following song, at its start, still not playing.
        app.apply(Action::Next, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("spotify:track:three"));
        assert_eq!(app.resume_position_ms, 0);
        assert!(
            app.queued_play.is_none() && app.local_list.is_none(),
            "skipping must not start the restored song"
        );
        // Its details come from the rows already loaded, so the bar fills
        // in at once rather than blanking.
        let now = app.now_playing().expect("the new song is shown");
        assert!(now.resuming && !now.playing);
        assert_eq!(now.uri, "spotify:track:three");

        // Previous steps back from the start of a song.
        app.apply(Action::Previous, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("spotify:track:two"));

        // Past the threshold, Previous restarts instead, as it does while
        // playing.
        app.resume_position_ms = 19_566;
        app.apply(Action::Previous, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("spotify:track:two"));
        assert_eq!(app.resume_position_ms, 0, "it restarts the song");

        // The ends of the list wrap rather than dead-ending.
        app.apply(Action::Previous, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("spotify:track:one"));
        app.apply(Action::Previous, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("spotify:track:three"));

        // And play still starts whatever the skipping settled on, in the
        // playlist it belongs to.
        app.apply(Action::TogglePlay, &ctx);
        let request = app.queued_play.as_ref().expect("play starts it");
        assert_eq!(
            request.context_uri.as_deref(),
            Some("spotify:playlist:pl1"),
            "the playlist it was left in is kept"
        );
        assert_eq!(request.offset_uri.as_deref(), Some("spotify:track:three"));
    }

    /// Shuffle is the listener's mode and outlives a close: a skip from the
    /// standstill lands elsewhere in the context, and shuffle stays on.
    #[test]
    fn skipping_a_restored_song_keeps_shuffle_on() {
        use crate::api::models::{PlayableItem, PlaylistItem, Track};
        use crate::model::PagedList;
        let row = |uri: &str| PlaylistItem {
            item: Some(PlayableItem::Track(Track {
                uri: uri.into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.playlist_pages.insert(
            "pl1".into(),
            PlaylistPage {
                items: PagedList {
                    items: vec![row("spotify:track:one"), row("spotify:track:two")],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        app.resume_context = Some("spotify:playlist:pl1".into());
        app.resume_track = Some("spotify:track:one".into());
        app.shuffle_wanted = true;
        app.apply(Action::Next, &ctx);
        assert!(app.shuffle_wanted, "shuffle survives the skip");
        assert_eq!(
            app.resume_track.as_deref(),
            Some("spotify:track:two"),
            "a shuffled skip still lands on another song in the context"
        );
    }

    /// The queue saved at close is owed to the resumed song alone:
    /// resuming it queues the songs again, starting anything else lets
    /// them go.
    #[test]
    fn the_saved_queue_follows_the_resumed_song_only() {
        let mut app = headless_app();
        app.resume_track = Some("spotify:track:abc".into());
        app.resume_queue = vec!["spotify:track:q1".into(), "spotify:track:q2".into()];
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:other".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.on_now_playing_changed();
        assert!(
            app.resume_queue.is_empty(),
            "a fresh start lets the saved queue go"
        );
        assert!(app.session_dirty);
    }

    /// A chosen row in a plain list loads the list straight; librespot
    /// shuffling first loses the chosen row and replays what was on.
    /// Shuffle is put back by a command after the load.
    #[test]
    fn a_chosen_row_in_a_list_never_loads_shuffled() {
        let request = PlayRequest::tracks(vec!["spotify:track:a".into(), "spotify:track:b".into()])
            .starting_at_index(1);
        let load = local_load(&request, true);
        assert_eq!(load.shuffle, None);
        assert_eq!(load.offset_index, Some(1));
        // Without a chosen row the list may shuffle from the start.
        let request = PlayRequest::tracks(vec!["spotify:track:a".into(), "spotify:track:b".into()]);
        assert_eq!(local_load(&request, true).shuffle, Some(true));
        // A context play keeps its shuffled load; the offset was already
        // picked to match.
        let request = PlayRequest::context("spotify:playlist:x").starting_at_uri("spotify:track:a");
        assert_eq!(local_load(&request, true).shuffle, Some(true));
    }

    fn queued_song(uri: &str) -> crate::api::models::PlayableItem {
        crate::api::models::PlayableItem::Track(crate::api::models::Track {
            uri: uri.into(),
            ..Default::default()
        })
    }

    fn loaded_queue(current: &str, next: &[&str]) -> Loadable<Queue> {
        Loadable::Loaded(Queue {
            currently_playing: Some(queued_song(current)),
            queue: next.iter().map(|uri| queued_song(uri)).collect(),
        })
    }

    fn queue_uris(app: &App) -> (Option<String>, Vec<String>) {
        let queue = app.queue.get().expect("the queue stays loaded");
        (
            queue
                .currently_playing
                .as_ref()
                .map(|item| item.uri().to_string()),
            queue
                .queue
                .iter()
                .map(|item| item.uri().to_string())
                .collect(),
        )
    }

    /// Next is a pop: the head of Next up becomes the playing row the
    /// moment the button is pressed, without waiting for the Web API.
    #[test]
    fn next_pops_the_queue_head_into_now_playing() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue("spotify:track:a", &["spotify:track:b", "spotify:track:c"]);
        app.apply(Action::Next, &ctx);
        let (current, next) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("spotify:track:b"));
        assert_eq!(next, vec!["spotify:track:c"]);
        assert_eq!(
            app.current_track_uri().as_deref(),
            Some("spotify:track:b"),
            "the popped row is already the one the interface marks as playing"
        );
    }

    /// A song that starts consumes its queue row at once, however it came
    /// on: pressed here, skipped from another device, or reached on its
    /// own when the song before it ended.
    #[test]
    fn a_song_starting_consumes_its_queue_row() {
        let mut app = headless_app();
        app.queue = loaded_queue("spotify:track:a", &["spotify:track:b", "spotify:track:c"]);
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:b".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.on_now_playing_changed();
        let (current, next) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("spotify:track:b"));
        assert_eq!(next, vec!["spotify:track:c"]);
    }

    /// A queue answer from before the user's skip must not undo what the
    /// interface already shows; only an answer Spotify keeps giving is
    /// finally believed.
    #[test]
    fn a_stale_queue_answer_does_not_undo_a_skip() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue("spotify:track:a", &["spotify:track:b", "spotify:track:c"]);
        app.apply(Action::Next, &ctx);

        let stale = Queue {
            currently_playing: Some(queued_song("spotify:track:a")),
            queue: vec![
                queued_song("spotify:track:b"),
                queued_song("spotify:track:c"),
            ],
        };
        app.handle_api(ApiResponse::Queue {
            seq: app.queue_seq,
            result: Ok(stale.clone()),
        });
        let (current, next) = queue_uris(&app);
        assert_eq!(
            current.as_deref(),
            Some("spotify:track:b"),
            "the pop stands"
        );
        assert_eq!(next, vec!["spotify:track:c"]);
        assert!(
            app.queue_recheck_at.is_some(),
            "the stale answer is asked again rather than believed"
        );

        // Spotify telling the same story every time eventually wins.
        for _ in 0..QUEUE_STALE_RETRIES {
            app.handle_api(ApiResponse::Queue {
                seq: app.queue_seq,
                result: Ok(stale.clone()),
            });
        }
        let (current, _) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("spotify:track:a"));
    }

    /// A hand-queued song that has started playing must not be put back on
    /// top of Next up by the pending add that created its row.
    #[test]
    fn a_played_pending_add_is_not_put_back() {
        let mut app = headless_app();
        app.pending_queue_adds = vec![("spotify:track:b".into(), Instant::now())];
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:b".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue("spotify:track:b", &["spotify:track:c"]);
        app.reconcile_pending_queue();
        let (_, next) = queue_uris(&app);
        assert_eq!(next, vec!["spotify:track:c"], "no resurrected row on top");
        assert!(
            app.pending_queue_adds.is_empty(),
            "the add has been consumed"
        );
    }

    /// A chosen row of Next up plays at once: the rows above it go with
    /// it and the rows after it stay put, like pressing Next down to it.
    #[test]
    fn a_chosen_queue_row_plays_at_once_and_takes_the_rows_above() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.manual_queue = vec!["spotify:track:b".into(), "spotify:track:c".into()];
        app.queue = loaded_queue(
            "spotify:track:a",
            &["spotify:track:b", "spotify:track:c", "spotify:track:d"],
        );
        app.apply(
            Action::PlayFromRow {
                context: RowContext::Queue,
                uri: "spotify:track:c".into(),
                index: 1,
            },
            &ctx,
        );
        let (current, next) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("spotify:track:c"));
        assert_eq!(
            next,
            vec!["spotify:track:d"],
            "the rows after the chosen one stay"
        );
        assert_eq!(
            app.current_track_uri().as_deref(),
            Some("spotify:track:c"),
            "the chosen row is marked as playing at once"
        );
        assert!(
            app.manual_queue.is_empty(),
            "hand-queued songs consumed by the jump are let go"
        );
        assert!(app.play_pending("spotify:track:c"));
    }

    /// The click names a song: when the rows have shifted under the
    /// pointer, the song wins over the row number.
    #[test]
    fn a_clicked_queue_row_is_found_by_its_song_when_rows_shifted() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue("spotify:track:a", &["spotify:track:b", "spotify:track:c"]);
        app.apply(
            Action::PlayFromRow {
                context: RowContext::Queue,
                uri: "spotify:track:c".into(),
                index: 0,
            },
            &ctx,
        );
        let (current, next) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("spotify:track:c"));
        assert!(
            next.is_empty(),
            "the row above the chosen song went with it"
        );
    }

    /// Clear queue takes the hand-queued rows out at once and keeps the
    /// context's upcoming songs; a song queued once that the context also
    /// carries keeps its context row.
    #[test]
    fn clear_queue_takes_the_hand_queued_rows_and_keeps_the_context() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.manual_queue = vec!["spotify:track:b".into(), "spotify:track:c".into()];
        app.queue = loaded_queue(
            "spotify:track:a",
            &[
                "spotify:track:b",
                "spotify:track:c",
                "spotify:track:c",
                "spotify:track:d",
            ],
        );
        assert!(app.can_clear_queue());
        app.apply(Action::ClearQueue, &ctx);
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec!["spotify:track:c", "spotify:track:d"],
            "one queued c goes, the context's own c stays"
        );
        assert!(app.manual_queue.is_empty());
        assert!(
            app.queue_recheck_at.is_some(),
            "a fetch follows to sweep rows queued from other devices"
        );
    }

    /// Rule: Play next goes in after the songs already queued and ahead
    /// of the playing context's own rows.
    #[test]
    fn play_next_queues_after_the_songs_already_queued() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue(
            "spotify:track:a",
            &["spotify:track:ctx1", "spotify:track:ctx2"],
        );
        app.apply(
            Action::AddToQueue {
                uri: "spotify:track:b".into(),
                label: "b".into(),
            },
            &ctx,
        );
        app.apply(
            Action::AddToQueue {
                uri: "spotify:track:c".into(),
                label: "c".into(),
            },
            &ctx,
        );
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec![
                "spotify:track:b",
                "spotify:track:c",
                "spotify:track:ctx1",
                "spotify:track:ctx2",
            ],
            "queued songs keep their order and stay ahead of the context"
        );
    }

    /// Rule: asking again queues it again; only a double-click's second
    /// click is the same ask.
    #[test]
    fn asking_play_next_twice_queues_two_rows() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue("spotify:track:a", &["spotify:track:ctx1"]);
        let add = Action::AddToQueue {
            uri: "spotify:track:b".into(),
            label: "b".into(),
        };
        app.apply(add.clone(), &ctx);
        app.apply(add.clone(), &ctx);
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec!["spotify:track:b", "spotify:track:ctx1"],
            "the double-click's second click is not a second wish"
        );
        // Deliberately asked again, later.
        for (_, at) in &mut app.pending_queue_adds {
            *at = Instant::now() - QUEUE_ADD_DEBOUNCE;
        }
        app.apply(add, &ctx);
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec!["spotify:track:b", "spotify:track:b", "spotify:track:ctx1"],
            "two asks are two rows, one after the other"
        );
    }

    /// Rule: an answer overtaken by a newer request is dropped unread,
    /// whatever it says.
    #[test]
    fn an_overtaken_queue_answer_is_dropped_unread() {
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue("spotify:track:a", &["spotify:track:b"]);
        app.queue_seq = 2;
        let old_story = Queue {
            currently_playing: Some(queued_song("spotify:track:a")),
            queue: Vec::new(),
        };
        app.handle_api(ApiResponse::Queue {
            seq: 1,
            result: Ok(old_story),
        });
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec!["spotify:track:b"],
            "the overtaken answer changed nothing"
        );
        let current_story = Queue {
            currently_playing: Some(queued_song("spotify:track:a")),
            queue: vec![
                queued_song("spotify:track:b"),
                queued_song("spotify:track:c"),
            ],
        };
        app.handle_api(ApiResponse::Queue {
            seq: 2,
            result: Ok(current_story),
        });
        let (_, next) = queue_uris(&app);
        assert_eq!(next, vec!["spotify:track:b", "spotify:track:c"]);
    }

    /// Rule: a row you queued is put back until Spotify confirms it, in
    /// its place after the queued section, not on top of it.
    #[test]
    fn a_missing_queued_row_returns_to_its_place() {
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.manual_queue = vec!["spotify:track:b".into(), "spotify:track:c".into()];
        app.pending_queue_adds = vec![("spotify:track:c".into(), Instant::now())];
        // Spotify's answer knows b already but not c yet.
        app.queue = loaded_queue(
            "spotify:track:a",
            &["spotify:track:b", "spotify:track:ctx1"],
        );
        app.reconcile_pending_queue();
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec!["spotify:track:b", "spotify:track:c", "spotify:track:ctx1"],
            "the missing row comes back after the queued section"
        );
    }

    /// The view splits the queue where the user's own songs end; rows
    /// queued elsewhere or belonging to the context stay below the line.
    #[test]
    fn the_queued_section_covers_only_the_users_own_rows() {
        let mut app = headless_app();
        app.manual_queue = vec!["spotify:track:b".into(), "spotify:track:c".into()];
        app.queue = loaded_queue(
            "spotify:track:a",
            &[
                "spotify:track:b",
                "spotify:track:c",
                "spotify:track:ctx1",
                "spotify:track:c",
            ],
        );
        assert_eq!(
            app.queued_rows_len(),
            2,
            "the context's own copy of c does not count as queued"
        );
        app.manual_queue.clear();
        assert_eq!(app.queued_rows_len(), 0);
    }

    /// Rule: closing the app keeps the queue. The rows come back on the
    /// next start, split where the user's own songs end, and stay until
    /// something actually plays.
    #[test]
    fn the_queue_comes_back_after_a_restart() {
        let root = std::env::temp_dir().join(format!(
            "fastpotify-queue-restart-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let options = AppOptions {
            media_controls: false,
            tray: false,
        };
        let mut app = App::new(
            &Waker::default(),
            dirs.clone(),
            Settings::default(),
            options,
        );
        app.local_ready = true;
        app.resume_track = Some("spotify:track:a".into());
        app.manual_queue = vec!["spotify:track:b".into()];
        app.queue = loaded_queue(
            "spotify:track:a",
            &["spotify:track:b", "spotify:track:ctx1"],
        );
        app.save_session();

        let options = AppOptions {
            media_controls: false,
            tray: false,
        };
        let app = App::new(&Waker::default(), dirs, Settings::default(), options);
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec!["spotify:track:b", "spotify:track:ctx1"],
            "the queue is shown as it was left"
        );
        assert_eq!(
            app.queued_rows_len(),
            1,
            "the remembered hand-queued song keeps its own section"
        );
    }

    /// Saving the queue writes the playing song and every row after it,
    /// each song once, in playing order.
    #[test]
    fn saving_the_queue_writes_each_song_once_in_order() {
        let mut app = headless_app();
        app.local.track = Some(crate::player::LocalTrack {
            uri: "spotify:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue(
            "spotify:track:a",
            &[
                "spotify:track:b",
                "spotify:track:a",
                "spotify:track:b",
                "spotify:track:c",
            ],
        );
        assert_eq!(
            app.queue_playlist_uris(),
            vec!["spotify:track:a", "spotify:track:b", "spotify:track:c"],
            "the playing song leads and a repeat wrap adds nothing"
        );
    }

    /// A saved song radio is named after its song; any other queue is
    /// named after the day.
    #[test]
    fn a_saved_radio_is_named_after_its_song() {
        let mut app = headless_app();
        app.track_cache.insert(
            "xyz".into(),
            crate::api::models::Track {
                id: Some("xyz".into()),
                uri: "spotify:track:xyz".into(),
                name: "Wish You Were Here".into(),
                ..Default::default()
            },
        );
        app.assumed_context = Some(AssumedContext {
            uri: "spotify:station:track:xyz".into(),
            shuffle: None,
            at: Instant::now(),
        });
        assert_eq!(app.queue_playlist_name(), "Wish You Were Here Radio");
        app.assumed_context = None;
        assert!(app.queue_playlist_name().starts_with("Queue "));
    }

    /// Song radio opens the queue: fifty songs appearing there is the
    /// whole visible result of the click.
    #[test]
    fn song_radio_opens_the_queue() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.apply(Action::PlayTrackRadio("spotify:track:xyz".into()), &ctx);
        assert!(app.show_queue_panel);
        assert_eq!(
            app.playing_context_uri().as_deref(),
            Some("spotify:station:track:xyz"),
            "the station is what the interface calls playing"
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
        app.local_ready = true;
        app
    }

    /// A shuffle play must not start at track one, wherever it is begun
    /// from: on the rows the app holds it picks one of them, and with no
    /// rows at hand the Web API is sent a position drawn from the length
    /// the library knows. Local playback is given neither, because
    /// librespot draws its own starting track.
    #[test]
    fn a_shuffled_play_does_not_start_at_track_one() {
        use crate::api::models::{
            Album, PlayableItem, Playlist, PlaylistItem, SavedAlbum, Track, TrackCount,
        };
        let track = |uri: &str| {
            Some(PlayableItem::Track(Track {
                uri: uri.into(),
                ..Default::default()
            }))
        };
        let mut app = headless_app();
        app.library.playlists = Loadable::Loaded(vec![
            Playlist {
                uri: "spotify:playlist:open".into(),
                tracks: Some(TrackCount { total: 3 }),
                ..Default::default()
            },
            Playlist {
                uri: "spotify:playlist:unopened".into(),
                tracks: Some(TrackCount { total: 57 }),
                ..Default::default()
            },
        ]);
        app.library.albums.items = vec![SavedAlbum {
            album: Album {
                uri: "spotify:album:saved".into(),
                total_tracks: Some(12),
                ..Default::default()
            },
            ..Default::default()
        }];
        app.library.liked.total = Some(9);
        app.playlist_pages.insert(
            "open".into(),
            PlaylistPage {
                items: PagedList {
                    items: vec![
                        PlaylistItem {
                            item: track("spotify:track:one"),
                            ..Default::default()
                        },
                        PlaylistItem {
                            item: track("spotify:track:two"),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        // Playing on a phone: the Web API needs the offset.
        app.selected_device = Some("phone".into());
        assert!(matches!(app.target(), Target::Remote(Some(_))));

        // The playlist on screen: the start is one of its own rows.
        let (uri, position) = app.shuffle_start("spotify:playlist:open");
        assert!(
            matches!(
                uri.as_deref(),
                Some("spotify:track:one" | "spotify:track:two")
            ),
            "the start comes from the rows, got {uri:?}"
        );
        assert_eq!(position, None);

        // Begun from the sidebar or a menu, with no rows loaded: a
        // position inside the length the library reported.
        for (context, len) in [
            ("spotify:playlist:unopened", 57),
            ("spotify:album:saved", 12),
            ("spotify:user:someone:collection", 9),
        ] {
            for _ in 0..50 {
                let (uri, position) = app.shuffle_start(context);
                assert_eq!(uri, None, "{context} has no rows to name");
                let position = position.unwrap_or_else(|| panic!("{context} got no offset"));
                assert!(
                    position < len,
                    "{context} offset {position} is outside {len}"
                );
            }
        }
        // Over 50 draws a 57-song playlist should not have sat still on
        // one song, let alone on the first.
        let drawn: std::collections::HashSet<Option<u32>> = (0..50)
            .map(|_| app.shuffle_start("spotify:playlist:unopened").1)
            .collect();
        assert!(drawn.len() > 1, "the starting position never moved");

        // Nothing saved, nothing loaded: no offset to give.
        assert_eq!(app.shuffle_start("spotify:playlist:unknown"), (None, None));

        // Local playback: librespot picks the starting track itself.
        app.selected_device = None;
        assert!(matches!(app.target(), Target::Local));
        assert_eq!(
            app.shuffle_start("spotify:playlist:unopened"),
            (None, None),
            "librespot is left to draw its own"
        );
    }

    /// A Free account is told once per sign-in that nothing will play;
    /// a Premium one is not bothered.
    #[test]
    fn a_free_account_is_told_once_that_it_cannot_play() {
        let me = |product: &str| {
            ApiResponse::Me(Ok(crate::api::models::User {
                id: "someone".into(),
                product: Some(product.into()),
                ..Default::default()
            }))
        };
        let mut app = headless_app();
        app.handle_api(me("free"));
        assert!(matches!(app.dialog, Some(Dialog::PremiumNeeded)));
        app.dialog = None;
        app.handle_api(me("free"));
        assert!(app.dialog.is_none(), "the notice is shown once");

        let mut app = headless_app();
        app.handle_api(me("premium"));
        assert!(app.dialog.is_none());
    }

    /// Only the personal playlists themselves belong on the shelf, not
    /// what Spotify generates for an artist who took one of their names.
    #[test]
    fn the_shelf_takes_the_playlist_and_not_an_artist_named_after_it() {
        assert!(is_made_for_you("Discover Weekly", "Discover Weekly"));
        assert!(is_made_for_you("release radar", "Release Radar"));
        assert!(is_made_for_you("daylist", "daylist"));
        assert!(is_made_for_you("Daily Mix 3", "Daily Mix"));
        assert!(is_made_for_you("Daily Mix", "Daily Mix"));
        assert!(!is_made_for_you("Discover Weekly Mix", "Discover Weekly"));
        assert!(!is_made_for_you(
            "This Is Discover Weekly",
            "Discover Weekly"
        ));
        assert!(!is_made_for_you("Release Radar Radio", "Release Radar"));
        assert!(!is_made_for_you("Daily Mix Radio", "Daily Mix"));
        assert!(!is_made_for_you("Daily Mix 3", "Discover Weekly"));
    }

    /// One song plays as a context of its own, so librespot's autoplay
    /// follows it; a list stays a list.
    #[test]
    fn one_song_is_loaded_as_a_context() {
        let one = local_load(&PlayRequest::tracks(vec!["spotify:track:a".into()]), false);
        assert_eq!(one.context_uri.as_deref(), Some("spotify:track:a"));
        assert!(one.uris.is_empty() && !one.autoplay);
        let two = local_load(
            &PlayRequest::tracks(vec!["spotify:track:a".into(), "spotify:track:b".into()])
                .starting_at_index(1),
            true,
        );
        assert_eq!(two.context_uri, None);
        assert_eq!(two.uris.len(), 2);
        assert_eq!(two.offset_index, Some(1));
        // A chosen row keeps the list load straight; shuffle follows as a
        // command (see a_chosen_row_in_a_list_never_loads_shuffled).
        assert_eq!(two.shuffle, None);
        let episode = local_load(
            &PlayRequest::tracks(vec!["spotify:episode:e".into()]),
            false,
        );
        assert_eq!(episode.context_uri, None);
        assert_eq!(episode.uris.len(), 1);
    }

    /// A plain list that plays out seeds autoplay with its last song; a
    /// stop anywhere else, a dropped session, or autoplay off does not.
    #[test]
    fn a_list_that_ends_seeds_autoplay_with_its_last_song() {
        let track = |uri: &str| {
            Some(crate::player::LocalTrack {
                uri: uri.into(),
                duration_ms: 200_000,
                ..Default::default()
            })
        };
        let playing = LocalState {
            playback: Playback::Playing,
            track: track("spotify:track:last"),
            position_ms: 198_500,
            connected: true,
            ..LocalState::default()
        };
        let stopped = LocalState {
            playback: Playback::Stopped,
            track: track("spotify:track:last"),
            connected: true,
            ..LocalState::default()
        };
        let list: Vec<String> = vec!["spotify:track:first".into(), "spotify:track:last".into()];
        assert_eq!(
            autoplay_seed(Some(&list), true, &playing, &stopped).as_deref(),
            Some("spotify:track:last")
        );
        assert_eq!(autoplay_seed(Some(&list), false, &playing, &stopped), None);
        assert_eq!(autoplay_seed(None, true, &playing, &stopped), None);
        let mid_song = LocalState {
            position_ms: 60_000,
            ..playing.clone()
        };
        assert_eq!(autoplay_seed(Some(&list), true, &mid_song, &stopped), None);
        let not_last = LocalState {
            track: track("spotify:track:first"),
            ..playing.clone()
        };
        assert_eq!(autoplay_seed(Some(&list), true, &not_last, &stopped), None);
        let dropped = LocalState {
            connected: false,
            ..stopped.clone()
        };
        assert_eq!(autoplay_seed(Some(&list), true, &playing, &dropped), None);
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

    /// The skin window toggles like the Winamp one: the setting flips,
    /// the loop is told to open the other kind of window, and the window
    /// being shown closes.
    #[test]
    fn the_wmp_window_toggles_like_the_winamp_one() {
        // #given
        let mut app = headless_app();

        // #when
        app.actions.push(Action::ToggleWmpWindow);
        app.apply_actions(&egui::Context::default());

        // #then
        assert!(app.settings.wmp_window);
        assert!(app.switch_intent);

        // #when
        app.actions.push(Action::ToggleWmpWindow);
        app.apply_actions(&egui::Context::default());

        // #then
        assert!(!app.settings.wmp_window);
        assert!(app.switch_intent);
    }

    /// The skin window comes back where it was, like the Winamp one.
    #[test]
    fn the_skin_window_remembers_where_it_was() {
        // #given
        let mut app = headless_app();
        app.settings.wmp_window = true;
        app.wmp.last_pos = Some([120.0, 80.0]);

        // #when
        app.actions.push(Action::ToggleWmpWindow);
        app.apply_actions(&egui::Context::default());

        // #then
        assert!(!app.settings.wmp_window);
        assert_eq!(app.wmp.restore_pos, Some([120.0, 80.0]));
    }

    /// The first skin a window mode wears arrives while the big window is
    /// up - the small one's size is only known from the skin - so the
    /// loop is asked for the small window; a re-worn skin asks for
    /// nothing.
    #[test]
    fn the_first_skin_worn_opens_the_skin_window() {
        // #given
        let mut app = headless_app();
        app.settings.wmp_window = true;
        app.settings.wmp_skin = Some("Toothy.wmz".into());

        // #when
        let document =
            crate::wmp::SkinDocument::from_files("toothy", [("toothy.wms", b"<theme/>".to_vec())])
                .unwrap();
        app.wmp_loaded(
            &egui::Context::default(),
            crate::wmp::Loaded {
                name: "Toothy.wmz".into(),
                result: Ok(document),
            },
        );

        // #then
        assert!(app.wmp.skin.is_some());
        assert_eq!(app.wmp.worn.as_deref(), Some("Toothy.wmz"));
        assert!(app.switch_intent, "the small window opens");

        // #when
        app.switch_intent = false;
        let document =
            crate::wmp::SkinDocument::from_files("toothy", [("toothy.wms", b"<theme/>".to_vec())])
                .unwrap();
        app.wmp_loaded(
            &egui::Context::default(),
            crate::wmp::Loaded {
                name: "Toothy.wmz".into(),
                result: Ok(document),
            },
        );

        // #then
        assert!(!app.switch_intent, "the window is already the small one");
    }

    /// A skin window whose skin the settings forget wears nothing, and
    /// the big window shows instead.
    #[test]
    fn a_skin_window_without_a_skin_wears_none() {
        // #given
        let mut app = headless_app();
        app.settings.wmp_window = true;
        let document =
            crate::wmp::SkinDocument::from_files("toothy", [("toothy.wms", b"<theme/>".to_vec())])
                .unwrap();
        app.wmp.wear(
            Some("Toothy.wmz".into()),
            crate::wmp::WmpSkin {
                document: std::sync::Arc::new(document),
                render: crate::ui::wmp::Render::default(),
            },
        );

        // #when
        app.settings.wmp_skin = None;
        app.sync_wmp_skin(&egui::Context::default());

        // #then
        assert!(app.wmp.skin.is_none());
    }

    /// A skin that will not assemble does not leave the app saying the
    /// skin window is on with nothing to show in it: the plain player
    /// is what the person has, and the setting says so.
    #[test]
    fn a_skin_that_fails_to_assemble_restores_the_plain_player() {
        // #given
        let mut app = headless_app();
        app.settings.wmp_window = true;
        app.settings.wmp_skin = Some("Broken.wmz".into());

        // #when
        app.wmp_loaded(
            &egui::Context::default(),
            crate::wmp::Loaded {
                name: "Broken.wmz".into(),
                result: Err(crate::wmp::WmpError::NoDefinition),
            },
        );

        // #then
        assert!(!app.settings.wmp_window);
        assert_eq!(app.settings.wmp_skin, None);
    }

    /// A skin that fails while another is worn falls back to the one
    /// still on, and the skin window stays on.
    #[test]
    fn a_skin_that_fails_falls_back_to_the_one_worn() {
        // #given
        let mut app = headless_app();
        app.settings.wmp_window = true;
        let document =
            crate::wmp::SkinDocument::from_files("toothy", [("toothy.wms", b"<theme/>".to_vec())])
                .unwrap();
        app.wmp.wear(
            Some("Toothy.wmz".into()),
            crate::wmp::WmpSkin {
                document: std::sync::Arc::new(document),
                render: crate::ui::wmp::Render::default(),
            },
        );
        app.settings.wmp_skin = Some("Broken.wmz".into());

        // #when
        app.wmp_loaded(
            &egui::Context::default(),
            crate::wmp::Loaded {
                name: "Broken.wmz".into(),
                result: Err(crate::wmp::WmpError::NoDefinition),
            },
        );

        // #then
        assert!(app.settings.wmp_window);
        assert_eq!(app.settings.wmp_skin, Some("Toothy.wmz".into()));
    }

    /// The verbs a Stream Deck key needs and a media key never asked for:
    /// a state said outright rather than toggled, an absolute position, a
    /// URI, and moving the sound to another device.
    #[test]
    fn a_key_can_ask_for_a_state_rather_than_a_toggle() {
        // #given
        let mut app = headless_app();
        let queue: std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>> = Default::default();
        app.control_commands = Some(std::sync::Arc::clone(&queue));

        // #when
        queue.lock().expect("the queue").extend([
            ControlCommand::SetShuffle(true),
            ControlCommand::SetRepeat(RepeatMode::Track),
            ControlCommand::SeekTo(90_000),
            ControlCommand::PlayUri("spotify:playlist:pl1".to_owned()),
            ControlCommand::Transfer("abc123".to_owned()),
            ControlCommand::RefreshDevices,
            // Nothing is playing in a headless app, so there is no track to
            // save and this one falls away rather than erroring.
            ControlCommand::ToggleSaved,
        ]);
        app.handle_control_commands();

        // #then
        assert!(
            matches!(
                app.actions.as_slice(),
                [
                    Action::SetShuffle(true),
                    Action::SetRepeat(RepeatMode::Track),
                    Action::Seek(90_000),
                    Action::PlayContext {
                        offset_uri: None,
                        offset_index: None,
                        ..
                    },
                    Action::Transfer(_),
                    Action::RefreshDevices,
                ]
            ),
            "{:?}",
            app.actions
        );
        assert!(queue.lock().expect("the queue").is_empty());
    }

    /// The snapshot a client polls keeps its first nine fields where they
    /// were, so a script written against the older shape still reads them,
    /// and says "unknown" rather than guessing at a saved flag nobody has
    /// told it yet.
    #[test]
    fn the_snapshot_appends_what_a_key_needs_without_moving_what_was_there() {
        // #given
        let mut app = headless_app();
        app.handle_local(LocalState {
            playback: Playback::Playing,
            track: Some(crate::player::LocalTrack {
                uri: "spotify:track:t1".to_owned(),
                title: "Go".to_owned(),
                artists: vec!["The Band".to_owned()],
                album: "First".to_owned(),
                art_url: Some("https://i.scdn.co/image/abc".to_owned()),
                duration_ms: 200_000,
                ..Default::default()
            }),
            position_ms: 20_000,
            volume: percent_to_volume(35),
            shuffle: true,
            repeat: RepeatMode::Track,
            ..LocalState::default()
        });

        // #when
        let snapshot = app.control_snapshot();
        let fields: Vec<&str> = snapshot.split('\t').collect();

        // #then
        assert_eq!(
            fields,
            [
                // The nine a media key or a Raycast script already read.
                "playing",
                "Go",
                "The Band",
                "First",
                "20000",
                "200000",
                "35",
                "on",
                "track",
                // The three a Stream Deck key needs, appended.
                "https://i.scdn.co/image/abc",
                // Not signed in, so nobody has said whether this is saved.
                "unknown",
                // Local playback is this computer, which Spotify has not
                // named because it is not a remote device.
                "Fastpotify",
            ]
        );
        // No devices seen yet is an empty array, not an empty string, so a
        // client never special-cases the answer.
        assert_eq!(
            app.control_devices_snapshot(),
            crate::single_instance::NO_DEVICES
        );
    }

    /// The device slot is written when Spotify answers rather than every
    /// frame, so the thing to check is that an answer still reaches it.
    #[test]
    fn a_device_list_reaches_the_slot_when_spotify_answers() {
        // #given
        let mut app = headless_app();
        let slot = std::sync::Arc::new(std::sync::Mutex::new(
            crate::single_instance::NO_DEVICES.to_owned(),
        ));
        app.control_devices = Some(std::sync::Arc::clone(&slot));
        app.control_devices_stale = false;

        // #when
        app.handle_api(ApiResponse::Devices(Ok(vec![Device {
            id: Some("abc123".to_owned()),
            name: "Kitchen\tspeaker".to_owned(),
            kind: "Speaker".to_owned(),
            is_active: true,
            ..Device::default()
        }])));
        app.sync_media_controls();

        // #then
        let written = slot.lock().expect("the slot").clone();
        assert_eq!(
            written,
            r#"[{"active":true,"id":"abc123","kind":"Speaker","name":"Kitchen\tspeaker"}]"#,
            "a name is carried whole, tab and all, because JSON escapes it \
             where the tab-separated snapshot could not"
        );
        // Written once and not again until the next answer.
        assert!(!app.control_devices_stale);
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

    /// A skin with a bitmap in it, for pretending one was read.
    fn some_skin(name: &str) -> crate::skin::Skin {
        let image = image::RgbImage::from_pixel(275, 116, image::Rgb([9, 9, 9]));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let archive = crate::skin::zip::write(&[("main.bmp", png.get_ref(), false)]);
        crate::skin::Skin::from_archive(name, &archive).unwrap()
    }

    #[test]
    fn a_skin_read_late_does_not_override_a_newer_choice() {
        let mut app = headless_app();
        app.settings.winamp_window = true;
        app.settings.skin = Some("B.wsz".into());
        app.skin_loaded(crate::winamp::Loaded {
            name: "A.wsz".into(),
            result: Ok(some_skin("A")),
            installed: false,
        });
        assert_eq!(app.winamp.worn.as_deref(), Some("A.wsz"));
        assert_eq!(app.settings.skin.as_deref(), Some("B.wsz"));
    }

    #[test]
    fn a_dropped_skin_becomes_the_choice_and_a_failed_one_is_forgotten() {
        let mut app = headless_app();
        app.settings.winamp_window = true;
        app.skin_loaded(crate::winamp::Loaded {
            name: "Dropped.wsz".into(),
            result: Ok(some_skin("Dropped")),
            installed: true,
        });
        assert_eq!(app.settings.skin.as_deref(), Some("Dropped.wsz"));
        assert_eq!(app.winamp.worn.as_deref(), Some("Dropped.wsz"));
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message == "Added the Dropped skin")
        );

        app.settings.skin = Some("Gone.wsz".into());
        app.skin_loaded(crate::winamp::Loaded {
            name: "Gone.wsz".into(),
            result: Err(crate::skin::SkinError::Empty),
            installed: false,
        });
        assert_eq!(app.settings.skin.as_deref(), Some("Dropped.wsz"));
        assert_eq!(app.winamp.worn.as_deref(), Some("Dropped.wsz"));
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message.starts_with("Gone: "))
        );
    }
}

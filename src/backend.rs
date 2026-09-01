//! The bridge between the interface thread and everything asynchronous.
//!
//! egui runs on the main thread and must never block. A dedicated tokio
//! runtime hosts the librespot engine, the Web API client, sign-in, and
//! artwork fetches; the two sides talk through channels. Every event wakes
//! the interface with `request_repaint`, so the app stays event-driven and
//! idle when nothing is happening.

use std::sync::Arc;
use std::time::{Duration, Instant};

use librespot_core::authentication::Credentials;
use tokio::sync::{mpsc, watch};

use crate::api::models::*;
use crate::api::{
    AccountId, ApiError, ApiGateway, ApiSource, NetActivity, Operation, PlayRequest, PlaylistId,
    SessionState, TokenProvider, WebTokens,
};
use crate::images::{ArtLoader, accent_color};
use crate::paths::AppDirs;
use crate::player::{Engine, EngineConfig, EngineEvent, LoadSpec, LocalState, PlayerCommand};

pub type ApiResult<T> = Result<T, ApiError>;

const PREMIUM_NEEDED: &str = "Local playback needs Spotify Premium.";
pub const PLAYLIST_PAGE_SIZE: u32 = 50;

#[derive(Clone, Debug, PartialEq)]
pub enum AuthStatus {
    Starting,
    SignedOut,
    WaitingForBrowser { url: String },
    Connecting,
    Connected { username: String },
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteAction {
    Play,
    Pause,
    Next,
    Previous,
    Seek,
    Volume,
    Shuffle,
    Repeat,
}

/// Which of the two readers of the recently-played endpoint an answer
/// belongs to: the shelf on Home, or the Recents tab in the queue panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentsFor {
    Home,
    Panel,
}

#[derive(Clone, Debug)]
pub enum ApiRequest {
    Me,
    Devices,
    PlaybackState {
        seq: u64,
    },
    Queue {
        seq: u64,
    },
    RecentlyPlayed {
        /// Who asked. The Home shelf and the queue panel's Recents tab
        /// both read this endpoint and each count their own requests, so
        /// the generation alone cannot tell their answers apart: two
        /// counters that both start at zero agree far more often than
        /// not, and an answer meant for one would land in the other.
        who: RecentsFor,
        generation: u64,
        before: Option<String>,
        limit: u32,
    },
    TopTracks {
        offset: u32,
        full: bool,
        generation: u64,
    },
    TopArtists {
        generation: u64,
    },
    Recommendations {
        seed_tracks: Vec<String>,
        seed_artists: Vec<String>,
        generation: u64,
    },
    Discover {
        term: String,
        generation: u64,
    },
    MyPlaylists {
        offset: u32,
    },
    Playlist {
        id: String,
        generation: u64,
    },
    PlaylistItems {
        id: String,
        offset: u32,
        generation: u64,
    },
    /// A slice of a playlist read only for who added its songs; the rows
    /// on screen stay untouched.
    PlaylistSample {
        id: String,
        offset: u32,
        generation: u64,
    },
    CreatePlaylist {
        name: String,
        public: bool,
        description: String,
    },
    UpdatePlaylist {
        id: String,
        name: Option<String>,
        description: Option<String>,
        public: Option<bool>,
    },
    AddToPlaylist {
        playlist_id: String,
        playlist_name: String,
        uris: Vec<String>,
    },
    RemoveFromPlaylist {
        playlist_id: String,
        uris: Vec<String>,
        snapshot_id: Option<String>,
    },
    ReorderPlaylist {
        playlist_id: String,
        range_start: u32,
        insert_before: u32,
        snapshot_id: Option<String>,
    },
    FollowPlaylist {
        id: String,
        follow: bool,
    },
    SavedTracks {
        offset: u32,
    },
    SavedAlbums {
        offset: u32,
    },
    FollowedArtists {
        after: Option<String>,
    },
    SavedShows {
        offset: u32,
    },
    SavedEpisodes {
        offset: u32,
    },
    SetSaved {
        uris: Vec<String>,
        saved: bool,
    },
    Contains {
        uris: Vec<String>,
    },
    Search {
        query: String,
        serial: u64,
    },
    Artist {
        id: String,
    },
    ArtistTopTracks {
        id: String,
    },
    ArtistAlbums {
        id: String,
        groups: String,
        offset: u32,
    },
    RelatedArtists {
        id: String,
    },
    Album {
        id: String,
    },
    AlbumTracks {
        id: String,
        offset: u32,
    },
    Show {
        id: String,
    },
    ShowEpisodes {
        id: String,
        offset: u32,
    },
    Track {
        id: String,
    },
    Remote {
        action: RemoteAction,
        device_id: Option<String>,
        play: Option<PlayRequest>,
        position_ms: u32,
        percent: u8,
        flag: bool,
        repeat: String,
    },
    Transfer {
        device_id: String,
        play: bool,
    },
    /// Shuffle on, then start the context, one after the other: sent as two
    /// independent requests they race, and shuffle sometimes lost.
    ShufflePlay {
        device_id: Option<String>,
        play: PlayRequest,
    },
    AddToQueue {
        uri: String,
        device_id: Option<String>,
        label: String,
    },
}

impl ApiRequest {
    fn background(&self) -> bool {
        matches!(
            self,
            Self::PlaybackState { .. }
                | Self::RecentlyPlayed { .. }
                | Self::TopTracks { .. }
                | Self::TopArtists { .. }
                | Self::Recommendations { .. }
                | Self::Discover { .. }
                | Self::MyPlaylists { .. }
                | Self::PlaylistSample { .. }
                | Self::Contains { .. }
        )
    }
}

#[derive(Debug)]
pub enum ApiResponse {
    Me(ApiResult<User>),
    Devices(ApiResult<Vec<Device>>),
    PlaybackState {
        seq: u64,
        result: ApiResult<Option<PlaybackState>>,
    },
    Queue {
        seq: u64,
        result: ApiResult<Queue>,
    },
    RecentlyPlayed {
        who: RecentsFor,
        generation: u64,
        limit: u32,
        result: ApiResult<CursorPage<PlayHistory>>,
    },
    TopTracks {
        offset: u32,
        full: bool,
        generation: u64,
        result: ApiResult<Page<Track>>,
    },
    TopArtists {
        generation: u64,
        result: ApiResult<Vec<Artist>>,
    },
    Recommendations {
        generation: u64,
        result: ApiResult<Vec<Track>>,
    },
    Discover {
        term: String,
        generation: u64,
        result: ApiResult<Vec<Playlist>>,
    },
    MyPlaylists {
        offset: u32,
        result: ApiResult<Page<Playlist>>,
    },
    Playlist {
        id: String,
        generation: u64,
        result: ApiResult<Playlist>,
    },
    PlaylistItems {
        id: String,
        offset: u32,
        generation: u64,
        result: ApiResult<Page<PlaylistItem>>,
    },
    PlaylistSample {
        id: String,
        generation: u64,
        result: ApiResult<Page<PlaylistItem>>,
    },
    PlaylistCreated(ApiResult<Playlist>),
    PlaylistUpdated {
        id: String,
        result: ApiResult<()>,
    },
    PlaylistItemsChanged {
        id: String,
        message: String,
        result: ApiResult<Option<String>>,
    },
    PlaylistFollowChanged {
        id: String,
        followed: bool,
        result: ApiResult<()>,
    },
    SavedTracks {
        offset: u32,
        result: ApiResult<Page<SavedTrack>>,
    },
    SavedAlbums {
        offset: u32,
        result: ApiResult<Page<SavedAlbum>>,
    },
    FollowedArtists {
        after: Option<String>,
        result: ApiResult<CursorPage<Artist>>,
    },
    SavedShows {
        offset: u32,
        result: ApiResult<Page<SavedShow>>,
    },
    SavedEpisodes {
        offset: u32,
        result: ApiResult<Page<SavedEpisode>>,
    },
    SavedChanged {
        uris: Vec<String>,
        saved: bool,
        result: ApiResult<()>,
    },
    Contains {
        uris: Vec<String>,
        result: ApiResult<Vec<bool>>,
    },
    Search {
        query: String,
        serial: u64,
        result: ApiResult<SearchResults>,
    },
    Artist {
        id: String,
        result: ApiResult<Artist>,
    },
    ArtistTopTracks {
        id: String,
        result: ApiResult<Vec<Track>>,
    },
    ArtistAlbums {
        id: String,
        groups: String,
        offset: u32,
        result: ApiResult<Page<Album>>,
    },
    RelatedArtists {
        id: String,
        result: ApiResult<Vec<Artist>>,
    },
    Album {
        id: String,
        result: ApiResult<Album>,
    },
    AlbumTracks {
        id: String,
        offset: u32,
        result: ApiResult<Page<Track>>,
    },
    Show {
        id: String,
        result: ApiResult<Show>,
    },
    ShowEpisodes {
        id: String,
        offset: u32,
        result: ApiResult<Page<Episode>>,
    },
    Track {
        id: String,
        result: ApiResult<Track>,
    },
    Remote {
        action: RemoteAction,
        result: ApiResult<()>,
    },
    Transferred {
        device_id: String,
        result: ApiResult<()>,
    },
    QueueAdded {
        label: String,
        result: ApiResult<()>,
    },
}

pub enum Command {
    /// Start (or restart) the Web API sign-in in the browser.
    SignIn,
    CancelSignIn,
    SignOut,
    /// Authorize local playback on this computer (a separate browser grant).
    AuthorizePlayback,
    /// Reload the engine config (audio settings changed).
    RestartEngine(EngineConfig),
    Player(PlayerCommand),
    Api(ApiRequest),
    Accent {
        url: String,
    },
    Shutdown,
    /// Internal: the Web API browser flow produced a grant.
    WebSignedIn {
        source: ApiSource,
        token: Box<crate::auth::StoredToken>,
    },
    WebVerified {
        source: ApiSource,
        token: Box<crate::auth::StoredToken>,
        user: Box<User>,
    },
    /// Internal: a Web API browser flow or verification ended (success or not).
    SignInEnded {
        source: ApiSource,
    },
    /// Internal: the playback browser flow ended without a credential.
    PlaybackAuthEnded,
    /// Internal: the Web API said which plan the account is on (`None` when
    /// it could not tell).
    AccountChecked {
        premium: Option<bool>,
    },
    /// Internal: the playback grant produced a streaming access token.
    PlaybackAuthorized {
        access_token: String,
    },
    /// Internal: an engine connection attempt finished.
    EngineConnected {
        engine: Box<Option<Engine>>,
        error: Option<String>,
    },
    /// Internal: librespot's session ended on its own.
    Reconnect,
    /// Look for Spotify Connect receivers on the local network.
    DiscoverReceivers,
    /// Hand the account to a receiver so it joins Spotify Connect.
    ActivateReceiver(Box<crate::zeroconf::Receiver>),
    /// Ask GitHub whether a newer release exists.
    CheckForUpdates,
    /// The words of a track, from LRCLIB.
    Lyrics(Box<LyricsRequest>),
    /// The account's playlist tree, folders and all, from the session.
    Rootlist,
    /// Check that a reconnect's pickup really started, and try again if not.
    VerifyResume,
    /// Add, replace, or remove the optional personal Web API application.
    ConfigurePersonalWebApp(Option<String>),
    /// Read a playlist's cached items from disk.
    LoadPlaylistCache {
        id: String,
    },
    /// Remember a fully loaded playlist on disk under its snapshot.
    StorePlaylistCache {
        id: String,
        snapshot: String,
        items: Vec<PlaylistItem>,
    },
    /// Resolve user ids to display names through the streaming session.
    UserNames(Vec<String>),
}

pub struct LyricsRequest {
    /// The track the answer is for, so a stale one is ignored.
    pub uri: String,
    pub query: crate::lyrics::Query,
}

pub enum Event {
    Auth(AuthStatus),
    Playback(LocalPlayback),
    /// Receivers seen on the local network that Spotify has not listed.
    Receivers(Vec<crate::zeroconf::Receiver>),
    ReceiverActivated {
        name: String,
        result: Result<(), String>,
    },
    Local(Box<LocalState>),
    Api(Box<ApiResponse>),
    Accent {
        url: String,
        color: [u8; 3],
    },
    Error(String),
    /// A newer release than this build exists.
    UpdateAvailable {
        version: String,
        url: String,
    },
    /// The words of a track, or `None` when nobody has transcribed it.
    Lyrics {
        uri: String,
        result: Result<Option<crate::lyrics::Lyrics>, String>,
    },
    /// The account's playlist tree, folders and all.
    Rootlist {
        result: Result<Vec<crate::player::RootlistEntry>, String>,
    },
    /// A playlist's items as last cached, with the snapshot they belong to.
    PlaylistCache {
        account_id: String,
        id: String,
        snapshot: String,
        items: Vec<PlaylistItem>,
    },
    /// A user id resolved to a display name (`None` when nothing answers).
    UserName {
        id: String,
        name: Option<String>,
    },
    /// The verified personal Web API app, or `None` when it is disabled.
    WebApp {
        client_id: Option<String>,
    },
}

/// The state of playback on this computer, independent of Web API sign-in.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalPlayback {
    /// Not authorized; local playback is unavailable but the app still works.
    Unavailable,
    /// The browser is open for the playback grant.
    Authorizing,
    /// Connecting the librespot engine.
    Connecting,
    /// This computer is a ready Spotify Connect device.
    Ready {
        device_id: String,
    },
    Failed(String),
}

/// Wakes whichever window currently exists, if any.
///
/// Background services (the runtime, MPRIS, the tray) outlive individual
/// windows: the window is destroyed when it closes to the tray and created
/// again on demand. They therefore hold this handle instead of an
/// `egui::Context`.
#[derive(Clone, Default)]
pub struct Waker(Arc<std::sync::Mutex<Option<egui::Context>>>);

impl Waker {
    pub fn attach(&self, ctx: &egui::Context) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(ctx.clone());
    }

    pub fn detach(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub fn wake(&self) {
        if let Some(ctx) = self.0.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            ctx.request_repaint();
        }
    }
}

/// The interface's handle to the runtime.
pub struct Backend {
    commands: mpsc::UnboundedSender<Command>,
    events: std::sync::mpsc::Receiver<Event>,
    art: ArtLoader,
    activity: Arc<NetActivity>,
    thread: Option<std::thread::JoinHandle<()>>,
    offline: bool,
}

impl Backend {
    pub fn spawn(
        dirs: AppDirs,
        engine_config: EngineConfig,
        web_client_id: Option<String>,
        waker: Waker,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("fastpotify-runtime")
            .enable_all()
            .build()
            .expect("unable to start the async runtime");
        let http = reqwest::Client::builder()
            .user_agent(concat!("fastpotify/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("unable to build the HTTP client");
        let art = ArtLoader::new(http.clone(), runtime.handle().clone(), dirs.art_cache_dir());
        let activity = Arc::new(NetActivity::default());

        let worker_activity = Arc::clone(&activity);
        let worker_art = art.clone();
        let worker_commands = command_tx.clone();
        let thread = std::thread::Builder::new()
            .name("fastpotify-backend".to_string())
            .spawn(move || {
                runtime.block_on(async move {
                    let mut worker = Worker::new(
                        dirs,
                        engine_config,
                        web_client_id,
                        http,
                        worker_art,
                        worker_activity,
                        event_tx,
                        worker_commands,
                        waker,
                    );
                    worker.run(command_rx).await;
                });
                // Give librespot's own threads a moment to release the audio device.
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .expect("unable to start the backend thread");

        Self {
            commands: command_tx,
            events: event_rx,
            art,
            activity,
            thread: Some(thread),
            offline: false,
        }
    }

    /// Live network activity, for the interface's busy indicator.
    pub fn activity(&self) -> &NetActivity {
        &self.activity
    }

    /// Stops Spotify-bound commands from leaving the process; artwork and
    /// shutdown still work. Used by the demo mode and by headless tests.
    #[cfg_attr(not(any(test, feature = "demo")), allow(dead_code))]
    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
    }

    pub fn send(&self, command: Command) {
        if self.offline && !matches!(command, Command::Accent { .. } | Command::Shutdown) {
            return;
        }
        let _ = self.commands.send(command);
    }

    pub fn api(&self, request: ApiRequest) {
        self.send(Command::Api(request));
    }

    pub fn player(&self, command: PlayerCommand) {
        self.send(Command::Player(command));
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub fn art(&self) -> &ArtLoader {
        &self.art
    }

    pub fn shutdown(&mut self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Worker {
    dirs: AppDirs,
    engine_config: EngineConfig,
    web_client_id: Option<String>,
    http: reqwest::Client,
    api: Arc<ApiGateway>,
    background_api: Arc<tokio::sync::Semaphore>,
    art: ArtLoader,
    events: std::sync::mpsc::Sender<Event>,
    commands: mpsc::UnboundedSender<Command>,
    waker: Waker,
    engine: Option<Arc<Engine>>,
    /// True while a playback grant or engine connection is in flight, so a
    /// second attempt does not pile up.
    engine_busy: bool,
    signed_in: bool,
    /// The plan, once the Web API has answered.
    premium: Option<bool>,
    cancel_signin: Option<watch::Sender<bool>>,
    authorizing_source: Option<ApiSource>,
    pending_authorization: Option<ApiSource>,
    reconnects: Vec<Instant>,
    /// What the engine was playing when it went down, to load again once
    /// the next one is up.
    resume: Option<LoadSpec>,
    /// A pickup in flight: the load to repeat and how often it was tried.
    resume_verify: Option<(LoadSpec, u8)>,
}

impl Worker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        dirs: AppDirs,
        engine_config: EngineConfig,
        web_client_id: Option<String>,
        http: reqwest::Client,
        art: ArtLoader,
        activity: Arc<NetActivity>,
        events: std::sync::mpsc::Sender<Event>,
        commands: mpsc::UnboundedSender<Command>,
        waker: Waker,
    ) -> Self {
        Self {
            dirs,
            engine_config,
            web_client_id,
            api: Arc::new(ApiGateway::new(http.clone(), activity)),
            background_api: Arc::new(tokio::sync::Semaphore::new(4)),
            http,
            art,
            events,
            commands,
            waker,
            engine: None,
            engine_busy: false,
            signed_in: false,
            premium: None,
            cancel_signin: None,
            authorizing_source: None,
            pending_authorization: None,
            reconnects: Vec::new(),
            resume: None,
            resume_verify: None,
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
        self.waker.wake();
    }

    async fn run(&mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        self.restore_session();
        while let Some(command) = commands.recv().await {
            match command {
                Command::Shutdown => break,
                Command::SignIn => self.sign_in(),
                Command::CancelSignIn => {
                    if let Some(cancel) = self.cancel_signin.take() {
                        let _ = cancel.send(true);
                    }
                    if let Some(source) = self.authorizing_source.take()
                        && matches!(self.api.state(source), SessionState::Authorizing)
                    {
                        self.api.clear(source);
                    }
                    self.pending_authorization = None;
                }
                Command::SignOut => self.sign_out(),
                Command::AuthorizePlayback => self.authorize_playback(),
                Command::RestartEngine(config) => {
                    self.engine_config = config;
                    self.reconnect_engine();
                }
                Command::Player(command) => match &self.engine {
                    Some(engine) => {
                        if let Err(error) = engine.command(command) {
                            self.emit(Event::Error(format!("Playback error: {error}")));
                        }
                    }
                    None => self.emit(Event::Error(
                        "Local playback isn't set up on this computer yet".into(),
                    )),
                },
                Command::Api(request) => self.dispatch(request),
                Command::Accent { url } => self.accent(url),
                Command::WebSignedIn { source, token } => {
                    if self.authorizing_source == Some(source) {
                        if let Err(error) = token.save(&self.token_path(source)) {
                            log::warn!("unable to save the Spotify sign-in: {error}");
                        }
                        self.on_web_signed_in(source, *token);
                    }
                }
                Command::WebVerified {
                    source,
                    token,
                    user,
                } => self.on_web_verified(source, *token, *user),
                Command::PlaybackAuthorized { access_token } => {
                    self.connect_engine(Credentials::with_access_token(access_token))
                }
                Command::EngineConnected { engine, error } => {
                    self.on_engine_connected(*engine, error)
                }
                Command::SignInEnded { source } => {
                    if self.authorizing_source == Some(source) {
                        self.cancel_signin = None;
                        self.authorizing_source = None;
                        if matches!(self.api.state(source), SessionState::Authorizing) {
                            self.api.clear(source);
                        }
                        if let Some(pending) = self.pending_authorization.take() {
                            self.sign_in_source(pending);
                        }
                    }
                }
                Command::PlaybackAuthEnded => {
                    self.cancel_signin = None;
                    if let Some(pending) = self.pending_authorization.take() {
                        self.sign_in_source(pending);
                    }
                }
                Command::AccountChecked { premium } => self.on_account_checked(premium),
                Command::Reconnect => self.reconnect_engine(),
                Command::DiscoverReceivers => self.discover_receivers(),
                Command::ActivateReceiver(receiver) => self.activate_receiver(*receiver),
                Command::CheckForUpdates => self.check_for_updates(),
                Command::Lyrics(request) => self.fetch_lyrics(*request),
                Command::Rootlist => self.fetch_rootlist(),
                Command::VerifyResume => self.verify_resume(),
                Command::LoadPlaylistCache { id } => self.load_playlist_cache(id),
                Command::StorePlaylistCache {
                    id,
                    snapshot,
                    items,
                } => self.store_playlist_cache(id, snapshot, items),
                Command::UserNames(ids) => self.fetch_user_names(ids),
                Command::ConfigurePersonalWebApp(client_id) => {
                    self.configure_personal_web_app(client_id)
                }
            }
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
    }

    // ---- Web API sign-in --------------------------------------------------

    fn restore_session(&mut self) {
        self.migrate_legacy_token();
        match crate::auth::StoredToken::load(&self.dirs.shared_web_token_file()) {
            Some(token) if token.has_scopes(crate::auth::WEB_SCOPES) => {
                self.emit(Event::Auth(AuthStatus::Connecting));
                self.on_web_signed_in(ApiSource::Shared, token);
            }
            Some(_) => self.emit(Event::Auth(AuthStatus::Failed(
                "Fastpotify needs one more Spotify permission. Please sign in again.".into(),
            ))),
            None => self.emit(Event::Auth(AuthStatus::SignedOut)),
        }
        let personal = self.web_client_id.as_deref().and_then(|client_id| {
            crate::auth::StoredToken::load(&self.dirs.personal_web_token_file())
                .filter(|token| token.client_id == client_id)
                .filter(|token| token.has_scopes(crate::auth::WEB_SCOPES))
        });
        if let Some(token) = personal {
            self.on_web_signed_in(ApiSource::Personal, token);
        }
    }

    fn migrate_legacy_token(&self) {
        if let Err(error) = crate::auth::StoredToken::migrate_legacy(
            &self.dirs.legacy_web_token_file(),
            &self.dirs.shared_web_token_file(),
            &self.dirs.personal_web_token_file(),
        ) {
            log::warn!("unable to migrate the previous Spotify sign-in: {error}");
        }
    }

    fn token_path(&self, source: ApiSource) -> std::path::PathBuf {
        match source {
            ApiSource::Shared => self.dirs.shared_web_token_file(),
            ApiSource::Personal => self.dirs.personal_web_token_file(),
        }
    }

    fn on_web_signed_in(&mut self, source: ApiSource, token: crate::auth::StoredToken) {
        let tokens = WebTokens::new(
            self.http.clone(),
            token.clone(),
            self.token_path(source),
            source,
        );
        self.api
            .begin_verification(source, TokenProvider::Web(tokens));
        let client = self.api.verification_client(source);
        let gateway = Arc::clone(&self.api);
        let commands = self.commands.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let mut wait = Duration::from_secs(2);
            let error = loop {
                match client.me().await {
                    Ok(user) => {
                        let _ = commands.send(Command::WebVerified {
                            source,
                            token: Box::new(token),
                            user: Box::new(user),
                        });
                        return;
                    }
                    Err(error @ ApiError::SignInExpired { .. }) => break error,
                    Err(error) if error.status().is_some_and(|status| status < 500) => break error,
                    Err(error) => {
                        log::warn!("Spotify sign-in verification will retry: {error}");
                        tokio::time::sleep(wait).await;
                        wait = (wait * 2).min(Duration::from_secs(60));
                        if !matches!(gateway.state(source), SessionState::Authorizing) {
                            return;
                        }
                    }
                }
            };
            gateway.clear(source);
            let message = match source {
                ApiSource::Shared => format!("Shared Spotify sign-in failed: {error}"),
                ApiSource::Personal => {
                    format!("Personal app authorization failed: {error}")
                }
            };
            let other_ready = match source {
                ApiSource::Shared => gateway.personal_ready(),
                ApiSource::Personal => {
                    matches!(gateway.state(ApiSource::Shared), SessionState::Ready { .. })
                }
            };
            if source == ApiSource::Shared || !other_ready {
                let _ = events.send(Event::Auth(AuthStatus::Failed(message.clone())));
            }
            let _ = events.send(Event::Error(message));
            let _ = commands.send(Command::SignInEnded { source });
            waker.wake();
        });
    }

    fn on_web_verified(&mut self, source: ApiSource, token: crate::auth::StoredToken, user: User) {
        if !matches!(self.api.state(source), SessionState::Authorizing)
            || source == ApiSource::Personal
                && self.web_client_id.as_deref() != Some(token.client_id.as_str())
        {
            return;
        }
        if let Err(error) = self.api.install(source, AccountId::new(user.id.clone())) {
            self.api.clear(source);
            if source == ApiSource::Shared {
                self.emit(Event::Auth(AuthStatus::Failed(error.to_string())));
            }
            self.emit(Event::Error(error.to_string()));
            self.finish_authorization(source);
            return;
        }
        match source {
            ApiSource::Shared => {
                self.signed_in = true;
                self.emit(Event::Auth(AuthStatus::Connected {
                    username: user.name().to_string(),
                }));
                self.emit(Event::Api(Box::new(ApiResponse::Me(Ok(user.clone())))));
                let premium = user.product.as_deref().map(|product| product == "premium");
                self.on_account_checked(premium);
            }
            ApiSource::Personal => {
                self.emit(Event::WebApp {
                    client_id: Some(token.client_id),
                });
            }
        }
        self.finish_authorization(source);
    }

    fn finish_authorization(&mut self, source: ApiSource) {
        if self.authorizing_source != Some(source) {
            return;
        }
        self.cancel_signin = None;
        self.authorizing_source = None;
        if let Some(pending) = self.pending_authorization.take() {
            self.sign_in_source(pending);
        }
    }

    fn sign_in(&mut self) {
        self.sign_in_source(ApiSource::Shared);
    }

    fn sign_in_source(&mut self, source: ApiSource) {
        if self.cancel_signin.is_some() {
            return;
        }
        let grant = match source {
            ApiSource::Shared => crate::auth::Grant::shared_web_api(),
            ApiSource::Personal => {
                let Some(client_id) = self.web_client_id.as_deref() else {
                    return;
                };
                match crate::auth::Grant::personal_web_api(client_id) {
                    Ok(grant) => grant,
                    Err(error) => {
                        self.emit(Event::Error(error.to_string()));
                        return;
                    }
                }
            }
        };
        let flow = crate::auth::begin(grant.clone());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.cancel_signin = Some(cancel_tx);
        self.authorizing_source = Some(source);
        self.api.set_state(source, SessionState::Authorizing);
        if source == ApiSource::Shared {
            self.emit(Event::Auth(AuthStatus::WaitingForBrowser {
                url: flow.url.clone(),
            }));
        }
        let browser_url = flow.url.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(error) = crate::opener::open(&browser_url) {
                log::warn!("unable to open a browser: {error}");
            }
        });
        let http = self.http.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = async {
                let code =
                    crate::auth::wait_for_code(grant.redirect_port, &flow.state, cancel_rx).await?;
                let response =
                    crate::auth::exchange_code(&http, &grant, &code, &flow.verifier).await?;
                crate::auth::StoredToken::from_response(&grant.client_id, response, None)
            }
            .await;
            match result {
                Ok(token) => {
                    let _ = commands.send(Command::WebSignedIn {
                        source,
                        token: Box::new(token),
                    });
                }
                Err(error) => {
                    if source == ApiSource::Shared {
                        let _ = events.send(Event::Auth(AuthStatus::SignedOut));
                    }
                    let message = error.to_string();
                    if !message.contains("cancelled") {
                        let _ = events.send(Event::Error(format!("Sign-in failed: {message}")));
                    }
                    waker.wake();
                    let _ = commands.send(Command::SignInEnded { source });
                }
            }
        });
    }

    fn configure_personal_web_app(&mut self, client_id: Option<String>) {
        let authorization_in_flight = if let Some(cancel) = self.cancel_signin.as_ref() {
            let _ = cancel.send(true);
            true
        } else {
            false
        };
        self.web_client_id = client_id;
        self.api.clear(ApiSource::Personal);
        if self.web_client_id.is_none() {
            crate::auth::StoredToken::remove(&self.dirs.personal_web_token_file());
        }
        self.emit(Event::WebApp { client_id: None });
        if self.web_client_id.is_some() {
            if authorization_in_flight {
                self.pending_authorization = Some(ApiSource::Personal);
            } else {
                self.sign_in_source(ApiSource::Personal);
            }
        } else {
            self.pending_authorization = None;
        }
    }

    fn sign_out(&mut self) {
        self.signed_in = false;
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
        if let Some(cancel) = self.cancel_signin.take() {
            let _ = cancel.send(true);
        }
        self.authorizing_source = None;
        self.pending_authorization = None;
        self.api.clear_all();
        crate::auth::StoredToken::remove(&self.dirs.shared_web_token_file());
        crate::auth::StoredToken::remove(&self.dirs.personal_web_token_file());
        crate::auth::StoredToken::remove(&self.dirs.legacy_web_token_file());
        let _ = std::fs::remove_file(self.dirs.credentials_dir().join("credentials.json"));
        self.emit(Event::Playback(LocalPlayback::Unavailable));
        self.emit(Event::Auth(AuthStatus::SignedOut));
    }

    // ---- local playback engine -------------------------------------------

    fn engine_notify(&self) -> crate::player::Notify {
        let events = self.events.clone();
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        Arc::new(move |event| match event {
            EngineEvent::State(state) => {
                let _ = events.send(Event::Local(Box::new(state)));
                waker.wake();
            }
            EngineEvent::SessionEnded => {
                let _ = commands.send(Command::Reconnect);
            }
        })
    }

    /// Bring the engine up from a credential stored by a previous playback
    /// authorization, if there is one. Silent when there is nothing to resume.
    fn resume_engine(&mut self) {
        if self.engine.is_some() || self.engine_busy || self.premium == Some(false) {
            return;
        }
        let credentials = self
            .engine_config
            .open_cache()
            .ok()
            .and_then(|cache| cache.credentials());
        if let Some(credentials) = credentials {
            self.connect_engine(credentials);
        }
    }

    /// Reconnect the engine after its session dropped or audio settings
    /// changed. Whatever was playing comes back at the same spot on the new
    /// session, so a dropped connection is a pause of a few seconds rather
    /// than silence.
    fn reconnect_engine(&mut self) {
        if !self.signed_in {
            return;
        }
        self.resume_verify = None;
        if let Some(engine) = self.engine.take() {
            self.resume = engine.interrupted().map(|interrupted| LoadSpec {
                uris: vec![interrupted.uri],
                position_ms: interrupted.position_ms,
                play: interrupted.playing,
                ..LoadSpec::default()
            });
            engine.shutdown();
        }
        let now = Instant::now();
        self.reconnects
            .retain(|attempt| now.duration_since(*attempt) < Duration::from_secs(600));
        if self.reconnects.len() >= 6 {
            self.resume = None;
            self.emit(Event::Playback(LocalPlayback::Failed(
                "Local playback keeps dropping. Re-enable it from Settings.".into(),
            )));
            return;
        }
        self.reconnects.push(now);
        log::info!(
            "local playback session ended; reconnecting ({} of 6 in ten minutes)",
            self.reconnects.len()
        );
        self.resume_engine();
    }

    /// Start (or re-enter) the playback authorization in the browser. This is
    /// a distinct grant from the Web API sign-in: it uses Spotify's streaming
    /// client identity, the one librespot can play with.
    fn authorize_playback(&mut self) {
        if self.engine_busy || self.cancel_signin.is_some() {
            return;
        }
        if self.premium == Some(false) {
            self.emit(Event::Playback(LocalPlayback::Failed(
                PREMIUM_NEEDED.into(),
            )));
            return;
        }
        let grant = crate::auth::Grant::playback();
        let flow = crate::auth::begin(grant.clone());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.cancel_signin = Some(cancel_tx);
        self.emit(Event::Playback(LocalPlayback::Authorizing));
        let browser_url = flow.url.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(error) = crate::opener::open(&browser_url) {
                log::warn!("unable to open a browser: {error}");
            }
        });
        let http = self.http.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = async {
                let code =
                    crate::auth::wait_for_code(grant.redirect_port, &flow.state, cancel_rx).await?;
                crate::auth::exchange_code(&http, &grant, &code, &flow.verifier).await
            }
            .await;
            match result {
                Ok(token) => {
                    let _ = commands.send(Command::PlaybackAuthorized {
                        access_token: token.access_token,
                    });
                }
                Err(error) => {
                    let message = error.to_string();
                    if message.contains("cancelled") {
                        let _ = events.send(Event::Playback(LocalPlayback::Unavailable));
                    } else {
                        let _ = events.send(Event::Playback(LocalPlayback::Failed(message)));
                    }
                    waker.wake();
                    let _ = commands.send(Command::PlaybackAuthEnded);
                }
            }
        });
    }

    /// Spawn an engine connection so a slow or hung librespot handshake can
    /// never block the command loop (this was the cause of the app freezing
    /// on "Connecting to Spotify"). `Engine::connect` stores the reusable
    /// credential itself, so authorizing once is enough.
    fn connect_engine(&mut self, credentials: Credentials) {
        if self.engine_busy {
            return;
        }
        if self.premium == Some(false) {
            self.emit(Event::Playback(LocalPlayback::Failed(
                PREMIUM_NEEDED.into(),
            )));
            return;
        }
        self.cancel_signin = None;
        self.engine_busy = true;
        self.emit(Event::Playback(LocalPlayback::Connecting));
        let config = self.engine_config.clone();
        let notify = self.engine_notify();
        let events = self.events.clone();
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let cache = match config.open_cache() {
                Ok(cache) => cache,
                Err(error) => {
                    let _ = commands.send(Command::EngineConnected {
                        engine: Box::new(None),
                        error: Some(error.to_string()),
                    });
                    return;
                }
            };
            let attempt = tokio::time::timeout(
                Duration::from_secs(45),
                Engine::connect(&config, credentials, cache, notify),
            )
            .await;
            let outcome = match attempt {
                Ok(Ok(engine)) => Command::EngineConnected {
                    engine: Box::new(Some(engine)),
                    error: None,
                },
                Ok(Err(error)) => {
                    log::error!("engine connect failed: {error:#}");
                    Command::EngineConnected {
                        engine: Box::new(None),
                        error: Some(friendly_connect_error(&error)),
                    }
                }
                Err(_) => Command::EngineConnected {
                    engine: Box::new(None),
                    error: Some("Connecting to Spotify timed out".into()),
                },
            };
            let _ = commands.send(outcome);
            let _ = events;
            waker.wake();
        });
    }

    fn on_engine_connected(&mut self, engine: Option<Engine>, error: Option<String>) {
        self.engine_busy = false;
        match engine {
            Some(engine) => {
                let device_id = engine.device_id().to_string();
                let engine = Arc::new(engine);
                if let Some(spec) = self.resume.take() {
                    // Not right away: a load fired into a session spirc is
                    // still registering came back 400 once, and the player
                    // sat stopped until a hand moved. The check below fires
                    // the load, sees whether anything is playing a few
                    // seconds later, and tries again when it is not.
                    self.resume_verify = Some((spec, 0));
                    self.schedule_resume_check(1_500);
                }
                self.engine = Some(engine);
                self.reconnects.clear();
                self.emit(Event::Playback(LocalPlayback::Ready { device_id }));
            }
            None => {
                self.resume = None;
                let message = error.unwrap_or_else(|| "Local playback is unavailable".into());
                self.emit(Event::Playback(LocalPlayback::Failed(message)));
            }
        }
    }

    /// The plan gates the engine because librespot 0.8 calls `exit(1)` from
    /// inside its session the moment Spotify tells it the account is not
    /// Premium; no error path of ours can catch that, so a Free account must
    /// never reach it. When the API cannot say, the engine comes back as it
    /// always did.
    fn on_account_checked(&mut self, premium: Option<bool>) {
        self.premium = premium;
        if premium == Some(false) {
            if let Some(engine) = self.engine.take() {
                engine.shutdown();
            }
            let credential_stored = self
                .engine_config
                .open_cache()
                .ok()
                .and_then(|cache| cache.credentials())
                .is_some();
            if credential_stored {
                self.emit(Event::Playback(LocalPlayback::Failed(
                    PREMIUM_NEEDED.into(),
                )));
            }
            return;
        }
        self.resume_engine();
    }

    // ---- receivers on the local network -----------------------------------

    /// Browses for receivers Spotify's device list does not know about. The
    /// browse blocks, so it runs off the runtime's worker threads.
    fn discover_receivers(&self) {
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::task::spawn_blocking(move || {
            match crate::zeroconf::discover(std::time::Duration::from_secs(3)) {
                Ok(receivers) => {
                    let _ = events.send(Event::Receivers(receivers));
                    waker.wake();
                }
                Err(error) => log::debug!("no receivers found on the network: {error}"),
            }
        });
    }

    /// Hands the stored playback credential to a receiver, which makes it log
    /// in and appear in the ordinary device list.
    fn activate_receiver(&self, receiver: crate::zeroconf::Receiver) {
        let events = self.events.clone();
        let waker = self.waker.clone();
        let credentials_dir = self.dirs.credentials_dir();
        tokio::task::spawn_blocking(move || {
            let name = receiver.name.clone();
            let result = (|| -> Result<(), String> {
                let credentials = crate::zeroconf::Credentials::load(&credentials_dir)
                    .map_err(|_| {
                        "Enable playback on this computer first, so there is an account to hand over"
                            .to_string()
                    })?;
                let http = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(8))
                    .build()
                    .map_err(|error| error.to_string())?;
                let info = crate::zeroconf::get_info(&http, &receiver)
                    .map_err(|error| error.to_string())?;
                crate::zeroconf::add_user(&http, &receiver, &info, &credentials, "Fastpotify")
                    .map_err(|error| error.to_string())
            })();
            let _ = events.send(Event::ReceiverActivated { name, result });
            waker.wake();
        });
    }

    fn check_for_updates(&self) {
        let http = self.http.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            match crate::updates::newer_release(&http).await {
                Ok(Some(release)) => {
                    let _ = events.send(Event::UpdateAvailable {
                        version: release.version,
                        url: release.url,
                    });
                    waker.wake();
                }
                Ok(None) => log::debug!("this is the newest release"),
                Err(error) => log::debug!("could not check for a newer release: {error:#}"),
            }
        });
    }

    /// A moment after a reconnect's pickup, look whether anything is
    /// actually playing; Spotify refused such a load once (a 400 from a
    /// spirc still settling in) and nothing asked twice. Runs on the
    /// backend's own clock, so no window needs to be awake.
    fn verify_resume(&mut self) {
        let Some((spec, attempts)) = self.resume_verify.take() else {
            return;
        };
        let Some(engine) = &self.engine else {
            return;
        };
        if engine.interrupted().is_some() {
            // Something is loaded and not stopped: the pickup took, or the
            // listener started something else. Either way, done.
            return;
        }
        if attempts >= 3 {
            log::warn!("gave up picking playback up again after {attempts} tries");
            return;
        }
        log::info!(
            "picking {} up again at {} ms on the new session (try {})",
            spec.uris.join(" "),
            spec.position_ms,
            attempts + 1
        );
        if let Err(error) = engine.command(PlayerCommand::Load(spec.clone())) {
            log::warn!("unable to pick playback up again: {error}");
        }
        self.resume_verify = Some((spec, attempts + 1));
        self.schedule_resume_check(4_000);
    }

    fn schedule_resume_check(&self, delay_ms: u64) {
        let commands = self.commands.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            let _ = commands.send(Command::VerifyResume);
        });
    }

    fn fetch_rootlist(&self) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let result = engine
                .rootlist()
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = events.send(Event::Rootlist { result });
            waker.wake();
        });
    }

    fn fetch_lyrics(&self, request: LyricsRequest) {
        let http = self.http.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        let cache_dir = self.dirs.lyrics_cache_dir();
        let engine = self.engine.clone();
        tokio::spawn(async move {
            // Spotify's own words go first: they follow the recording
            // exactly. Everything else, a signed-out session included,
            // falls back to LRCLIB.
            let result = match spotify_lyrics(engine, &request.uri, &cache_dir).await {
                Some(found) => Ok(Some(found)),
                None => crate::lyrics::fetch(&http, &cache_dir, &request.query)
                    .await
                    .map_err(|error| format!("{error:#}")),
            };
            let _ = events.send(Event::Lyrics {
                uri: request.uri,
                result,
            });
            waker.wake();
        });
    }

    /// Hand the interface a playlist's cached items, if any are on disk.
    /// Whether they are still true is the interface's call: it compares
    /// the snapshot against the live playlist before adopting them.
    fn load_playlist_cache(&self, id: String) {
        let Some(account) = self.api.account() else {
            return;
        };
        let events = self.events.clone();
        let waker = self.waker.clone();
        let path = self
            .dirs
            .account_playlist_cache_dir(account.as_str())
            .join(format!("{id}.json"));
        let account_id = account.as_str().to_string();
        tokio::spawn(async move {
            let Ok(text) = tokio::fs::read_to_string(&path).await else {
                return;
            };
            let Ok(cached) = serde_json::from_str::<CachedPlaylist>(&text) else {
                return;
            };
            let _ = events.send(Event::PlaylistCache {
                account_id,
                id,
                snapshot: cached.snapshot,
                items: cached.items,
            });
            waker.wake();
        });
    }

    fn store_playlist_cache(&self, id: String, snapshot: String, items: Vec<PlaylistItem>) {
        let Some(account) = self.api.account() else {
            return;
        };
        let path = self
            .dirs
            .account_playlist_cache_dir(account.as_str())
            .join(format!("{id}.json"));
        tokio::spawn(async move {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Ok(text) = serde_json::to_string(&CachedPlaylist { snapshot, items }) {
                let temporary = path.with_extension("json.tmp");
                if tokio::fs::write(&temporary, text).await.is_ok() {
                    let _ = tokio::fs::rename(temporary, path).await;
                }
            }
        });
    }

    /// Ask Spotify who is behind each user id. Only the streaming session
    /// can ask; without one the interface shows the bare ids.
    fn fetch_user_names(&self, ids: Vec<String>) {
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            for id in ids {
                let name = engine.user_display_name(&id).await;
                let _ = events.send(Event::UserName { id, name });
                waker.wake();
            }
        });
    }

    // ---- api ----------------------------------------------------------------

    fn dispatch(&self, request: ApiRequest) {
        let api = Arc::clone(&self.api);
        let background_api = Arc::clone(&self.background_api);
        let background = request.background();
        let events = self.events.clone();
        let waker = self.waker.clone();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let _background_permit = if background {
                background_api.acquire_owned().await.ok()
            } else {
                None
            };
            let (response, expired) = handle(&api, request).await;
            if let Some(api_source) = expired {
                api.clear(api_source);
                if api_source == ApiSource::Personal {
                    let _ = events.send(Event::WebApp { client_id: None });
                } else {
                    let _ = events.send(Event::Auth(AuthStatus::Failed(
                        "Your Spotify sign-in expired. Please sign in again.".into(),
                    )));
                }
            }
            if let ApiResponse::Me(result) = &response {
                let premium = result
                    .as_ref()
                    .ok()
                    .and_then(|user| user.product.as_deref())
                    .map(|product| product == "premium");
                let _ = commands.send(Command::AccountChecked { premium });
            }
            let _ = events.send(Event::Api(Box::new(response)));
            waker.wake();
        });
    }

    fn accent(&self, url: String) {
        let art = self.art.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            if let Ok(bytes) = art.fetch(&url).await {
                let color = tokio::task::spawn_blocking(move || accent_color(&bytes))
                    .await
                    .ok()
                    .flatten();
                if let Some(color) = color {
                    let _ = events.send(Event::Accent { url, color });
                    waker.wake();
                }
            }
        });
    }
}

fn friendly_connect_error(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    let lower = text.to_lowercase();
    if lower.contains("badcredentials") || lower.contains("bad credentials") {
        "Spotify rejected the saved sign-in. Please sign in again.".to_string()
    } else if lower.contains("premium") {
        PREMIUM_NEEDED.to_string()
    } else if lower.contains("dns") || lower.contains("connect") || lower.contains("resolve") {
        format!("Couldn't reach Spotify: {text}")
    } else {
        text
    }
}

fn operation_for(api: &ApiGateway, request: &ApiRequest) -> Operation {
    match request {
        ApiRequest::Me => Operation::CanonicalAccount,
        ApiRequest::Devices
        | ApiRequest::PlaybackState { .. }
        | ApiRequest::Queue { .. }
        | ApiRequest::Remote { .. }
        | ApiRequest::Transfer { .. }
        | ApiRequest::ShufflePlay { .. }
        | ApiRequest::AddToQueue { .. } => Operation::Playback,
        ApiRequest::RecentlyPlayed { .. }
        | ApiRequest::TopTracks { .. }
        | ApiRequest::TopArtists { .. }
        | ApiRequest::SavedTracks { .. }
        | ApiRequest::SavedAlbums { .. }
        | ApiRequest::FollowedArtists { .. }
        | ApiRequest::SavedShows { .. }
        | ApiRequest::SavedEpisodes { .. }
        | ApiRequest::SetSaved { .. } => Operation::UserData,
        // Development Mode cannot answer membership for playlists it omits.
        ApiRequest::Contains { uris } => {
            if uris.iter().any(|uri| uri.starts_with("spotify:playlist:")) {
                Operation::UnsupportedDevelopmentMode
            } else {
                Operation::UserData
            }
        }
        ApiRequest::MyPlaylists { .. } => Operation::PlaylistLibrary,
        ApiRequest::CreatePlaylist { .. } => Operation::PlaylistCreation,
        ApiRequest::Discover { .. } | ApiRequest::Search { .. } => Operation::PlaylistSearch,
        ApiRequest::Playlist { id, .. } => Operation::PlaylistMetadata(api.playlist_access(id)),
        ApiRequest::PlaylistItems { id, .. } | ApiRequest::PlaylistSample { id, .. } => {
            Operation::PlaylistItems(api.playlist_access(id))
        }
        ApiRequest::UpdatePlaylist { id, .. } | ApiRequest::FollowPlaylist { id, .. } => {
            Operation::PlaylistMutation(api.playlist_access(id))
        }
        ApiRequest::AddToPlaylist { playlist_id, .. }
        | ApiRequest::RemoveFromPlaylist { playlist_id, .. }
        | ApiRequest::ReorderPlaylist { playlist_id, .. } => {
            Operation::PlaylistMutation(api.playlist_access(playlist_id))
        }
        ApiRequest::Recommendations { .. }
        | ApiRequest::ArtistTopTracks { .. }
        | ApiRequest::RelatedArtists { .. } => Operation::UnsupportedDevelopmentMode,
        ApiRequest::Artist { .. }
        | ApiRequest::ArtistAlbums { .. }
        | ApiRequest::Album { .. }
        | ApiRequest::AlbumTracks { .. }
        | ApiRequest::Show { .. }
        | ApiRequest::ShowEpisodes { .. }
        | ApiRequest::Track { .. } => Operation::Catalog,
    }
}

fn observe_playlists(api: &ApiGateway, response: &ApiResponse) {
    match response {
        ApiResponse::Discover {
            result: Ok(playlists),
            ..
        } => api.observe_playlists(playlists),
        ApiResponse::MyPlaylists {
            result: Ok(page), ..
        } => api.observe_playlists(&page.items),
        ApiResponse::Playlist {
            result: Ok(playlist),
            ..
        }
        | ApiResponse::PlaylistCreated(Ok(playlist)) => api.observe_playlist(playlist),
        ApiResponse::Search {
            result: Ok(results),
            ..
        } => {
            if let Some(playlists) = &results.playlists {
                api.observe_playlists(&playlists.items);
            }
        }
        ApiResponse::PlaylistUpdated {
            id,
            result: Err(error),
        }
        | ApiResponse::PlaylistItemsChanged {
            id,
            result: Err(error),
            ..
        }
        | ApiResponse::PlaylistItems {
            id,
            result: Err(error),
            ..
        }
        | ApiResponse::PlaylistSample {
            id,
            result: Err(error),
            ..
        }
        | ApiResponse::PlaylistFollowChanged {
            id,
            result: Err(error),
            ..
        } if error.status() == Some(403) => {
            api.invalidate_playlist_access(&PlaylistId::new(id.clone()));
        }
        _ => {}
    }
}

async fn handle(api: &ApiGateway, request: ApiRequest) -> (ApiResponse, Option<ApiSource>) {
    let selected = api.client_for(operation_for(api, &request));
    let expired = std::cell::Cell::new(None);
    macro_rules! routed {
        ($method:ident($($argument:expr),* $(,)?)) => {{
            let result = match &selected {
                Ok(client) => client.$method($($argument),*).await,
                Err(error) => Err(error.clone()),
            };
            if let Err(ApiError::SignInExpired { api_source }) = &result {
                expired.set(Some(*api_source));
            }
            result
        }};
    }

    let response = match request {
        ApiRequest::Me => ApiResponse::Me(routed!(me())),
        ApiRequest::Devices => ApiResponse::Devices(routed!(devices())),
        ApiRequest::PlaybackState { seq } => ApiResponse::PlaybackState {
            seq,
            result: routed!(playback_state()),
        },
        ApiRequest::Queue { seq } => ApiResponse::Queue {
            seq,
            result: routed!(queue()),
        },
        ApiRequest::RecentlyPlayed {
            who,
            generation,
            before,
            limit,
        } => ApiResponse::RecentlyPlayed {
            who,
            generation,
            limit,
            result: routed!(recently_played(limit, None, before.as_deref())),
        },
        ApiRequest::TopTracks {
            offset,
            full,
            generation,
        } => ApiResponse::TopTracks {
            result: routed!(top_tracks("short_term", if full { 50 } else { 20 }, offset)),
            offset,
            full,
            generation,
        },
        ApiRequest::TopArtists { generation } => ApiResponse::TopArtists {
            generation,
            result: routed!(top_artists("medium_term", 20)).map(|page| page.items),
        },
        ApiRequest::Recommendations {
            seed_tracks,
            seed_artists,
            generation,
        } => ApiResponse::Recommendations {
            generation,
            result: routed!(recommendations(&seed_tracks, &seed_artists, 20)),
        },
        ApiRequest::Discover { term, generation } => {
            let result = routed!(search(&term, &["playlist"]))
                .map(|results| results.playlists.map(|page| page.items).unwrap_or_default());
            ApiResponse::Discover {
                term,
                generation,
                result,
            }
        }
        ApiRequest::MyPlaylists { offset } => ApiResponse::MyPlaylists {
            offset,
            result: routed!(my_playlists(offset, 50)),
        },
        ApiRequest::Playlist { id, generation } => ApiResponse::Playlist {
            result: routed!(playlist(&id)),
            id,
            generation,
        },
        ApiRequest::PlaylistItems {
            id,
            offset,
            generation,
        } => ApiResponse::PlaylistItems {
            result: routed!(playlist_items(&id, offset, PLAYLIST_PAGE_SIZE)),
            id,
            offset,
            generation,
        },
        ApiRequest::PlaylistSample {
            id,
            offset,
            generation,
        } => ApiResponse::PlaylistSample {
            result: routed!(playlist_items(&id, offset, PLAYLIST_PAGE_SIZE)),
            id,
            generation,
        },
        ApiRequest::CreatePlaylist {
            name,
            public,
            description,
        } => ApiResponse::PlaylistCreated(routed!(create_playlist(&name, public, &description))),
        ApiRequest::UpdatePlaylist {
            id,
            name,
            description,
            public,
        } => ApiResponse::PlaylistUpdated {
            result: routed!(update_playlist(
                &id,
                name.as_deref(),
                description.as_deref(),
                public
            )),
            id,
        },
        ApiRequest::AddToPlaylist {
            playlist_id,
            playlist_name,
            uris,
        } => ApiResponse::PlaylistItemsChanged {
            result: routed!(add_playlist_items(&playlist_id, &uris, None)),
            id: playlist_id,
            message: format!("Added to {playlist_name}"),
        },
        ApiRequest::RemoveFromPlaylist {
            playlist_id,
            uris,
            snapshot_id,
        } => ApiResponse::PlaylistItemsChanged {
            result: routed!(remove_playlist_items(
                &playlist_id,
                &uris,
                snapshot_id.as_deref()
            )),
            id: playlist_id,
            message: "Removed from playlist".to_string(),
        },
        ApiRequest::ReorderPlaylist {
            playlist_id,
            range_start,
            insert_before,
            snapshot_id,
        } => ApiResponse::PlaylistItemsChanged {
            result: routed!(reorder_playlist(
                &playlist_id,
                range_start,
                insert_before,
                snapshot_id.as_deref()
            )),
            id: playlist_id,
            message: String::new(),
        },
        ApiRequest::FollowPlaylist { id, follow } => ApiResponse::PlaylistFollowChanged {
            result: if follow {
                routed!(follow_playlist(&id))
            } else {
                routed!(unfollow_playlist(&id))
            },
            id,
            followed: follow,
        },
        ApiRequest::SavedTracks { offset } => ApiResponse::SavedTracks {
            offset,
            result: routed!(saved_tracks(offset, 50)),
        },
        ApiRequest::SavedAlbums { offset } => ApiResponse::SavedAlbums {
            offset,
            result: routed!(saved_albums(offset, 50)),
        },
        ApiRequest::FollowedArtists { after } => ApiResponse::FollowedArtists {
            result: routed!(followed_artists(after.as_deref(), 50)),
            after,
        },
        ApiRequest::SavedShows { offset } => ApiResponse::SavedShows {
            offset,
            result: routed!(saved_shows(offset, 50)),
        },
        ApiRequest::SavedEpisodes { offset } => ApiResponse::SavedEpisodes {
            offset,
            result: routed!(saved_episodes(offset, 50)),
        },
        ApiRequest::SetSaved { uris, saved } => ApiResponse::SavedChanged {
            result: if saved {
                routed!(save(&uris))
            } else {
                routed!(unsave(&uris))
            },
            uris,
            saved,
        },
        ApiRequest::Contains { uris } => ApiResponse::Contains {
            result: routed!(contains(&uris)),
            uris,
        },
        ApiRequest::Search { query, serial } => ApiResponse::Search {
            result: routed!(search(
                &query,
                &["track", "artist", "album", "playlist", "show", "episode"]
            )),
            query,
            serial,
        },
        ApiRequest::Artist { id } => ApiResponse::Artist {
            result: routed!(artist(&id)),
            id,
        },
        ApiRequest::ArtistTopTracks { id } => ApiResponse::ArtistTopTracks {
            result: routed!(artist_top_tracks(&id)),
            id,
        },
        ApiRequest::ArtistAlbums { id, groups, offset } => ApiResponse::ArtistAlbums {
            result: routed!(artist_albums(&id, &groups, offset, 50)),
            id,
            groups,
            offset,
        },
        ApiRequest::RelatedArtists { id } => ApiResponse::RelatedArtists {
            result: routed!(related_artists(&id)),
            id,
        },
        ApiRequest::Album { id } => ApiResponse::Album {
            result: routed!(album(&id)),
            id,
        },
        ApiRequest::AlbumTracks { id, offset } => ApiResponse::AlbumTracks {
            result: routed!(album_tracks(&id, offset, 50)),
            id,
            offset,
        },
        ApiRequest::Show { id } => ApiResponse::Show {
            result: routed!(show(&id)),
            id,
        },
        ApiRequest::ShowEpisodes { id, offset } => ApiResponse::ShowEpisodes {
            result: routed!(show_episodes(&id, offset, 50)),
            id,
            offset,
        },
        ApiRequest::Track { id } => ApiResponse::Track {
            result: routed!(track(&id)),
            id,
        },
        ApiRequest::Remote {
            action,
            device_id,
            play,
            position_ms,
            percent,
            flag,
            repeat,
        } => {
            let device = device_id.as_deref();
            let result = match action {
                RemoteAction::Play => routed!(play(device, play.as_ref())),
                RemoteAction::Pause => routed!(pause(device)),
                RemoteAction::Next => routed!(next(device)),
                RemoteAction::Previous => routed!(previous(device)),
                RemoteAction::Seek => routed!(seek(position_ms, device)),
                RemoteAction::Volume => routed!(set_volume(percent, device)),
                RemoteAction::Shuffle => routed!(set_shuffle(flag, device)),
                RemoteAction::Repeat => routed!(set_repeat(&repeat, device)),
            };
            ApiResponse::Remote { action, result }
        }
        ApiRequest::ShufflePlay { device_id, play } => {
            let device = device_id.as_deref();
            let result = match routed!(set_shuffle(true, device)) {
                Ok(()) => routed!(play(device, Some(&play))),
                Err(error) => Err(error),
            };
            ApiResponse::Remote {
                action: RemoteAction::Play,
                result,
            }
        }
        ApiRequest::Transfer { device_id, play } => ApiResponse::Transferred {
            result: routed!(transfer(&device_id, play)),
            device_id,
        },
        ApiRequest::AddToQueue {
            uri,
            device_id,
            label,
        } => ApiResponse::QueueAdded {
            result: routed!(add_to_queue(&uri, device_id.as_deref())),
            label,
        },
    };
    observe_playlists(api, &response);
    (response, expired.get())
}

/// Spotify's transcription of the track, when the local session can ask for
/// one. Answers are cached like LRCLIB's, "none" included; `None` falls
/// back to LRCLIB.
async fn spotify_lyrics(
    engine: Option<Arc<Engine>>,
    uri: &str,
    cache_dir: &std::path::Path,
) -> Option<crate::lyrics::Lyrics> {
    let id = uri.strip_prefix("spotify:track:")?;
    let path = cache_dir.join(format!("spotify-{id}.json"));
    if let Some(cached) = crate::lyrics::cached(&path) {
        return cached;
    }
    match engine?.lyrics_json(uri).await {
        Ok(json) => {
            let found = json.as_ref().and_then(crate::lyrics::from_spotify);
            crate::lyrics::store(&path, &found);
            found
        }
        Err(error) => {
            log::debug!("spotify lyrics unavailable: {error:#}");
            None
        }
    }
}

/// A playlist's items on disk, valid for exactly one snapshot.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedPlaylist {
    snapshot: String,
    items: Vec<PlaylistItem>,
}

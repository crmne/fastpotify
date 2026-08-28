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
use crate::api::{ApiClient, ApiError, NetActivity, PlayRequest, TokenProvider, WebTokens};
use crate::images::{ArtLoader, accent_color};
use crate::paths::AppDirs;
use crate::player::{Engine, EngineConfig, EngineEvent, LocalState, PlayerCommand};

pub type ApiResult<T> = Result<T, ApiError>;

const PREMIUM_NEEDED: &str = "Local playback needs Spotify Premium.";

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

#[derive(Clone, Debug)]
pub enum ApiRequest {
    Me,
    Devices,
    PlaybackState,
    Queue,
    RecentlyPlayed,
    TopTracks {
        offset: u32,
        full: bool,
    },
    TopArtists,
    Recommendations {
        seed_tracks: Vec<String>,
        seed_artists: Vec<String>,
    },
    Discover {
        term: String,
    },
    MyPlaylists {
        offset: u32,
    },
    Playlist {
        id: String,
    },
    PlaylistItems {
        id: String,
        offset: u32,
    },
    CreatePlaylist {
        user_id: String,
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
        user_id: String,
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
        name: String,
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
    AddToQueue {
        uri: String,
        device_id: Option<String>,
        label: String,
    },
}

#[derive(Debug)]
pub enum ApiResponse {
    Me(ApiResult<User>),
    Devices(ApiResult<Vec<Device>>),
    PlaybackState(ApiResult<Option<PlaybackState>>),
    Queue(ApiResult<Queue>),
    RecentlyPlayed(ApiResult<Vec<PlayHistory>>),
    TopTracks {
        offset: u32,
        full: bool,
        result: ApiResult<Page<Track>>,
    },
    TopArtists(ApiResult<Vec<Artist>>),
    Recommendations(ApiResult<Vec<Track>>),
    Discover {
        term: String,
        result: ApiResult<Vec<Playlist>>,
    },
    MyPlaylists {
        offset: u32,
        result: ApiResult<Page<Playlist>>,
    },
    Playlist {
        id: String,
        result: ApiResult<Playlist>,
    },
    PlaylistItems {
        id: String,
        offset: u32,
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
    WebSignedIn(Box<crate::auth::StoredToken>),
    /// Internal: a browser flow ended (success or not).
    SignInEnded,
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
    /// Sign in again with another Web API application (`None` for the
    /// shared one). Local playback keeps its own grant.
    SwitchWebApp(Option<String>),
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
    /// The Web API application the current sign-in belongs to.
    WebApp {
        client_id: String,
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
    api: Arc<ApiClient>,
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
    reconnects: Vec<Instant>,
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
            api: Arc::new(ApiClient::new(http.clone(), activity)),
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
            reconnects: Vec::new(),
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
                Command::WebSignedIn(token) => self.on_web_signed_in(*token),
                Command::PlaybackAuthorized { access_token } => {
                    self.connect_engine(Credentials::with_access_token(access_token))
                }
                Command::EngineConnected { engine, error } => {
                    self.on_engine_connected(*engine, error)
                }
                Command::SignInEnded => self.cancel_signin = None,
                Command::AccountChecked { premium } => self.on_account_checked(premium),
                Command::Reconnect => self.reconnect_engine(),
                Command::DiscoverReceivers => self.discover_receivers(),
                Command::ActivateReceiver(receiver) => self.activate_receiver(*receiver),
                Command::CheckForUpdates => self.check_for_updates(),
                Command::Lyrics(request) => self.fetch_lyrics(*request),
                Command::SwitchWebApp(client_id) => self.switch_web_app(client_id),
            }
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
    }

    // ---- Web API sign-in --------------------------------------------------

    /// On startup, resume a saved Web API grant. The local engine follows
    /// once the plan is known (`on_account_checked`), never before.
    fn restore_session(&mut self) {
        match crate::auth::StoredToken::load(&self.dirs.web_token_file()) {
            Some(token) if !token.has_scopes(crate::auth::WEB_SCOPES) => {
                // Granted before a scope this version relies on; only the
                // browser can widen it.
                self.emit(Event::Auth(AuthStatus::Failed(
                    "Fastpotify needs one more Spotify permission. Please sign in again.".into(),
                )));
            }
            Some(token) => {
                self.activate_web_token(token);
                self.emit(Event::Auth(AuthStatus::Connecting));
                self.dispatch(ApiRequest::Me);
                self.signed_in = true;
                self.emit(Event::Auth(AuthStatus::Connected {
                    username: String::new(),
                }));
            }
            None => self.emit(Event::Auth(AuthStatus::SignedOut)),
        }
    }

    fn activate_web_token(&self, token: crate::auth::StoredToken) {
        self.emit(Event::WebApp {
            client_id: token.client_id.clone(),
        });
        let tokens = WebTokens::new(self.http.clone(), token, self.dirs.web_token_file());
        self.api
            .set_token_provider(Some(TokenProvider::Web(tokens)));
    }

    fn on_web_signed_in(&mut self, token: crate::auth::StoredToken) {
        self.cancel_signin = None;
        if let Err(error) = token.save(&self.dirs.web_token_file()) {
            log::warn!("unable to save the Spotify sign-in: {error}");
        }
        self.activate_web_token(token);
        self.signed_in = true;
        self.emit(Event::Auth(AuthStatus::Connected {
            username: String::new(),
        }));
        self.dispatch(ApiRequest::Me);
    }

    fn sign_in(&mut self) {
        if self.cancel_signin.is_some() {
            return;
        }
        let grant = crate::auth::Grant::web_api(self.web_client_id.as_deref());
        let flow = crate::auth::begin(grant.clone());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.cancel_signin = Some(cancel_tx);
        self.emit(Event::Auth(AuthStatus::WaitingForBrowser {
            url: flow.url.clone(),
        }));
        if let Err(error) = open::that_detached(&flow.url) {
            log::warn!("unable to open a browser: {error}");
        }
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
                    let _ = commands.send(Command::WebSignedIn(Box::new(token)));
                }
                Err(error) => {
                    let _ = events.send(Event::Auth(AuthStatus::SignedOut));
                    let message = error.to_string();
                    if !message.contains("cancelled") {
                        let _ = events.send(Event::Error(format!("Sign-in failed: {message}")));
                    }
                    waker.wake();
                    let _ = commands.send(Command::SignInEnded);
                }
            }
        });
    }

    /// Signs in again with another Web API application, without a restart.
    /// Only the Web API grant changes hands: the browser opens once for the
    /// new application, and local playback keeps its own credential.
    fn switch_web_app(&mut self, client_id: Option<String>) {
        if let Some(cancel) = self.cancel_signin.take() {
            let _ = cancel.send(true);
        }
        self.web_client_id = client_id;
        self.api.set_token_provider(None);
        crate::auth::StoredToken::remove(&self.dirs.web_token_file());
        self.signed_in = false;
        self.emit(Event::Auth(AuthStatus::SignedOut));
        self.sign_in();
    }

    fn sign_out(&mut self) {
        self.signed_in = false;
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
        if let Some(cancel) = self.cancel_signin.take() {
            let _ = cancel.send(true);
        }
        self.api.set_token_provider(None);
        crate::auth::StoredToken::remove(&self.dirs.web_token_file());
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

    /// Reconnect the engine after its session dropped or audio settings changed.
    fn reconnect_engine(&mut self) {
        if !self.signed_in {
            return;
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
        let now = Instant::now();
        self.reconnects
            .retain(|attempt| now.duration_since(*attempt) < Duration::from_secs(600));
        if self.reconnects.len() >= 6 {
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
        if let Err(error) = open::that_detached(&flow.url) {
            log::warn!("unable to open a browser: {error}");
        }
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
                    let _ = commands.send(Command::SignInEnded);
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
                self.engine = Some(Arc::new(engine));
                self.reconnects.clear();
                self.emit(Event::Playback(LocalPlayback::Ready { device_id }));
            }
            None => {
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

    fn fetch_lyrics(&self, request: LyricsRequest) {
        let http = self.http.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        let cache_dir = self.dirs.lyrics_cache_dir();
        tokio::spawn(async move {
            let result = crate::lyrics::fetch(&http, &cache_dir, &request.query)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = events.send(Event::Lyrics {
                uri: request.uri,
                result,
            });
            waker.wake();
        });
    }

    // ---- api ----------------------------------------------------------------

    fn dispatch(&self, request: ApiRequest) {
        let api = Arc::clone(&self.api);
        let events = self.events.clone();
        let waker = self.waker.clone();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let response = handle(&api, request).await;
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

async fn handle(api: &ApiClient, request: ApiRequest) -> ApiResponse {
    match request {
        ApiRequest::Me => ApiResponse::Me(api.me().await),
        ApiRequest::Devices => ApiResponse::Devices(api.devices().await),
        ApiRequest::PlaybackState => ApiResponse::PlaybackState(api.playback_state().await),
        ApiRequest::Queue => ApiResponse::Queue(api.queue().await),
        ApiRequest::RecentlyPlayed => {
            ApiResponse::RecentlyPlayed(api.recently_played(50).await.map(|page| page.items))
        }
        ApiRequest::TopTracks { offset, full } => ApiResponse::TopTracks {
            result: api
                .top_tracks("short_term", if full { 50 } else { 20 }, offset)
                .await,
            offset,
            full,
        },
        ApiRequest::TopArtists => ApiResponse::TopArtists(
            api.top_artists("medium_term", 20)
                .await
                .map(|page| page.items),
        ),
        ApiRequest::Recommendations {
            seed_tracks,
            seed_artists,
        } => {
            ApiResponse::Recommendations(api.recommendations(&seed_tracks, &seed_artists, 20).await)
        }
        ApiRequest::Discover { term } => {
            let result = api
                .search(&term, &["playlist"])
                .await
                .map(|results| results.playlists.map(|page| page.items).unwrap_or_default());
            ApiResponse::Discover { term, result }
        }
        ApiRequest::MyPlaylists { offset } => ApiResponse::MyPlaylists {
            offset,
            result: api.my_playlists(offset, 50).await,
        },
        ApiRequest::Playlist { id } => ApiResponse::Playlist {
            result: api.playlist(&id).await,
            id,
        },
        ApiRequest::PlaylistItems { id, offset } => ApiResponse::PlaylistItems {
            result: api.playlist_items(&id, offset, 100).await,
            id,
            offset,
        },
        ApiRequest::CreatePlaylist {
            user_id,
            name,
            public,
            description,
        } => ApiResponse::PlaylistCreated(
            api.create_playlist(&user_id, &name, public, &description)
                .await,
        ),
        ApiRequest::UpdatePlaylist {
            id,
            name,
            description,
            public,
        } => ApiResponse::PlaylistUpdated {
            result: api
                .update_playlist(&id, name.as_deref(), description.as_deref(), public)
                .await,
            id,
        },
        ApiRequest::AddToPlaylist {
            playlist_id,
            playlist_name,
            uris,
        } => ApiResponse::PlaylistItemsChanged {
            result: api.add_playlist_items(&playlist_id, &uris, None).await,
            id: playlist_id,
            message: format!("Added to {playlist_name}"),
        },
        ApiRequest::RemoveFromPlaylist {
            playlist_id,
            uris,
            snapshot_id,
        } => ApiResponse::PlaylistItemsChanged {
            result: api
                .remove_playlist_items(&playlist_id, &uris, snapshot_id.as_deref())
                .await,
            id: playlist_id,
            message: "Removed from playlist".to_string(),
        },
        ApiRequest::ReorderPlaylist {
            playlist_id,
            range_start,
            insert_before,
            snapshot_id,
        } => ApiResponse::PlaylistItemsChanged {
            result: api
                .reorder_playlist(
                    &playlist_id,
                    range_start,
                    insert_before,
                    snapshot_id.as_deref(),
                )
                .await,
            id: playlist_id,
            message: String::new(),
        },
        ApiRequest::FollowPlaylist { id, follow } => ApiResponse::PlaylistFollowChanged {
            result: if follow {
                api.follow_playlist(&id).await
            } else {
                api.unfollow_playlist(&id).await
            },
            id,
            followed: follow,
        },
        ApiRequest::SavedTracks { offset } => ApiResponse::SavedTracks {
            offset,
            result: api.saved_tracks(offset, 50).await,
        },
        ApiRequest::SavedAlbums { offset } => ApiResponse::SavedAlbums {
            offset,
            result: api.saved_albums(offset, 50).await,
        },
        ApiRequest::FollowedArtists { after } => ApiResponse::FollowedArtists {
            result: api.followed_artists(after.as_deref(), 50).await,
            after,
        },
        ApiRequest::SavedShows { offset } => ApiResponse::SavedShows {
            offset,
            result: api.saved_shows(offset, 50).await,
        },
        ApiRequest::SavedEpisodes { offset } => ApiResponse::SavedEpisodes {
            offset,
            result: api.saved_episodes(offset, 50).await,
        },
        ApiRequest::SetSaved { uris, saved } => ApiResponse::SavedChanged {
            result: if saved {
                api.save(&uris).await
            } else {
                api.unsave(&uris).await
            },
            uris,
            saved,
        },
        ApiRequest::Contains { uris, user_id } => ApiResponse::Contains {
            result: api.contains(&uris, &user_id).await,
            uris,
        },
        ApiRequest::Search { query, serial } => ApiResponse::Search {
            result: api
                .search(
                    &query,
                    &["track", "artist", "album", "playlist", "show", "episode"],
                )
                .await,
            query,
            serial,
        },
        ApiRequest::Artist { id } => ApiResponse::Artist {
            result: api.artist(&id).await,
            id,
        },
        ApiRequest::ArtistTopTracks { id, name } => ApiResponse::ArtistTopTracks {
            result: api.artist_top_tracks(&id, &name).await,
            id,
        },
        ApiRequest::ArtistAlbums { id, groups, offset } => ApiResponse::ArtistAlbums {
            result: api.artist_albums(&id, &groups, offset, 50).await,
            id,
            groups,
            offset,
        },
        ApiRequest::RelatedArtists { id } => ApiResponse::RelatedArtists {
            result: api.related_artists(&id).await,
            id,
        },
        ApiRequest::Album { id } => ApiResponse::Album {
            result: api.album(&id).await,
            id,
        },
        ApiRequest::AlbumTracks { id, offset } => ApiResponse::AlbumTracks {
            result: api.album_tracks(&id, offset, 50).await,
            id,
            offset,
        },
        ApiRequest::Show { id } => ApiResponse::Show {
            result: api.show(&id).await,
            id,
        },
        ApiRequest::ShowEpisodes { id, offset } => ApiResponse::ShowEpisodes {
            result: api.show_episodes(&id, offset, 50).await,
            id,
            offset,
        },
        ApiRequest::Track { id } => ApiResponse::Track {
            result: api.track(&id).await,
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
                RemoteAction::Play => api.play(device, play.as_ref()).await,
                RemoteAction::Pause => api.pause(device).await,
                RemoteAction::Next => api.next(device).await,
                RemoteAction::Previous => api.previous(device).await,
                RemoteAction::Seek => api.seek(position_ms, device).await,
                RemoteAction::Volume => api.set_volume(percent, device).await,
                RemoteAction::Shuffle => api.set_shuffle(flag, device).await,
                RemoteAction::Repeat => api.set_repeat(&repeat, device).await,
            };
            ApiResponse::Remote { action, result }
        }
        ApiRequest::Transfer { device_id, play } => ApiResponse::Transferred {
            result: api.transfer(&device_id, play).await,
            device_id,
        },
        ApiRequest::AddToQueue {
            uri,
            device_id,
            label,
        } => ApiResponse::QueueAdded {
            result: api.add_to_queue(&uri, device_id.as_deref()).await,
            label,
        },
    }
}

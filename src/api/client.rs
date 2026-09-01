//! An authenticated, rate-limited transport for the Spotify Web API.
//!
//! Typed calls share one `request` helper that adds the bearer token, limits
//! concurrency, honors `Retry-After`, and formats API errors. The gateway
//! handles capability differences before dispatch.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::Semaphore;

use super::ApiSource;
use super::models::*;

const BASE_URL: &str = "https://api.spotify.com/v1";
const MAX_IN_FLIGHT: usize = 6;
const RATE_LIMIT_RETRIES: u32 = 3;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Error)]
pub enum ApiError {
    #[error("not signed in")]
    NotSignedIn,
    #[error("{message}")]
    Status { status: u16, message: String },
    #[error("Spotify is rate limiting requests; try again in a moment")]
    RateLimited,
    #[error("Spotify's Development Mode quota is exhausted; try again after the quota resets")]
    QuotaExhausted,
    #[error("your Spotify sign-in expired; please sign in again")]
    SignInExpired { api_source: ApiSource },
    #[error("network error: {0}")]
    Network(String),
    #[error("unexpected response from Spotify: {0}")]
    Decode(String),
}

impl ApiError {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_decode() {
            Self::Decode(error.to_string())
        } else {
            Self::Network(error.to_string())
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

fn is_quota_exhausted(body: &str) -> bool {
    serde_json::from_str::<ApiErrorBody>(body)
        .ok()
        .and_then(|body| body.error.reason)
        .is_some_and(|reason| reason == "QUOTA_EXCEEDED")
}

/// Where bearer tokens come from.
///
/// The Web API is driven by a registered application's PKCE grant, refreshed
/// on demand and persisted so the browser is needed once per machine. Tokens
/// minted for Spotify's own desktop client are throttled on the Web API, so
/// they are never used here. `Fixed` exists only for tests.
#[derive(Clone)]
pub enum TokenProvider {
    Web(std::sync::Arc<WebTokens>),
}

impl TokenProvider {
    async fn access_token(&self) -> Result<String> {
        match self {
            Self::Web(tokens) => tokens.access_token(false).await,
        }
    }

    async fn invalidate(&self) {
        let Self::Web(tokens) = self;
        let _ = tokens.access_token(true).await;
    }
}

/// The Web API grant, refreshed and persisted as it ages.
pub struct WebTokens {
    http: reqwest::Client,
    token: tokio::sync::Mutex<crate::auth::StoredToken>,
    path: std::path::PathBuf,
    source: ApiSource,
}

impl WebTokens {
    pub fn new(
        http: reqwest::Client,
        token: crate::auth::StoredToken,
        path: std::path::PathBuf,
        source: ApiSource,
    ) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            http,
            token: tokio::sync::Mutex::new(token),
            path,
            source,
        })
    }

    /// A valid access token, refreshing first when it is close to expiry or
    /// `force` asks for a fresh one after a 401.
    async fn access_token(&self, force: bool) -> Result<String> {
        let mut guard = self.token.lock().await;
        if force || guard.needs_refresh() {
            let client_id = guard.client_id.clone();
            let refresh_token = guard.refresh_token.clone();
            match crate::auth::refresh(&self.http, &client_id, &refresh_token).await {
                Ok(response) => match crate::auth::StoredToken::from_response(
                    &client_id,
                    response,
                    Some(&refresh_token),
                ) {
                    Ok(updated) => {
                        let _ = updated.save(&self.path);
                        *guard = updated;
                    }
                    Err(error) => {
                        log::warn!("token refresh returned an unusable response: {error}")
                    }
                },
                Err(crate::auth::TokenEndpointError::Rejected { .. }) => {
                    return Err(ApiError::SignInExpired {
                        api_source: self.source,
                    });
                }
                Err(crate::auth::TokenEndpointError::Unreachable(detail)) => {
                    if force || guard.expired() {
                        return Err(ApiError::Network(detail));
                    }
                    log::warn!("token refresh failed, using the current token: {detail}");
                }
            }
        }
        Ok(guard.access_token.clone())
    }
}

/// What to start playing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayRequest {
    pub context_uri: Option<String>,
    pub uris: Vec<String>,
    pub offset_uri: Option<String>,
    pub offset_position: Option<u32>,
    pub position_ms: u32,
}

impl PlayRequest {
    pub fn context(uri: impl Into<String>) -> Self {
        Self {
            context_uri: Some(uri.into()),
            ..Self::default()
        }
    }

    pub fn tracks(uris: Vec<String>) -> Self {
        Self {
            uris,
            ..Self::default()
        }
    }

    pub fn starting_at_uri(mut self, uri: impl Into<String>) -> Self {
        self.offset_uri = Some(uri.into());
        self
    }

    pub fn starting_at_index(mut self, index: u32) -> Self {
        self.offset_position = Some(index);
        self
    }

    fn body(&self) -> Value {
        let mut body = serde_json::Map::new();
        if let Some(context) = &self.context_uri {
            body.insert("context_uri".into(), json!(context));
        } else if !self.uris.is_empty() {
            body.insert("uris".into(), json!(self.uris));
        }
        if let Some(uri) = &self.offset_uri {
            body.insert("offset".into(), json!({ "uri": uri }));
        } else if let Some(position) = self.offset_position {
            body.insert("offset".into(), json!({ "position": position }));
        }
        if self.position_ms > 0 {
            body.insert("position_ms".into(), json!(self.position_ms));
        }
        Value::Object(body)
    }
}

/// Live view of the client's traffic, shared with the interface so it can
/// show that the app is talking to Spotify rather than being slow itself.
pub struct NetActivity {
    started_at: Instant,
    in_flight: AtomicUsize,
    /// Milliseconds since `started_at` when the oldest current burst began.
    busy_since_ms: AtomicU64,
}

impl Default for NetActivity {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            in_flight: AtomicUsize::new(0),
            busy_since_ms: AtomicU64::new(0),
        }
    }
}

impl NetActivity {
    fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    fn begin(&self) {
        if self.in_flight.fetch_add(1, Ordering::SeqCst) == 0 {
            self.busy_since_ms.store(self.now_ms(), Ordering::SeqCst);
        }
    }

    fn end(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    /// Requests have been in flight continuously for at least `for_at_least`.
    pub fn busy(&self, for_at_least: Duration) -> bool {
        self.in_flight.load(Ordering::SeqCst) > 0
            && self
                .now_ms()
                .saturating_sub(self.busy_since_ms.load(Ordering::SeqCst))
                >= for_at_least.as_millis() as u64
    }
}

/// Decrements the in-flight count even if the request future is dropped.
struct ActivityGuard<'a>(&'a NetActivity);

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.0.end();
    }
}

pub struct ApiClient {
    http: reqwest::Client,
    tokens: Mutex<Option<TokenProvider>>,
    limiter: Semaphore,
    cooldown_until: tokio::sync::Mutex<Instant>,
    search_limit: u32,
    artist_albums_limit: u32,
    source: ApiSource,
    activity: Arc<NetActivity>,
}

impl ApiClient {
    pub fn new(
        http: reqwest::Client,
        activity: Arc<NetActivity>,
        search_limit: u32,
        artist_albums_limit: u32,
        source: ApiSource,
    ) -> Self {
        Self {
            http,
            tokens: Mutex::new(None),
            limiter: Semaphore::new(MAX_IN_FLIGHT),
            cooldown_until: tokio::sync::Mutex::new(Instant::now()),
            search_limit,
            artist_albums_limit,
            source,
            activity,
        }
    }

    pub fn set_token_provider(&self, provider: Option<TokenProvider>) {
        *self.tokens.lock().unwrap_or_else(|p| p.into_inner()) = provider;
    }

    fn provider(&self) -> Result<TokenProvider> {
        self.tokens
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or(ApiError::NotSignedIn)
    }

    async fn wait_for_cooldown(&self) {
        loop {
            let until = *self.cooldown_until.lock().await;
            let Some(wait) = until.checked_duration_since(Instant::now()) else {
                return;
            };
            tokio::time::sleep(wait).await;
        }
    }

    async fn extend_cooldown(&self, wait: Duration) {
        let mut until = self.cooldown_until.lock().await;
        *until = (*until).max(Instant::now() + wait);
    }

    // ---- transport -------------------------------------------------------

    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<String> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{BASE_URL}{path}")
        };
        let provider = self.provider()?;
        let started = Instant::now();

        let mut attempt = 0;
        loop {
            attempt += 1;
            self.wait_for_cooldown().await;
            let permit = self
                .limiter
                .acquire()
                .await
                .map_err(|_| ApiError::NotSignedIn)?;
            self.activity.begin();
            let activity = ActivityGuard(&self.activity);
            let token = provider.access_token().await?;
            let mut request = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(&token)
                .query(query);
            if let Some(body) = body {
                request = request.json(body);
            } else if matches!(method, Method::PUT | Method::POST | Method::DELETE) {
                request = request.header(reqwest::header::CONTENT_LENGTH, "0");
            }
            let response = request.send().await?;
            let status = response.status();

            if status == StatusCode::UNAUTHORIZED && attempt == 1 {
                drop(activity);
                drop(permit);
                provider.invalidate().await;
                continue;
            }
            if status == StatusCode::TOO_MANY_REQUESTS && attempt <= RATE_LIMIT_RETRIES {
                let wait = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .map_or(Duration::from_secs(1), Duration::from_secs)
                    .min(MAX_RETRY_AFTER);
                let text = response.text().await.unwrap_or_default();
                if is_quota_exhausted(&text) {
                    return Err(ApiError::QuotaExhausted);
                }
                log::warn!("Spotify rate limit source={} wait={wait:?}", self.source);
                log::info!(
                    "Spotify cooldown source={} duration_ms={}",
                    self.source,
                    wait.as_millis()
                );
                drop(activity);
                drop(permit);
                self.extend_cooldown(wait).await;
                continue;
            }
            if status.is_server_error() && method == Method::GET && attempt == 1 {
                drop(activity);
                drop(permit);
                tokio::time::sleep(Duration::from_millis(800)).await;
                continue;
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(ApiError::RateLimited);
            }
            let text = response.text().await?;
            log::debug!(
                "Spotify request source={} method={} status={} duration_ms={}",
                self.source,
                method,
                status.as_u16(),
                started.elapsed().as_millis()
            );
            if status.is_success() {
                return Ok(text);
            }
            let message = serde_json::from_str::<ApiErrorBody>(&text)
                .ok()
                .map(|body| body.error.message)
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| {
                    status
                        .canonical_reason()
                        .unwrap_or("request failed")
                        .to_string()
                });
            return Err(ApiError::Status {
                status: status.as_u16(),
                message,
            });
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let text = self.send(Method::GET, path, query, None).await?;
        if text.trim().is_empty() {
            return serde_json::from_value(Value::Null)
                .map_err(|error| ApiError::Decode(error.to_string()));
        }
        serde_json::from_str(&text).map_err(|error| ApiError::Decode(error.to_string()))
    }

    async fn get_optional<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Option<T>> {
        // Spotify answers 204 with no body when nothing is playing.
        let text = self.send(Method::GET, path, query, None).await?;
        if text.trim().is_empty() || text.trim() == "null" {
            return Ok(None);
        }
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| ApiError::Decode(error.to_string()))
    }

    /// Performs a change. The status decides success; the body is only
    /// consulted where a caller needs something from it, because Spotify's
    /// replies to player commands are not reliably JSON and contain nothing
    /// this client uses. Treating an unparseable body as failure told people
    /// their music had not started while it was already playing.
    async fn write(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        let text = self.send(method, path, query, body).await?;
        if text.trim().is_empty() {
            return Ok(None);
        }
        match serde_json::from_str(&text) {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                log::debug!(
                    "Spotify write source={} returned a non-JSON success body: {error}",
                    self.source
                );
                Ok(None)
            }
        }
    }

    // ---- identity and player ---------------------------------------------

    pub async fn me(&self) -> Result<User> {
        self.get("/me", &[]).await
    }

    pub async fn devices(&self) -> Result<Vec<Device>> {
        let list: DeviceList = self.get("/me/player/devices", &[]).await?;
        Ok(list.devices)
    }

    pub async fn playback_state(&self) -> Result<Option<PlaybackState>> {
        self.get_optional(
            "/me/player",
            &[("additional_types", "track,episode".to_string())],
        )
        .await
    }

    pub async fn queue(&self) -> Result<Queue> {
        self.get("/me/player/queue", &[]).await
    }

    pub async fn recently_played(
        &self,
        limit: u32,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<CursorPage<PlayHistory>> {
        let mut query = vec![("limit", limit.to_string())];
        if let Some(after) = after {
            query.push(("after", after.to_string()));
        }
        if let Some(before) = before {
            query.push(("before", before.to_string()));
        }
        self.get("/me/player/recently-played", &query).await
    }

    fn device_query(device_id: Option<&str>) -> Vec<(&'static str, String)> {
        device_id
            .map(|id| vec![("device_id", id.to_string())])
            .unwrap_or_default()
    }

    pub async fn play(&self, device_id: Option<&str>, request: Option<&PlayRequest>) -> Result<()> {
        let body = request.map(PlayRequest::body);
        self.write(
            Method::PUT,
            "/me/player/play",
            &Self::device_query(device_id),
            body.as_ref(),
        )
        .await?;
        Ok(())
    }

    pub async fn pause(&self, device_id: Option<&str>) -> Result<()> {
        self.write(
            Method::PUT,
            "/me/player/pause",
            &Self::device_query(device_id),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn next(&self, device_id: Option<&str>) -> Result<()> {
        self.write(
            Method::POST,
            "/me/player/next",
            &Self::device_query(device_id),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn previous(&self, device_id: Option<&str>) -> Result<()> {
        self.write(
            Method::POST,
            "/me/player/previous",
            &Self::device_query(device_id),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn seek(&self, position_ms: u32, device_id: Option<&str>) -> Result<()> {
        let mut query = Self::device_query(device_id);
        query.push(("position_ms", position_ms.to_string()));
        self.write(Method::PUT, "/me/player/seek", &query, None)
            .await?;
        Ok(())
    }

    pub async fn set_volume(&self, percent: u8, device_id: Option<&str>) -> Result<()> {
        let mut query = Self::device_query(device_id);
        query.push(("volume_percent", percent.min(100).to_string()));
        self.write(Method::PUT, "/me/player/volume", &query, None)
            .await?;
        Ok(())
    }

    pub async fn set_shuffle(&self, state: bool, device_id: Option<&str>) -> Result<()> {
        let mut query = Self::device_query(device_id);
        query.push(("state", state.to_string()));
        self.write(Method::PUT, "/me/player/shuffle", &query, None)
            .await?;
        Ok(())
    }

    /// `state` is `off`, `context`, or `track`.
    pub async fn set_repeat(&self, state: &str, device_id: Option<&str>) -> Result<()> {
        let mut query = Self::device_query(device_id);
        query.push(("state", state.to_string()));
        self.write(Method::PUT, "/me/player/repeat", &query, None)
            .await?;
        Ok(())
    }

    pub async fn transfer(&self, device_id: &str, play: bool) -> Result<()> {
        let body = json!({ "device_ids": [device_id], "play": play });
        self.write(Method::PUT, "/me/player", &[], Some(&body))
            .await?;
        Ok(())
    }

    pub async fn add_to_queue(&self, uri: &str, device_id: Option<&str>) -> Result<()> {
        let mut query = Self::device_query(device_id);
        query.push(("uri", uri.to_string()));
        self.write(Method::POST, "/me/player/queue", &query, None)
            .await?;
        Ok(())
    }

    // ---- playlists ---------------------------------------------------------

    pub async fn my_playlists(&self, offset: u32, limit: u32) -> Result<Page<Playlist>> {
        self.get(
            "/me/playlists",
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
        )
        .await
    }

    pub async fn playlist(&self, id: &str) -> Result<Playlist> {
        self.get(&format!("/playlists/{id}"), &[]).await
    }

    pub async fn playlist_items(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<PlaylistItem>> {
        self.get(
            &format!("/playlists/{id}/items"),
            &[
                ("limit", limit.to_string()),
                ("offset", offset.to_string()),
                ("additional_types", "track,episode".to_string()),
            ],
        )
        .await
    }

    pub async fn create_playlist(
        &self,
        name: &str,
        public: bool,
        description: &str,
    ) -> Result<Playlist> {
        let body = json!({ "name": name, "public": public, "description": description });
        let value = self
            .write(Method::POST, "/me/playlists", &[], Some(&body))
            .await?
            .unwrap_or(Value::Null);
        serde_json::from_value(value).map_err(|error| ApiError::Decode(error.to_string()))
    }

    pub async fn update_playlist(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        public: Option<bool>,
    ) -> Result<()> {
        let mut body = serde_json::Map::new();
        if let Some(name) = name {
            body.insert("name".into(), json!(name));
        }
        if let Some(description) = description {
            body.insert("description".into(), json!(description));
        }
        if let Some(public) = public {
            body.insert("public".into(), json!(public));
        }
        self.write(
            Method::PUT,
            &format!("/playlists/{id}"),
            &[],
            Some(&Value::Object(body)),
        )
        .await?;
        Ok(())
    }

    pub async fn add_playlist_items(
        &self,
        id: &str,
        uris: &[String],
        position: Option<u32>,
    ) -> Result<Option<String>> {
        let mut body = json!({ "uris": uris });
        if let Some(position) = position {
            body["position"] = json!(position);
        }
        let value = self
            .write(
                Method::POST,
                &format!("/playlists/{id}/items"),
                &[],
                Some(&body),
            )
            .await?;
        Ok(Self::snapshot(value))
    }

    pub async fn remove_playlist_items(
        &self,
        id: &str,
        uris: &[String],
        snapshot_id: Option<&str>,
    ) -> Result<Option<String>> {
        let entries: Vec<Value> = uris.iter().map(|uri| json!({ "uri": uri })).collect();
        let mut body = json!({ "items": entries });
        if let Some(snapshot) = snapshot_id {
            body["snapshot_id"] = json!(snapshot);
        }
        let value = self
            .write(
                Method::DELETE,
                &format!("/playlists/{id}/items"),
                &[],
                Some(&body),
            )
            .await?;
        Ok(Self::snapshot(value))
    }

    pub async fn reorder_playlist(
        &self,
        id: &str,
        range_start: u32,
        insert_before: u32,
        snapshot_id: Option<&str>,
    ) -> Result<Option<String>> {
        let mut body = json!({
            "range_start": range_start,
            "insert_before": insert_before,
            "range_length": 1,
        });
        if let Some(snapshot) = snapshot_id {
            body["snapshot_id"] = json!(snapshot);
        }
        let value = self
            .write(
                Method::PUT,
                &format!("/playlists/{id}/items"),
                &[],
                Some(&body),
            )
            .await?;
        Ok(Self::snapshot(value))
    }

    fn snapshot(value: Option<Value>) -> Option<String> {
        value
            .and_then(|value| serde_json::from_value::<SnapshotId>(value).ok())
            .and_then(|snapshot| snapshot.snapshot_id)
    }

    pub async fn follow_playlist(&self, id: &str) -> Result<()> {
        self.write(
            Method::PUT,
            "/me/library",
            &[("uris", format!("spotify:playlist:{id}"))],
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn unfollow_playlist(&self, id: &str) -> Result<()> {
        self.write(
            Method::DELETE,
            "/me/library",
            &[("uris", format!("spotify:playlist:{id}"))],
            None,
        )
        .await?;
        Ok(())
    }

    // ---- library -----------------------------------------------------------

    pub async fn saved_tracks(&self, offset: u32, limit: u32) -> Result<Page<SavedTrack>> {
        self.get(
            "/me/tracks",
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
        )
        .await
    }

    pub async fn saved_albums(&self, offset: u32, limit: u32) -> Result<Page<SavedAlbum>> {
        self.get(
            "/me/albums",
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
        )
        .await
    }

    pub async fn followed_artists(
        &self,
        after: Option<&str>,
        limit: u32,
    ) -> Result<CursorPage<Artist>> {
        let mut query = vec![("type", "artist".to_string()), ("limit", limit.to_string())];
        if let Some(after) = after {
            query.push(("after", after.to_string()));
        }
        let followed: FollowedArtists = self.get("/me/following", &query).await?;
        Ok(followed.artists)
    }

    pub async fn saved_shows(&self, offset: u32, limit: u32) -> Result<Page<SavedShow>> {
        self.get(
            "/me/shows",
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
        )
        .await
    }

    pub async fn saved_episodes(&self, offset: u32, limit: u32) -> Result<Page<SavedEpisode>> {
        self.get(
            "/me/episodes",
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
        )
        .await
    }

    pub async fn top_tracks(
        &self,
        time_range: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Page<Track>> {
        self.get(
            "/me/top/tracks",
            &[
                ("limit", limit.to_string()),
                ("offset", offset.to_string()),
                ("time_range", time_range.to_string()),
            ],
        )
        .await
    }

    pub async fn top_artists(&self, time_range: &str, limit: u32) -> Result<Page<Artist>> {
        self.get(
            "/me/top/artists",
            &[
                ("limit", limit.to_string()),
                ("time_range", time_range.to_string()),
            ],
        )
        .await
    }

    async fn library_write(&self, method: Method, uris: &[String]) -> Result<()> {
        self.write(method, "/me/library", &[("uris", uris.join(","))], None)
            .await?;
        Ok(())
    }

    /// Saves tracks, albums, artists, shows, episodes, or playlists.
    pub async fn save(&self, uris: &[String]) -> Result<()> {
        self.library_write(Method::PUT, uris).await
    }

    pub async fn unsave(&self, uris: &[String]) -> Result<()> {
        self.library_write(Method::DELETE, uris).await
    }

    /// Whether each URI is in the library, in the same order as `uris`.
    pub async fn contains(&self, uris: &[String]) -> Result<Vec<bool>> {
        self.get("/me/library/contains", &[("uris", uris.join(","))])
            .await
    }

    // ---- catalog -----------------------------------------------------------

    pub async fn search(&self, query: &str, types: &[&str]) -> Result<SearchResults> {
        self.get(
            "/search",
            &[
                ("q", query.to_string()),
                ("type", types.join(",")),
                ("limit", self.search_limit.to_string()),
            ],
        )
        .await
    }

    pub async fn artist(&self, id: &str) -> Result<Artist> {
        self.get(&format!("/artists/{id}"), &[]).await
    }

    pub async fn artist_top_tracks(&self, id: &str) -> Result<Vec<Track>> {
        self.get::<TopTracks>(&format!("/artists/{id}/top-tracks"), &[])
            .await
            .map(|top| top.tracks)
    }

    pub async fn artist_albums(
        &self,
        id: &str,
        include_groups: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<Album>> {
        let limit = limit.min(self.artist_albums_limit);
        self.get(
            &format!("/artists/{id}/albums"),
            &[
                ("include_groups", include_groups.to_string()),
                ("limit", limit.to_string()),
                ("offset", offset.to_string()),
            ],
        )
        .await
    }

    pub async fn related_artists(&self, id: &str) -> Result<Vec<Artist>> {
        let related: RelatedArtists = self
            .get(&format!("/artists/{id}/related-artists"), &[])
            .await?;
        Ok(related.artists)
    }

    pub async fn album(&self, id: &str) -> Result<Album> {
        self.get(&format!("/albums/{id}"), &[]).await
    }

    pub async fn album_tracks(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Track>> {
        self.get(
            &format!("/albums/{id}/tracks"),
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
        )
        .await
    }

    pub async fn show(&self, id: &str) -> Result<Show> {
        self.get(&format!("/shows/{id}"), &[]).await
    }

    pub async fn show_episodes(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Episode>> {
        self.get(
            &format!("/shows/{id}/episodes"),
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
        )
        .await
    }

    pub async fn track(&self, id: &str) -> Result<Track> {
        self.get(&format!("/tracks/{id}"), &[]).await
    }

    pub async fn recommendations(
        &self,
        seed_tracks: &[String],
        seed_artists: &[String],
        limit: u32,
    ) -> Result<Vec<Track>> {
        let mut query = vec![("limit", limit.to_string())];
        if !seed_tracks.is_empty() {
            query.push(("seed_tracks", seed_tracks.join(",")));
        }
        if !seed_artists.is_empty() {
            query.push(("seed_artists", seed_artists.join(",")));
        }
        let recommendations: Recommendations = self.get("/recommendations", &query).await?;
        Ok(recommendations.tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_request_body_shapes() {
        let context = PlayRequest::context("spotify:album:x").starting_at_uri("spotify:track:y");
        assert_eq!(
            context.body(),
            json!({ "context_uri": "spotify:album:x", "offset": { "uri": "spotify:track:y" } })
        );
        let tracks = PlayRequest::tracks(vec!["spotify:track:a".into()]).starting_at_index(0);
        assert_eq!(
            tracks.body(),
            json!({ "uris": ["spotify:track:a"], "offset": { "position": 0 } })
        );
    }

    #[test]
    fn quota_exhaustion_is_distinct_from_an_ordinary_rate_limit() {
        assert!(is_quota_exhausted(
            r#"{"error":{"status":429,"reason":"QUOTA_EXCEEDED"}}"#
        ));
        assert!(!is_quota_exhausted(
            r#"{"error":{"status":429,"message":"Too many requests"}}"#
        ));
    }

    #[tokio::test]
    async fn cooldown_state_is_owned_by_one_session() {
        let activity = Arc::new(NetActivity::default());
        let shared = ApiClient::new(
            reqwest::Client::new(),
            activity.clone(),
            20,
            50,
            ApiSource::Shared,
        );
        let personal = ApiClient::new(
            reqwest::Client::new(),
            activity,
            10,
            10,
            ApiSource::Personal,
        );
        shared.extend_cooldown(Duration::from_secs(10)).await;
        assert!(*shared.cooldown_until.lock().await > Instant::now());
        assert!(*personal.cooldown_until.lock().await <= Instant::now());
    }
}

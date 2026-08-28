use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use super::ApiSource;
use super::client::{ApiClient, ApiError, NetActivity, TokenProvider};
use super::models::Playlist;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountId(String);

impl AccountId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlaylistId(String);

impl PlaylistId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for PlaylistId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    Unavailable,
    Authorizing,
    Ready { account: AccountId },
}

impl SessionState {
    pub fn account(&self) -> Option<&AccountId> {
        match self {
            Self::Ready { account } => Some(account),
            Self::Unavailable | Self::Authorizing => None,
        }
    }
}

#[derive(Clone, Copy)]
struct ApiProfile {
    source: ApiSource,
    search_limit: u32,
    artist_albums_limit: u32,
}

impl ApiProfile {
    pub const SHARED: Self = Self {
        source: ApiSource::Shared,
        search_limit: 20,
        artist_albums_limit: 50,
    };
    pub const PERSONAL: Self = Self {
        source: ApiSource::Personal,
        search_limit: 10,
        artist_albums_limit: 10,
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaylistAccess {
    Owned,
    Collaborative,
    External,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    CanonicalAccount,
    Playback,
    UserData,
    PlaylistLibrary,
    PlaylistCreation,
    PlaylistSearch,
    Catalog,
    PlaylistMetadata(PlaylistAccess),
    PlaylistItems(PlaylistAccess),
    PlaylistMutation(PlaylistAccess),
    UnsupportedDevelopmentMode,
}

/// A playlist with unknown access dispatches to the shared app, which can
/// serve anything; later requests move once a response reveals the owner.
fn plan(operation: Operation, personal_ready: bool) -> ApiSource {
    use Operation::*;
    match operation {
        CanonicalAccount | PlaylistLibrary | PlaylistSearch | UnsupportedDevelopmentMode => {
            ApiSource::Shared
        }
        PlaylistMetadata(PlaylistAccess::External | PlaylistAccess::Unknown)
        | PlaylistItems(PlaylistAccess::External | PlaylistAccess::Unknown)
        | PlaylistMutation(PlaylistAccess::External | PlaylistAccess::Unknown) => ApiSource::Shared,
        PlaylistMetadata(_) | PlaylistItems(_) | PlaylistMutation(_) if personal_ready => {
            ApiSource::Personal
        }
        Playback | UserData | PlaylistCreation | Catalog if personal_ready => ApiSource::Personal,
        Playback | UserData | PlaylistCreation | Catalog | PlaylistMetadata(_)
        | PlaylistItems(_) | PlaylistMutation(_) => ApiSource::Shared,
    }
}

fn classify_playlist(account: &AccountId, playlist: &Playlist) -> PlaylistAccess {
    if playlist.owner.id.as_deref() == Some(account.as_str()) {
        PlaylistAccess::Owned
    } else if playlist.collaborative {
        PlaylistAccess::Collaborative
    } else if playlist.owner.id.is_some() {
        PlaylistAccess::External
    } else {
        PlaylistAccess::Unknown
    }
}

struct Session {
    state: RwLock<SessionState>,
    client: Arc<ApiClient>,
}

impl Session {
    fn new(http: reqwest::Client, activity: Arc<NetActivity>, profile: ApiProfile) -> Self {
        Self {
            state: RwLock::new(SessionState::Unavailable),
            client: Arc::new(ApiClient::new(
                http,
                activity,
                profile.search_limit,
                profile.artist_albums_limit,
                profile.source,
            )),
        }
    }

    fn state(&self) -> SessionState {
        self.state
            .read()
            .unwrap_or_else(|lock| lock.into_inner())
            .clone()
    }

    fn set_state(&self, state: SessionState) {
        *self.state.write().unwrap_or_else(|lock| lock.into_inner()) = state;
    }
}

pub struct ApiGateway {
    shared: Session,
    personal: Session,
    playlist_access: Mutex<HashMap<PlaylistId, PlaylistAccess>>,
}

impl ApiGateway {
    pub fn new(http: reqwest::Client, activity: Arc<NetActivity>) -> Self {
        Self {
            shared: Session::new(http.clone(), activity.clone(), ApiProfile::SHARED),
            personal: Session::new(http, activity, ApiProfile::PERSONAL),
            playlist_access: Mutex::new(HashMap::new()),
        }
    }

    fn session(&self, source: ApiSource) -> &Session {
        match source {
            ApiSource::Shared => &self.shared,
            ApiSource::Personal => &self.personal,
        }
    }

    pub fn state(&self, source: ApiSource) -> SessionState {
        self.session(source).state()
    }

    pub fn set_state(&self, source: ApiSource, state: SessionState) {
        self.session(source).set_state(state);
    }

    /// Marks a verified session ready, keeping the token provider that
    /// `begin_verification` installed.
    pub fn install(&self, source: ApiSource, account: AccountId) -> Result<(), ApiError> {
        let other = match source {
            ApiSource::Shared => ApiSource::Personal,
            ApiSource::Personal => ApiSource::Shared,
        };
        if self
            .state(other)
            .account()
            .is_some_and(|active| active != &account)
        {
            return Err(ApiError::Status {
                status: 403,
                message: "The Spotify grants belong to different accounts".into(),
            });
        }
        self.session(source)
            .set_state(SessionState::Ready { account });
        Ok(())
    }

    pub fn begin_verification(&self, source: ApiSource, provider: TokenProvider) {
        let session = self.session(source);
        session.client.set_token_provider(Some(provider));
        session.set_state(SessionState::Authorizing);
    }

    pub fn verification_client(&self, source: ApiSource) -> Arc<ApiClient> {
        Arc::clone(&self.session(source).client)
    }

    pub fn clear(&self, source: ApiSource) {
        let session = self.session(source);
        session.client.set_token_provider(None);
        session.set_state(SessionState::Unavailable);
    }

    pub fn clear_all(&self) {
        self.clear(ApiSource::Shared);
        self.clear(ApiSource::Personal);
        self.playlist_access
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .clear();
    }

    pub fn account(&self) -> Option<AccountId> {
        self.state(ApiSource::Shared)
            .account()
            .cloned()
            .or_else(|| self.state(ApiSource::Personal).account().cloned())
    }

    pub fn personal_ready(&self) -> bool {
        matches!(self.state(ApiSource::Personal), SessionState::Ready { .. })
    }

    pub fn client_for(&self, operation: Operation) -> Result<Arc<ApiClient>, ApiError> {
        let source = plan(operation, self.personal_ready());
        let session = self.session(source);
        if !matches!(session.state(), SessionState::Ready { .. }) {
            return Err(ApiError::NotSignedIn);
        }
        log::debug!("Spotify route operation={operation:?} source={source}");
        Ok(Arc::clone(&session.client))
    }

    pub fn playlist_access(&self, id: &str) -> PlaylistAccess {
        self.playlist_access
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .get(id)
            .copied()
            .unwrap_or_default()
    }

    pub fn observe_playlist(&self, playlist: &Playlist) {
        let Some(account) = self.account() else {
            return;
        };
        let access = classify_playlist(&account, playlist);
        self.playlist_access
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .insert(PlaylistId::new(playlist.id.clone()), access);
    }

    pub fn observe_playlists<'a>(&self, playlists: impl IntoIterator<Item = &'a Playlist>) {
        for playlist in playlists {
            self.observe_playlist(playlist);
        }
    }

    pub fn invalidate_playlist_access(&self, id: &PlaylistId) {
        self.playlist_access
            .lock()
            .unwrap_or_else(|lock| lock.into_inner())
            .insert(id.clone(), PlaylistAccess::Unknown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, source: ApiSource) -> TokenProvider {
        let path = std::env::temp_dir().join(format!("fastpotify-{name}-unused-token.json"));
        TokenProvider::Web(super::super::client::WebTokens::new(
            reqwest::Client::new(),
            crate::auth::StoredToken::default(),
            path,
            source,
        ))
    }

    #[test]
    fn routing_matrix_selects_source() {
        let personal = true;
        for operation in [
            Operation::Playback,
            Operation::UserData,
            Operation::PlaylistCreation,
            Operation::Catalog,
            Operation::PlaylistMetadata(PlaylistAccess::Owned),
            Operation::PlaylistMetadata(PlaylistAccess::Collaborative),
            Operation::PlaylistItems(PlaylistAccess::Owned),
            Operation::PlaylistItems(PlaylistAccess::Collaborative),
        ] {
            assert_eq!(plan(operation, personal), ApiSource::Personal);
        }
        for operation in [
            Operation::CanonicalAccount,
            Operation::PlaylistLibrary,
            Operation::PlaylistSearch,
            Operation::UnsupportedDevelopmentMode,
            Operation::PlaylistMetadata(PlaylistAccess::External),
            Operation::PlaylistMetadata(PlaylistAccess::Unknown),
            Operation::PlaylistItems(PlaylistAccess::External),
            Operation::PlaylistItems(PlaylistAccess::Unknown),
            Operation::PlaylistMutation(PlaylistAccess::External),
            Operation::PlaylistMutation(PlaylistAccess::Unknown),
        ] {
            assert_eq!(plan(operation, personal), ApiSource::Shared);
        }
        for operation in [
            Operation::Playback,
            Operation::UserData,
            Operation::PlaylistLibrary,
            Operation::PlaylistCreation,
            Operation::PlaylistSearch,
            Operation::Catalog,
            Operation::PlaylistMetadata(PlaylistAccess::Unknown),
            Operation::PlaylistItems(PlaylistAccess::Unknown),
        ] {
            assert_eq!(plan(operation, false), ApiSource::Shared);
        }
    }

    #[test]
    fn playlist_access_keeps_unknown_explicit() {
        let account = AccountId::new("me");
        let mut playlist = Playlist {
            id: "p".into(),
            ..Playlist::default()
        };
        assert_eq!(
            classify_playlist(&account, &playlist),
            PlaylistAccess::Unknown
        );
        playlist.owner.id = Some("other".into());
        assert_eq!(
            classify_playlist(&account, &playlist),
            PlaylistAccess::External
        );
        playlist.collaborative = true;
        assert_eq!(
            classify_playlist(&account, &playlist),
            PlaylistAccess::Collaborative
        );
        playlist.owner.id = Some("me".into());
        assert_eq!(
            classify_playlist(&account, &playlist),
            PlaylistAccess::Owned
        );
    }

    #[test]
    fn dual_sessions_require_the_same_account() {
        let gateway = ApiGateway::new(reqwest::Client::new(), Arc::new(NetActivity::default()));
        gateway.begin_verification(ApiSource::Shared, provider("shared", ApiSource::Shared));
        gateway
            .install(ApiSource::Shared, AccountId::new("same"))
            .unwrap();
        gateway.begin_verification(
            ApiSource::Personal,
            provider("personal", ApiSource::Personal),
        );
        gateway
            .install(ApiSource::Personal, AccountId::new("same"))
            .unwrap();
        assert!(gateway.personal_ready());
        gateway.clear(ApiSource::Personal);
        gateway.begin_verification(ApiSource::Personal, provider("other", ApiSource::Personal));
        assert!(
            gateway
                .install(ApiSource::Personal, AccountId::new("other"))
                .is_err()
        );
        assert!(matches!(
            gateway.state(ApiSource::Shared),
            SessionState::Ready { .. }
        ));

        gateway.begin_verification(
            ApiSource::Personal,
            provider("personal-again", ApiSource::Personal),
        );
        gateway
            .install(ApiSource::Personal, AccountId::new("same"))
            .unwrap();
        gateway.clear(ApiSource::Shared);
        gateway.begin_verification(
            ApiSource::Shared,
            provider("other-shared", ApiSource::Shared),
        );
        assert!(
            gateway
                .install(ApiSource::Shared, AccountId::new("other"))
                .is_err()
        );
        assert!(matches!(
            gateway.state(ApiSource::Personal),
            SessionState::Ready { .. }
        ));
    }
}

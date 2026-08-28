//! Interface-side state: what is open, what is loaded, what was asked for.

use std::collections::HashMap;
use std::time::Instant;

use crate::api::models::*;

/// Every screen the central panel can show.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Home,
    TopSongs,
    Search,
    LikedSongs,
    Albums,
    Artists,
    Podcasts,
    Episodes,
    Playlist(String),
    Album(String),
    Artist(String),
    Show(String),
    Queue,
    Settings,
}

impl Page {
    pub fn encode(&self) -> String {
        match self {
            Page::Home => "home".into(),
            Page::TopSongs => "top-songs".into(),
            Page::Search => "search".into(),
            Page::LikedSongs => "liked".into(),
            Page::Albums => "albums".into(),
            Page::Artists => "artists".into(),
            Page::Podcasts => "podcasts".into(),
            Page::Episodes => "episodes".into(),
            Page::Playlist(id) => format!("playlist:{id}"),
            Page::Album(id) => format!("album:{id}"),
            Page::Artist(id) => format!("artist:{id}"),
            Page::Show(id) => format!("show:{id}"),
            Page::Queue => "queue".into(),
            Page::Settings => "settings".into(),
        }
    }

    pub fn decode(text: &str) -> Option<Self> {
        Some(match text {
            "home" => Page::Home,
            "top-songs" => Page::TopSongs,
            "search" => Page::Search,
            "liked" => Page::LikedSongs,
            "albums" => Page::Albums,
            "artists" => Page::Artists,
            "podcasts" => Page::Podcasts,
            "episodes" => Page::Episodes,
            "queue" => Page::Queue,
            "settings" => Page::Settings,
            other => {
                let (kind, id) = other.split_once(':')?;
                match kind {
                    "playlist" => Page::Playlist(id.into()),
                    "album" => Page::Album(id.into()),
                    "artist" => Page::Artist(id.into()),
                    "show" => Page::Show(id.into()),
                    _ => return None,
                }
            }
        })
    }

    /// Opens whatever a Spotify URI points at.
    pub fn from_uri(uri: &str) -> Option<Self> {
        let mut parts = uri.split(':');
        let _ = parts.next()?;
        let kind = parts.next()?;
        let id = parts.next()?.to_string();
        Some(match kind {
            "playlist" => Page::Playlist(id),
            "album" => Page::Album(id),
            "artist" => Page::Artist(id),
            "show" => Page::Show(id),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Loadable<T> {
    #[default]
    NotLoaded,
    Loading,
    Loaded(T),
    Failed(String),
}

impl<T> Loadable<T> {
    pub fn get(&self) -> Option<&T> {
        match self {
            Loadable::Loaded(value) => Some(value),
            _ => None,
        }
    }

    pub fn get_mut(&mut self) -> Option<&mut T> {
        match self {
            Loadable::Loaded(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Loadable::Loading)
    }

    pub fn needs_load(&self) -> bool {
        matches!(self, Loadable::NotLoaded | Loadable::Failed(_))
    }

    pub fn from_result<E: std::fmt::Display>(result: Result<T, E>) -> Self {
        match result {
            Ok(value) => Loadable::Loaded(value),
            Err(error) => Loadable::Failed(error.to_string()),
        }
    }
}

/// An offset-paginated list that loads on demand as the user scrolls.
#[derive(Clone, Debug)]
pub struct PagedList<T> {
    pub items: Vec<T>,
    pub total: Option<u32>,
    pub next_offset: Option<u32>,
    pub loading: bool,
    pub error: Option<String>,
    pub loaded_once: bool,
}

impl<T> Default for PagedList<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: None,
            next_offset: Some(0),
            loading: false,
            error: None,
            loaded_once: false,
        }
    }
}

impl<T> PagedList<T> {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn can_load_more(&self) -> bool {
        !self.loading && self.next_offset.is_some()
    }

    pub fn is_complete(&self) -> bool {
        self.loaded_once && self.next_offset.is_none()
    }

    pub fn absorb(&mut self, offset: u32, page: Page_<T>) {
        if offset == 0 {
            self.items.clear();
        }
        if (offset as usize) < self.items.len() {
            self.items.truncate(offset as usize);
        }
        let received = page.items.len() as u32;
        self.items.extend(page.items);
        self.total = Some(page.total);
        let more = page.next.is_some() && received > 0;
        self.next_offset = more.then_some(offset + received);
        self.loading = false;
        self.error = None;
        self.loaded_once = true;
    }

    pub fn fail(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
        self.loaded_once = true;
    }
}

type Page_<T> = crate::api::models::Page<T>;

/// A cursor-paginated list (followed artists).
#[derive(Clone, Debug)]
pub struct CursorList<T> {
    pub items: Vec<T>,
    pub after: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub loaded_once: bool,
    pub complete: bool,
}

impl<T> Default for CursorList<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            after: None,
            loading: false,
            error: None,
            loaded_once: false,
            complete: false,
        }
    }
}

impl<T> CursorList<T> {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn can_load_more(&self) -> bool {
        !self.loading && !self.complete
    }
}

#[derive(Default)]
pub struct Library {
    pub playlists: Loadable<Vec<Playlist>>,
    pub playlists_next: Option<u32>,
    pub liked: PagedList<SavedTrack>,
    pub albums: PagedList<SavedAlbum>,
    pub artists: CursorList<Artist>,
    pub shows: PagedList<SavedShow>,
    pub episodes: PagedList<SavedEpisode>,
    pub filter: String,
}

#[derive(Default)]
pub struct HomeData {
    pub recently_played: Loadable<Vec<PlayHistory>>,
    pub top_artists: Loadable<Vec<Artist>>,
    /// The 20-track preview shown on Home.
    pub top_tracks: Loadable<Vec<Track>>,
    /// The separately loaded, complete ranking shown by the Top Songs page.
    pub top_songs: Loadable<Vec<Track>>,
    pub top_songs_loading: bool,
    pub top_songs_complete: bool,
    pub recommendations: Loadable<Vec<Track>>,
    pub discover: HashMap<String, Loadable<Vec<Playlist>>>,
    pub requested: bool,
    pub loaded_at: Option<Instant>,
}

pub const DISCOVER_TERMS: &[&str] = &["Discover Weekly", "Release Radar", "Daily Mix", "daylist"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchFilter {
    #[default]
    All,
    Songs,
    Artists,
    Albums,
    Playlists,
    Podcasts,
    Episodes,
}

impl SearchFilter {
    pub const ALL: [SearchFilter; 7] = [
        Self::All,
        Self::Songs,
        Self::Artists,
        Self::Albums,
        Self::Playlists,
        Self::Podcasts,
        Self::Episodes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Songs => "Songs",
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Playlists => "Playlists",
            Self::Podcasts => "Podcasts",
            Self::Episodes => "Episodes",
        }
    }
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub committed: String,
    pub serial: u64,
    pub results: Loadable<SearchResults>,
    pub filter: SearchFilter,
    pub typed_at: Option<Instant>,
    pub focus_requested: bool,
}

#[derive(Default)]
pub struct PlaylistPage {
    pub playlist: Loadable<Playlist>,
    pub items: PagedList<PlaylistItem>,
    pub filter: String,
    /// Ids of everyone who added songs, from the pages seen so far and one
    /// look at the tail.
    pub contributors: std::collections::BTreeSet<String>,
    /// Whether the tail was sampled for who added its songs.
    pub tail_checked: bool,
    /// The whole list came from disk and matches the live snapshot.
    pub cache_complete: bool,
    /// Items read from disk, waiting for the live snapshot to confirm.
    pub pending_cache: Option<(String, Vec<PlaylistItem>)>,
}

#[derive(Default)]
pub struct AlbumPage {
    pub album: Loadable<Album>,
    pub tracks: PagedList<Track>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiscographyFilter {
    #[default]
    All,
    Albums,
    Singles,
    AppearsOn,
}

impl DiscographyFilter {
    pub const ALL: [DiscographyFilter; 4] =
        [Self::All, Self::Albums, Self::Singles, Self::AppearsOn];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Albums => "Albums",
            Self::Singles => "Singles & EPs",
            Self::AppearsOn => "Appears On",
        }
    }

    pub fn groups(self) -> &'static str {
        match self {
            Self::All => "album,single,compilation",
            Self::Albums => "album",
            Self::Singles => "single",
            Self::AppearsOn => "appears_on",
        }
    }
}

#[derive(Default)]
pub struct ArtistPage {
    pub artist: Loadable<Artist>,
    pub top_tracks: Loadable<Vec<Track>>,
    pub albums: HashMap<String, PagedList<Album>>,
    pub related: Loadable<Vec<Artist>>,
    pub filter: DiscographyFilter,
    pub show_all_top: bool,
}

#[derive(Default)]
pub struct ShowPage {
    pub show: Loadable<Show>,
    pub episodes: PagedList<Episode>,
}

/// A table's sort, chosen by clicking a column heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableSort {
    pub column: SortColumn,
    pub ascending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SortColumn {
    Title,
    Album,
    Added,
    Duration,
    AddedBy,
}

/// One of the things a track row can be part of, for playback context and
/// for the actions the row offers.
#[derive(Clone, Debug, PartialEq)]
pub enum RowContext {
    /// A Spotify context (playlist, album) that can be played from an offset.
    Context {
        uri: String,
        /// The playlist id when the user owns it, enabling removal.
        editable_playlist: Option<(String, Option<String>)>,
    },
    /// A loose list of tracks, played as a queue of URIs.
    Uris(Vec<String>),
}

#[derive(Clone, Debug)]
pub enum Dialog {
    CreatePlaylist {
        name: String,
        public: bool,
        add_uris: Vec<String>,
    },
    EditPlaylist {
        id: String,
        name: String,
        description: String,
        public: bool,
    },
    ConfirmDeletePlaylist {
        id: String,
        name: String,
        owned: bool,
    },
    Shortcuts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub message: String,
    pub kind: ToastKind,
    pub created: Instant,
}

/// What views ask the app to do. Collected while drawing, applied after, so
/// a view can iterate over app data without fighting the borrow checker.
#[derive(Clone, Debug)]
pub enum Action {
    Open(Page),
    OpenUri(String),
    Back,
    Forward,
    PlayContext {
        uri: String,
        offset_uri: Option<String>,
        offset_index: Option<u32>,
    },
    PlayUris {
        uris: Vec<String>,
        index: u32,
    },
    PlayFromRow {
        context: RowContext,
        uri: String,
        index: u32,
    },
    ShufflePlay(String),
    TogglePlay,
    Next,
    Previous,
    Seek(u32),
    SeekBy(i64),
    SetVolume(u8),
    /// The slider mid-drag: heard at once, told to Spotify on release.
    PreviewVolume(u8),
    VolumeBy(i8),
    ToggleMute,
    ToggleShuffle,
    CycleRepeat,
    SetShuffle(bool),
    SetRepeat(crate::player::RepeatMode),
    AddToQueue {
        uri: String,
        label: String,
    },
    ToggleSaved(String),
    AddToPlaylist {
        playlist_id: String,
        playlist_name: String,
        uris: Vec<String>,
    },
    RemoveFromPlaylist {
        playlist_id: String,
        uris: Vec<String>,
    },
    MoveInPlaylist {
        playlist_id: String,
        from: u32,
        to: u32,
    },
    ShowDialog(Dialog),
    CloseDialog,
    CreatePlaylist {
        name: String,
        public: bool,
        add_uris: Vec<String>,
    },
    UpdatePlaylist {
        id: String,
        name: String,
        description: String,
        public: bool,
    },
    DeletePlaylist(String),
    Transfer(String),
    /// Hand the account to a receiver found on the local network.
    ActivateReceiver(Box<crate::zeroconf::Receiver>),
    RefreshDevices,
    RefreshQueue,
    CopyLink(String),
    /// A web page, in the browser.
    OpenUrl(String),
    OpenInSpotify(String),
    Search(String),
    SetSearchFilter(SearchFilter),
    FocusSearch,
    LoadMore(Page),
    LoadMoreArtistAlbums(String),
    SetDiscographyFilter {
        artist_id: String,
        filter: DiscographyFilter,
    },
    ToggleShowAllTop(String),
    Reload(Page),
    SignIn,
    CancelSignIn,
    SignOut,
    /// Sign in again with the Web API application named in Settings.
    SwitchWebApp,
    ToggleSidebar,
    ToggleQueuePanel,
    ToggleLyricsPanel,
    ToggleDevicesPopup,
    SettingsChanged,
    RestartEngine,
    EnablePlayback,
    ShowWindow,
    HideWindow,
    ClearArtCache,
    Quit,
}

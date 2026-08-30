//! Sample data for screenshots and headless rendering tests.
//!
//! Nothing here talks to Spotify: the backend is switched offline, the
//! session is marked connected, and every page is filled with plausible
//! content. Cover art comes from a public placeholder service so the artwork
//! pipeline (download, disk cache, accent colour) is exercised too.

use std::time::Instant;

use crate::api::models::{
    Album, Artist, ArtistRef, Context, Copyright, Device, Episode, Followers, Image, Owner,
    Page as ApiPage, PlayHistory, PlayableItem, PlaybackState, Playlist, PlaylistItem, Queue,
    ResumePoint, SavedAlbum, SavedEpisode, SavedShow, SavedTrack, SearchResults, Show, Track,
    TrackCount, User,
};
use crate::app::{App, RemoteSnapshot};
use crate::backend::AuthStatus;
use crate::model::*;

fn image(seed: u32) -> Vec<Image> {
    vec![
        Image {
            url: format!("https://picsum.photos/seed/fastpotify{seed}/640/640"),
            width: Some(640),
            height: Some(640),
        },
        Image {
            url: format!("https://picsum.photos/seed/fastpotify{seed}/300/300"),
            width: Some(300),
            height: Some(300),
        },
        Image {
            url: format!("https://picsum.photos/seed/fastpotify{seed}/64/64"),
            width: Some(64),
            height: Some(64),
        },
    ]
}

const ARTISTS: &[&str] = &[
    "Bonobo",
    "Khruangbin",
    "Nils Frahm",
    "Little Simz",
    "Floating Points",
    "Jon Hopkins",
    "Sault",
    "Four Tet",
];

const ALBUMS: &[(&str, usize, &str)] = &[
    ("Fragments", 0, "2022"),
    ("Mordechai", 1, "2020"),
    ("All Melody", 2, "2018"),
    ("Sometimes I Might Be Introvert", 3, "2021"),
    ("Promises", 4, "2021"),
    ("Immunity", 5, "2013"),
    ("Untitled (Black Is)", 6, "2020"),
    ("There Is Love in You", 7, "2010"),
];

const TRACKS: &[&str] = &[
    "Rosewood",
    "Otomo",
    "Shadows",
    "Tides",
    "Elysian",
    "Closer",
    "Counterpart",
    "Sapien",
    "From You",
    "Day by Day",
    "Age of Phase",
    "Polyghost",
    "Time Moves Slow",
    "August 10",
    "So Rare",
    "Fugue",
    "Encores",
    "Sunlight",
    "My Friend the Forest",
    "Kaleidoscope",
];

const PLAYLISTS: &[&str] = &[
    "Discover Weekly",
    "Late night focus",
    "Sunday morning",
    "Running 2026",
    "Release Radar",
    "Berlin nights",
    "Dinner party",
    "Deep work",
    "Road trip",
    "Kitchen jams",
];

fn artist_ref(index: usize) -> ArtistRef {
    ArtistRef {
        id: Some(format!("art{index}")),
        name: ARTISTS[index % ARTISTS.len()].to_string(),
        uri: Some(format!("spotify:artist:art{index}")),
    }
}

fn artist(index: usize) -> Artist {
    Artist {
        id: format!("art{index}"),
        name: ARTISTS[index % ARTISTS.len()].to_string(),
        uri: format!("spotify:artist:art{index}"),
        images: image(100 + index as u32),
        genres: vec!["electronic".into(), "downtempo".into(), "ambient".into()],
        followers: Some(Followers {
            total: 1_284_930 + index as u64 * 10_431,
        }),
        popularity: Some(70),
        ..Artist::default()
    }
}

fn album(index: usize) -> Album {
    let (name, artist_index, year) = ALBUMS[index % ALBUMS.len()];
    Album {
        id: format!("alb{index}"),
        name: name.to_string(),
        uri: format!("spotify:album:alb{index}"),
        album_type: Some(if index % 4 == 3 {
            "single".into()
        } else {
            "album".into()
        }),
        total_tracks: Some(12),
        images: image(200 + index as u32),
        artists: vec![artist_ref(artist_index)],
        release_date: Some(format!("{year}-03-1{}", index % 9)),
        label: Some("Ninja Tune".into()),
        copyrights: vec![Copyright {
            text: format!("{year} Ninja Tune"),
            kind: "C".into(),
        }],
        ..Album::default()
    }
}

fn track(index: usize) -> Track {
    let album_index = index % ALBUMS.len();
    let mut album = album(album_index);
    album.tracks = None;
    Track {
        id: Some(format!("trk{index}")),
        name: TRACKS[index % TRACKS.len()].to_string(),
        uri: format!("spotify:track:trk{index}"),
        duration_ms: 180_000 + (index as u32 * 37_000) % 240_000,
        explicit: index % 7 == 3,
        artists: vec![artist_ref(album_index)],
        album: Some(album),
        track_number: Some((index % 12) as u32 + 1),
        disc_number: Some(1),
        popularity: Some(60 + (index % 40) as u8),
        ..Track::default()
    }
}

fn playlist(index: usize) -> Playlist {
    let name = PLAYLISTS[index % PLAYLISTS.len()];
    let spotify_owned = matches!(name, "Discover Weekly" | "Release Radar");
    Playlist {
        id: format!("pl{index}"),
        name: name.to_string(),
        uri: format!("spotify:playlist:pl{index}"),
        description: Some(if spotify_owned {
            "Your weekly mixtape of fresh music. Enjoy new music and deep cuts picked for you. Updates every Monday.".into()
        } else {
            String::new()
        }),
        images: image(300 + index as u32),
        owner: Owner {
            id: Some(if spotify_owned {
                "spotify".into()
            } else {
                "demo".into()
            }),
            display_name: Some(if spotify_owned {
                "Spotify".into()
            } else {
                "Carmine".into()
            }),
            uri: None,
        },
        public: Some(index.is_multiple_of(2)),
        collaborative: false,
        snapshot_id: Some("snap".into()),
        tracks: Some(TrackCount {
            total: 30 + index as u32 * 7,
        }),
        ..Playlist::default()
    }
}

fn episode(index: usize, show_index: usize) -> Episode {
    Episode {
        id: format!("ep{show_index}_{index}"),
        name: format!("Episode {}: {}", 120 - index, TRACKS[(index * 3) % TRACKS.len()]),
        uri: format!("spotify:episode:ep{show_index}_{index}"),
        duration_ms: 2_400_000 + (index as u32 * 311_000) % 2_000_000,
        description: "A conversation about how software gets made, why some tools feel fast, and what we can learn from the people who build them. Recorded live.".into(),
        images: image(400 + index as u32),
        release_date: Some(format!("2026-0{}-{:02}", 1 + index % 8, 1 + index % 27)),
        resume_point: Some(ResumePoint {
            fully_played: index.is_multiple_of(5),
            resume_position_ms: if index % 3 == 1 { 600_000 } else { 0 },
        }),
        show: Some(show(show_index)),
        ..Episode::default()
    }
}

fn show(index: usize) -> Show {
    Show {
        id: format!("sh{index}"),
        name: ["Rework", "Song Exploder", "The Rest Is History", "Darknet Diaries"][index % 4].into(),
        uri: format!("spotify:show:sh{index}"),
        publisher: ["37signals", "Hrishikesh Hirway", "Goalhanger", "Jack Rhysider"][index % 4].into(),
        description: "A podcast about a better way to work and run your business. Hosted by the founders of 37signals.".into(),
        images: image(500 + index as u32),
        total_episodes: Some(84),
        ..Show::default()
    }
}

fn page<T>(items: Vec<T>) -> ApiPage<T> {
    let total = items.len() as u32;
    ApiPage {
        items,
        total,
        limit: total,
        offset: 0,
        next: None,
    }
}

pub fn populate(app: &mut App) {
    app.backend.set_offline(true);
    app.offline = true;
    app.auth = AuthStatus::Connected {
        username: "demo".into(),
    };
    app.local_device_id = Some("local-demo".into());
    app.local_ready = true;
    app.local_playback = crate::backend::LocalPlayback::Ready {
        device_id: "local-demo".into(),
    };
    app.user = Some(User {
        id: "demo".into(),
        display_name: Some("Carmine".into()),
        images: image(1),
        product: Some("premium".into()),
        country: Some("DE".into()),
        uri: Some("spotify:user:demo".into()),
    });

    let playlists: Vec<Playlist> = (0..PLAYLISTS.len()).map(playlist).collect();
    for playlist in &playlists {
        app.saved.insert(playlist.uri.clone(), true);
    }
    app.library.playlists = Loadable::Loaded(playlists.clone());

    let tracks: Vec<Track> = (0..40).map(track).collect();
    for (index, track) in tracks.iter().enumerate() {
        app.saved.insert(track.uri.clone(), index % 3 == 0);
    }

    // Playlist page.
    let mut playlist_page = PlaylistPage {
        playlist: Loadable::Loaded(playlists[1].clone()),
        ..PlaylistPage::default()
    };
    playlist_page.items.absorb(
        0,
        page(
            tracks
                .iter()
                .enumerate()
                .map(|(index, track)| PlaylistItem {
                    added_at: Some(format!(
                        "2026-0{}-{:02}T10:00:00Z",
                        1 + index % 8,
                        1 + index % 27
                    )),
                    is_local: false,
                    // A second pair of hands, so the page reads as made
                    // together: the Added By column and the byline show.
                    added_by: Some(crate::api::models::UserRef {
                        id: Some(if index % 3 == 1 { "kasia" } else { "sam" }.into()),
                    }),
                    item: Some(PlayableItem::Track(track.clone())),
                    track: None,
                })
                .collect(),
        ),
    );
    playlist_page.contributors.insert("kasia".into());
    playlist_page.contributors.insert("sam".into());
    app.playlist_pages.insert("pl1".into(), playlist_page);
    app.user_names.insert("kasia".into(), Some("Kasia".into()));
    app.user_names.insert("sam".into(), Some("Sam".into()));
    let mut discover_page = PlaylistPage {
        playlist: Loadable::Loaded(playlists[0].clone()),
        ..PlaylistPage::default()
    };
    discover_page.items.absorb(
        0,
        page(
            tracks
                .iter()
                .rev()
                .take(30)
                .map(|track| PlaylistItem {
                    added_at: Some("2026-08-24T05:00:00Z".into()),
                    is_local: false,
                    added_by: None,
                    item: Some(PlayableItem::Track(track.clone())),
                    track: None,
                })
                .collect(),
        ),
    );
    app.playlist_pages.insert("pl0".into(), discover_page);

    // Album page.
    let mut album_page = AlbumPage {
        album: Loadable::Loaded(album(0)),
        ..AlbumPage::default()
    };
    album_page
        .tracks
        .absorb(0, page(tracks.iter().take(12).cloned().collect()));
    app.album_pages.insert("alb0".into(), album_page);
    app.saved.insert("spotify:album:alb0".into(), true);

    // Artist page.
    let mut artist_page = ArtistPage {
        artist: Loadable::Loaded(artist(0)),
        top_tracks: Loadable::Loaded(tracks.iter().take(10).cloned().collect()),
        related: Loadable::Loaded((1..8).map(artist).collect()),
        ..ArtistPage::default()
    };
    let mut albums = PagedList::default();
    albums.absorb(0, page((0..8).map(album).collect()));
    artist_page
        .albums
        .insert(DiscographyFilter::All.groups().to_string(), albums);
    app.artist_pages.insert("art0".into(), artist_page);
    app.saved.insert("spotify:artist:art0".into(), true);

    // Show page.
    let mut show_page = ShowPage {
        show: Loadable::Loaded(show(0)),
        ..ShowPage::default()
    };
    show_page
        .episodes
        .absorb(0, page((0..15).map(|index| episode(index, 0)).collect()));
    app.show_pages.insert("sh0".into(), show_page);

    // Library.
    app.library.liked.absorb(
        0,
        page(
            tracks
                .iter()
                .filter(|track| app.saved.get(&track.uri) == Some(&true))
                .map(|track| SavedTrack {
                    added_at: Some("2026-06-12T08:30:00Z".into()),
                    track: track.clone(),
                })
                .collect(),
        ),
    );
    app.library.albums.absorb(
        0,
        page(
            (0..8)
                .map(|index| SavedAlbum {
                    added_at: None,
                    album: album(index),
                })
                .collect(),
        ),
    );
    app.library.artists.items = (0..8).map(artist).collect();
    app.library.artists.loaded_once = true;
    app.library.artists.complete = true;
    app.library.shows.absorb(
        0,
        page(
            (0..4)
                .map(|index| SavedShow {
                    added_at: None,
                    show: show(index),
                })
                .collect(),
        ),
    );
    app.library.episodes.absorb(
        0,
        page(
            (0..6)
                .map(|index| SavedEpisode {
                    added_at: None,
                    episode: episode(index, index % 4),
                })
                .collect(),
        ),
    );

    // Home.
    app.home.requested = true;
    app.home.loaded_at = Some(Instant::now());
    app.home.recently_played = Loadable::Loaded(
        tracks
            .iter()
            .skip(5)
            .take(12)
            .map(|track| PlayHistory {
                track: track.clone(),
                played_at: Some("2026-08-26T21:12:00Z".into()),
                context: None,
            })
            .collect(),
    );
    app.home.top_artists = Loadable::Loaded((0..8).map(artist).collect());
    app.home.top_tracks = Loadable::Loaded(tracks.iter().skip(10).take(10).cloned().collect());
    app.home.top_songs = Loadable::Loaded(tracks.iter().skip(10).cloned().collect());
    app.home.top_songs_complete = true;
    app.home.recommendations = Loadable::Loaded(tracks.iter().skip(20).take(10).cloned().collect());
    for term in DISCOVER_TERMS {
        let matching: Vec<Playlist> = playlists
            .iter()
            .filter(|playlist| playlist.name.to_lowercase().contains(&term.to_lowercase()))
            .cloned()
            .collect();
        app.home
            .discover
            .insert((*term).to_string(), Loadable::Loaded(matching));
    }

    // Search.
    app.search.query = "Bonobo".into();
    app.search.committed = "Bonobo".into();
    app.search.results = Loadable::Loaded(SearchResults {
        tracks: Some(page(tracks.iter().take(10).cloned().collect())),
        artists: Some(page((0..6).map(artist).collect())),
        albums: Some(page((0..6).map(album).collect())),
        playlists: Some(page(playlists.iter().take(6).cloned().collect())),
        shows: Some(page((0..4).map(show).collect())),
        episodes: Some(page((0..4).map(|index| episode(index, 1)).collect())),
    });
    app.settings.search_history = vec!["Khruangbin".into(), "ambient".into(), "Rework".into()];

    // Playback: a remote speaker is playing the second playlist.
    app.queue = Loadable::Loaded(Queue {
        currently_playing: Some(PlayableItem::Track(tracks[0].clone())),
        queue: tracks
            .iter()
            .skip(1)
            .take(12)
            .cloned()
            .map(PlayableItem::Track)
            .collect(),
    });
    app.devices = vec![
        Device {
            id: Some("local-demo".into()),
            name: "Fastpotify".into(),
            is_active: false,
            is_restricted: false,
            volume_percent: Some(70),
            supports_volume: Some(true),
            kind: "computer".into(),
        },
        Device {
            id: Some("remote1".into()),
            name: "Kitchen speaker".into(),
            is_active: true,
            is_restricted: false,
            volume_percent: Some(62),
            supports_volume: Some(true),
            kind: "speaker".into(),
        },
        Device {
            id: Some("remote2".into()),
            name: "Pixel 9".into(),
            is_active: false,
            is_restricted: false,
            volume_percent: Some(40),
            supports_volume: Some(true),
            kind: "smartphone".into(),
        },
    ];
    // A speaker announcing itself over ZeroConf that the account has not
    // adopted yet, so the device picker shows the row that offers to hand it
    // the account.
    app.receivers = vec![crate::zeroconf::Receiver {
        name: "House Spotify".into(),
        address: std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 42)),
        port: 5555,
        path: "/zc".into(),
    }];
    app.remote = Some(RemoteSnapshot {
        state: PlaybackState {
            device: Some(app.devices[1].clone()),
            repeat_state: "off".into(),
            shuffle_state: true,
            context: Some(Context {
                uri: playlists[1].uri.clone(),
                kind: "playlist".into(),
            }),
            timestamp: 0,
            progress_ms: Some(83_000),
            is_playing: true,
            item: Some(PlayableItem::Track(tracks[0].clone())),
            currently_playing_type: Some("track".into()),
        },
        received_at: Instant::now(),
    });
    for track in &tracks {
        if let Some(id) = &track.id {
            app.track_cache.insert(id.clone(), track.clone());
        }
    }
}

/// Words to go with the sample track, timed so that the one being sung
/// sits mid-panel at the demo's playback position.
#[cfg(feature = "demo")]
fn sample_lyrics() -> crate::lyrics::Lyrics {
    let lines = [
        (40_000, "Streetlights blinking down the river road"),
        (46_500, "Every window holding someone's evening"),
        (53_000, "I keep the radio low so you can sleep"),
        (59_500, "Counting mile markers like a rosary"),
        (66_000, "We left the city with the tank half full"),
        (72_500, "And a map that only shows the way back"),
        (79_000, "But the night is wide and the road is long"),
        (85_500, "And there's nowhere I would rather be"),
        (92_000, "Coffee going cold in the cup holder"),
        (98_500, "Your hand asleep on the gear stick"),
        (105_000, "Somewhere past the county line"),
        (111_500, "The stars come out to see us through"),
        (118_000, "Still the night is wide and the road is long"),
        (124_500, "And there's nowhere I would rather be"),
    ];
    crate::lyrics::Lyrics {
        lines: lines
            .iter()
            .map(|(at_ms, text)| crate::lyrics::Line {
                at_ms: Some(*at_ms),
                text: (*text).to_string(),
            })
            .collect(),
        synced: true,
        instrumental: false,
    }
}

/// Applies `--demo-page` and `--demo-show`.
#[cfg(feature = "demo")]
pub fn apply_flags(app: &mut App, page: Option<&str>, show: Option<&str>) {
    // Whatever the settings on this machine say, a shot is of the big
    // window unless the mini player is asked for.
    app.settings.winamp_window = false;
    if let Some(page) = page.and_then(Page::decode) {
        app.open(page);
    }
    for surface in show.unwrap_or("").split(',').map(str::trim) {
        match surface {
            "queue" => app.show_queue_panel = true,
            "devices" => app.show_devices = true,
            "shortcuts" => app.dialog = Some(Dialog::Shortcuts),
            "premium" => app.dialog = Some(Dialog::PremiumNeeded),
            "create" => {
                app.dialog = Some(Dialog::CreatePlaylist {
                    name: "Autumn drives".into(),
                    public: false,
                    add_uris: vec!["spotify:track:trk1".into()],
                })
            }
            "light" => {
                app.settings.theme = crate::settings::ThemeChoice::Light;
                app.actions.push(Action::SettingsChanged);
            }
            "focus" => app.settings.sidebar_visible = false,
            // The built-in skin, whatever the settings say, so shots are
            // the same everywhere and never show someone else's art.
            "winamp" => {
                app.settings.winamp_window = true;
                app.settings.skin = None;
            }
            "playlist" => app.settings.playlist_open = true,
            "shade" => app.settings.winamp_shaded = true,
            "playlist-shade" => app.settings.playlist_shaded = true,
            "eq" => {
                app.settings.eq_open = true;
                app.settings.eq_on = true;
                app.settings.eq_bands_db = crate::eq::PRESETS[13].bands_db;
            }
            "compact" => {
                app.settings.sidebar_compact = true;
                app.settings.tracklist_compact = true;
            }
            "eq-shade" => {
                app.settings.eq_open = true;
                app.settings.eq_shaded = true;
            }
            "pins" => {
                app.settings.pinned_contexts =
                    vec!["spotify:playlist:pl2".into(), "spotify:playlist:pl4".into()];
            }
            "sorted" => {
                app.table_sorts.insert(
                    Page::Playlist("pl1".into()),
                    crate::model::TableSort {
                        column: crate::model::SortColumn::Added,
                        ascending: false,
                    },
                );
            }
            "lyrics" => {
                app.lyrics_uri = app.now_playing().map(|now| now.uri);
                app.lyrics = Loadable::Loaded(Some(sample_lyrics()));
                app.lyrics_following = true;
                app.show_lyrics_panel = true;
            }
            // Titles in scripts the interface font does not cover.
            "scripts" => {
                let titles = [
                    ("\u{591c}\u{306b}\u{99c6}\u{3051}\u{308b}", "YOASOBI"),
                    (
                        "\u{8d77}\u{98ce}\u{4e86}",
                        "\u{4e70}\u{8fa3}\u{6912}\u{4e5f}\u{7528}\u{5238}",
                    ),
                    (
                        "\u{bd04}\u{c5ec}\u{b984}\u{ac00}\u{c744}\u{aca8}\u{c6b8} (Still Life)",
                        "BIGBANG",
                    ),
                    (
                        "\u{6253}\u{4e0a}\u{82b1}\u{706b}",
                        "DAOKO, \u{7c73}\u{6d25}\u{7384}\u{5e2b}",
                    ),
                    (
                        "\u{5149}\u{5e74}\u{4e4b}\u{5916}",
                        "G.E.M. \u{9093}\u{7d2b}\u{68cb}",
                    ),
                    ("\u{bc24}\u{d3b8}\u{c9c0}", "IU"),
                    ("Lemon", "\u{7c73}\u{6d25}\u{7384}\u{5e2b}"),
                    (
                        "\u{7ea2}\u{8272}\u{9ad8}\u{8ddf}\u{978b}",
                        "\u{8521}\u{5065}\u{96c5}",
                    ),
                ];
                let rename = |track: &mut Track, (title, artist): (&str, &str)| {
                    track.name = title.to_string();
                    track.artists = vec![ArtistRef {
                        id: None,
                        name: artist.to_string(),
                        uri: None,
                    }];
                };
                if let Some(page) = app.playlist_pages.get_mut("pl1") {
                    for (entry, names) in page.items.items.iter_mut().zip(titles) {
                        if let Some(PlayableItem::Track(track)) = &mut entry.item {
                            rename(track, names);
                        }
                    }
                }
                if let Loadable::Loaded(queue) = &mut app.queue {
                    for (item, names) in queue.queue.iter_mut().zip(titles) {
                        if let PlayableItem::Track(track) = item {
                            rename(track, names);
                        }
                    }
                }
                if let Some(remote) = &mut app.remote
                    && let Some(PlayableItem::Track(track)) = &mut remote.state.item
                {
                    rename(track, titles[0]);
                }
                if let Some(track) = app.track_cache.get_mut("trk0") {
                    rename(track, titles[0]);
                }
                if let Loadable::Loaded(playlists) = &mut app.library.playlists {
                    let names = [
                        "\u{901a}\u{52e4}\u{306e}BGM",
                        "\u{7761}\u{524d}\u{6b4c}\u{5355}",
                        "\u{cd9c}\u{adfc}\u{ae38} \u{d50c}\u{b808}\u{c774}\u{b9ac}\u{c2a4}\u{d2b8}",
                    ];
                    for (playlist, name) in playlists.iter_mut().skip(3).zip(names) {
                        playlist.name = name.to_string();
                    }
                }
            }
            // A Spotify app of one's own, in use.
            "faster" => {
                let id = "8f2c1d0e4a6b4c3d9e7f5a1b2c3d4e5f".to_string();
                app.settings.web_client_id = Some(id.clone());
                app.web_app = Some(id);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppOptions;
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    fn frame(ctx: &egui::Context, app: &mut App) {
        frame_events(ctx, app, Vec::new());
    }

    fn frame_events(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.frame_ui(ui);
        });
        output.textures_delta.clear();
    }

    /// Every page, panel, and dialog lays out without panicking.
    #[test]
    fn every_surface_renders_headless() {
        let root =
            std::env::temp_dir().join(format!("fastpotify-render-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);

        let pages = [
            Page::Home,
            Page::TopSongs,
            Page::Search,
            Page::LikedSongs,
            Page::Albums,
            Page::Artists,
            Page::Podcasts,
            Page::Episodes,
            Page::Playlist("pl1".into()),
            Page::Playlist("missing".into()),
            Page::Album("alb0".into()),
            Page::Artist("art0".into()),
            Page::Show("sh0".into()),
            Page::Queue,
            Page::Settings,
        ];
        for page in pages {
            app.open(page.clone());
            for _ in 0..3 {
                frame(&ctx, &mut app);
            }
            assert_eq!(app.page(), &page);
        }
        app.settings.sidebar_visible = false;
        frame(&ctx, &mut app);
        app.settings.sidebar_visible = true;
        app.show_queue_panel = true;
        app.show_devices = true;
        frame(&ctx, &mut app);
        for dialog in [
            Dialog::Shortcuts,
            Dialog::CreatePlaylist {
                name: "x".into(),
                public: true,
                add_uris: vec![],
            },
            Dialog::EditPlaylist {
                id: "pl1".into(),
                name: "x".into(),
                description: String::new(),
                public: false,
            },
            Dialog::ConfirmDeletePlaylist {
                id: "pl1".into(),
                name: "x".into(),
                owned: true,
            },
        ] {
            app.dialog = Some(dialog);
            frame(&ctx, &mut app);
        }
        app.settings.theme = crate::settings::ThemeChoice::Light;
        app.actions.push(Action::SettingsChanged);
        app.open(Page::Home);
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }
        assert!(!app.palette.dark);
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// A drag in flight renders, and releasing it over an owned playlist
    /// row lands in the same add-to-playlist plumbing the row menu uses.
    #[test]
    fn dropping_a_song_on_a_sidebar_playlist_adds_it() {
        let root =
            std::env::temp_dir().join(format!("fastpotify-drag-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.open(Page::Playlist("pl1".into()));
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }

        // Sweep a held track down the sidebar; somewhere along the sweep
        // the pointer crosses an owned playlist row, and releasing there
        // must mark the playlist edit busy through the existing plumbing.
        // Where exactly the rows sit depends on the loaded fonts, so the
        // sweep does not hardcode a row position.
        let mut dropped = false;
        for step in 0..40 {
            let pos = egui::pos2(120.0, 120.0 + step as f32 * 15.0);
            egui::DragAndDrop::set_payload(
                &ctx,
                DragTrack {
                    uri: "spotify:track:trk0".into(),
                    title: "Fragments".into(),
                    image: None,
                    from: None,
                },
            );
            frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(!egui::DragAndDrop::has_any_payload(&ctx));
            if app.playlist_busy {
                dropped = true;
                break;
            }
        }
        assert!(dropped, "no sweep position landed on an owned playlist row");
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// Pins are pins: dropping a pinned row at the top of the block
    /// reorders the pins themselves, and the rest of the shelf stays in
    /// its automatic order.
    #[test]
    fn dragging_within_the_pinned_block_reorders_it() {
        let root =
            std::env::temp_dir().join(format!("fastpotify-reorder-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.settings.pinned_contexts =
            vec!["spotify:playlist:pl2".into(), "spotify:playlist:pl4".into()];
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }

        // Sweep from the top: the first slot inside the list drops the
        // dragged row right under Liked Songs. Where the list begins
        // depends on the loaded fonts, so the sweep does not hardcode it.
        let mut dropped = false;
        for step in 0..40 {
            let pos = egui::pos2(120.0, 100.0 + step as f32 * 10.0);
            egui::DragAndDrop::set_payload(
                &ctx,
                DragEntry {
                    uri: "spotify:playlist:pl4".into(),
                    title: "Release Radar".into(),
                    image: None,
                },
            );
            frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(&ctx);
            if app.settings.pinned_contexts.first().map(String::as_str)
                == Some("spotify:playlist:pl4")
            {
                dropped = true;
                break;
            }
        }
        assert!(dropped, "no sweep position landed in the pinned block");
        assert_eq!(
            app.settings.pinned_contexts,
            vec![
                "spotify:playlist:pl4".to_string(),
                "spotify:playlist:pl2".to_string(),
            ],
        );
        assert!(app.settings.sidebar_order.is_empty());
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// A drop between two unpinned playlists creates the custom order too:
    /// nothing was pinned, so the snapshot is simply the shelf as shown
    /// with the moved row in its new place.
    #[test]
    fn dropping_between_unpinned_playlists_creates_the_custom_order() {
        let root =
            std::env::temp_dir().join(format!("fastpotify-unpinned-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        assert!(app.settings.pinned_contexts.is_empty());
        assert!(app.settings.sidebar_order.is_empty());
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }

        // Sweep from the top; the first slot inside the list is the one
        // right under Liked Songs, between what were the first two
        // unpinned playlists.
        let mut dropped = false;
        for step in 0..40 {
            let pos = egui::pos2(120.0, 100.0 + step as f32 * 10.0);
            egui::DragAndDrop::set_payload(
                &ctx,
                DragEntry {
                    uri: "spotify:playlist:pl4".into(),
                    title: "Release Radar".into(),
                    image: None,
                },
            );
            frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(&ctx);
            if !app.settings.sidebar_order.is_empty() {
                dropped = true;
                break;
            }
        }
        assert!(dropped, "no sweep position landed below Liked Songs");
        let expected: Vec<String> = std::iter::once(4)
            .chain((0..PLAYLISTS.len()).filter(|index| *index != 4))
            .map(|index| format!("spotify:playlist:pl{index}"))
            .collect();
        assert_eq!(app.settings.sidebar_order, expected);
        assert!(app.settings.pinned_contexts.is_empty());
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// Dragging a row within an owned playlist's table moves it through
    /// the same MoveInPlaylist action the menu's move items use: the slot
    /// is Spotify's insert-before, which the handler mirrors locally
    /// before asking the server.
    #[test]
    fn dragging_a_row_within_a_playlist_reorders_it() {
        let root =
            std::env::temp_dir().join(format!("fastpotify-move-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.open(Page::Playlist("pl1".into()));
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }
        let order = |app: &App| -> Vec<String> {
            app.playlist_pages["pl1"]
                .items
                .items
                .iter()
                .filter_map(|item| item.playable().map(|playable| playable.uri().to_string()))
                .collect()
        };
        let original = order(&app);
        let from = 5usize;
        let held = |from: usize, uri: &str| DragTrack {
            uri: uri.to_string(),
            title: "Closer".into(),
            image: None,
            from: Some(("pl1".into(), from as u32)),
        };

        // Sweep the held row down the page; above the table nothing
        // bites, and the first slot inside it lands the row above its old
        // place. Where the table begins depends on the loaded fonts, so
        // the sweep does not hardcode it.
        let mut landed = None;
        for step in 0..45 {
            let pos = egui::pos2(700.0, 120.0 + step as f32 * 15.0);
            egui::DragAndDrop::set_payload(&ctx, held(from, &original[from]));
            frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(&ctx);
            if app.playlist_busy {
                landed = Some(pos);
                break;
            }
        }
        let landed = landed.expect("no sweep position landed inside the table");
        let drop_at = |ctx: &egui::Context, app: &mut App, payload: DragTrack| {
            egui::DragAndDrop::set_payload(ctx, payload);
            frame_events(ctx, app, vec![egui::Event::PointerMoved(landed)]);
            frame_events(
                ctx,
                app,
                vec![egui::Event::PointerButton {
                    pos: landed,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(ctx);
        };
        // The handler mirrored the move locally: the dragged row moved
        // up, everything else kept its order.
        let now = order(&app);
        let to = now
            .iter()
            .position(|uri| *uri == original[from])
            .expect("the dragged row vanished");
        assert!(to < from, "the row should have moved up, not to {to}");
        let mut expected = original.clone();
        let moved = expected.remove(from);
        expected.insert(to, moved);
        assert_eq!(now, expected);

        // Dropping the row on the same slot again moves nothing: the slot
        // is insert-before, so a row's own edges are a no-op. A slot sent
        // one row out would move it here.
        app.playlist_busy = false;
        drop_at(&ctx, &mut app, held(to, &expected[to]));
        assert!(!app.playlist_busy, "a row dropped on its own slot moved");
        assert_eq!(order(&app), expected);

        // A sorted view refuses the move: positions on screen no longer
        // match the server's.
        app.table_sorts.insert(
            Page::Playlist("pl1".into()),
            TableSort {
                column: SortColumn::Title,
                ascending: true,
            },
        );
        frame(&ctx, &mut app);
        drop_at(&ctx, &mut app, held(to, &expected[to]));
        assert!(!app.playlist_busy, "a sorted view accepted a move");
        assert_eq!(order(&app), expected);
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// The custom order is a setting like any other: it survives the trip
    /// through the settings file, and older files without it stay in the
    /// automatic order.
    #[test]
    fn custom_sidebar_order_round_trips_through_settings() {
        let settings = Settings {
            sidebar_order: vec![
                "spotify:playlist:pl4".to_string(),
                "spotify:playlist:pl0".to_string(),
            ],
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.sidebar_order, settings.sidebar_order);
        let older: Settings = serde_json::from_str("{}").unwrap();
        assert!(older.sidebar_order.is_empty());
    }
}

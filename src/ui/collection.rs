//! Playlist, album, and Liked Songs pages: a hero, actions, and a track table.

use egui::{Align, Layout, Rect, Sense, Vec2, pos2, vec2};

use crate::api::models::{Album, PlayableItem, Playlist, pick_image};
use crate::app::App;
use crate::model::{Action, Dialog, Loadable, Page, PagedList, RowContext};
use crate::theme::{self, Icon, Palette};
use crate::util;

use super::widgets::{self, TrackRow};

pub struct Hero<'a> {
    pub image: Option<&'a str>,
    pub liked: bool,
    pub kind: &'a str,
    pub title: &'a str,
    pub description: Option<String>,
    pub byline: Vec<(String, Option<Page>)>,
    pub round: bool,
}

pub fn hero(app: &mut App, ui: &mut egui::Ui, hero: Hero<'_>) {
    let palette = app.palette;
    ui.add_space(12.0);
    let cover_size = if ui.available_width() > 720.0 {
        212.0
    } else {
        160.0
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(cover_size), Sense::hover());
        let radius = if hero.round { cover_size / 2.0 } else { 6.0 };
        widgets::paint_shadow(ui, &palette, rect, radius);
        if hero.liked {
            super::sidebar::liked_cover(ui, rect, radius);
        } else {
            widgets::paint_cover(
                ui,
                &palette,
                hero.image,
                rect,
                radius,
                if hero.round { Icon::User } else { Icon::Music },
            );
        }
        ui.vertical(|ui| {
            let width = ui.available_width();
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add_space(cover_size * 0.08);
            theme::text(ui, hero.kind, theme::medium(12.5), palette.text);
            let mut size = if cover_size > 200.0 { 56.0 } else { 40.0 };
            loop {
                let galley = ui.painter().layout_no_wrap(
                    hero.title.to_string(),
                    theme::bold(size),
                    palette.text,
                );
                if galley.size().x <= width || size <= 22.0 {
                    break;
                }
                size -= 6.0;
            }
            ui.add(
                egui::Label::new(
                    egui::RichText::new(hero.title)
                        .font(theme::bold(size))
                        .color(palette.text),
                )
                .truncate()
                .selectable(false),
            );
            if let Some(description) = &hero.description
                && !description.is_empty()
            {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(description)
                            .font(theme::regular(13.5))
                            .color(palette.secondary),
                    )
                    .truncate()
                    .selectable(false),
                );
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (index, (text, page)) in hero.byline.iter().enumerate() {
                    if index > 0 {
                        theme::text(ui, "•", theme::regular(13.5), palette.secondary);
                    }
                    match page {
                        Some(page) => {
                            if theme::link(ui, text, theme::semibold(13.5), palette.text).clicked()
                            {
                                app.actions.push(Action::Open(page.clone()));
                            }
                        }
                        None => {
                            theme::text(ui, text, theme::regular(13.5), palette.secondary);
                        }
                    }
                }
            });
        });
    });
    ui.add_space(20.0);
}

pub struct Actions<'a> {
    pub play_uri: Option<String>,
    pub saved: Option<(String, bool)>,
    pub saved_icons: (Icon, Icon),
    pub saved_tooltips: (&'a str, &'a str),
    pub owned_playlist: Option<Playlist>,
    pub name: &'a str,
}

/// The big play button and its neighbours; returns the filter text if a
/// filter field was shown.
pub fn actions_row(
    app: &mut App,
    ui: &mut egui::Ui,
    actions: Actions<'_>,
    filter: Option<&mut String>,
) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        if let Some(uri) = &actions.play_uri {
            let now_playing_here = app.now_playing().is_some_and(|now| now.playing)
                && app
                    .remote
                    .as_ref()
                    .and_then(|remote| remote.state.context.as_ref())
                    .is_some_and(|context| context.uri == *uri);
            let icon = if now_playing_here {
                Icon::PauseFilled
            } else {
                Icon::PlayFilled
            };
            if app.play_pending(uri) {
                theme::circle_spinner(ui, 56.0, palette.accent, palette.on_accent, "Starting…");
            } else if theme::circle_button(
                ui,
                icon,
                56.0,
                palette.accent,
                palette.accent_hover,
                palette.on_accent,
                if now_playing_here { "Pause" } else { "Play" },
            )
            .clicked()
            {
                if now_playing_here {
                    app.actions.push(Action::TogglePlay);
                } else {
                    app.actions.push(Action::PlayContext {
                        uri: uri.clone(),
                        offset_uri: None,
                        offset_index: None,
                    });
                }
            }
            if theme::icon_button(
                ui,
                Icon::Shuffle,
                26.0,
                palette.secondary,
                palette.text,
                "Shuffle play",
            )
            .clicked()
            {
                app.actions.push(Action::ShufflePlay(uri.clone()));
            }
        }
        if let Some((uri, saved)) = &actions.saved {
            let (icon, tooltip, color) = if *saved {
                (
                    actions.saved_icons.1,
                    actions.saved_tooltips.1,
                    palette.accent,
                )
            } else {
                (
                    actions.saved_icons.0,
                    actions.saved_tooltips.0,
                    palette.secondary,
                )
            };
            if theme::icon_button(ui, icon, 26.0, color, palette.text, tooltip).clicked() {
                app.actions.push(Action::ToggleSaved(uri.clone()));
            }
        }
        if let Some(uri) = &actions.play_uri {
            let more = theme::icon_button(
                ui,
                Icon::Ellipsis,
                26.0,
                palette.secondary,
                palette.text,
                "More",
            );
            egui::Popup::menu(&more)
                .frame(widgets::menu_frame(&palette))
                .show(|ui| {
                    widgets::context_menu_items(
                        ui,
                        app,
                        uri,
                        actions.name,
                        actions.owned_playlist.as_ref(),
                    )
                });
        }
        if let Some(filter) = filter {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::search_field(
                    ui,
                    &palette,
                    egui::Id::new(("collection-filter", actions.name)),
                    filter,
                    "Filter",
                    220.0,
                );
            });
        }
    });
    ui.add_space(14.0);
}

/// A track table with virtualised rows and paging.
pub struct Table<'a> {
    pub items: &'a [(PlayableItem, Option<String>)],
    pub context: RowContext,
    pub show_album: bool,
    pub show_cover: bool,
    pub show_added: bool,
    pub page: Page,
    pub loading: bool,
    pub error: Option<&'a str>,
    pub can_load_more: bool,
    pub filter: &'a str,
}

pub fn table(app: &mut App, ui: &mut egui::Ui, table: Table<'_>) {
    let palette = app.palette;
    let needle = table.filter.trim().to_lowercase();
    let visible: Vec<usize> = table
        .items
        .iter()
        .enumerate()
        .filter(|(_, (item, _))| {
            if needle.is_empty() {
                return true;
            }
            let haystack = match item {
                PlayableItem::Track(track) => format!(
                    "{} {} {}",
                    track.name,
                    track.artist_names(),
                    track
                        .album
                        .as_ref()
                        .map(|album| album.name.as_str())
                        .unwrap_or("")
                ),
                PlayableItem::Episode(episode) => episode.name.clone(),
            };
            haystack.to_lowercase().contains(&needle)
        })
        .map(|(index, _)| index)
        .collect();

    if !table.items.is_empty() {
        widgets::table_header(
            ui,
            &palette,
            table.show_album,
            table.show_added,
            table.show_cover,
        );
    }
    let context = table.context.clone();
    widgets::virtual_rows(ui, visible.len(), theme::ROW_HEIGHT, |ui, row| {
        let index = visible[row];
        let (item, added_at) = &table.items[index];
        widgets::track_row(
            ui,
            app,
            TrackRow {
                index,
                number: Some(index + 1),
                item,
                context: &context,
                show_cover: table.show_cover,
                show_album: table.show_album,
                added_at: added_at.as_deref(),
                compact: false,
            },
        );
    });
    if table.loading {
        ui.add_space(8.0);
        widgets::loading_row(ui, &palette);
    }
    if let Some(error) = table.error {
        ui.add_space(8.0);
        widgets::error_row(ui, app, error, Some(table.page.clone()));
    }
    if table.items.is_empty() && !table.loading && table.error.is_none() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Music,
            "Nothing here yet",
            "Songs you add will show up here.",
        );
    } else if visible.is_empty() && !needle.is_empty() && table.can_load_more && !table.loading {
        // Filtering a partially loaded list: keep fetching so matches appear.
        app.actions.push(Action::LoadMore(table.page));
    } else {
        widgets::load_more_when_near_end(
            ui,
            app,
            table.page,
            table.can_load_more && !table.loading,
        );
    }
}

fn total_duration(items: &[(PlayableItem, Option<String>)]) -> u64 {
    items
        .iter()
        .map(|(item, _)| item.duration_ms() as u64)
        .sum()
}

fn items_of(
    list: &PagedList<crate::api::models::PlaylistItem>,
) -> Vec<(PlayableItem, Option<String>)> {
    list.items
        .iter()
        .filter_map(|item| {
            item.playable()
                .cloned()
                .map(|playable| (playable, item.added_at.clone()))
        })
        .collect()
}

/// A complete, ranked view of the listener's current top tracks.
pub fn top_songs(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(12.0);
    theme::text(ui, "Your top songs", theme::bold(30.0), palette.text);
    ui.add_space(4.0);
    theme::text(
        ui,
        "Your most-listened tracks from the last four weeks.",
        theme::regular(13.5),
        palette.secondary,
    );
    ui.add_space(18.0);

    let tracks = match &app.home.top_songs {
        Loadable::Loaded(tracks) => tracks.clone(),
        Loadable::Loading | Loadable::NotLoaded => {
            widgets::loading_row(ui, &palette);
            return;
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            widgets::error_row(ui, app, &error, Some(Page::TopSongs));
            return;
        }
    };
    let items: Vec<(PlayableItem, Option<String>)> = tracks
        .iter()
        .cloned()
        .map(|track| (PlayableItem::Track(track), None))
        .collect();
    let uris = tracks.into_iter().map(|track| track.uri).collect();
    table(
        app,
        ui,
        Table {
            items: &items,
            context: RowContext::Uris(uris),
            show_album: true,
            show_cover: true,
            show_added: false,
            page: Page::TopSongs,
            loading: app.home.top_songs_loading,
            error: None,
            can_load_more: false,
            filter: "",
        },
    );
}

pub fn playlist(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let Some(mut page) = app.playlist_pages.remove(id) else {
        app.ensure_loaded(Page::Playlist(id.to_string()));
        return;
    };
    let palette = app.palette;
    let user_id = app.user_id().unwrap_or("").to_string();
    match &page.playlist {
        Loadable::Loaded(playlist) => {
            let items = items_of(&page.items);
            let count = playlist.track_total().max(items.len() as u32);
            let mut byline = vec![(playlist.owner_name().to_string(), None)];
            let count_text = if page.items.is_complete() {
                format!(
                    "{} songs, {}",
                    util::format_count(count as u64),
                    util::format_total_ms(total_duration(&items))
                )
            } else {
                format!("{} songs", util::format_count(count as u64))
            };
            byline.push((count_text, None));
            hero(
                app,
                ui,
                Hero {
                    image: pick_image(&playlist.images, 300),
                    liked: false,
                    kind: if playlist.public == Some(true) {
                        "Public Playlist"
                    } else {
                        "Playlist"
                    },
                    title: &playlist.name,
                    description: playlist.description.as_deref().map(util::strip_html),
                    byline,
                    round: false,
                },
            );
            let owned = playlist.owned_by(&user_id);
            let saved = app.is_saved(&playlist.uri).unwrap_or(false);
            let playlist_clone = playlist.clone();
            actions_row(
                app,
                ui,
                Actions {
                    play_uri: Some(playlist.uri.clone()),
                    saved: (!owned).then(|| (playlist.uri.clone(), saved)),
                    saved_icons: (Icon::CirclePlus, Icon::CircleCheck),
                    saved_tooltips: ("Add to Your Library", "Remove from Your Library"),
                    owned_playlist: owned.then_some(playlist_clone),
                    name: &playlist.name,
                },
                Some(&mut page.filter),
            );
            let editable = (owned || playlist.collaborative)
                .then(|| (playlist.id.clone(), playlist.snapshot_id.clone()));
            table(
                app,
                ui,
                Table {
                    items: &items,
                    context: RowContext::Context {
                        uri: playlist.uri.clone(),
                        editable_playlist: editable,
                    },
                    show_album: true,
                    show_cover: true,
                    show_added: true,
                    page: Page::Playlist(id.to_string()),
                    loading: page.items.loading,
                    error: page.items.error.as_deref(),
                    can_load_more: page.items.can_load_more(),
                    filter: &page.filter,
                },
            );
        }
        Loadable::Loading | Loadable::NotLoaded => {
            ui.add_space(40.0);
            widgets::loading_row(ui, &palette);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            ui.add_space(40.0);
            widgets::error_row(ui, app, &error, Some(Page::Playlist(id.to_string())));
        }
    }
    app.playlist_pages.insert(id.to_string(), page);
}

pub fn album(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let Some(page) = app.album_pages.remove(id) else {
        app.ensure_loaded(Page::Album(id.to_string()));
        return;
    };
    let palette = app.palette;
    match &page.album {
        Loadable::Loaded(album) => {
            album_hero(app, ui, album, &page.tracks);
            let saved = app.is_saved(&album.uri).unwrap_or(false);
            actions_row(
                app,
                ui,
                Actions {
                    play_uri: Some(album.uri.clone()),
                    saved: Some((album.uri.clone(), saved)),
                    saved_icons: (Icon::CirclePlus, Icon::CircleCheck),
                    saved_tooltips: ("Save to Your Library", "Remove from Your Library"),
                    owned_playlist: None,
                    name: &album.name,
                },
                None,
            );
            let items: Vec<(PlayableItem, Option<String>)> = page
                .tracks
                .items
                .iter()
                .cloned()
                .map(|mut track| {
                    if track.album.is_none() {
                        track.album = Some(Album {
                            id: album.id.clone(),
                            name: album.name.clone(),
                            uri: album.uri.clone(),
                            images: album.images.clone(),
                            ..Album::default()
                        });
                    }
                    (PlayableItem::Track(track), None)
                })
                .collect();
            table(
                app,
                ui,
                Table {
                    items: &items,
                    context: RowContext::Context {
                        uri: album.uri.clone(),
                        editable_playlist: None,
                    },
                    show_album: false,
                    show_cover: false,
                    show_added: false,
                    page: Page::Album(id.to_string()),
                    loading: page.tracks.loading,
                    error: page.tracks.error.as_deref(),
                    can_load_more: page.tracks.can_load_more(),
                    filter: "",
                },
            );
            ui.add_space(24.0);
            if let Some(date) = &album.release_date {
                theme::text(
                    ui,
                    util::format_date(date),
                    theme::regular(12.5),
                    palette.secondary,
                );
            }
            for copyright in &album.copyrights {
                let prefix = if copyright.kind == "P" { "℗ " } else { "© " };
                let text = if copyright.text.starts_with('©')
                    || copyright.text.starts_with('℗')
                    || copyright.text.starts_with("(C)")
                    || copyright.text.starts_with("(P)")
                {
                    copyright.text.clone()
                } else {
                    format!("{prefix}{}", copyright.text)
                };
                theme::text(ui, text, theme::regular(11.5), palette.dim);
            }
        }
        Loadable::Loading | Loadable::NotLoaded => {
            ui.add_space(40.0);
            widgets::loading_row(ui, &palette);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            ui.add_space(40.0);
            widgets::error_row(ui, app, &error, Some(Page::Album(id.to_string())));
        }
    }
    app.album_pages.insert(id.to_string(), page);
}

fn album_hero(
    app: &mut App,
    ui: &mut egui::Ui,
    album: &Album,
    tracks: &PagedList<crate::api::models::Track>,
) {
    let mut byline: Vec<(String, Option<Page>)> = album
        .artists
        .iter()
        .map(|artist| (artist.name.clone(), artist.id.clone().map(Page::Artist)))
        .collect();
    if let Some(year) = album.year() {
        byline.push((year.to_string(), None));
    }
    let count = album.total_tracks.unwrap_or(tracks.items.len() as u32);
    let duration: u64 = tracks
        .items
        .iter()
        .map(|track| track.duration_ms as u64)
        .sum();
    let count_text = if tracks.is_complete() {
        format!("{count} songs, {}", util::format_total_ms(duration))
    } else {
        format!("{count} songs")
    };
    byline.push((count_text, None));
    hero(
        app,
        ui,
        Hero {
            image: pick_image(&album.images, 300),
            liked: false,
            kind: album.kind_label(),
            title: &album.name,
            description: None,
            byline,
            round: false,
        },
    );
}

pub fn liked(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let items: Vec<(PlayableItem, Option<String>)> = app
        .library
        .liked
        .items
        .iter()
        .map(|saved| {
            (
                PlayableItem::Track(saved.track.clone()),
                saved.added_at.clone(),
            )
        })
        .collect();
    let total = app.library.liked.total.unwrap_or(items.len() as u32);
    let user = app
        .user
        .as_ref()
        .map(|user| user.name().to_string())
        .unwrap_or_default();
    let count_text = if app.library.liked.is_complete() {
        format!(
            "{} songs, {}",
            util::format_count(total as u64),
            util::format_total_ms(total_duration(&items))
        )
    } else {
        format!("{} songs", util::format_count(total as u64))
    };
    hero(
        app,
        ui,
        Hero {
            image: None,
            liked: true,
            kind: "Playlist",
            title: "Liked Songs",
            description: None,
            byline: vec![(user, None), (count_text, None)],
            round: false,
        },
    );
    let collection_uri = app
        .user
        .as_ref()
        .map(|user| format!("spotify:user:{}:collection", user.id));
    let filter_id = egui::Id::new("liked-filter");
    let mut filter = ui
        .data(|data| data.get_temp::<String>(filter_id))
        .unwrap_or_default();
    actions_row(
        app,
        ui,
        Actions {
            play_uri: collection_uri.clone(),
            saved: None,
            saved_icons: (Icon::Heart, Icon::HeartFilled),
            saved_tooltips: ("", ""),
            owned_playlist: None,
            name: "Liked Songs",
        },
        Some(&mut filter),
    );
    ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
    let uris: Vec<String> = items
        .iter()
        .map(|(item, _)| item.uri().to_string())
        .collect();
    let context = match collection_uri {
        Some(uri) if app.library.liked.is_complete() => RowContext::Context {
            uri,
            editable_playlist: None,
        },
        _ => RowContext::Uris(uris),
    };
    let loading = app.library.liked.loading;
    let error = app.library.liked.error.clone();
    let can_load_more = app.library.liked.can_load_more();
    let _ = &palette;
    table(
        app,
        ui,
        Table {
            items: &items,
            context,
            show_album: true,
            show_cover: true,
            show_added: true,
            page: Page::LikedSongs,
            loading,
            error: error.as_deref(),
            can_load_more,
            filter: &filter,
        },
    );
}

#[allow(dead_code)]
fn playlist_dialog(app: &mut App, playlist: &Playlist) {
    app.actions.push(Action::ShowDialog(Dialog::EditPlaylist {
        id: playlist.id.clone(),
        name: playlist.name.clone(),
        description: playlist.description.clone().unwrap_or_default(),
        public: playlist.public.unwrap_or(false),
    }));
}

#[allow(dead_code)]
fn rect_after(ui: &egui::Ui, height: f32) -> Rect {
    let cursor = ui.cursor();
    Rect::from_min_size(
        pos2(cursor.left(), cursor.top()),
        vec2(ui.available_width(), height),
    )
}

#[allow(dead_code)]
fn palette_of(app: &App) -> Palette {
    app.palette
}

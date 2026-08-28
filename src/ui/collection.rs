//! Playlist, album, and Liked Songs pages: a hero, actions, and a track table.

use egui::{Align, Layout, Rect, Sense, UiBuilder, Vec2, pos2, vec2};

use crate::api::models::{Album, PlayableItem, Playlist, pick_image};
use crate::app::App;
use crate::model::{Action, Dialog, Loadable, Page, PagedList, RowContext, SortColumn, TableSort};
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
            let now_playing_here = app.playing_context_uri().as_deref() == Some(uri.as_str())
                && app.believed_playing();
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
            let context_here = app.playing_context_uri().as_deref() == Some(uri.as_str());
            let shuffling_here = context_here && app.playing_context_shuffle();
            if theme::icon_button(
                ui,
                Icon::Shuffle,
                26.0,
                if shuffling_here {
                    palette.accent
                } else {
                    palette.secondary
                },
                palette.text,
                if shuffling_here {
                    "Shuffle off"
                } else if context_here {
                    "Shuffle"
                } else {
                    "Shuffle play"
                },
            )
            .clicked()
            {
                if context_here {
                    app.actions.push(Action::SetShuffle(!shuffling_here));
                } else {
                    app.actions.push(Action::ShufflePlay(uri.clone()));
                }
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

/// One table row's data: the item, when it was added, who added it.
pub type TableItem = (PlayableItem, Option<String>, Option<String>);

/// A track table with virtualised rows and paging.
pub struct Table<'a> {
    pub items: &'a [TableItem],
    pub context: RowContext,
    pub show_album: bool,
    pub show_cover: bool,
    pub show_added: bool,
    pub show_added_by: bool,
    pub page: Page,
    pub loading: bool,
    pub error: Option<&'a str>,
    pub can_load_more: bool,
    pub filter: &'a str,
    pub sticky_title: Option<&'a str>,
    pub sticky_play_uri: Option<&'a str>,
}

#[derive(Clone)]
struct StickyHeader {
    title: String,
    play: StickyPlay,
    page: Page,
    show_album: bool,
    show_cover: bool,
    show_added: bool,
    show_added_by: bool,
    show_columns: bool,
}

#[derive(Clone)]
enum StickyPlay {
    Context(String),
    Uris(Vec<String>),
}

#[derive(Clone, Default)]
struct StickyState {
    header: Option<StickyHeader>,
    visible: bool,
}

fn sticky_id(page: &Page) -> egui::Id {
    egui::Id::new(("collection-sticky", page.encode()))
}

const STICKY_ACTIONS_HEIGHT: f32 = 58.0;
const STICKY_TABLE_HEIGHT: f32 = 88.0;

impl StickyHeader {
    fn height(&self) -> f32 {
        if self.show_columns {
            STICKY_TABLE_HEIGHT
        } else {
            STICKY_ACTIONS_HEIGHT
        }
    }
}

fn sticky_animation_id(page: &Page) -> egui::Id {
    egui::Id::new(("collection-sticky-visible", page.encode()))
}

pub(super) fn sticky_height(ui: &egui::Ui, page: &Page) -> f32 {
    let visible = ui
        .data(|data| data.get_temp::<StickyState>(sticky_id(page)))
        .is_some_and(|state| state.visible);
    let height = ui
        .data(|data| data.get_temp::<StickyState>(sticky_id(page)))
        .and_then(|state| state.header)
        .map_or(STICKY_TABLE_HEIGHT, |header| header.height());
    height
        * ui.ctx()
            .animate_bool_with_time(sticky_animation_id(page), visible, 0.15)
}

pub(super) fn clear_sticky(ui: &mut egui::Ui, page: &Page) {
    ui.data_mut(|data| {
        let mut state = data
            .get_temp::<StickyState>(sticky_id(page))
            .unwrap_or_default();
        state.visible = false;
        data.insert_temp(sticky_id(page), state);
    });
}

fn mark_sticky(ui: &mut egui::Ui, page: &Page, header: StickyHeader) {
    ui.data_mut(|data| {
        let mut state = data
            .get_temp::<StickyState>(sticky_id(page))
            .unwrap_or_default();
        state.header = Some(header);
        state.visible = true;
        data.insert_temp(sticky_id(page), state);
    });
}

pub(super) fn mark_artist_sticky(ui: &mut egui::Ui, page: Page, title: &str, uris: Vec<String>) {
    mark_sticky(
        ui,
        &page,
        StickyHeader {
            title: title.to_string(),
            play: StickyPlay::Uris(uris),
            page: page.clone(),
            show_album: false,
            show_cover: true,
            show_added: false,
            show_added_by: false,
            show_columns: false,
        },
    );
}

pub(super) fn show_sticky(
    app: &mut App,
    ui: &mut egui::Ui,
    page: &Page,
    viewport: Rect,
    backdrop: &super::PageBackdrop,
) {
    let state = ui
        .data(|data| data.get_temp::<StickyState>(sticky_id(page)))
        .unwrap_or_default();
    let progress = ui
        .ctx()
        .animate_bool_with_time(sticky_animation_id(page), state.visible, 0.15);
    let Some(header) = state.header else {
        return;
    };
    if progress <= 0.0 {
        return;
    }

    let height = header.height();
    let rect = Rect::from_min_size(
        pos2(viewport.left(), viewport.top() - height * (1.0 - progress)),
        vec2(viewport.width(), height),
    );
    let palette = app.palette;
    let mut shadow_ui = ui.new_child(UiBuilder::new().max_rect(rect));
    shadow_ui.shrink_clip_rect(viewport);
    shadow_ui.painter().add(
        egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: palette.shadow.gamma_multiply(progress),
        }
        .as_shape(rect, egui::CornerRadius::ZERO),
    );
    let mut backdrop_ui = ui.new_child(UiBuilder::new().max_rect(viewport));
    backdrop_ui.shrink_clip_rect(viewport.intersect(rect));
    backdrop.paint(&backdrop_ui);
    let mut sticky = ui.new_child(
        UiBuilder::new()
            .id_salt(("sticky-table", &header.page))
            .max_rect(rect.shrink2(vec2(widgets::PAGE_PADDING, 0.0))),
    );
    sticky.shrink_clip_rect(viewport);
    sticky.set_opacity(progress);
    sticky.add_space(4.0);
    sticky.horizontal(|ui| {
        let current = match &header.play {
            StickyPlay::Context(uri) => app.playing_context_uri().as_deref() == Some(uri),
            StickyPlay::Uris(uris) => {
                app.playing_context_uri().is_none()
                    && app
                        .now_playing()
                        .is_some_and(|now| uris.iter().any(|uri| uri == &now.uri))
            }
        };
        let playing = current && app.believed_playing();
        let pending = match &header.play {
            StickyPlay::Context(uri) => app.play_pending(uri),
            StickyPlay::Uris(uris) => uris.first().is_some_and(|uri| app.play_pending(uri)),
        };
        if pending {
            theme::circle_spinner(ui, 40.0, palette.accent, palette.on_accent, "Starting…");
        } else if theme::circle_button(
            ui,
            if playing {
                Icon::PauseFilled
            } else {
                Icon::PlayFilled
            },
            40.0,
            palette.accent,
            palette.accent_hover,
            palette.on_accent,
            if playing { "Pause" } else { "Play" },
        )
        .clicked()
        {
            app.actions.push(if current {
                Action::TogglePlay
            } else {
                match &header.play {
                    StickyPlay::Context(uri) => Action::PlayContext {
                        uri: uri.clone(),
                        offset_uri: None,
                        offset_index: None,
                    },
                    StickyPlay::Uris(uris) => Action::PlayUris {
                        uris: uris.clone(),
                        index: 0,
                    },
                }
            });
        }
        theme::text(ui, &header.title, theme::bold(24.0), palette.text);
    });
    if header.show_columns {
        let clicked = widgets::table_header(
            &mut sticky,
            &palette,
            header.show_album,
            header.show_added,
            header.show_added_by,
            header.show_cover,
            app.table_sorts.get(&header.page).copied(),
            false,
        );
        if let Some(column) = clicked {
            apply_sort(app, &header.page, column);
        }
    }
}

fn apply_sort(app: &mut App, page: &Page, column: SortColumn) {
    let sort = app.table_sorts.get(page).copied();
    let next = match sort {
        Some(sort) if sort.column == column && sort.ascending => Some(TableSort {
            column,
            ascending: false,
        }),
        Some(sort) if sort.column == column => None,
        _ => Some(TableSort {
            column,
            ascending: true,
        }),
    };
    match next {
        Some(sort) => {
            app.table_sorts.insert(page.clone(), sort);
            app.actions.push(Action::LoadMore(page.clone()));
        }
        None => {
            app.table_sorts.remove(page);
        }
    }
}

pub fn table(app: &mut App, ui: &mut egui::Ui, table: Table<'_>) {
    let palette = app.palette;
    let needle = table.filter.trim().to_lowercase();
    let visible: Vec<usize> = table
        .items
        .iter()
        .enumerate()
        .filter(|(_, (item, _, _))| {
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

    let sort = app.table_sorts.get(&table.page).copied();
    let mut visible = visible;
    if let Some(sort) = sort {
        let album_of = |item: &PlayableItem| match item {
            PlayableItem::Track(track) => track
                .album
                .as_ref()
                .map(|album| album.name.to_lowercase())
                .unwrap_or_default(),
            PlayableItem::Episode(_) => String::new(),
        };
        let duration_of = |item: &PlayableItem| match item {
            PlayableItem::Track(track) => track.duration_ms,
            PlayableItem::Episode(episode) => episode.duration_ms,
        };
        visible.sort_by(|a, b| {
            let (item_a, added_a, adder_a) = &table.items[*a];
            let (item_b, added_b, adder_b) = &table.items[*b];
            let ordering = match sort.column {
                SortColumn::Title => item_a
                    .name()
                    .to_lowercase()
                    .cmp(&item_b.name().to_lowercase()),
                SortColumn::Album => album_of(item_a).cmp(&album_of(item_b)),
                SortColumn::Added => added_a.cmp(added_b),
                SortColumn::AddedBy => adder_a
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .cmp(&adder_b.as_deref().unwrap_or_default().to_lowercase()),
                SortColumn::Duration => duration_of(item_a).cmp(&duration_of(item_b)),
            };
            if sort.ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });
    }

    let table_top = ui.cursor().top();
    let clicked = if !table.items.is_empty() {
        widgets::table_header(
            ui,
            &palette,
            table.show_album,
            table.show_added,
            table.show_added_by,
            table.show_cover,
            sort,
            true,
        )
    } else {
        None
    };
    // What is displayed is what plays: a sorted view plays in its own
    // order, as a plain list of tracks, and its rows cannot edit server
    // positions that no longer match the screen.
    let context = if sort.is_some() {
        RowContext::Uris(
            visible
                .iter()
                .map(|&index| table.items[index].0.uri().to_string())
                .collect(),
        )
    } else {
        table.context.clone()
    };
    let sorted = sort.is_some();
    widgets::virtual_rows(ui, visible.len(), theme::ROW_HEIGHT, |ui, row| {
        let index = visible[row];
        let (item, added_at, added_by) = &table.items[index];
        widgets::track_row(
            ui,
            app,
            TrackRow {
                index: if sorted { row } else { index },
                number: Some(if sorted { row + 1 } else { index + 1 }),
                item,
                context: &context,
                show_cover: table.show_cover,
                show_album: table.show_album,
                added_at: added_at.as_deref(),
                added_by: added_by.as_deref(),
                show_added_by: table.show_added_by,
                compact: false,
            },
        );
    });
    if table_top < ui.clip_rect().top()
        && let (Some(title), Some(uri)) = (table.sticky_title, table.sticky_play_uri)
    {
        mark_sticky(
            ui,
            &table.page,
            StickyHeader {
                title: title.to_string(),
                play: StickyPlay::Context(uri.to_string()),
                page: table.page.clone(),
                show_album: table.show_album,
                show_cover: table.show_cover,
                show_added: table.show_added,
                show_added_by: table.show_added_by,
                show_columns: true,
            },
        );
    }
    if let Some(column) = clicked {
        apply_sort(app, &table.page, column);
    }
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

fn total_duration(items: &[TableItem]) -> u64 {
    items
        .iter()
        .map(|(item, _, _)| item.duration_ms() as u64)
        .sum()
}

fn items_of(
    list: &PagedList<crate::api::models::PlaylistItem>,
    owner_id: Option<&str>,
    owner_name: &str,
    names: &std::collections::HashMap<String, Option<String>>,
) -> Vec<TableItem> {
    list.items
        .iter()
        .filter_map(|item| {
            let playable = item.playable().cloned()?;
            let adder = item
                .added_by
                .as_ref()
                .and_then(|user| user.id.as_deref())
                .map(|id| {
                    if Some(id) == owner_id {
                        owner_name.to_string()
                    } else {
                        names
                            .get(id)
                            .and_then(|name| name.clone())
                            .unwrap_or_else(|| id.to_string())
                    }
                });
            Some((playable, item.added_at.clone(), adder))
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
    let items: Vec<TableItem> = tracks
        .iter()
        .cloned()
        .map(|track| (PlayableItem::Track(track), None, None))
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
            show_added_by: false,
            page: Page::TopSongs,
            loading: app.home.top_songs_loading,
            error: None,
            can_load_more: false,
            filter: "",
            sticky_title: None,
            sticky_play_uri: None,
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
            let items = items_of(
                &page.items,
                playlist.owner.id.as_deref(),
                playlist.owner_name(),
                &app.user_names,
            );
            let count = playlist.track_total().max(items.len() as u32);
            // The legacy collaborative flag covers only old-style secret
            // collaborations; a playlist made together today is recognised
            // by who added its songs.
            let owner_id = playlist.owner.id.as_deref();
            let others = page
                .contributors
                .iter()
                .filter(|id| Some(id.as_str()) != owner_id)
                .count();
            let made_together = playlist.collaborative || others > 0;
            let mut byline = vec![(playlist.owner_name().to_string(), None)];
            if others > 0 {
                let named: Vec<String> = page
                    .contributors
                    .iter()
                    .filter(|id| Some(id.as_str()) != owner_id)
                    .filter_map(|id| app.user_names.get(id)?.clone())
                    .collect();
                byline.push((
                    if named.len() == others && others <= 2 {
                        format!("with {}", named.join(" and "))
                    } else if others == 1 {
                        "and 1 other".to_string()
                    } else {
                        format!("and {others} others")
                    },
                    None,
                ));
            }
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
                    kind: if made_together {
                        "Collaborative Playlist"
                    } else if playlist.public == Some(true) {
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
                    show_added_by: made_together,
                    page: Page::Playlist(id.to_string()),
                    loading: page.items.loading,
                    error: page.items.error.as_deref(),
                    can_load_more: page.items.can_load_more(),
                    filter: &page.filter,
                    sticky_title: Some(&playlist.name),
                    sticky_play_uri: Some(&playlist.uri),
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
            let items: Vec<TableItem> = page
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
                    (PlayableItem::Track(track), None, None)
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
                    show_added_by: false,
                    page: Page::Album(id.to_string()),
                    loading: page.tracks.loading,
                    error: page.tracks.error.as_deref(),
                    can_load_more: page.tracks.can_load_more(),
                    filter: "",
                    sticky_title: Some(&album.name),
                    sticky_play_uri: Some(&album.uri),
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
    let items: Vec<TableItem> = app
        .library
        .liked
        .items
        .iter()
        .map(|saved| {
            (
                PlayableItem::Track(saved.track.clone()),
                saved.added_at.clone(),
                None,
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
        .map(|(item, _, _)| item.uri().to_string())
        .collect();
    let context = match collection_uri.as_ref() {
        Some(uri) if app.library.liked.is_complete() => RowContext::Context {
            uri: uri.clone(),
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
            show_added_by: false,
            page: Page::LikedSongs,
            loading,
            error: error.as_deref(),
            can_load_more,
            filter: &filter,
            sticky_title: Some("Liked Songs"),
            sticky_play_uri: collection_uri.as_deref(),
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

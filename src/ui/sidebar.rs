//! The left panel: navigation and Your Library.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Dialog, DragEntry, DragTrack, Loadable, Page};
use crate::theme::{self, Icon, Palette};

const DEFAULT_ROW_HEIGHT: f32 = 60.0;
const COMPACT_ROW_HEIGHT: f32 = 32.0;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    Playlists,
    Albums,
    Artists,
    Podcasts,
}

struct Entry {
    image: Option<String>,
    name: String,
    subtitle: String,
    page: Page,
    uri: String,
    round: bool,
    liked: bool,
    owned: bool,
    playlist_index: Option<usize>,
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    // The traffic lights float over the top-left of the sidebar now, so the
    // first nav row has to start below them.
    let top = 12 + theme::titlebar_inset(ui.ctx()) as i8;
    let panel = egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(app.settings.sidebar_width)
        .size_range(210.0..=440.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.panel).inner_margin(Margin {
            left: 12,
            right: 8,
            top,
            bottom: 8,
        }));
    let response = panel.show(ui, |ui| {
        contents(app, ui);
    });
    let width = response.response.rect.width();
    if (width - app.settings.sidebar_width).abs() > 1.0 {
        app.settings.sidebar_width = width;
        app.actions.push(Action::SettingsChanged);
    }
}

fn nav_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    label: &str,
    active: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let color = if active || response.hovered() {
            palette.text
        } else {
            palette.secondary
        };
        let icon_rect =
            Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), Vec2::splat(22.0));
        icon.image(color, 22.0).paint_at(ui, icon_rect);
        ui.painter().text(
            pos2(rect.left() + 46.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            theme::bold(15.0),
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn contents(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let page = app.page().clone();
    ui.add_space(4.0);
    if nav_row(ui, &palette, Icon::House, "Home", page == Page::Home).clicked() {
        app.actions.push(Action::Open(Page::Home));
    }
    if nav_row(ui, &palette, Icon::Search, "Search", page == Page::Search).clicked() {
        app.actions.push(Action::FocusSearch);
    }
    ui.add_space(10.0);
    ui.painter().hline(
        ui.max_rect().x_range().shrink(4.0),
        ui.cursor().top(),
        egui::Stroke::new(1.0, palette.outline),
    );
    ui.add_space(10.0);

    let filter_id = egui::Id::new("sidebar-filter");
    let mut filter = ui
        .data(|data| data.get_temp::<Filter>(filter_id))
        .unwrap_or_default();
    let show_search_id = egui::Id::new("sidebar-show-search");
    let mut show_search = ui
        .data(|data| data.get_temp::<bool>(show_search_id))
        .unwrap_or(false);

    ui.horizontal(|ui| {
        ui.add_space(6.0);
        theme::icon(ui, Icon::Library, 22.0, palette.secondary);
        ui.add_space(2.0);
        theme::text(ui, "Your Library", theme::bold(15.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if theme::icon_button(
                ui,
                Icon::PanelLeft,
                16.0,
                palette.secondary,
                palette.text,
                "Hide sidebar (Cmd+B)",
            )
            .clicked()
            {
                app.actions.push(Action::ToggleSidebar);
            }
            // One item never deserved a menu: the plus creates directly.
            if theme::icon_button(
                ui,
                Icon::Plus,
                16.0,
                palette.secondary,
                palette.text,
                "Create a playlist",
            )
            .clicked()
            {
                app.actions.push(Action::ShowDialog(Dialog::CreatePlaylist {
                    name: String::new(),
                    public: false,
                    add_uris: Vec::new(),
                }));
            }
            if theme::icon_button(
                ui,
                Icon::Search,
                16.0,
                palette.secondary,
                palette.text,
                "Search Your Library",
            )
            .clicked()
            {
                show_search = !show_search;
                if show_search {
                    ui.memory_mut(|memory| memory.request_focus(egui::Id::new("sidebar-search")));
                } else {
                    app.library.filter.clear();
                }
            }
        });
    });
    ui.add_space(6.0);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (value, label) in [
            (Filter::Playlists, "Playlists"),
            (Filter::Albums, "Albums"),
            (Filter::Artists, "Artists"),
            (Filter::Podcasts, "Podcasts"),
        ] {
            if theme::soft_button(ui, &palette, None, label, filter == value).clicked() {
                filter = value;
            }
        }
    });
    ui.data_mut(|data| {
        data.insert_temp(filter_id, filter);
        data.insert_temp(show_search_id, show_search);
    });
    if show_search {
        ui.add_space(4.0);
        super::widgets::search_field(
            ui,
            &palette,
            egui::Id::new("sidebar-search"),
            &mut app.library.filter,
            "Search in Your Library",
            ui.available_width() - 4.0,
        );
    }
    ui.add_space(6.0);

    // Make sure the selected shelf is loading.
    match filter {
        Filter::Playlists => {}
        Filter::Albums => {
            if !app.library.albums.loaded_once && !app.library.albums.loading {
                app.actions.push(Action::LoadMore(Page::Albums));
            }
        }
        Filter::Artists => {
            if !app.library.artists.loaded_once && !app.library.artists.loading {
                app.actions.push(Action::LoadMore(Page::Artists));
            }
        }
        Filter::Podcasts => {
            if !app.library.shows.loaded_once && !app.library.shows.loading {
                app.actions.push(Action::LoadMore(Page::Podcasts));
            }
        }
    }

    let needle = app.library.filter.trim().to_lowercase();
    let user_id = app.user_id().unwrap_or("").to_string();
    let mut entries: Vec<Entry> = Vec::new();
    let mut loading = false;
    let mut error: Option<String> = None;
    let mut more_page: Option<Page> = None;
    match filter {
        Filter::Playlists => {
            if needle.is_empty() || "liked songs".contains(&needle) {
                entries.push(Entry {
                    image: None,
                    name: "Liked Songs".into(),
                    subtitle: match app.library.liked.total {
                        Some(total) => format!("Playlist • {total} songs"),
                        None => "Playlist".into(),
                    },
                    page: Page::LikedSongs,
                    uri: String::new(),
                    round: false,
                    liked: true,
                    owned: false,
                    playlist_index: None,
                });
            }
            match &app.library.playlists {
                Loadable::Loaded(playlists) => {
                    // Recently played first, the way Spotify orders its own
                    // sidebar; the rest keep the library's order.
                    let rank = |uri: &str| {
                        app.recent_contexts
                            .iter()
                            .position(|held| held == uri)
                            .unwrap_or(usize::MAX)
                    };
                    let mut ordered: Vec<_> = playlists.iter().enumerate().collect();
                    ordered.sort_by_key(|(index, playlist)| (rank(&playlist.uri), *index));
                    for (index, playlist) in ordered {
                        if !needle.is_empty() && !playlist.name.to_lowercase().contains(&needle) {
                            continue;
                        }
                        let owned = playlist.owned_by(&user_id);
                        entries.push(Entry {
                            image: pick_image(&playlist.images, 64).map(str::to_string),
                            name: playlist.name.clone(),
                            subtitle: format!("Playlist • {}", playlist.owner_name()),
                            page: Page::Playlist(playlist.id.clone()),
                            uri: playlist.uri.clone(),
                            round: false,
                            liked: false,
                            owned,
                            playlist_index: Some(index),
                        });
                    }
                }
                Loadable::Loading | Loadable::NotLoaded => loading = true,
                Loadable::Failed(message) => error = Some(message.clone()),
            }
        }
        Filter::Albums => {
            for saved in &app.library.albums.items {
                let album = &saved.album;
                if !needle.is_empty()
                    && !album.name.to_lowercase().contains(&needle)
                    && !album
                        .artists
                        .iter()
                        .any(|a| a.name.to_lowercase().contains(&needle))
                {
                    continue;
                }
                entries.push(Entry {
                    image: pick_image(&album.images, 64).map(str::to_string),
                    name: album.name.clone(),
                    subtitle: format!(
                        "{} • {}",
                        album.kind_label(),
                        crate::api::models::join_names(
                            album.artists.iter().map(|a| a.name.as_str())
                        )
                    ),
                    page: Page::Album(album.id.clone()),
                    uri: album.uri.clone(),
                    round: false,
                    liked: false,
                    owned: false,
                    playlist_index: None,
                });
            }
            loading = app.library.albums.loading && app.library.albums.items.is_empty();
            error = app.library.albums.error.clone();
            if app.library.albums.can_load_more() {
                more_page = Some(Page::Albums);
            }
        }
        Filter::Artists => {
            for artist in &app.library.artists.items {
                if !needle.is_empty() && !artist.name.to_lowercase().contains(&needle) {
                    continue;
                }
                entries.push(Entry {
                    image: pick_image(&artist.images, 64).map(str::to_string),
                    name: artist.name.clone(),
                    subtitle: "Artist".into(),
                    page: Page::Artist(artist.id.clone()),
                    uri: artist.uri.clone(),
                    round: true,
                    liked: false,
                    owned: false,
                    playlist_index: None,
                });
            }
            loading = app.library.artists.loading && app.library.artists.items.is_empty();
            error = app.library.artists.error.clone();
            if app.library.artists.can_load_more() {
                more_page = Some(Page::Artists);
            }
        }
        Filter::Podcasts => {
            for saved in &app.library.shows.items {
                let show = &saved.show;
                if !needle.is_empty() && !show.name.to_lowercase().contains(&needle) {
                    continue;
                }
                entries.push(Entry {
                    image: pick_image(&show.images, 64).map(str::to_string),
                    name: show.name.clone(),
                    subtitle: format!("Podcast • {}", show.publisher),
                    page: Page::Show(show.id.clone()),
                    uri: show.uri.clone(),
                    round: false,
                    liked: false,
                    owned: false,
                    playlist_index: None,
                });
            }
            loading = app.library.shows.loading && app.library.shows.items.is_empty();
            error = app.library.shows.error.clone();
            if app.library.shows.can_load_more() {
                more_page = Some(Page::Podcasts);
            }
        }
    }

    // Pinned entries sit on top, in the order they were pinned; Liked
    // Songs stays above them, and everyone else keeps their order. Once
    // the playlists shelf has an order of its own, that order wins there:
    // rows sit where they were dropped, and playlists the saved order has
    // not met yet, the newly created and followed, wait at the top.
    let pin_rank = |uri: &str| {
        app.settings
            .pinned_contexts
            .iter()
            .position(|held| held == uri)
            .unwrap_or(usize::MAX)
    };
    let custom_order = filter == Filter::Playlists && !app.settings.sidebar_order.is_empty();
    let saved_rank = |uri: &str| {
        app.settings
            .sidebar_order
            .iter()
            .position(|held| held == uri)
    };
    // Pins are pins, whatever orders the rest: Liked Songs, then the
    // pinned block, then everyone else by the listener's own order or,
    // failing one, by recency.
    entries.sort_by_key(|entry| {
        if entry.liked {
            (0, 0)
        } else {
            match pin_rank(&entry.uri) {
                usize::MAX if custom_order => match saved_rank(&entry.uri) {
                    Some(rank) => (3, rank),
                    None => (2, entry.playlist_index.unwrap_or(0)),
                },
                usize::MAX => (2, 0),
                rank => (1, rank),
            }
        }
    });
    // The zones a dragged row can land in: everything sits below Liked
    // Songs, and the pinned entries form one block right after it.
    let liked_rows = entries.iter().take_while(|entry| entry.liked).count();
    let pinned_rows = entries
        .iter()
        .filter(|entry| !entry.liked && pin_rank(&entry.uri) != usize::MAX)
        .count();
    let playing_context = app.playing_context_uri();
    let context_playing = app.believed_playing();
    let current_page = app.page().clone();

    egui::ScrollArea::vertical()
        .id_salt("sidebar-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if loading {
                super::widgets::loading_row(ui, &palette);
            }
            if let Some(error) = &error {
                super::widgets::error_row(ui, app, error, None);
            }
            if entries.is_empty() && !loading && error.is_none() {
                ui.add_space(12.0);
                theme::subtle(
                    ui,
                    &palette,
                    if needle.is_empty() {
                        "Nothing here yet."
                    } else {
                        "No matches."
                    },
                );
            }
            let compact = app.settings.sidebar_compact;
            let row_height = if compact {
                COMPACT_ROW_HEIGHT
            } else {
                DEFAULT_ROW_HEIGHT
            };
            // While something is in hand, find where it hangs up front:
            // neighbours shift before that row draws, so the spot cannot
            // be discovered row by row. The fixed row height makes it
            // arithmetic.
            let list_top = ui.cursor().top();
            let pointer = ui
                .ctx()
                .pointer_latest_pos()
                .filter(|pos| ui.clip_rect().contains(*pos));
            // A song in hand lands on a row, when that row can take one.
            let dragging_song = egui::DragAndDrop::has_payload_of_type::<DragTrack>(ui.ctx());
            let drop_target = dragging_song
                .then_some(pointer)
                .flatten()
                .map(|pos| ((pos.y - list_top) / row_height).floor())
                .filter(|row| *row >= 0.0 && *row < entries.len() as f32)
                .map(|row| row as usize)
                .filter(|row| entries[*row].liked || entries[*row].owned);
            // A sidebar row in hand lands between rows: the slot nearest
            // the pointer, never above Liked Songs.
            let reordering = egui::DragAndDrop::has_payload_of_type::<DragEntry>(ui.ctx());
            let reorder_slot = reordering.then_some(pointer).flatten().map(|pos| {
                (((pos.y - list_top) / row_height).round().max(0.0) as usize)
                    .clamp(liked_rows, entries.len())
            });
            super::widgets::virtual_rows(ui, entries.len(), row_height, |ui, index| {
                let entry = &entries[index];
                let droppable = entry.liked || entry.owned;
                let drop_hover = drop_target == Some(index);
                let active = entry.page == current_page;
                let playing = context_playing
                    && !entry.uri.is_empty()
                    && playing_context.as_deref() == Some(entry.uri.as_str());
                let pinned =
                    !entry.uri.is_empty() && app.settings.pinned_contexts.contains(&entry.uri);
                let (rect, response) = ui.allocate_exact_size(
                    vec2(ui.available_width(), row_height),
                    Sense::click_and_drag(),
                );
                // Past the drag threshold the row itself is in hand, to be
                // pinned into place; clicks and the context menu keep their
                // meaning. Liked Songs stays where it is.
                if !entry.liked
                    && !entry.uri.is_empty()
                    && response.drag_started_by(egui::PointerButton::Primary)
                {
                    egui::DragAndDrop::set_payload(
                        ui.ctx(),
                        DragEntry {
                            uri: entry.uri.clone(),
                            title: entry.name.clone(),
                            image: entry.image.clone(),
                        },
                    );
                }
                // Neighbours ease apart around the row that would take a
                // song, macOS style, and part at the slot a dragged row
                // would land in. Each row keeps one animated offset, which
                // also eases everything back after the drag ends.
                let shift = ui.ctx().animate_value_with_time(
                    ui.id().with(("drop-shift", index)),
                    if let Some(slot) = reorder_slot {
                        if index < slot { -4.0 } else { 4.0 }
                    } else {
                        match drop_target {
                            Some(target) if index < target => -4.0,
                            Some(target) if index > target => 4.0,
                            _ => 0.0,
                        }
                    },
                    0.12,
                );
                let rect = rect.translate(vec2(0.0, shift));
                if ui.is_rect_visible(rect) {
                    if active {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(6), palette.surface);
                    } else if response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(6),
                            palette.surface_hover.gamma_multiply(0.6),
                        );
                    }
                    if drop_hover {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(6),
                            palette.accent.gamma_multiply(0.18),
                        );
                        ui.painter().rect_stroke(
                            rect,
                            CornerRadius::same(6),
                            egui::Stroke::new(1.5, palette.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let name_color = if playing {
                        palette.accent
                    } else {
                        palette.text
                    };
                    if compact {
                        let text_left = rect.left() + 8.0;
                        let text_right = rect.right() - if playing || pinned { 28.0 } else { 8.0 };
                        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
                            pos2(text_left, rect.top()),
                            pos2(text_right, rect.bottom()),
                        ));
                        crate::bidi::paint_line(
                            &painter,
                            text_left,
                            text_right,
                            rect.center().y,
                            &entry.name,
                            theme::medium(13.5),
                            name_color,
                        );
                    } else {
                        let cover_rect = Rect::from_center_size(
                            pos2(rect.left() + 8.0 + 22.0, rect.center().y),
                            Vec2::splat(44.0),
                        );
                        if entry.liked {
                            liked_cover(ui, cover_rect, 6.0);
                        } else {
                            super::widgets::paint_cover(
                                ui,
                                &palette,
                                entry.image.as_deref(),
                                cover_rect,
                                if entry.round { 22.0 } else { 6.0 },
                                if entry.round { Icon::User } else { Icon::Music },
                            );
                        }
                        let text_left = cover_rect.right() + 12.0;
                        let text_right = rect.right() - if playing || pinned { 28.0 } else { 8.0 };
                        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
                            pos2(text_left, rect.top()),
                            pos2(text_right, rect.bottom()),
                        ));
                        crate::bidi::paint_line(
                            &painter,
                            text_left,
                            text_right,
                            rect.center().y - 9.0,
                            &entry.name,
                            theme::medium(14.0),
                            name_color,
                        );
                        crate::bidi::paint_line(
                            &painter,
                            text_left,
                            text_right,
                            rect.center().y + 10.0,
                            &entry.subtitle,
                            theme::regular(12.5),
                            palette.secondary,
                        );
                        // Hovering the art offers to play right from here.
                        let can_play = !entry.uri.is_empty() || entry.liked;
                        let play_response = can_play.then(|| {
                            ui.interact(
                                cover_rect,
                                ui.id().with(("sidebar-play", index)),
                                Sense::click(),
                            )
                        });
                        let play_hover = play_response.as_ref().is_some_and(|play| play.hovered());
                        if play_hover || (response.hovered() && can_play) {
                            ui.painter().rect_filled(
                                cover_rect,
                                CornerRadius::same(if entry.round { 22 } else { 6 }),
                                egui::Color32::from_black_alpha(120),
                            );
                            Icon::PlayFilled
                                .image(
                                    if play_hover {
                                        palette.accent
                                    } else {
                                        egui::Color32::WHITE
                                    },
                                    18.0,
                                )
                                .paint_at(
                                    ui,
                                    Rect::from_center_size(
                                        cover_rect.center()
                                            + theme::play_glyph_offset(Icon::PlayFilled, 18.0),
                                        Vec2::splat(18.0),
                                    ),
                                );
                            if let Some(play) = &play_response {
                                play.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
                            }
                        }
                        if play_response.is_some_and(|play| play.clicked()) {
                            let uri = if entry.liked {
                                app.user
                                    .as_ref()
                                    .map(|user| format!("spotify:user:{}:collection", user.id))
                            } else {
                                Some(entry.uri.clone())
                            };
                            if let Some(uri) = uri {
                                app.actions.push(Action::PlayContext {
                                    uri,
                                    offset_uri: None,
                                    offset_index: None,
                                });
                            }
                        }
                    }
                    if playing {
                        let icon_rect = Rect::from_center_size(
                            pos2(rect.right() - 16.0, rect.center().y),
                            Vec2::splat(16.0),
                        );
                        Icon::Volume2
                            .image(palette.accent, 16.0)
                            .paint_at(ui, icon_rect);
                    } else if pinned {
                        let icon_rect = Rect::from_center_size(
                            pos2(rect.right() - 16.0, rect.center().y),
                            Vec2::splat(13.0),
                        );
                        Icon::Pin
                            .image(palette.secondary, 13.0)
                            .paint_at(ui, icon_rect);
                    }
                    // Rows that cannot take the song step back a little.
                    if dragging_song && !droppable {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(6),
                            palette.panel.gamma_multiply(0.5),
                        );
                    }
                }
                if dragging_song
                    && droppable
                    && let Some(track) = response.dnd_release_payload::<DragTrack>()
                {
                    if entry.liked {
                        // Dropping on Liked Songs saves; a song already
                        // saved is left alone.
                        if app.is_saved(&track.uri) != Some(true) {
                            app.actions.push(Action::ToggleSaved(track.uri.clone()));
                        }
                    } else if let Page::Playlist(id) = &entry.page {
                        app.actions.push(Action::AddToPlaylist {
                            playlist_id: id.clone(),
                            playlist_name: entry.name.clone(),
                            uris: vec![track.uri.clone()],
                        });
                    }
                }
                if response.clicked() {
                    app.actions.push(Action::Open(entry.page.clone()));
                }
                if !entry.uri.is_empty() {
                    let owned_playlist = entry
                        .owned
                        .then_some(entry.playlist_index)
                        .flatten()
                        .and_then(|index| {
                            app.library
                                .playlists
                                .get()
                                .and_then(|list| list.get(index))
                                .cloned()
                        });
                    egui::Popup::context_menu(&response)
                        .frame(super::widgets::menu_frame(&palette))
                        .show(|ui| {
                            super::widgets::context_menu_items(
                                ui,
                                app,
                                &entry.uri,
                                &entry.name,
                                owned_playlist.as_ref(),
                            );
                            let pinned = app.settings.pinned_contexts.contains(&entry.uri);
                            if super::widgets::menu_item(
                                ui,
                                &palette,
                                Some(if pinned { Icon::PinOff } else { Icon::Pin }),
                                if pinned { "Unpin" } else { "Pin to top" },
                            ) {
                                if pinned {
                                    app.settings
                                        .pinned_contexts
                                        .retain(|held| held != &entry.uri);
                                } else {
                                    app.settings.pinned_contexts.push(entry.uri.clone());
                                    app.settings.sidebar_order.retain(|held| held != &entry.uri);
                                }
                                app.mark_settings_dirty();
                            }
                            if custom_order
                                && super::widgets::menu_item(
                                    ui,
                                    &palette,
                                    Some(Icon::Clock),
                                    "Sort by recently played",
                                )
                            {
                                // Dragging a row brings the listener's own
                                // order back, so this asks no confirmation.
                                app.settings.sidebar_order.clear();
                                app.mark_settings_dirty();
                            }
                        });
                } else if entry.liked {
                    egui::Popup::context_menu(&response)
                        .frame(super::widgets::menu_frame(&palette))
                        .show(|ui| {
                            if super::widgets::menu_item(ui, &palette, Some(Icon::Play), "Play")
                                && let Some(user) = &app.user
                            {
                                app.actions.push(Action::PlayContext {
                                    uri: format!("spotify:user:{}:collection", user.id),
                                    offset_uri: None,
                                    offset_index: None,
                                });
                            }
                        });
                }
                response.on_hover_cursor(egui::CursorIcon::PointingHand);
            });
            if let Some(slot) = reorder_slot {
                // A line in the gap the rows opened, so the eye lands
                // where the row will.
                let y = list_top + slot as f32 * row_height;
                ui.painter().hline(
                    ui.max_rect().x_range().shrink(6.0),
                    y,
                    egui::Stroke::new(2.0, palette.accent),
                );
                if ui.input(|input| input.pointer.any_released())
                    && let Some(drag) = egui::DragAndDrop::take_payload::<DragEntry>(ui.ctx())
                {
                    if filter == Filter::Playlists {
                        drop_playlist_row(app, &entries, liked_rows, pinned_rows, slot, &drag.uri);
                    } else {
                        drop_row(app, &entries, liked_rows, pinned_rows, slot, &drag.uri);
                    }
                }
            }
            if let Some(page) = more_page {
                super::widgets::load_more_when_near_end(ui, app, page, true);
            }
        });
}

/// A dropped playlist row lands in one of two worlds. Inside the pinned
/// block it pins, or reorders the pins, exactly where it fell. Below the
/// block it orders the rest: the first such drop snapshots the order on
/// screen so nothing jumps, and rows then sit where they are put. A
/// pinned row dropped below the block is unpinned.
fn drop_playlist_row(
    app: &mut App,
    entries: &[Entry],
    liked_rows: usize,
    pinned_rows: usize,
    slot: usize,
    uri: &str,
) {
    let section_end = liked_rows + pinned_rows;
    let was_pinned = app.settings.pinned_contexts.iter().any(|held| held == uri);
    if pinned_rows > 0 && slot < section_end {
        // Into the pinned block: the pinned entry the drop lands in front
        // of anchors the new pin position.
        let anchor = entries[liked_rows..section_end]
            .iter()
            .skip(slot.saturating_sub(liked_rows))
            .map(|entry| entry.uri.as_str())
            .find(|held| *held != uri)
            .map(str::to_string);
        let mut pinned = app.settings.pinned_contexts.clone();
        pinned.retain(|held| held != uri);
        let at = anchor
            .and_then(|anchor| pinned.iter().position(|held| *held == anchor))
            .unwrap_or(pinned.len());
        pinned.insert(at, uri.to_string());
        if pinned != app.settings.pinned_contexts {
            app.settings.pinned_contexts = pinned;
            app.settings.sidebar_order.retain(|held| held != uri);
            app.mark_settings_dirty();
        }
        return;
    }
    // Below the block: the rest of the shelf takes the listener's own
    // order, and a pinned row dropped here stops being pinned.
    if was_pinned {
        app.settings.pinned_contexts.retain(|held| held != uri);
        app.mark_settings_dirty();
        if app.settings.sidebar_order.is_empty() {
            // The automatic order stays automatic: the row returns to
            // living by recency.
            return;
        }
    }
    let mut order = full_playlist_order(app);
    let anchor = entries
        .iter()
        .skip(slot)
        .filter(|entry| !entry.liked)
        .map(|entry| entry.uri.as_str())
        .find(|held| *held != uri)
        .map(str::to_string);
    order.retain(|held| held != uri);
    let at = anchor
        .and_then(|anchor| order.iter().position(|held| *held == anchor))
        .unwrap_or(order.len());
    order.insert(at, uri.to_string());
    if order != app.settings.sidebar_order {
        app.settings.sidebar_order = order;
        app.mark_settings_dirty();
    }
}

/// Every loaded playlist in the order the shelf presents them when no
/// filter narrows the view: the saved order once one exists, with the
/// playlists it has not met yet first, otherwise the pinned block and
/// then recency. The saved order is rewritten from this, so it covers
/// the whole library rather than the rows that happened to be visible.
fn full_playlist_order(app: &App) -> Vec<String> {
    let Some(playlists) = app.library.playlists.get() else {
        return Vec::new();
    };
    let mut ordered: Vec<_> = playlists.iter().enumerate().collect();
    if app.settings.sidebar_order.is_empty() {
        let recent = |uri: &str| {
            app.recent_contexts
                .iter()
                .position(|held| held == uri)
                .unwrap_or(usize::MAX)
        };
        let pinned = |uri: &str| {
            app.settings
                .pinned_contexts
                .iter()
                .position(|held| held == uri)
        };
        ordered.sort_by_key(|(index, playlist)| match pinned(&playlist.uri) {
            Some(rank) => (0, rank, 0),
            None => (1, recent(&playlist.uri), *index),
        });
    } else {
        let saved = |uri: &str| {
            app.settings
                .sidebar_order
                .iter()
                .position(|held| held == uri)
        };
        ordered.sort_by_key(|(index, playlist)| match saved(&playlist.uri) {
            Some(rank) => (1, rank, 0),
            None => (0, *index, 0),
        });
    }
    ordered
        .into_iter()
        .map(|(_, playlist)| playlist.uri.clone())
        // Pins live in their own list; the saved order holds the rest.
        .filter(|uri| !app.settings.pinned_contexts.contains(uri))
        .collect()
}

/// A dropped album, artist, or podcast row lands in the pinned block:
/// within the block the drop position is its new pin order, and below the
/// block the row goes back to living by recency, so it simply stops being
/// pinned. Liked Songs is not part of the pinned list and never moves.
fn drop_row(
    app: &mut App,
    entries: &[Entry],
    liked_rows: usize,
    pinned_rows: usize,
    slot: usize,
    uri: &str,
) {
    let mut pinned = app.settings.pinned_contexts.clone();
    let section_end = liked_rows + pinned_rows;
    if slot <= section_end {
        // The pinned entry the drop lands in front of anchors the new
        // position, so entries pinned from another shelf keep theirs.
        let anchor = entries[liked_rows..section_end]
            .iter()
            .skip(slot - liked_rows)
            .map(|entry| entry.uri.as_str())
            .find(|held| *held != uri)
            .map(str::to_string);
        pinned.retain(|held| held != uri);
        let at = anchor
            .and_then(|anchor| pinned.iter().position(|held| *held == anchor))
            .unwrap_or(pinned.len());
        pinned.insert(at, uri.to_string());
    } else {
        pinned.retain(|held| held != uri);
    }
    if pinned != app.settings.pinned_contexts {
        app.settings.pinned_contexts = pinned;
        app.mark_settings_dirty();
    }
}

/// The purple-to-blue Liked Songs tile.
pub fn liked_cover(ui: &egui::Ui, rect: Rect, radius: f32) {
    let texture_id = egui::Id::new("liked-cover-gradient");
    let texture = ui
        .data(|data| data.get_temp::<egui::TextureHandle>(texture_id))
        .unwrap_or_else(|| {
            let size = 64;
            let lerp = |a: u8, b: u8, t: f32| (a as f32 + (b as f32 - a as f32) * t) as u8;
            let top_left = [0x45, 0x0a, 0xf5];
            let top_right = [0x6a, 0x3a, 0xe8];
            let bottom_left = [0x8e, 0x9f, 0xe5];
            let bottom_right = [0xc4, 0xef, 0xd9];
            let pixels = (0..size)
                .flat_map(|y| {
                    let y = y as f32 / (size - 1) as f32;
                    (0..size).map(move |x| {
                        let x = x as f32 / (size - 1) as f32;
                        egui::Color32::from_rgb(
                            lerp(
                                lerp(top_left[0], top_right[0], x),
                                lerp(bottom_left[0], bottom_right[0], x),
                                y,
                            ),
                            lerp(
                                lerp(top_left[1], top_right[1], x),
                                lerp(bottom_left[1], bottom_right[1], x),
                                y,
                            ),
                            lerp(
                                lerp(top_left[2], top_right[2], x),
                                lerp(bottom_left[2], bottom_right[2], x),
                                y,
                            ),
                        )
                    })
                })
                .collect();
            let texture = ui.ctx().load_texture(
                "liked-cover-gradient",
                egui::ColorImage::new([size, size], pixels),
                egui::TextureOptions::LINEAR,
            );
            ui.data_mut(|data| data.insert_temp(texture_id, texture.clone()));
            texture
        });
    egui::Image::new(&texture)
        .corner_radius(CornerRadius::same(radius.min(127.0) as u8))
        .paint_at(ui, rect);
    let size = rect.width() * 0.45;
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(size));
    Icon::HeartFilled
        .image(egui::Color32::WHITE, size)
        .paint_at(ui, icon_rect);
}

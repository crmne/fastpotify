//! The left panel: navigation and Your Library.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Dialog, Loadable, Page};
use crate::theme::{self, Icon, Palette};

const ROW_HEIGHT: f32 = 60.0;

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
    let panel = egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(app.settings.sidebar_width)
        .size_range(210.0..=440.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.panel).inner_margin(Margin {
            left: 12,
            right: 8,
            top: 12,
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
            let add = theme::icon_button(
                ui,
                Icon::Plus,
                18.0,
                palette.secondary,
                palette.text,
                "Create playlist",
            );
            egui::Popup::menu(&add)
                .frame(super::widgets::menu_frame(&palette))
                .show(|ui| {
                    if super::widgets::menu_item(
                        ui,
                        &palette,
                        Some(Icon::ListPlus),
                        "Create a new playlist",
                    ) {
                        app.actions.push(Action::ShowDialog(Dialog::CreatePlaylist {
                            name: String::new(),
                            public: false,
                            add_uris: Vec::new(),
                        }));
                    }
                });
            if theme::icon_button(
                ui,
                Icon::Search,
                17.0,
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
    // Songs stays above them, and everyone else keeps their order.
    let pin_rank = |uri: &str| {
        app.settings
            .pinned_contexts
            .iter()
            .position(|held| held == uri)
            .unwrap_or(usize::MAX)
    };
    entries.sort_by_key(|entry| {
        if entry.liked {
            (0, 0)
        } else {
            match pin_rank(&entry.uri) {
                usize::MAX => (2, 0),
                rank => (1, rank),
            }
        }
    });
    let playing_context = app.playing_context_uri();
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
            super::widgets::virtual_rows(ui, entries.len(), ROW_HEIGHT, |ui, index| {
                let entry = &entries[index];
                let active = entry.page == current_page;
                let playing =
                    !entry.uri.is_empty() && playing_context.as_deref() == Some(entry.uri.as_str());
                let pinned =
                    !entry.uri.is_empty() && app.settings.pinned_contexts.contains(&entry.uri);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
                if ui.is_rect_visible(rect) {
                    if active {
                        // The active row's pill sits 10px above the row and
                        // runs 10px short of the panel edge, matching the
                        // frame's inner margin.
                        let pill = Rect::from_min_max(
                            pos2(rect.left(), rect.top() + 8.0),
                            pos2(rect.right() - 4.0, rect.bottom() - 8.0),
                        );
                        ui.painter()
                            .rect_filled(pill, CornerRadius::same(6), palette.surface);
                        // A soft accent edge on the active row.
                        ui.painter().rect_filled(
                            Rect::from_min_size(
                                pos2(rect.left(), pill.top() + 6.0),
                                vec2(3.0, pill.height() - 12.0),
                            ),
                            CornerRadius::same(1),
                            palette.accent.gamma_multiply(0.55),
                        );
                    } else {
                        let hover_t = ui.ctx().animate_bool_with_time(
                            response.id.with("sidebar-row-hover"),
                            response.hovered(),
                            0.12,
                        );
                        if hover_t > 0.0 {
                            let pill = Rect::from_min_max(
                                pos2(rect.left(), rect.top() + 8.0),
                                pos2(rect.right() - 4.0, rect.bottom() - 8.0),
                            );
                            ui.painter().rect_filled(
                                pill,
                                CornerRadius::same(6),
                                palette
                                    .surface_hover
                                    .gamma_multiply(0.6 * theme::ease_out(hover_t) * 0.6 + 0.24),
                            );
                        }
                    }
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
                    let name_color = if playing {
                        palette.accent
                    } else {
                        palette.text
                    };
                    painter.text(
                        pos2(text_left, rect.center().y - 9.0),
                        egui::Align2::LEFT_CENTER,
                        &entry.name,
                        theme::medium(14.0),
                        name_color,
                    );
                    painter.text(
                        pos2(text_left, rect.center().y + 10.0),
                        egui::Align2::LEFT_CENTER,
                        &entry.subtitle,
                        theme::regular(12.5),
                        palette.secondary,
                    );
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
                        Icon::BookmarkFilled
                            .image(palette.secondary, 13.0)
                            .paint_at(ui, icon_rect);
                    }
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
                                Rect::from_center_size(cover_rect.center(), Vec2::splat(18.0)),
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
                                Some(if pinned {
                                    Icon::Bookmark
                                } else {
                                    Icon::BookmarkFilled
                                }),
                                if pinned { "Unpin" } else { "Pin to top" },
                            ) {
                                if pinned {
                                    app.settings
                                        .pinned_contexts
                                        .retain(|held| held != &entry.uri);
                                } else {
                                    app.settings.pinned_contexts.push(entry.uri.clone());
                                }
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
            if let Some(page) = more_page {
                super::widgets::load_more_when_near_end(ui, app, page, true);
            }
        });
}

/// The purple-to-blue Liked Songs tile.
pub fn liked_cover(ui: &egui::Ui, rect: Rect, radius: f32) {
    let top = egui::Color32::from_rgb(0x45, 0x0a, 0xf5);
    let bottom = egui::Color32::from_rgb(0xc4, 0xef, 0xd9);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), egui::Color32::from_rgb(0x6a, 0x3a, 0xe8));
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(
        rect.left_bottom(),
        egui::Color32::from_rgb(0x8e, 0x9f, 0xe5),
    );
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    let painter = ui.painter().with_clip_rect(rect);
    let _ = radius;
    painter.add(egui::Shape::mesh(mesh));
    let size = rect.width() * 0.45;
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(size));
    Icon::HeartFilled
        .image(egui::Color32::WHITE, size)
        .paint_at(ui, icon_rect);
}

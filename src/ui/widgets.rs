//! Widgets shared by every view: covers, cards, track rows, menus, sliders.

use egui::{
    Align, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};

use crate::api::models::*;
use crate::app::App;
use crate::model::{Action, Dialog, DragEntry, DragTrack, Page, RowContext};
use crate::theme::{self, Icon, Palette};
use crate::util;

pub const CARD_WIDTH: f32 = 172.0;
pub const CARD_GAP: f32 = 14.0;
pub const PAGE_PADDING: f32 = 24.0;

/// Draws an image (or a placeholder) in a square.
pub fn cover(
    ui: &mut Ui,
    palette: &Palette,
    url: Option<&str>,
    size: f32,
    radius: f32,
    fallback: Icon,
) -> Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_cover(ui, palette, url, rect, radius, fallback);
    rect
}

pub fn paint_cover(
    ui: &Ui,
    palette: &Palette,
    url: Option<&str>,
    rect: Rect,
    radius: f32,
    fallback: Icon,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let corner = CornerRadius::same(radius.min(127.0) as u8);
    let painter = ui.painter();
    let loaded = url.is_some_and(|url| {
        let image = egui::Image::new(url).show_loading_spinner(false);
        let Ok(egui::load::TexturePoll::Ready { texture }) =
            image.load_for_size(ui.ctx(), rect.size())
        else {
            return false;
        };

        let image_aspect = texture.size.x / texture.size.y;
        let rect_aspect = rect.width() / rect.height();
        let uv = if image_aspect > rect_aspect {
            let visible_width = rect_aspect / image_aspect;
            let inset = (1.0 - visible_width) / 2.0;
            Rect::from_min_max(pos2(inset, 0.0), pos2(1.0 - inset, 1.0))
        } else {
            let visible_height = image_aspect / rect_aspect;
            let inset = (1.0 - visible_height) / 2.0;
            Rect::from_min_max(pos2(0.0, inset), pos2(1.0, 1.0 - inset))
        };
        egui::Image::new(texture)
            .uv(uv)
            .corner_radius(corner)
            .paint_at(ui, rect);
        true
    });
    if !loaded {
        let fill = if palette.dark {
            palette.surface_hover
        } else {
            palette.surface_active
        };
        if radius >= rect.width() / 2.0 - 0.5 {
            painter.circle_filled(rect.center(), rect.width() / 2.0, fill);
        } else {
            painter.rect_filled(rect, corner, fill);
        }
        let icon_size = (rect.width() * 0.42).clamp(12.0, 64.0);
        theme::paint_icon(ui, fallback, rect, icon_size, palette.dim);
    }
}

/// A soft drop shadow under a cover or card.
pub fn paint_shadow(ui: &Ui, palette: &Palette, rect: Rect, radius: f32) {
    if !palette.dark {
        return;
    }
    let shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    ui.painter()
        .add(shadow.as_shape(rect, CornerRadius::same(radius as u8)));
}

/// Fills `rect` with a vertical gradient from `top` to `bottom`.
pub fn paint_vertical_gradient(ui: &Ui, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// Lays out only the rows that intersect the visible area of the enclosing
/// scroll view. Every row must occupy exactly `row_height`.
pub fn virtual_rows(
    ui: &mut Ui,
    count: usize,
    row_height: f32,
    mut row: impl FnMut(&mut Ui, usize),
) {
    if count == 0 {
        return;
    }
    let previous_spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.y = 0.0;
    let clip = ui.clip_rect();
    let start_y = ui.cursor().top();
    let width = ui.available_width();
    let first = (((clip.top() - start_y) / row_height).floor().max(0.0) as usize).min(count);
    let last = (((clip.bottom() - start_y) / row_height).ceil().max(0.0) as usize + 1).min(count);
    if first > 0 {
        ui.allocate_space(vec2(width, first as f32 * row_height));
    }
    for index in first..last {
        row(ui, index);
    }
    if last < count {
        ui.allocate_space(vec2(width, (count - last) as f32 * row_height));
    }
    ui.spacing_mut().item_spacing = previous_spacing;
}

/// Asks for the next page when the user scrolls near the end of a list.
pub fn load_more_when_near_end(ui: &Ui, app: &mut App, page: Page, can_load: bool) {
    if !can_load {
        return;
    }
    let clip = ui.clip_rect();
    let cursor = ui.cursor().top();
    if cursor - clip.bottom() < 900.0 {
        app.actions.push(Action::LoadMore(page));
    }
}

/// One entry in a popup menu. Closes the menu when chosen.
pub fn menu_item(ui: &mut Ui, palette: &Palette, icon: Option<Icon>, label: &str) -> bool {
    menu_item_enabled(ui, palette, icon, label, true)
}

pub fn menu_item_enabled(
    ui: &mut Ui,
    palette: &Palette,
    icon: Option<Icon>,
    label: &str,
    enabled: bool,
) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, 28.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if ui.is_rect_visible(rect) {
        if response.hovered() && enabled {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), palette.surface_hover);
        }
        let color = if enabled { palette.text } else { palette.dim };
        let mut x = rect.left() + 10.0;
        if let Some(icon) = icon {
            let icon_rect =
                Rect::from_center_size(pos2(x + 8.0, rect.center().y), Vec2::splat(16.0));
            icon.image(
                if enabled {
                    palette.secondary
                } else {
                    palette.dim
                },
                16.0,
            )
            .paint_at(ui, icon_rect);
            x += 26.0;
        }
        // A playlist can be named a paragraph; the label ends at the menu's
        // edge instead of running past it.
        let galley = crate::bidi::layout(
            ui.painter(),
            label,
            theme::regular(13.5),
            color,
            (rect.right() - 10.0 - x).max(0.0),
            1,
            Some(crate::bidi::ELLIPSIS),
        );
        let text_rect = Rect::from_min_max(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            pos2(rect.right() - 10.0, rect.center().y + galley.size().y / 2.0),
        );
        ui.painter()
            .galley(crate::bidi::galley_pos(text_rect, &galley), galley, color);
    }
    let clicked = enabled && response.clicked();
    if clicked {
        ui.close();
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    clicked
}

pub fn menu_separator(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(6.0),
        rect.center().y,
        Stroke::new(1.0, palette.outline),
    );
}

/// The frame every popup menu uses.
pub fn menu_frame(palette: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::same(6))
        .shadow(egui::epaint::Shadow {
            offset: [0, 6],
            blur: 20,
            spread: 0,
            color: palette.shadow,
        })
}

/// Everything a track (or episode) can be asked to do, as a menu.
pub fn item_menu(
    ui: &mut Ui,
    app: &mut App,
    item: &PlayableItem,
    context: Option<&RowContext>,
    index: Option<usize>,
) {
    let palette = app.palette;
    ui.set_min_width(220.0);
    ui.set_max_width(300.0);
    let uri = item.uri().to_string();
    let label = item.name().to_string();
    if menu_item(ui, &palette, Some(Icon::ListEnd), "Play next") {
        app.actions.push(Action::AddToQueue {
            uri: uri.clone(),
            label: label.clone(),
        });
    }
    if item.is_track() {
        let saved = app.is_saved(&uri).unwrap_or(false);
        let (icon, text) = if saved {
            (Icon::HeartFilled, "Remove from Liked Songs")
        } else {
            (Icon::Heart, "Save to Liked Songs")
        };
        if menu_item(ui, &palette, Some(icon), text) {
            app.actions.push(Action::ToggleSaved(uri.clone()));
        }
        let playlists = app.editable_playlists();
        ui.menu_button("Add to playlist", |ui| {
            ui.set_min_width(220.0);
            ui.set_max_width(300.0);
            if menu_item(ui, &palette, Some(Icon::Plus), "New playlist") {
                app.actions.push(Action::ShowDialog(Dialog::CreatePlaylist {
                    name: String::new(),
                    public: false,
                    add_uris: vec![uri.clone()],
                }));
            }
            if !playlists.is_empty() {
                menu_separator(ui, &palette);
            }
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (id, name) in &playlists {
                        if menu_item(ui, &palette, Some(Icon::ListMusic), name) {
                            app.actions.push(Action::AddToPlaylist {
                                playlist_id: id.clone(),
                                playlist_name: name.clone(),
                                uris: vec![uri.clone()],
                            });
                        }
                    }
                });
        });
    } else if menu_item(ui, &palette, Some(Icon::Bookmark), "Save episode") {
        app.actions.push(Action::ToggleSaved(uri.clone()));
    }
    if let Some(RowContext::Context {
        editable_playlist: Some((playlist_id, _)),
        ..
    }) = context
    {
        if let Some(index) = index {
            if index > 0 && menu_item(ui, &palette, Some(Icon::ChevronUp), "Move up") {
                app.actions.push(Action::MoveInPlaylist {
                    playlist_id: playlist_id.clone(),
                    from: index as u32,
                    to: index as u32 - 1,
                });
            }
            if menu_item(ui, &palette, Some(Icon::ChevronDown), "Move down") {
                app.actions.push(Action::MoveInPlaylist {
                    playlist_id: playlist_id.clone(),
                    from: index as u32,
                    to: index as u32 + 2,
                });
            }
        }
        if menu_item(ui, &palette, Some(Icon::Minus), "Remove from this playlist") {
            app.actions.push(Action::RemoveFromPlaylist {
                playlist_id: playlist_id.clone(),
                uris: vec![uri.clone()],
            });
        }
    }
    menu_separator(ui, &palette);
    match item {
        PlayableItem::Track(track) => {
            if menu_item(ui, &palette, Some(Icon::Radio), "Go to song radio") {
                app.actions.push(Action::PlayTrackRadio(uri.clone()));
            }
            let artists: Vec<&ArtistRef> = track
                .artists
                .iter()
                .filter(|artist| artist.id.is_some())
                .collect();
            if artists.len() == 1 {
                if menu_item(ui, &palette, Some(Icon::User), "Go to artist") {
                    app.actions.push(Action::Open(Page::Artist(
                        artists[0].id.clone().unwrap_or_default(),
                    )));
                }
            } else if artists.len() > 1 {
                ui.menu_button("Go to artist", |ui| {
                    ui.set_min_width(200.0);
                    for artist in &artists {
                        if menu_item(ui, &palette, Some(Icon::User), &artist.name) {
                            app.actions.push(Action::Open(Page::Artist(
                                artist.id.clone().unwrap_or_default(),
                            )));
                        }
                    }
                });
            }
            if let Some(album) = &track.album
                && !album.id.is_empty()
                && menu_item(ui, &palette, Some(Icon::Disc), "Go to album")
            {
                app.actions
                    .push(Action::Open(Page::Album(album.id.clone())));
            }
        }
        PlayableItem::Episode(episode) => {
            if let Some(show) = &episode.show
                && menu_item(ui, &palette, Some(Icon::Mic), "Go to podcast")
            {
                app.actions.push(Action::Open(Page::Show(show.id.clone())));
            }
        }
    }
    menu_separator(ui, &palette);
    if menu_item(ui, &palette, Some(Icon::Copy), "Copy link") {
        app.actions.push(Action::CopyLink(uri.clone()));
    }
    if menu_item(ui, &palette, Some(Icon::ExternalLink), "Open in Spotify") {
        app.actions.push(Action::OpenInSpotify(uri));
    }
}

/// Menu for a context (playlist, album, artist, show).
pub fn context_menu_items(
    ui: &mut Ui,
    app: &mut App,
    uri: &str,
    name: &str,
    owned_playlist: Option<&Playlist>,
) {
    let palette = app.palette;
    ui.set_min_width(200.0);
    ui.set_max_width(300.0);
    let kind = util::uri_kind(uri).unwrap_or("");
    if menu_item(ui, &palette, Some(Icon::Play), "Play") {
        app.actions.push(Action::PlayContext {
            uri: uri.to_string(),
            offset_uri: None,
            offset_index: None,
        });
    }
    if kind != "artist" && menu_item(ui, &palette, Some(Icon::Shuffle), "Shuffle play") {
        app.actions.push(Action::ShufflePlay(uri.to_string()));
    }
    if kind == "album" && menu_item(ui, &palette, Some(Icon::ListEnd), "Play next") {
        app.actions.push(Action::AddToQueue {
            uri: uri.to_string(),
            label: name.to_string(),
        });
    }
    let saved = app.is_saved(uri).unwrap_or(false);
    let (icon, text) = match (kind, saved) {
        ("artist", true) => (Icon::CircleX, "Unfollow"),
        ("artist", false) => (Icon::CirclePlus, "Follow"),
        (_, true) => (Icon::CircleX, "Remove from Your Library"),
        (_, false) => (Icon::CirclePlus, "Add to Your Library"),
    };
    if owned_playlist.is_none() && menu_item(ui, &palette, Some(icon), text) {
        app.actions.push(Action::ToggleSaved(uri.to_string()));
    }
    if let Some(playlist) = owned_playlist {
        if menu_item(ui, &palette, Some(Icon::Pencil), "Edit details") {
            app.actions.push(Action::ShowDialog(Dialog::EditPlaylist {
                id: playlist.id.clone(),
                name: playlist.name.clone(),
                description: playlist
                    .description
                    .clone()
                    .map(|d| util::strip_html(&d))
                    .unwrap_or_default(),
                public: playlist.public.unwrap_or(false),
            }));
        }
        if menu_item(ui, &palette, Some(Icon::Trash), "Delete") {
            app.actions
                .push(Action::ShowDialog(Dialog::ConfirmDeletePlaylist {
                    id: playlist.id.clone(),
                    name: playlist.name.clone(),
                    owned: true,
                }));
        }
    }
    menu_separator(ui, &palette);
    if menu_item(ui, &palette, Some(Icon::Copy), "Copy link") {
        app.actions.push(Action::CopyLink(uri.to_string()));
    }
    if menu_item(ui, &palette, Some(Icon::ExternalLink), "Open in Spotify") {
        app.actions.push(Action::OpenInSpotify(uri.to_string()));
    }
}

/// Describes one row of a track table.
pub struct TrackRow<'a> {
    pub index: usize,
    pub number: Option<usize>,
    pub item: &'a PlayableItem,
    pub context: &'a RowContext,
    pub show_cover: bool,
    pub show_album: bool,
    pub added_at: Option<&'a str>,
    /// Who put the song here, on playlists made together.
    pub added_by: Option<&'a str>,
    pub show_added_by: bool,
    pub compact: bool,
    /// One line for the name and the artists in a shorter row without the
    /// cover: the compact track list. `compact` stays the queue's narrow row.
    pub thin: bool,
    /// Vertical offset while rows part around the slot a dragged row
    /// would land in; 0.0 everywhere else.
    pub shift: f32,
}

/// Draw each credited artist separately so its Spotify id remains clickable.
pub(crate) fn artist_links(
    ui: &mut Ui,
    app: &mut App,
    artists: &[ArtistRef],
    font: egui::FontId,
    color: Color32,
) {
    let spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.x = 0.0;
    for (index, artist) in artists.iter().enumerate() {
        if index > 0 {
            theme::text(ui, ", ", font.clone(), color);
        }
        if let Some(id) = &artist.id {
            if theme::link(ui, &artist.name, font.clone(), color).clicked() {
                app.actions.push(Action::Open(Page::Artist(id.clone())));
            }
        } else {
            theme::text(ui, &artist.name, font.clone(), color);
        }
    }
    ui.spacing_mut().item_spacing = spacing;
}

/// Column widths of the track table, computed from the available width.
struct Columns {
    number: f32,
    cover: f32,
    album: f32,
    added_by: f32,
    added: f32,
    heart: f32,
    duration: f32,
    more: f32,
}

fn columns(width: f32, row: &TrackRow<'_>) -> Columns {
    let extra_wide = width > 920.0;
    let wide = width > 760.0;
    let medium = width > 560.0;
    Columns {
        number: if row.compact { 0.0 } else { 44.0 },
        cover: if row.show_cover {
            if row.compact { 44.0 } else { 52.0 }
        } else {
            0.0
        },
        album: if row.show_album && medium {
            (width * 0.28).clamp(140.0, 360.0)
        } else {
            0.0
        },
        added_by: if row.show_added_by && extra_wide {
            130.0
        } else {
            0.0
        },
        added: if row.added_at.is_some() && wide {
            120.0
        } else {
            0.0
        },
        heart: if row.compact { 0.0 } else { 36.0 },
        duration: if row.compact { 44.0 } else { 56.0 },
        more: if row.compact { 0.0 } else { 36.0 },
    }
}

/// Draws a track row; pushes actions for what the user did.
pub fn track_row(ui: &mut Ui, app: &mut App, row: TrackRow<'_>) {
    let palette = app.palette;
    let row_height = if row.thin {
        theme::THIN_ROW_HEIGHT
    } else if row.compact {
        theme::COMPACT_ROW_HEIGHT
    } else {
        theme::ROW_HEIGHT
    };
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, row_height), Sense::click_and_drag());
    let rect = rect.translate(vec2(0.0, row.shift));
    if !ui.is_rect_visible(rect) {
        return;
    }
    // Moving past the drag threshold puts the track in hand for the sidebar
    // to catch. egui tells clicks and drags apart by that threshold, so
    // single click, double click, and the context menu stay as they were.
    if row.item.is_track() && response.drag_started_by(egui::PointerButton::Primary) {
        // A drag that begins on an editable playlist's own row remembers
        // where, so that playlist's table can move the row while every
        // other target keeps treating the drop as a copy.
        let from = match row.context {
            RowContext::Context {
                editable_playlist: Some((id, _)),
                ..
            } => Some((id.clone(), row.index as u32)),
            _ => None,
        };
        egui::DragAndDrop::set_payload(
            ui.ctx(),
            DragTrack {
                uri: row.item.uri().to_string(),
                title: row.item.name().to_string(),
                image: row.item.image(64).map(str::to_string),
                from,
            },
        );
    }
    // A queue row is a position, not the song itself: the same song can
    // sit in the queue while it plays (a repeat wrapping around, a song
    // queued twice), and only the Now playing row is the playing one.
    let is_current = !matches!(row.context, RowContext::Queue)
        && app
            .current_track_uri()
            .is_some_and(|uri| uri == row.item.uri());
    let playing = is_current && app.believed_playing();
    let hovered = ui.rect_contains_pointer(rect);
    let unavailable = match row.item {
        PlayableItem::Track(track) => track.is_playable == Some(false) || track.is_local,
        PlayableItem::Episode(_) => false,
    };

    if hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            palette
                .surface_hover
                .gamma_multiply(if palette.dark { 0.7 } else { 1.0 }),
        );
    }
    let cols = columns(width, &row);
    let painter = ui.painter().clone();
    let mut x = rect.left() + 8.0;

    // Number / play.
    if cols.number > 0.0 {
        let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(cols.number, row_height));
        if app.play_pending(row.item.uri()) {
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(cell)
                    .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
            );
            theme::spinner(&mut child, 16.0, palette.accent);
        } else if hovered && !unavailable {
            let icon = if playing {
                Icon::PauseFilled
            } else {
                Icon::PlayFilled
            };
            theme::paint_icon(ui, icon, cell, 14.0, palette.text);
        } else if playing {
            theme::paint_icon(ui, Icon::AudioLines, cell, 16.0, palette.accent);
        } else {
            let color = if is_current {
                palette.accent
            } else {
                palette.secondary
            };
            let label = row.number.unwrap_or(row.index + 1).to_string();
            painter.text(
                cell.center(),
                egui::Align2::CENTER_CENTER,
                label,
                theme::regular(14.0),
                color,
            );
        }
        x += cols.number;
    }
    // Cover.
    if cols.cover > 0.0 {
        let size = if row.compact { 36.0 } else { 40.0 };
        let cover_rect = Rect::from_center_size(
            pos2(x + size / 2.0 + 2.0, rect.center().y),
            Vec2::splat(size),
        );
        paint_cover(
            ui,
            &palette,
            row.item.image(64),
            cover_rect,
            4.0,
            if row.item.is_track() {
                Icon::Music
            } else {
                Icon::Mic
            },
        );
        // Without a number column the cover carries the play control:
        // hover shows it, a click uses it, and what plays shows there.
        if cols.number == 0.0 {
            let scrim = |alpha: u8| {
                painter.rect_filled(
                    cover_rect,
                    CornerRadius::same(4),
                    Color32::from_black_alpha(alpha),
                );
            };
            if app.play_pending(row.item.uri()) {
                scrim(140);
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(cover_rect)
                        .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
                );
                theme::spinner(&mut child, 16.0, Color32::WHITE);
            } else if hovered && !unavailable {
                scrim(140);
                let icon = if playing {
                    Icon::PauseFilled
                } else {
                    Icon::PlayFilled
                };
                theme::paint_icon(ui, icon, cover_rect, 16.0, Color32::WHITE);
            } else if playing {
                scrim(110);
                theme::paint_icon(ui, Icon::AudioLines, cover_rect, 16.0, palette.accent);
            }
        }
        x += cols.cover;
    }
    let right_fixed = cols.heart + cols.duration + cols.more + 8.0;
    let text_right = rect.right() - right_fixed - cols.added - cols.added_by - cols.album;
    let title_rect =
        Rect::from_min_max(pos2(x, rect.top()), pos2(text_right - 12.0, rect.bottom()));

    // Title and artists.
    let title_color = if unavailable {
        palette.dim
    } else if is_current {
        palette.accent
    } else {
        palette.text
    };
    let subtitle_color = if hovered {
        palette.text
    } else {
        palette.secondary
    };
    if row.thin {
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(title_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        child.set_clip_rect(title_rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing = vec2(6.0, 0.0);
        theme::text(
            &mut child,
            row.item.name(),
            theme::medium(14.0),
            title_color,
        );
        match row.item {
            PlayableItem::Track(track) => {
                if track.explicit {
                    explicit_badge(&mut child, &palette);
                }
                theme::text(
                    &mut child,
                    "•",
                    theme::regular(12.0),
                    palette.secondary.gamma_multiply(0.6),
                );
                artist_links(
                    &mut child,
                    app,
                    &track.artists,
                    theme::regular(13.0),
                    subtitle_color,
                );
                if let Some(added) = row.added_at.filter(|a| !a.starts_with("1970-01-01"))
                    && cols.added == 0.0
                {
                    let label = util::format_relative_date(added, jiff::Timestamp::now());
                    theme::text(
                        &mut child,
                        "•",
                        theme::regular(12.0),
                        palette.secondary.gamma_multiply(0.6),
                    );
                    theme::text(&mut child, &label, theme::regular(12.0), palette.secondary);
                    if label.ends_with(" ago") {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs(1));
                    }
                }
            }
            PlayableItem::Episode(episode) => {
                let subtitle = episode
                    .show
                    .as_ref()
                    .map(|show| show.name.clone())
                    .unwrap_or_default();
                if !subtitle.is_empty() {
                    theme::text(
                        &mut child,
                        "•",
                        theme::regular(12.0),
                        palette.secondary.gamma_multiply(0.6),
                    );
                    let show_id = episode.show.as_ref().map(|show| show.id.clone());
                    let response =
                        theme::link(&mut child, subtitle, theme::regular(13.0), subtitle_color);
                    if response.clicked()
                        && let Some(id) = show_id
                    {
                        app.actions.push(Action::Open(Page::Show(id)));
                    }
                }
                if let Some(added) = row.added_at.filter(|a| !a.starts_with("1970-01-01"))
                    && cols.added == 0.0
                {
                    theme::text(
                        &mut child,
                        "•",
                        theme::regular(12.0),
                        palette.secondary.gamma_multiply(0.6),
                    );
                    let label = util::format_relative_date(added, jiff::Timestamp::now());
                    theme::text(&mut child, &label, theme::regular(12.0), palette.secondary);
                    if label.ends_with(" ago") {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs(1));
                    }
                }
            }
        }
    } else {
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(title_rect)
                .layout(Layout::top_down(Align::LEFT)),
        );
        child.set_clip_rect(title_rect.intersect(ui.clip_rect()));
        child.spacing_mut().item_spacing = vec2(6.0, 1.0);
        child.spacing_mut().interact_size.y = 16.0;
        let vertical_pad = ((row_height - 37.0) / 2.0).max(4.0);
        child.add_space(vertical_pad);
        child.horizontal(|ui| {
            ui.set_max_width(title_rect.width());
            theme::text(ui, row.item.name(), theme::medium(14.5), title_color);
        });
        child.horizontal(|ui| {
            ui.set_max_width(title_rect.width());
            match row.item {
                PlayableItem::Track(track) => {
                    if track.explicit {
                        explicit_badge(ui, &palette);
                    }
                    artist_links(
                        ui,
                        app,
                        &track.artists,
                        theme::regular(12.5),
                        subtitle_color,
                    );
                    if let Some(added) = row.added_at.filter(|a| !a.starts_with("1970-01-01"))
                        && cols.added == 0.0
                    {
                        theme::text(
                            ui,
                            "•",
                            theme::regular(12.0),
                            palette.secondary.gamma_multiply(0.6),
                        );
                        let label = util::format_relative_date(added, jiff::Timestamp::now());
                        theme::text(ui, &label, theme::regular(12.0), palette.secondary);
                        if label.ends_with(" ago") {
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_secs(1));
                        }
                    }
                }
                PlayableItem::Episode(episode) => {
                    let subtitle = episode
                        .show
                        .as_ref()
                        .map(|show| show.name.clone())
                        .unwrap_or_default();
                    let show_id = episode.show.as_ref().map(|show| show.id.clone());
                    let response = theme::link(ui, subtitle, theme::regular(12.5), subtitle_color);
                    if response.clicked()
                        && let Some(id) = show_id
                    {
                        app.actions.push(Action::Open(Page::Show(id)));
                    }
                    if let Some(added) = row.added_at.filter(|a| !a.starts_with("1970-01-01"))
                        && cols.added == 0.0
                    {
                        theme::text(
                            ui,
                            "•",
                            theme::regular(12.0),
                            palette.secondary.gamma_multiply(0.6),
                        );
                        let label = util::format_relative_date(added, jiff::Timestamp::now());
                        theme::text(ui, &label, theme::regular(12.0), palette.secondary);
                        if label.ends_with(" ago") {
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_secs(1));
                        }
                    }
                }
            }
        });
    }
    x = text_right;

    // Album.
    if cols.album > 0.0 {
        if let PlayableItem::Track(track) = row.item
            && let Some(album) = &track.album
        {
            let album_rect = Rect::from_min_max(
                pos2(x, rect.top()),
                pos2(x + cols.album - 12.0, rect.bottom()),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(album_rect)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            child.set_clip_rect(album_rect.intersect(ui.clip_rect()));
            let response = theme::link(
                &mut child,
                album.name.clone(),
                theme::regular(13.0),
                subtitle_color,
            );
            if response.clicked() && !album.id.is_empty() {
                app.actions
                    .push(Action::Open(Page::Album(album.id.clone())));
            }
        }
        x += cols.album;
    }
    // Added by.
    if cols.added_by > 0.0 {
        if let Some(adder) = row.added_by {
            let cell = Rect::from_min_max(
                pos2(x, rect.top()),
                pos2(x + cols.added_by - 12.0, rect.bottom()),
            );
            let clipped = painter.with_clip_rect(cell.intersect(ui.clip_rect()));
            crate::bidi::paint_line(
                &clipped,
                cell.left(),
                cell.right(),
                cell.center().y,
                adder,
                theme::regular(13.0),
                palette.secondary,
            );
        }
        x += cols.added_by;
    }
    // Date added.
    if cols.added > 0.0 {
        // Spotify stamps the epoch on dates it never recorded; an empty
        // cell is truer than January 1970.
        if let Some(added) = row
            .added_at
            .filter(|added| !added.starts_with("1970-01-01"))
        {
            let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(cols.added, row_height));
            let label = util::format_relative_date(added, jiff::Timestamp::now());
            painter.text(
                pos2(cell.left(), cell.center().y),
                egui::Align2::LEFT_CENTER,
                &label,
                theme::regular(13.0),
                palette.secondary,
            );
            // Relative labels cross a boundary while the table is idle, so
            // keep the visible value in step with the clock.
            if label.ends_with(" ago") {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs(1));
            }
        }
        x += cols.added;
    }

    // Heart.
    if cols.heart > 0.0 {
        let saved = app.is_saved(row.item.uri());
        let heart_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(cols.heart, row_height));
        if row.item.is_track() && (hovered || saved == Some(true)) {
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(heart_rect)
                    .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
            );
            let (icon, color) = if saved == Some(true) {
                (Icon::HeartFilled, palette.accent)
            } else {
                (Icon::Heart, palette.secondary)
            };
            let tooltip = if saved == Some(true) {
                "Remove from Liked Songs"
            } else {
                "Save to Liked Songs"
            };
            if theme::icon_button(&mut child, icon, 16.0, color, palette.text, tooltip).clicked() {
                app.actions
                    .push(Action::ToggleSaved(row.item.uri().to_string()));
            }
        }
        x += cols.heart;
    }

    // Duration.
    let duration_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(cols.duration, row_height));
    painter.text(
        pos2(duration_rect.right() - 6.0, duration_rect.center().y),
        egui::Align2::RIGHT_CENTER,
        util::format_duration_ms(row.item.duration_ms()),
        theme::regular(13.0),
        palette.secondary,
    );
    x += cols.duration;

    // More.
    // The row's menu stays alive while it is open: when the button existed
    // only on a hovered row, the pointer's trip to the menu could leave
    // the row and close it before anything was clicked.
    let menu_id = ui.id().with(("row-menu", row.index));
    if cols.more > 0.0 && (hovered || egui::Popup::is_id_open(ui.ctx(), menu_id)) {
        let more_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(cols.more, row_height));
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(more_rect)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        let more = theme::icon_button(
            &mut child,
            Icon::Ellipsis,
            18.0,
            palette.secondary,
            palette.text,
            "More",
        );
        egui::Popup::menu(&more)
            .id(menu_id)
            .frame(menu_frame(&palette))
            .show(|ui| item_menu(ui, app, row.item, Some(row.context), Some(row.index)));
    }

    // Row interactions.
    if response.double_clicked() && !unavailable {
        app.actions.push(Action::PlayFromRow {
            context: row.context.clone(),
            uri: row.item.uri().to_string(),
            index: row.index as u32,
        });
    } else if response.clicked() {
        // The cell that holds the play control: the number column when
        // there is one, the cover when there is not.
        let control = if cols.number > 0.0 {
            Some(vec2(cols.number, row_height))
        } else if cols.cover > 0.0 {
            Some(vec2(cols.cover, row_height))
        } else {
            None
        };
        if let Some(size) = control {
            let control_rect = Rect::from_min_size(pos2(rect.left() + 8.0, rect.top()), size);
            if response
                .interact_pointer_pos()
                .is_some_and(|pos| control_rect.contains(pos))
                && !unavailable
            {
                if is_current {
                    app.actions.push(Action::TogglePlay);
                } else {
                    app.actions.push(Action::PlayFromRow {
                        context: row.context.clone(),
                        uri: row.item.uri().to_string(),
                        index: row.index as u32,
                    });
                }
            }
        }
    }
    egui::Popup::context_menu(&response)
        .frame(menu_frame(&palette))
        .show(|ui| item_menu(ui, app, row.item, Some(row.context), Some(row.index)));
}

/// The chip that rides the pointer while a song is being dragged.
pub fn drag_ghost(ctx: &egui::Context, palette: &Palette) {
    // A song and a sidebar row ride the pointer the same way.
    let chip = egui::DragAndDrop::payload::<DragTrack>(ctx)
        .map(|track| (track.title.clone(), track.image.clone()))
        .or_else(|| {
            egui::DragAndDrop::payload::<DragEntry>(ctx)
                .map(|entry| (entry.title.clone(), entry.image.clone()))
        });
    let Some((title, image)) = chip else {
        return;
    };
    // The payload lives through the release frame; the chip should not.
    if !ctx.input(|input| input.pointer.any_down()) {
        return;
    }
    let Some(pos) = ctx.pointer_latest_pos() else {
        return;
    };
    egui::Area::new(egui::Id::new("drag-ghost"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(pos + vec2(16.0, 6.0))
        .show(ctx, |ui| {
            ui.set_opacity(0.9);
            egui::Frame::new()
                .fill(palette.overlay)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_max_width(280.0);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        cover(ui, palette, image.as_deref(), 24.0, 4.0, Icon::Music);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&title)
                                    .font(theme::medium(13.0))
                                    .color(palette.text),
                            )
                            .truncate()
                            .selectable(false),
                        );
                    });
                });
        });
}

pub fn explicit_badge(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(2), palette.secondary);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "E",
        theme::bold(9.5),
        palette.window,
    );
}

/// The header row above a track table.
/// The column headings above a track table. Answers with the heading that
/// was clicked, so the table can sort by it.
#[expect(clippy::fn_params_excessive_bools)]
pub fn table_header(
    ui: &mut Ui,
    palette: &Palette,
    show_album: bool,
    show_added: bool,
    show_added_by: bool,
    show_cover: bool,
    sort: Option<crate::model::TableSort>,
) -> Option<crate::model::SortColumn> {
    use crate::model::SortColumn;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    let font = theme::regular(12.0);
    let color = palette.secondary;
    let mut clicked = None;
    let mut heading = |ui: &mut Ui, x: f32, text: &str, column: SortColumn| {
        let active = sort.filter(|sort| sort.column == column);
        let galley =
            ui.painter()
                .layout_no_wrap(text.to_string(), font.clone(), egui::Color32::PLACEHOLDER);
        let size = galley.size();
        let arrow_room = if active.is_some() { 13.0 } else { 0.0 };
        let top_left = pos2(x, rect.center().y - size.y / 2.0);
        let head =
            Rect::from_min_size(top_left, size + vec2(arrow_room, 0.0)).expand2(vec2(4.0, 8.0));
        let response = ui.interact(head, ui.id().with(("table-header", text)), Sense::click());
        let color = if active.is_some() {
            palette.accent
        } else if response.hovered() {
            palette.text
        } else {
            color
        };
        ui.painter().galley(top_left, galley, color);
        if let Some(sort) = active {
            // Drawn, not typed: an arrow glyph relies on the loaded fonts
            // and rendered as a hollow box on some machines.
            let center = pos2(top_left.x + size.x + 8.0, rect.center().y);
            let (wing, tip) = if sort.ascending {
                (2.8, -3.2)
            } else {
                (-2.8, 3.2)
            };
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    center + vec2(-4.0, wing),
                    center + vec2(4.0, wing),
                    center + vec2(0.0, tip),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = Some(column);
        }
    };
    let mut number_clicked = false;
    let mut x = rect.left() + 8.0;
    {
        let number = Rect::from_center_size(pos2(x + 22.0, rect.center().y), vec2(30.0, 22.0));
        // With no sort chosen the list already plays its own order, and
        // the # says so: lit, arrow pointing down the list.
        let natural = sort.is_none();
        let active = sort.filter(|sort| sort.column == SortColumn::Index);
        let response = ui.interact(number, ui.id().with("table-header-number"), Sense::click());
        let number_color = if natural || active.is_some() {
            palette.accent
        } else if response.hovered() {
            palette.text
        } else {
            color
        };
        ui.painter().text(
            number.center(),
            egui::Align2::CENTER_CENTER,
            "#",
            font.clone(),
            number_color,
        );
        if let Some(ascending) = active
            .map(|sort| sort.ascending)
            .or(natural.then_some(true))
        {
            let center = pos2(number.center().x + 12.0, rect.center().y);
            let (wing, tip) = if ascending { (2.8, -3.2) } else { (-2.8, 3.2) };
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    center + vec2(-4.0, wing),
                    center + vec2(4.0, wing),
                    center + vec2(0.0, tip),
                ],
                number_color,
                egui::Stroke::NONE,
            ));
        }
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("The list's own order, reversed")
            .clicked()
        {
            number_clicked = true;
        }
    }
    x += 44.0;
    if show_cover {
        x += 52.0;
    }
    heading(ui, x, "TITLE", SortColumn::Title);
    let medium = width > 560.0;
    let wide = width > 760.0;
    let album_width = if show_album && medium {
        (width * 0.28).clamp(140.0, 360.0)
    } else {
        0.0
    };
    let added_width = if show_added && wide { 120.0 } else { 0.0 };
    let extra_wide = width > 920.0;
    let added_by_width = if show_added_by && extra_wide {
        130.0
    } else {
        0.0
    };
    let right_fixed = 36.0 + 56.0 + 36.0 + 8.0;
    let mut cx = rect.right() - right_fixed - added_width - added_by_width - album_width;
    if album_width > 0.0 {
        heading(ui, cx, "ALBUM", SortColumn::Album);
        cx += album_width;
    }
    if added_by_width > 0.0 {
        heading(ui, cx, "ADDED BY", SortColumn::AddedBy);
        cx += added_by_width;
    }
    if added_width > 0.0 {
        heading(ui, cx, "DATE ADDED", SortColumn::Added);
    }
    if number_clicked {
        clicked = Some(SortColumn::Index);
    }
    let clock = Rect::from_center_size(
        pos2(rect.right() - 36.0 - 56.0 / 2.0 - 6.0, rect.center().y),
        Vec2::splat(15.0),
    );
    let duration_active = sort.is_some_and(|sort| sort.column == SortColumn::Duration);
    let response = ui.interact(
        clock.expand(8.0),
        ui.id().with("table-header-duration"),
        Sense::click(),
    );
    let clock_color = if duration_active {
        palette.accent
    } else if response.hovered() {
        palette.text
    } else {
        color
    };
    Icon::Clock.image(clock_color, 15.0).paint_at(ui, clock);
    if let Some(sort) = sort.filter(|sort| sort.column == SortColumn::Duration) {
        let center = pos2(clock.right() + 9.0, rect.center().y);
        let (wing, tip) = if sort.ascending {
            (2.8, -3.2)
        } else {
            (-2.8, 3.2)
        };
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                center + vec2(-4.0, wing),
                center + vec2(4.0, wing),
                center + vec2(0.0, tip),
            ],
            clock_color,
            egui::Stroke::NONE,
        ));
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Sort by duration")
        .clicked()
    {
        clicked = Some(SortColumn::Duration);
    }
    ui.painter().hline(
        rect.x_range().shrink(8.0),
        rect.bottom() - 0.5,
        Stroke::new(1.0, palette.outline),
    );
    ui.add_space(6.0);
    clicked
}

/// Lays out text limited to `max_rows` lines, ending with an ellipsis.
pub fn ellipsized(
    ui: &Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
    width: f32,
    max_rows: usize,
) -> std::sync::Arc<egui::Galley> {
    crate::bidi::layout(
        ui.painter(),
        text,
        font,
        color,
        width,
        max_rows,
        Some(crate::bidi::ELLIPSIS),
    )
}

pub struct CardResponse {
    pub clicked: bool,
    pub play: bool,
}

/// A cover-and-title card for grids and shelves.
pub fn card(
    ui: &mut Ui,
    app: &mut App,
    image: Option<&str>,
    title: &str,
    subtitle: &str,
    round: bool,
    playable: bool,
) -> CardResponse {
    let palette = app.palette;
    const PAD: f32 = 12.0;
    const TITLE_GAP: f32 = 10.0;
    const SUBTITLE_GAP: f32 = 2.0;
    const BOTTOM_PAD: f32 = 8.0;
    let image_size = CARD_WIDTH - 2.0 * PAD;
    let text_width = image_size;
    let title_font = theme::semibold(14.0);
    let subtitle_font = theme::regular(12.5);
    // Every card reserves the title row and two subtitle rows, whatever its
    // own subtitle needs: a two-line subtitle then sits inside the hover
    // background, and a shelf that mixes one- and two-line subtitles keeps
    // its covers on one line instead of centring cards of different heights.
    let (title_row, subtitle_row) = ui.fonts_mut(|fonts| {
        (
            fonts.row_height(&title_font),
            fonts.row_height(&subtitle_font),
        )
    });
    let height =
        PAD + image_size + TITLE_GAP + title_row + SUBTITLE_GAP + 2.0 * subtitle_row + BOTTOM_PAD;
    let (rect, response) = ui.allocate_exact_size(vec2(CARD_WIDTH, height), Sense::click());
    let mut play = false;
    if ui.is_rect_visible(rect) {
        let hovered = ui.rect_contains_pointer(rect);
        if hovered {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(theme::RADIUS),
                palette
                    .surface_hover
                    .gamma_multiply(if palette.dark { 0.8 } else { 1.0 }),
            );
        }
        let image_rect = Rect::from_min_size(rect.min + vec2(PAD, PAD), Vec2::splat(image_size));
        let radius = if round { image_size / 2.0 } else { 6.0 };
        paint_shadow(ui, &palette, image_rect, radius);
        paint_cover(
            ui,
            &palette,
            image,
            image_rect,
            radius,
            if round { Icon::User } else { Icon::Music },
        );
        let text_left = rect.left() + PAD;
        let title_galley = ellipsized(ui, title, title_font, palette.text, text_width, 1);
        let title_rect = Rect::from_min_size(
            pos2(text_left, image_rect.bottom() + TITLE_GAP),
            vec2(text_width, title_row),
        );
        let title_pos = match title_galley.job.halign {
            Align::RIGHT => pos2(title_rect.right(), title_rect.top()),
            Align::Center => pos2(title_rect.center().x, title_rect.top()),
            _ => title_rect.min,
        };
        ui.painter().galley(title_pos, title_galley, palette.text);
        let subtitle_galley = ellipsized(
            ui,
            subtitle,
            subtitle_font,
            palette.secondary,
            text_width,
            2,
        );
        let subtitle_rect = Rect::from_min_size(
            pos2(text_left, title_rect.bottom() + SUBTITLE_GAP),
            vec2(text_width, 2.0 * subtitle_row),
        );
        let subtitle_pos = match subtitle_galley.job.halign {
            Align::RIGHT => pos2(subtitle_rect.right(), subtitle_rect.top()),
            Align::Center => pos2(subtitle_rect.center().x, subtitle_rect.top()),
            _ => subtitle_rect.min,
        };
        ui.painter()
            .galley(subtitle_pos, subtitle_galley, palette.secondary);

        if playable && hovered {
            let button_rect = Rect::from_center_size(
                pos2(image_rect.right() - 26.0, image_rect.bottom() - 26.0),
                Vec2::splat(44.0),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(button_rect)
                    .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
            );
            play = theme::circle_button(
                &mut child,
                Icon::PlayFilled,
                44.0,
                palette.accent,
                palette.accent_hover,
                palette.on_accent,
                "Play",
            )
            .clicked();
        }
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    CardResponse {
        clicked: response.clicked() && !play,
        play,
    }
}

/// A horizontal shelf of cards with a title.
pub fn shelf(
    ui: &mut Ui,
    palette: &Palette,
    id: &str,
    title: &str,
    add_contents: impl FnOnce(&mut Ui),
) {
    ui.add_space(8.0);
    theme::section_title(ui, palette, title);
    ui.add_space(4.0);
    egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP / 2.0;
            add_contents(ui);
        });
    });
    ui.add_space(12.0);
}

/// A wrapping grid of cards.
pub fn grid(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(CARD_GAP / 2.0, CARD_GAP);
        add_contents(ui);
    });
}

pub fn loading_row(ui: &mut Ui, palette: &Palette) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        theme::spinner(ui, 18.0, palette.accent);
        theme::subtle(ui, palette, "Loading…");
    });
}

pub fn error_row(ui: &mut Ui, app: &mut App, message: &str, retry: Option<Page>) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        theme::icon(ui, Icon::CircleAlert, 16.0, palette.danger);
        theme::text(ui, message, theme::regular(13.0), palette.secondary);
        if let Some(page) = retry
            && theme::soft_button(ui, &palette, Some(Icon::Refresh), "Retry", false).clicked()
        {
            app.actions.push(Action::Reload(page));
        }
    });
}

pub fn empty_state(ui: &mut Ui, palette: &Palette, icon: Icon, title: &str, body: &str) {
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        theme::icon(ui, icon, 40.0, palette.dim);
        ui.add_space(8.0);
        theme::text(ui, title, theme::semibold(16.0), palette.text);
        ui.add_space(2.0);
        theme::text(ui, body, theme::regular(13.5), palette.secondary);
    });
}

pub enum SliderEvent {
    None,
    Dragging(f32),
    Committed(f32),
}

/// Whole notches the wheel turned over `response` since last asked, up
/// being positive. A mouse's detent is one event, however many lines the
/// system multiplies it into (Windows says three by default, #103); a
/// free-spinning wheel's fractional lines and a trackpad's points add up
/// to the same steps, fifty points to a notch.
pub fn wheel_notches(ui: &Ui, response: &egui::Response) -> i32 {
    const NOTCH: f32 = 50.0;
    if !response.hovered() {
        return 0;
    }
    let (lines, points) = ui.input(|input| {
        let mut lines = 0.0f32;
        let mut points = 0.0f32;
        for event in &input.events {
            if let egui::Event::MouseWheel { unit, delta, .. } = event {
                match unit {
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                        lines += if delta.y.abs() >= 1.0 {
                            delta.y.signum()
                        } else {
                            delta.y
                        };
                    }
                    egui::MouseWheelUnit::Point => points += delta.y,
                }
            }
        }
        (lines, points)
    });
    let id = response.id.with("wheel");
    let total = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(0.0) + points + lines * NOTCH;
    let notches = (total / NOTCH).trunc();
    ui.data_mut(|data| data.insert_temp(id, total - notches * NOTCH));
    notches as i32
}

/// A thin horizontal slider whose handle appears on hover, for seeking and
/// volume. `value` is 0..=1.
pub fn thin_slider(
    ui: &mut Ui,
    palette: &Palette,
    id: egui::Id,
    value: f32,
    width: f32,
    accent: Color32,
    wheel_step: Option<f32>,
) -> SliderEvent {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 16.0), Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let dragging_value = ui.data(|data| data.get_temp::<f32>(id));
    let pointer_value = response
        .interact_pointer_pos()
        .map(|pos| ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    let mut event = SliderEvent::None;
    if (response.drag_started() || response.dragged())
        && let Some(v) = pointer_value
    {
        ui.data_mut(|data| data.insert_temp(id, v));
        event = SliderEvent::Dragging(v);
    }
    if response.drag_stopped() {
        let v = dragging_value.or(pointer_value).unwrap_or(value);
        ui.data_mut(|data| data.remove::<f32>(id));
        event = SliderEvent::Committed(v);
    } else if response.clicked()
        && let Some(v) = pointer_value
    {
        event = SliderEvent::Committed(v);
    }
    if let Some(step) = wheel_step {
        let notches = wheel_notches(ui, &response);
        if notches != 0 {
            event = SliderEvent::Committed((value + step * notches as f32).clamp(0.0, 1.0));
        }
    }
    let shown = match &event {
        SliderEvent::Dragging(v) => *v,
        SliderEvent::Committed(v) => *v,
        SliderEvent::None => dragging_value.unwrap_or(value),
    };
    if ui.is_rect_visible(rect) {
        let active = response.hovered() || response.dragged() || dragging_value.is_some();
        let bar = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
        let track_color = if palette.dark {
            Color32::from_white_alpha(50)
        } else {
            Color32::from_black_alpha(40)
        };
        ui.painter().rect_filled(bar, 2.0, track_color);
        let filled = Rect::from_min_max(
            bar.min,
            pos2(bar.left() + bar.width() * shown.clamp(0.0, 1.0), bar.max.y),
        );
        let fill = if active { accent } else { palette.text };
        ui.painter().rect_filled(filled, 2.0, fill);
        if active {
            ui.painter()
                .circle_filled(pos2(filled.right(), bar.center().y), 6.0, palette.text);
        }
    }
    event
}

/// A tab-like chip row: returns the newly selected index, if any.
pub fn chips<T: PartialEq + Copy>(
    ui: &mut Ui,
    palette: &Palette,
    options: &[(T, &str)],
    current: T,
) -> Option<T> {
    let mut selected = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (value, label) in options {
            if theme::soft_button(ui, palette, None, label, *value == current).clicked() {
                selected = Some(*value);
            }
        }
    });
    selected
}

/// A text field with a leading search icon.
pub fn search_field(
    ui: &mut Ui,
    palette: &Palette,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    let height = 34.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let has_focus = ui.memory(|memory| memory.has_focus(id));
    let fill = if has_focus {
        palette.surface_hover
    } else {
        palette.surface
    };
    ui.painter().rect_filled(rect, height / 2.0, fill);
    if has_focus {
        ui.painter().rect_stroke(
            rect,
            height / 2.0,
            Stroke::new(1.5, palette.text.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    }
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(16.0));
    Icon::Search
        .image(palette.secondary, 16.0)
        .paint_at(ui, icon_rect);
    let field_rect = Rect::from_min_max(
        pos2(rect.left() + 34.0, rect.top() + 1.0),
        pos2(rect.right() - 30.0, rect.bottom() - 1.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(field_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    // A right-to-left query is shown in reading order. The caret keeps
    // egui's own idea of where it is: at the end of what was typed.
    let text_color = palette.text;
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _wrap_width: f32| {
        let shown = crate::bidi::display_text(buffer.as_str()).into_owned();
        ui.painter()
            .layout_job(egui::text::LayoutJob::simple_singleline(
                shown,
                theme::regular(14.0),
                text_color,
            ))
    };
    let response = child.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(egui::RichText::new(hint).color(palette.dim))
            .font(theme::regular(14.0))
            .text_color(palette.text)
            .frame(egui::Frame::NONE)
            .desired_width(field_rect.width())
            .vertical_align(Align::Center)
            .layouter(&mut layouter),
    );
    if !text.is_empty() {
        let clear_rect = Rect::from_center_size(
            pos2(rect.right() - 17.0, rect.center().y),
            Vec2::splat(24.0),
        );
        let mut clear = ui.new_child(
            UiBuilder::new()
                .max_rect(clear_rect)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        if theme::icon_button(
            &mut clear,
            Icon::X,
            15.0,
            palette.secondary,
            palette.text,
            "Clear",
        )
        .clicked()
        {
            text.clear();
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }
    response
}

/// A toggle drawn as a switch.
pub fn switch(ui: &mut Ui, palette: &Palette, on: &mut bool) -> egui::Response {
    let size = vec2(40.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(response.id, *on);
        let fill = egui::lerp(
            egui::Rgba::from(palette.surface_active)..=egui::Rgba::from(palette.accent),
            t,
        );
        ui.painter()
            .rect_filled(rect, rect.height() / 2.0, Color32::from(fill));
        let knob_x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
        ui.painter()
            .circle_filled(pos2(knob_x, rect.center().y), 8.0, Color32::WHITE);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A labelled row in a settings section.
pub fn setting_row(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    description: &str,
    control: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            // A frame can arrive before the window has its size (a fullscreen
            // request on Wayland answers a frame late), so never go negative.
            ui.set_width((ui.available_width() - 260.0).max(0.0));
            theme::text(ui, label, theme::medium(14.0), palette.text);
            if !description.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(description)
                            .font(theme::regular(12.5))
                            .color(palette.secondary),
                    )
                    .wrap(),
                );
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    ui.add_space(10.0);
}

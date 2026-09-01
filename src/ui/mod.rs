//! Window layout: panels, overlays, keyboard shortcuts.

pub mod artist;
pub mod collection;
mod devices;
mod dialogs;
pub mod home;
mod keys;
pub mod library;
pub mod login;
mod lyrics;
pub mod player_bar;
pub mod queue;
pub mod search;
pub mod settings;
pub mod show;
pub mod sidebar;
pub mod topbar;
pub mod widgets;
pub mod winamp;

use egui::{Align2, Color32, CornerRadius, Frame, Margin, Rect, Stroke, vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::backend::AuthStatus;
use crate::model::{Action, Page, ToastKind};
use crate::theme::{self, Icon};

/// Enough room for the compact track table to keep a title column. Side
/// panels yield before the listening workspace becomes unusable.
const MIN_WORKSPACE_WIDTH: f32 = 320.0;
const MIN_CONTEXT_WIDTH: f32 = 240.0;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let ctx = &ctx;
    keys::handle(app, ctx);
    for path in winamp::dropped_skins(ctx) {
        app.actions.push(Action::InstallSkin(path));
    }
    let signed_in = app.is_connected() && app.user.is_some();
    let connecting = matches!(app.auth, AuthStatus::Connecting | AuthStatus::Starting)
        || (app.is_connected() && app.user.is_none());
    if !signed_in {
        login::show(app, ui, connecting);
        toasts(app, ctx, 20.0);
        return;
    }
    player_bar::show(app, ui);
    topbar::show(app, ui);
    if app.settings.sidebar_visible {
        let context_reserve = if app.show_queue_panel || app.show_lyrics_panel {
            MIN_CONTEXT_WIDTH
        } else {
            0.0
        };
        sidebar::show(app, ui, context_reserve);
    }
    if app.show_queue_panel {
        queue::side_panel(app, ui, MIN_WORKSPACE_WIDTH);
    }
    if app.show_lyrics_panel {
        lyrics::side_panel(app, ui, MIN_WORKSPACE_WIDTH);
    }
    central(app, ui);
    devices::popup(app, ctx);
    dialogs::show(app, ctx);
    widgets::drag_ghost(ctx, &app.palette);
    toasts(app, ctx, theme::PLAYER_BAR_HEIGHT + 16.0);
}

fn page_tint(app: &mut App) -> Option<Color32> {
    let page = app.page().clone();
    let image = match &page {
        Page::Playlist(id) => app
            .playlist_pages
            .get(id)
            .and_then(|page| page.playlist.get())
            .and_then(|playlist| pick_image(&playlist.images, 300))
            .map(str::to_string),
        Page::Album(id) => app
            .album_pages
            .get(id)
            .and_then(|page| page.album.get())
            .and_then(|album| pick_image(&album.images, 300))
            .map(str::to_string),
        Page::Artist(id) => app
            .artist_pages
            .get(id)
            .and_then(|page| page.artist.get())
            .and_then(|artist| pick_image(&artist.images, 300))
            .map(str::to_string),
        Page::Show(id) => app
            .show_pages
            .get(id)
            .and_then(|page| page.show.get())
            .and_then(|show| pick_image(&show.images, 300))
            .map(str::to_string),
        Page::LikedSongs => return Some(Color32::from_rgb(0x50, 0x38, 0xc8)),
        _ => None,
    };
    if !app.settings.accent_from_art && image.is_some() {
        return None;
    }
    match image {
        Some(url) => app.tint_for(Some(&url)),
        None => app.now_playing_tint(),
    }
}

fn central(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let tint = page_tint(app);
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            if let Some(tint) = tint {
                let strength = if matches!(
                    app.page(),
                    Page::Home | Page::Search | Page::Settings | Page::Queue
                ) {
                    0.45
                } else {
                    0.85
                };
                let top = blend(palette.window, tint, strength);
                let header = Rect::from_min_size(rect.min, vec2(rect.width(), 340.0));
                widgets::paint_vertical_gradient(ui, header, top, palette.window);
            }
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            let page = app.page().clone();
            egui::ScrollArea::vertical()
                .id_salt(("page", page.encode()))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin {
                            left: widgets::PAGE_PADDING as i8,
                            right: widgets::PAGE_PADDING as i8,
                            top: 4,
                            bottom: 48,
                        })
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            match page {
                                Page::Home => home::show(app, ui),
                                Page::TopSongs => collection::top_songs(app, ui),
                                Page::Search => search::show(app, ui),
                                Page::LikedSongs => collection::liked(app, ui),
                                Page::Albums | Page::Artists | Page::Podcasts | Page::Episodes => {
                                    library::show(app, ui, page)
                                }
                                Page::Playlist(id) => collection::playlist(app, ui, &id),
                                Page::Album(id) => collection::album(app, ui, &id),
                                Page::Artist(id) => artist::show(app, ui, &id),
                                Page::Show(id) => show::show(app, ui, &id),
                                Page::Queue => queue::page(app, ui),
                                Page::Settings => settings::show(app, ui),
                            }
                        });
                });
        });
}

/// Makes `rect` drag the borderless window. Register it before child widgets so
/// they keep their clicks.
pub fn titlebar_drag(ui: &mut egui::Ui, rect: egui::Rect) {
    if theme::titlebar_inset(ui.ctx()) == 0.0 {
        return;
    }
    let response = ui.interact(
        rect,
        ui.id().with("titlebar-drag"),
        egui::Sense::click_and_drag(),
    );
    // AppKit must start the move from the live mouse-down event. Waiting for
    // egui's drag threshold makes the event stale. Native dragging consumes the
    // gesture, so double-click zoom is unavailable here.
    if response.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_pressed()) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

pub fn blend(base: Color32, tint: Color32, amount: f32) -> Color32 {
    let a = egui::Rgba::from(base);
    let b = egui::Rgba::from(tint);
    let mixed = a * (1.0 - amount) + b * amount;
    let mut color = Color32::from(mixed);
    color[3] = 255;
    color
}

fn toasts(app: &mut App, ctx: &egui::Context, bottom_offset: f32) {
    if app.toasts.is_empty() {
        return;
    }
    let palette = app.palette;
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -bottom_offset))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for toast in &app.toasts {
                let age = toast.created.elapsed().as_secs_f32();
                let alpha = if age < 0.15 {
                    age / 0.15
                } else if age > 2.8 {
                    ((3.2 - age) / 0.4).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                ui.set_opacity(alpha);
                Frame::new()
                    .fill(palette.overlay)
                    .stroke(Stroke::new(1.0, palette.outline))
                    .corner_radius(CornerRadius::same(theme::RADIUS))
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: palette.shadow,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (icon, color) = match toast.kind {
                                ToastKind::Info => (Icon::CircleCheck, palette.accent),
                                ToastKind::Error => (Icon::CircleAlert, palette.danger),
                            };
                            theme::icon(ui, icon, 16.0, color);
                            // Laid out at its own width. The area
                            // remembers its size, so after a short toast a
                            // label left to wrap at the area's width broke
                            // long messages on every word.
                            let galley = ui.painter().layout(
                                toast.message.clone(),
                                theme::medium(13.5),
                                palette.text,
                                280.0,
                            );
                            ui.add(egui::Label::new(galley));
                        });
                    });
            }
        });
}

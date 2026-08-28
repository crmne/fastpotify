//! The compact square player, Spotify mini-player style: a big cover, the
//! track title, and the essentials — transport, seek, and time.

use egui::{Align, Layout, Rect, Sense, UiBuilder, Vec2, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Page};
use crate::theme::{self, Icon};
use crate::util;

use super::widgets::{SliderEvent, thin_slider};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let tint = app.eased_tint(ui.ctx(), app.now_playing_tint());
    let fill = match tint {
        Some(tint) => super::blend(palette.panel, tint, 0.10),
        None => palette.panel,
    };
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(16, 12)),
        )
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let now = app.now_playing();

            // Leave the compact player with the corner X (the cover does too).
            let close = Rect::from_center_size(
                pos2(rect.right() - 14.0, rect.top() + 14.0),
                Vec2::splat(28.0),
            );
            let mut close_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(close)
                    .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
            );
            if theme::icon_button(
                &mut close_ui,
                Icon::Maximize2,
                14.0,
                palette.secondary,
                palette.text,
                "Leave mini-player (Ctrl+M)",
            )
            .clicked()
            {
                app.actions.push(Action::ToggleMiniPlayer);
            }

            // The cover dominates the square; clicking it returns to the
            // full window, like Spotify.
            let cover_size = (rect.width() * 0.56).clamp(140.0, 180.0);
            let cover_rect = Rect::from_center_size(
                pos2(rect.center().x, rect.top() + cover_size * 0.52),
                Vec2::splat(cover_size),
            );
            super::widgets::paint_cover(
                ui,
                &palette,
                now.as_ref()
                    .and_then(|now| now.art_small.as_deref().or(now.art_url.as_deref())),
                cover_rect,
                8.0,
                Icon::Music,
            );
            if ui
                .interact(cover_rect, egui::Id::new("mini-cover"), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                app.actions.push(Action::ToggleMiniPlayer);
            }

            // Title and artist, centred under the cover.
            let text_rect = Rect::from_min_size(
                pos2(rect.left() + 6.0, cover_rect.bottom() + 8.0),
                vec2(rect.width() - 12.0, 42.0),
            );
            let mut text_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(text_rect)
                    .layout(Layout::top_down(Align::Center)),
            );
            text_ui.set_clip_rect(text_rect.intersect(ui.clip_rect()));
            text_ui.spacing_mut().item_spacing.y = 2.0;
            let now_ref = now.as_ref();
            if let Some(now) = now_ref {
                if theme::link(
                    &mut text_ui,
                    &now.title,
                    theme::semibold(14.0),
                    palette.text,
                )
                .clicked()
                {
                    if let Some(id) = &now.album_id {
                        app.actions.push(Action::Open(Page::Album(id.clone())));
                    } else if let Some(id) = &now.show_id {
                        app.actions.push(Action::Open(Page::Show(id.clone())));
                    }
                }
                theme::text(
                    &mut text_ui,
                    &now.subtitle,
                    theme::regular(12.0),
                    palette.secondary,
                );
            } else {
                theme::text(
                    &mut text_ui,
                    "Nothing playing",
                    theme::semibold(14.0),
                    palette.text,
                );
                theme::text(
                    &mut text_ui,
                    "Pick a song, album, or playlist",
                    theme::regular(12.0),
                    palette.dim,
                );
            }

            // Transport: previous / play-pause / next, centred. Explicit
            // rects keep everything aligned on the square's centre line.
            let enabled = now_ref.is_some_and(|now| now.can_control) || app.is_connected();
            let playing = now_ref.is_some_and(|now| now.playing);
            let loading = now_ref.is_some_and(|now| now.loading);
            let dim = if enabled {
                palette.secondary
            } else {
                palette.dim
            };
            let row_cy = rect.bottom() - 56.0;
            let widths = [34.0, 42.0, 34.0];
            let gap = 16.0;
            let total: f32 = widths.iter().sum::<f32>() + gap * 2.0;
            let mut x = rect.center().x - total / 2.0;
            let mut slot = |width: f32| -> Rect {
                let cell = Rect::from_center_size(pos2(x + width / 2.0, row_cy), vec2(width, 40.0));
                x += width + gap;
                cell
            };
            let centered = |ui: &mut egui::Ui, cell: Rect| {
                ui.new_child(
                    UiBuilder::new()
                        .max_rect(cell)
                        .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
                )
            };

            let mut cell = centered(ui, slot(widths[0]));
            if theme::icon_button(
                &mut cell,
                Icon::SkipBackFilled,
                18.0,
                dim,
                palette.text,
                "Previous",
            )
            .clicked()
            {
                app.actions.push(Action::Previous);
            }

            let disc = slot(widths[1]);
            if loading || app.any_play_pending() {
                ui.painter()
                    .circle_filled(disc.center(), 20.0, palette.text);
                let mut cell = centered(ui, disc);
                theme::spinner(&mut cell, 24.0, palette.window);
            } else {
                let icon = if playing {
                    Icon::PauseFilled
                } else {
                    Icon::PlayFilled
                };
                let hover = if palette.dark {
                    egui::Color32::WHITE
                } else {
                    palette.text
                };
                let mut cell = centered(ui, disc);
                if theme::circle_button(
                    &mut cell,
                    icon,
                    42.0,
                    palette.text,
                    hover,
                    palette.window,
                    if playing { "Pause" } else { "Play" },
                )
                .clicked()
                {
                    app.actions.push(Action::TogglePlay);
                }
            }

            let mut cell = centered(ui, slot(widths[2]));
            if theme::icon_button(
                &mut cell,
                Icon::SkipForwardFilled,
                18.0,
                dim,
                palette.text,
                "Next",
            )
            .clicked()
            {
                app.actions.push(Action::Next);
            }

            // Seek slider with the time either side, just below the buttons.
            let (position, duration) = now_ref
                .map(|now| (now.position_ms, now.duration_ms))
                .unwrap_or((0, 0));
            let shown_position = match app.seek_preview {
                Some(fraction) => (fraction * duration as f32) as u32,
                None => position,
            };
            let fraction = if duration > 0 {
                position as f32 / duration as f32
            } else {
                0.0
            };
            let time_color = if now_ref.is_some() {
                palette.secondary
            } else {
                palette.dim
            };
            let slider_width = (rect.width() - 104.0).clamp(120.0, 240.0);
            let slider_left = rect.center().x - slider_width / 2.0;
            let seek_cy = row_cy + 34.0;
            ui.painter().text(
                pos2(slider_left - 8.0, seek_cy),
                egui::Align2::RIGHT_CENTER,
                util::format_duration_ms(shown_position),
                theme::regular(10.5),
                time_color,
            );
            let slider_rect =
                Rect::from_center_size(pos2(rect.center().x, seek_cy), vec2(slider_width, 16.0));
            let mut slider_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(slider_rect)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            match thin_slider(
                &mut slider_ui,
                &palette,
                egui::Id::new("mini-seek"),
                fraction,
                slider_width,
                palette.accent,
            ) {
                SliderEvent::Dragging(value) => app.seek_preview = Some(value),
                SliderEvent::Committed(value) => {
                    app.seek_preview = None;
                    if duration > 0 {
                        app.actions
                            .push(Action::Seek((value * duration as f32) as u32));
                    }
                }
                SliderEvent::None => {}
            }
            ui.painter().text(
                pos2(slider_left + slider_width + 8.0, seek_cy),
                egui::Align2::LEFT_CENTER,
                util::format_duration_ms(duration),
                theme::regular(10.5),
                time_color,
            );
        });
}

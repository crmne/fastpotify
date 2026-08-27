//! The compact player: a single-line now-playing strip used when the
//! window is shrunk to a small bar.

use egui::{Align, CornerRadius, Layout, Rect, Sense, UiBuilder, Vec2, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Page};
use crate::player::RepeatMode;
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
        .frame(egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(12, 8)))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let now = app.now_playing();

            // Exit the compact player by clicking the cover, or the corner X.
            let close = Rect::from_center_size(pos2(rect.right() - 14.0, rect.top() + 14.0), Vec2::splat(28.0));
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

            let cover_rect = Rect::from_min_size(rect.min + vec2(2.0, 2.0), Vec2::splat(72.0));
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
                .interact(
                    cover_rect,
                    egui::Id::new("mini-cover"),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                app.actions.push(Action::ToggleMiniPlayer);
            }

            let text_left = cover_rect.right() + 14.0;
            let text_rect = Rect::from_min_max(
                pos2(text_left, rect.top() + 10.0),
                pos2(rect.right() - 340.0, rect.bottom() - 10.0),
            );
            let mut text_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(text_rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            text_ui.set_clip_rect(text_rect.intersect(ui.clip_rect()));
            text_ui.spacing_mut().item_spacing.y = 3.0;
            let now_ref = now.as_ref();
            if let Some(now) = now_ref {
                if theme::link(&mut text_ui, &now.title, theme::semibold(14.5), palette.text)
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
                theme::text(&mut text_ui, "Nothing playing", theme::semibold(14.0), palette.text);
                theme::text(
                    &mut text_ui,
                    "Pick a song, album, or playlist",
                    theme::regular(12.0),
                    palette.dim,
                );
            }

            // Transport, right of the text.
            let cy = rect.center().y;
            let mut x = text_rect.right() + 6.0;
            let mut slot = |width: f32| -> Rect {
                let cell = Rect::from_center_size(pos2(x + width / 2.0, cy), vec2(width, 36.0));
                x += width + 4.0;
                cell
            };
            let centered = |ui: &mut egui::Ui, cell: Rect| {
                ui.new_child(
                    UiBuilder::new()
                        .max_rect(cell)
                        .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
                )
            };
            let enabled = now.as_ref().is_some_and(|now| now.can_control) || app.is_connected();
            let playing = now.as_ref().is_some_and(|now| now.playing);
            let loading = now.as_ref().is_some_and(|now| now.loading);
            let shuffle = now.as_ref().is_some_and(|now| now.shuffle);
            let repeat = now.as_ref().map(|now| now.repeat).unwrap_or_default();
            let volume = now
                .as_ref()
                .map(|now| now.volume_percent)
                .unwrap_or_else(|| crate::app::volume_to_percent(app.local.volume));
            let (position, duration) = now
                .as_ref()
                .map(|now| (now.position_ms, now.duration_ms))
                .unwrap_or((0, 0));
            let dim = if enabled {
                palette.secondary
            } else {
                palette.dim
            };
            let local_playback = now.is_none_or(|now| now.local);
            let _ = now;

            // Previous.
            let mut cell = centered(ui, slot(30.0));
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
            // Play / pause.
            let disc = slot(36.0);
            if loading || app.any_play_pending() {
                ui.painter().circle_filled(disc.center(), 18.0, palette.text);
                let mut cell = centered(ui, disc);
                theme::spinner(&mut cell, 22.0, palette.window);
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
                    36.0,
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
            // Next.
            let mut cell = centered(ui, slot(30.0));
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
            // Shuffle.
            let mut cell = centered(ui, slot(29.0));
            if theme::icon_button(
                &mut cell,
                Icon::Shuffle,
                17.0,
                if shuffle { palette.accent } else { dim },
                palette.text,
                "Shuffle",
            )
            .clicked()
            {
                app.actions.push(Action::ToggleShuffle);
            }
            // Repeat.
            let (repeat_icon, repeat_color, tooltip) = match repeat {
                RepeatMode::Off => (Icon::Repeat, dim, "Repeat"),
                RepeatMode::Context => (Icon::Repeat, palette.accent, "Repeat one"),
                RepeatMode::Track => (Icon::Repeat1, palette.accent, "Repeat off"),
            };
            let mut cell = centered(ui, slot(29.0));
            if theme::icon_button(
                &mut cell,
                repeat_icon,
                17.0,
                repeat_color,
                palette.text,
                tooltip,
            )
            .clicked()
            {
                app.actions.push(Action::CycleRepeat);
            }

            // Volume, far right.
            let shown = match app.volume_preview {
                Some(fraction) => (fraction * 100.0).round() as u8,
                None => volume,
            };
            let volume_rect = Rect::from_center_size(
                pos2(rect.right() - 96.0 - 28.0, cy),
                vec2(96.0, 16.0),
            );
            let mut volume_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(volume_rect)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            match thin_slider(
                &mut volume_ui,
                &palette,
                egui::Id::new("mini-volume"),
                volume as f32 / 100.0,
                96.0,
                palette.accent,
            ) {
                SliderEvent::Dragging(value) => {
                    app.volume_preview = Some(value);
                    if local_playback {
                        app.actions
                            .push(Action::SetVolume((value * 100.0).round() as u8));
                    }
                }
                SliderEvent::Committed(value) => {
                    app.volume_preview = None;
                    app.actions
                        .push(Action::SetVolume((value * 100.0).round() as u8));
                }
                SliderEvent::None => {}
            }
            let volume_icon = match shown {
                0 => Icon::VolumeX,
                1..=33 => Icon::Volume,
                34..=66 => Icon::Volume1,
                _ => Icon::Volume2,
            };
            let mute = Rect::from_center_size(pos2(rect.right() - 12.0, cy), Vec2::splat(30.0));
            let mut mute_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(mute)
                    .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
            );
            if theme::icon_button(
                &mut mute_ui,
                volume_icon,
                18.0,
                palette.secondary,
                palette.text,
                if shown == 0 { "Unmute" } else { "Mute" },
            )
            .clicked()
            {
                app.actions.push(Action::ToggleMute);
            }

            // Seek, running under everything but the covers.
            let fraction = if duration > 0 {
                position as f32 / duration as f32
            } else {
                0.0
            };
            let seek_rect = Rect::from_center_size(
                pos2((rect.left() + rect.right()) / 2.0, rect.bottom() - 6.0),
                vec2(rect.width() - 24.0, 8.0),
            );
            let mut seek_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(seek_rect)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            match thin_slider(
                &mut seek_ui,
                &palette,
                egui::Id::new("mini-seek"),
                fraction,
                seek_rect.width(),
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
            let _ = util::format_duration_ms;
            let _ = CornerRadius::same(0);
        });
}

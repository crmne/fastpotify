//! The playback queue, as a page or as a side panel.

use egui::{Align, Frame, Layout, Margin, Stroke, Vec2, pos2};

use crate::api::models::PlayableItem;
use crate::app::App;
use crate::model::{Action, Loadable, QueueTab, RowContext};
use crate::theme::{self, Icon};

use super::widgets::{self, TrackRow};

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(8.0);
    // No refresh button: the queue keeps itself fresh, on every track
    // change, every add, and a rolling poll while it shows.
    ui.horizontal(|ui| {
        theme::text(ui, "Queue", theme::bold(28.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            save_button(app, ui);
        });
    });
    ui.add_space(12.0);
    contents(app, ui, false);
}

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let panel = egui::Panel::right("queue-panel")
        .resizable(true)
        .default_size(app.settings.queue_width)
        .size_range(280.0..=560.0)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        );
    let response = panel.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            tabs(app, ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(ui, Icon::X, 18.0, palette.secondary, palette.text, "Close")
                    .clicked()
                {
                    app.actions.push(Action::ToggleQueuePanel);
                }
                if app.queue_tab == QueueTab::Queue {
                    save_button(app, ui);
                }
            });
        });
        ui.add_space(8.0);
        // Lazy load recents when tab becomes visible.
        if app.queue_tab == QueueTab::Recents
            && !app.recents.loading
            && !app.recents.complete
            && app.recents.items.is_empty()
            && app.recents.error.is_none()
        {
            app.actions.push(Action::LoadMoreRecents);
        }
        egui::ScrollArea::vertical()
            .id_salt("queue-panel-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match app.queue_tab {
                QueueTab::Queue => contents(app, ui, true),
                QueueTab::Recents => recents_contents(app, ui),
            });
    });
    let width = response.response.rect.width();
    if (width - app.settings.queue_width).abs() > 1.0 {
        app.settings.queue_width = width;
        app.actions.push(Action::SettingsChanged);
    }
}

fn tabs(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let current = app.queue_tab;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (tab, label) in [
            (QueueTab::Queue, "Queue"),
            (QueueTab::Recents, "Recently played"),
        ] {
            let active = current == tab;
            let color = if active {
                palette.text
            } else {
                palette.secondary
            };
            let font = if active {
                theme::semibold(14.0)
            } else {
                theme::regular(14.0)
            };
            let galley = ui
                .painter()
                .layout_no_wrap(label.to_string(), font.clone(), color);
            let size = galley.size() + Vec2::new(4.0, 6.0);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            if ui.is_rect_visible(rect) {
                let pos = pos2(
                    rect.center().x - galley.size().x / 2.0,
                    rect.center().y - galley.size().y / 2.0,
                );
                ui.painter().galley(pos, galley, color);
                if active {
                    let y = rect.bottom() + 2.0;
                    ui.painter()
                        .hline(rect.x_range(), y, Stroke::new(2.0, palette.accent));
                }
                if response.hovered() && !active {
                    let y = rect.bottom() + 2.0;
                    ui.painter()
                        .hline(rect.x_range(), y, Stroke::new(1.0, palette.outline));
                }
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
                && !active
            {
                app.actions.push(Action::SetQueueTab(tab));
            }
        }
    });
}

/// The queue, made permanent: a new playlist of the playing song and
/// every row after it. A station someone likes becomes theirs this way.
fn save_button(app: &mut App, ui: &mut egui::Ui) {
    if app.queue_playlist_uris().is_empty() {
        return;
    }
    let palette = app.palette;
    if theme::icon_button(
        ui,
        Icon::ListPlus,
        18.0,
        palette.secondary,
        palette.text,
        "Save as a playlist",
    )
    .clicked()
    {
        app.actions.push(Action::SaveQueueAsPlaylist);
    }
}

/// Empties Playing next. Only where it can keep the promise: the queue
/// of this computer's own player.
fn clear_button(app: &mut App, ui: &mut egui::Ui) {
    if !app.can_clear_queue() {
        return;
    }
    let palette = app.palette;
    if theme::icon_button(
        ui,
        Icon::Trash,
        18.0,
        palette.secondary,
        palette.text,
        "Clear queue",
    )
    .clicked()
    {
        app.actions.push(Action::ClearQueue);
    }
}

fn contents(app: &mut App, ui: &mut egui::Ui, compact: bool) {
    let palette = app.palette;
    let queue = match &app.queue {
        Loadable::Loaded(queue) => queue.clone(),
        Loadable::Loading | Loadable::NotLoaded => {
            widgets::loading_row(ui, &palette);
            return;
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            widgets::error_row(ui, app, &error, Some(crate::model::Page::Queue));
            return;
        }
    };
    let now = app.now_playing();
    // The player's own report wins over the fetched snapshot: after a skip
    // the Web API tells the old story for a second or two, and the row on
    // top must be the song being heard, not the one before it.
    let current: Option<PlayableItem> = match &now {
        Some(now) => queue
            .currently_playing
            .clone()
            .filter(|item| item.uri() == now.uri)
            .or_else(|| app.now_playing_item()),
        None => queue.currently_playing.clone(),
    };
    if let Some(current) = &current {
        theme::text(ui, "Now playing", theme::semibold(14.0), palette.text);
        ui.add_space(4.0);
        let context = RowContext::Uris(vec![current.uri().to_string()]);
        widgets::track_row(
            ui,
            app,
            TrackRow {
                index: 0,
                number: Some(1),
                item: current,
                context: &context,
                show_cover: true,
                show_album: !compact,
                added_at: None,
                added_by: None,
                show_added_by: false,
                compact,
                thin: false,
                shift: 0.0,
            },
        );
        ui.add_space(14.0);
    }
    if queue.queue.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::ListVideo,
            "Nothing queued",
            "Add songs to your queue and they'll show up here.",
        );
        return;
    }
    let row_height = if compact {
        theme::COMPACT_ROW_HEIGHT
    } else {
        theme::ROW_HEIGHT
    };
    let items = queue.queue.clone();
    // The user's own songs get their own section on top; the playing
    // context's rows follow under the usual heading. One numbering runs
    // through both, because that is the order things play.
    let queued_len = app.queued_rows_len().min(items.len());
    if queued_len > 0 {
        // The trash sits with the songs it removes: only this section is
        // the user's to clear, the context below plays itself.
        ui.horizontal(|ui| {
            theme::text(ui, "Playing next", theme::semibold(14.0), palette.text);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clear_button(app, ui);
            });
        });
        ui.add_space(4.0);
        for index in 0..queued_len {
            queue_row(app, ui, &items, index, compact);
        }
        ui.add_space(14.0);
    }
    if items.len() > queued_len {
        theme::text(ui, "Next up", theme::semibold(14.0), palette.text);
        ui.add_space(4.0);
        widgets::virtual_rows(ui, items.len() - queued_len, row_height, |ui, index| {
            queue_row(app, ui, &items, queued_len + index, compact);
        });
    }
}

fn recents_contents(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    // Snapshot to avoid borrow issues while drawing.
    let items = app.recents.items.clone();
    let loading = app.recents.loading;
    let error = app.recents.error.clone();
    let complete = app.recents.complete;
    let loaded_once = app.recents.loaded_once;

    if items.is_empty() {
        if loading {
            widgets::loading_row(ui, &palette);
            return;
        }
        if let Some(err) = error {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                theme::icon(ui, Icon::CircleAlert, 16.0, palette.danger);
                theme::text(ui, &err, theme::regular(13.0), palette.secondary);
                if theme::soft_button(ui, &palette, Some(Icon::Refresh), "Retry", false).clicked() {
                    app.actions.push(Action::ReloadRecents);
                }
            });
            return;
        }
        if loaded_once {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Clock,
                "No recent plays",
                "Play something and it will show up here.",
            );
        } else {
            widgets::loading_row(ui, &palette);
        }
        return;
    }

    // Show error inline if we have items but also an error on next page.
    if let Some(err) = error {
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::CircleAlert, 14.0, palette.danger);
            theme::text(ui, &err, theme::regular(12.0), palette.secondary);
            if theme::soft_button(ui, &palette, Some(Icon::Refresh), "Retry", false).clicked() {
                app.actions.push(Action::LoadMoreRecents);
            }
        });
        ui.add_space(6.0);
    }

    let row_height = theme::COMPACT_ROW_HEIGHT;
    // Build PlayableItems on the fly; virtual_rows needs stable index.
    widgets::virtual_rows(ui, items.len(), row_height, |ui, index| {
        let entry = &items[index];
        // Need owned PlayableItem for track_row; clone track.
        let item = PlayableItem::Track(entry.track.clone());
        let context = RowContext::Uris(vec![entry.track.uri.clone()]);
        widgets::track_row(
            ui,
            app,
            TrackRow {
                index,
                number: None,
                item: &item,
                context: &context,
                show_cover: true,
                show_album: false,
                added_at: entry.played_at.as_deref(),
                added_by: None,
                show_added_by: false,
                compact: true,
                thin: false,
                shift: 0.0,
            },
        );
    });

    // Footer: loading more or load more trigger
    if loading {
        ui.add_space(8.0);
        widgets::loading_row(ui, &palette);
    } else if !complete {
        ui.add_space(8.0);
        // Auto-load when near end, plus manual button as fallback.
        let can_load = app.recents.can_load_more();
        // Check if scroll is near end (same heuristic as widgets::load_more_when_near_end)
        let clip = ui.clip_rect();
        let cursor = ui.cursor().top();
        if can_load && cursor - clip.bottom() < 900.0 {
            app.actions.push(Action::LoadMoreRecents);
        }
        if theme::soft_button(ui, &palette, Some(Icon::Refresh), "Load more", false).clicked() {
            app.actions.push(Action::LoadMoreRecents);
        }
    }
}

/// One row of the queue, numbered and indexed by its place in the whole
/// queue, whichever section it sits in.
fn queue_row(
    app: &mut App,
    ui: &mut egui::Ui,
    items: &[PlayableItem],
    index: usize,
    compact: bool,
) {
    widgets::track_row(
        ui,
        app,
        TrackRow {
            index,
            number: Some(index + 1),
            item: &items[index],
            context: &RowContext::Queue,
            show_cover: true,
            show_album: !compact,
            added_at: None,
            added_by: None,
            show_added_by: false,
            compact,
            thin: false,
            shift: 0.0,
        },
    );
}

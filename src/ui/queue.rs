//! The playback queue, as a page or as a side panel.

use egui::{Align, Frame, Layout, Margin};

use crate::api::models::PlayableItem;
use crate::app::App;
use crate::model::{Action, Loadable, RowContext};
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

pub fn side_panel(app: &mut App, ui: &mut egui::Ui, workspace_reserve: f32) {
    let palette = app.palette;
    let max_width = (ui.available_width() - workspace_reserve).clamp(240.0, 560.0);
    let min_width = 280.0_f32.min(max_width);
    let width_constrained = app.settings.queue_width > max_width;
    let panel = egui::Panel::right("queue-panel")
        .resizable(true)
        .default_size(app.settings.queue_width)
        .size_range(min_width..=max_width)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        );
    let response = panel.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            theme::text(ui, "Queue", theme::bold(18.0), palette.text);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(ui, Icon::X, 18.0, palette.secondary, palette.text, "Close")
                    .clicked()
                {
                    app.actions.push(Action::ToggleQueuePanel);
                }
                save_button(app, ui);
            });
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("queue-panel-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| contents(app, ui, true));
    });
    let width = response.response.rect.width();
    if !width_constrained && (width - app.settings.queue_width).abs() > 1.0 {
        app.settings.queue_width = width;
        app.actions.push(Action::SettingsChanged);
    }
    ui.painter().vline(
        response.response.rect.left() + 0.5,
        response.response.rect.y_range(),
        egui::Stroke::new(1.0, palette.outline.gamma_multiply(0.45)),
    );
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

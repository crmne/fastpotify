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
    ui.horizontal(|ui| {
        theme::text(ui, "Queue", theme::bold(28.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(
                ui,
                Icon::Refresh,
                18.0,
                palette.secondary,
                palette.text,
                "Refresh",
            )
            .clicked()
            {
                app.actions.push(Action::RefreshQueue);
            }
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
            theme::text(ui, "Queue", theme::bold(18.0), palette.text);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(ui, Icon::X, 18.0, palette.secondary, palette.text, "Close")
                    .clicked()
                {
                    app.actions.push(Action::ToggleQueuePanel);
                }
                if theme::icon_button(
                    ui,
                    Icon::Refresh,
                    16.0,
                    palette.secondary,
                    palette.text,
                    "Refresh",
                )
                .clicked()
                {
                    app.actions.push(Action::RefreshQueue);
                }
            });
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("queue-panel-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| contents(app, ui, true));
    });
    let width = response.response.rect.width();
    if (width - app.settings.queue_width).abs() > 1.0 {
        app.settings.queue_width = width;
        app.actions.push(Action::SettingsChanged);
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
    let current: Option<PlayableItem> = queue.currently_playing.clone().or_else(|| {
        now.as_ref().and_then(|now| {
            now.id
                .as_ref()
                .and_then(|id| app.track_cache.get(id).cloned().map(PlayableItem::Track))
        })
    });
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
    theme::text(ui, "Next up", theme::semibold(14.0), palette.text);
    ui.add_space(4.0);
    let uris: Vec<String> = queue
        .queue
        .iter()
        .map(|item| item.uri().to_string())
        .collect();
    let context = RowContext::Uris(uris);
    let row_height = if compact {
        theme::COMPACT_ROW_HEIGHT
    } else {
        theme::ROW_HEIGHT
    };
    let items = queue.queue.clone();
    widgets::virtual_rows(ui, items.len(), row_height, |ui, index| {
        widgets::track_row(
            ui,
            app,
            TrackRow {
                index,
                number: Some(index + 1),
                item: &items[index],
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
    });
}

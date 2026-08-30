//! Modal dialogs: playlist details, confirmations, shortcuts.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke};

use crate::app::App;
use crate::model::{Action, Dialog};
use crate::theme;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(dialog) = app.dialog.clone() else {
        return;
    };
    let palette = app.palette;
    let frame = Frame::new()
        .fill(palette.overlay)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 4))
        .inner_margin(Margin::same(24))
        .shadow(egui::epaint::Shadow {
            offset: [0, 10],
            blur: 40,
            spread: 0,
            color: palette.shadow,
        });
    let response = egui::Modal::new(egui::Id::new("dialog"))
        .frame(frame)
        .backdrop_color(egui::Color32::from_black_alpha(if palette.dark {
            150
        } else {
            80
        }))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            match dialog {
                Dialog::CreatePlaylist { .. } => create_playlist(app, ui),
                Dialog::EditPlaylist { .. } => edit_playlist(app, ui),
                Dialog::ConfirmDeletePlaylist { id, name, owned } => {
                    theme::text(
                        ui,
                        if owned {
                            app.t("dialog.delete_playlist.title")
                        } else {
                            app.t("dialog.remove_library.title")
                        },
                        theme::bold(20.0),
                        palette.text,
                    );
                    ui.add_space(8.0);
                    let body = if owned {
                        app.tf("dialog.delete_playlist.body", &[("name", &name)])
                    } else {
                        app.tf("dialog.remove_library.body", &[("name", &name)])
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(body)
                                .font(theme::regular(14.0))
                                .color(palette.secondary),
                        )
                        .wrap(),
                    );
                    ui.add_space(20.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::pill_button(
                            ui,
                            &palette,
                            if owned {
                                app.t("common.delete")
                            } else {
                                app.t("common.remove")
                            },
                            true,
                        )
                        .clicked()
                        {
                            app.actions.push(Action::DeletePlaylist(id.clone()));
                        }
                        if theme::pill_button(ui, &palette, app.t("common.cancel"), false).clicked()
                        {
                            app.actions.push(Action::CloseDialog);
                        }
                    });
                }
                Dialog::Shortcuts => {
                    theme::text(
                        ui,
                        app.t("dialog.shortcuts.title"),
                        theme::bold(20.0),
                        palette.text,
                    );
                    ui.add_space(12.0);
                    // `theme::text` truncates, which in a grid makes each cell
                    // claim almost no width and turns "Ctrl+Shift+A" into
                    // "Ctrl…". A shortcut is unusable when abbreviated, so
                    // these cells are sized to their content.
                    let cell = |ui: &mut egui::Ui, text: &str, font: egui::FontId, color| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(text).font(font).color(color))
                                .extend()
                                .selectable(false),
                        );
                    };
                    egui::Grid::new("shortcuts")
                        .num_columns(2)
                        .spacing([24.0, 8.0])
                        .show(ui, |ui| {
                            for (keys, description) in super::keys::shortcuts(app) {
                                cell(ui, &keys, theme::semibold(13.0), palette.text);
                                cell(ui, description, theme::regular(13.5), palette.secondary);
                                ui.end_row();
                            }
                        });
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::pill_button(ui, &palette, app.t("common.done"), true).clicked() {
                            app.actions.push(Action::CloseDialog);
                        }
                    });
                }
                Dialog::PremiumNeeded => {
                    theme::text(
                        ui,
                        app.t("dialog.premium.title"),
                        theme::bold(20.0),
                        palette.text,
                    );
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(app.t("dialog.premium.body"))
                                .font(theme::regular(14.0))
                                .color(palette.secondary),
                        )
                        .wrap(),
                    );
                    ui.add_space(20.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::pill_button(ui, &palette, app.t("common.ok"), true).clicked() {
                            app.actions.push(Action::CloseDialog);
                        }
                    });
                }
            }
        });
    if response.should_close() {
        app.actions.push(Action::CloseDialog);
    }
}

fn text_field(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    id: &str,
    text: &mut String,
    hint: &str,
    focus: bool,
) -> egui::Response {
    let response = Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .id(egui::Id::new(id))
                    .hint_text(egui::RichText::new(hint).color(palette.dim))
                    .font(theme::regular(14.0))
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            )
        })
        .inner;
    if focus && ui.memory(|memory| memory.focused().is_none()) {
        response.request_focus();
    }
    response
}

fn create_playlist(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let catalog = app.catalog;
    let busy = app.playlist_busy;
    let Some(Dialog::CreatePlaylist {
        name,
        public,
        add_uris,
    }) = &mut app.dialog
    else {
        return;
    };
    theme::text(
        ui,
        catalog.get("dialog.new_playlist.title"),
        theme::bold(20.0),
        palette.text,
    );
    ui.add_space(12.0);
    theme::text(
        ui,
        catalog.get("common.name"),
        theme::medium(13.0),
        palette.secondary,
    );
    let field = text_field(
        ui,
        &palette,
        "playlist-name",
        name,
        catalog.get("dialog.playlist_name_hint"),
        true,
    );
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        super::widgets::switch(ui, &palette, public);
        theme::text(
            ui,
            catalog.get("dialog.public_playlist"),
            theme::regular(14.0),
            palette.text,
        );
    });
    if !add_uris.is_empty() {
        ui.add_space(6.0);
        let count = add_uris.len().to_string();
        theme::text(
            ui,
            catalog.format("dialog.songs_will_be_added", &[("count", &count)]),
            theme::regular(13.0),
            palette.secondary,
        );
    }
    ui.add_space(20.0);
    let submit = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let name_value = name.trim().to_string();
    let public_value = *public;
    let uris = add_uris.clone();
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if busy {
            theme::spinner(ui, 18.0, palette.accent);
        } else {
            let create = theme::pill_button(ui, &palette, catalog.get("common.create"), true)
                .clicked()
                || submit;
            if create && !name_value.is_empty() {
                app.actions.push(Action::CreatePlaylist {
                    name: name_value.clone(),
                    public: public_value,
                    add_uris: uris.clone(),
                });
            }
            if theme::pill_button(ui, &palette, catalog.get("common.cancel"), false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        }
    });
}

fn edit_playlist(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let catalog = app.catalog;
    let busy = app.playlist_busy;
    let Some(Dialog::EditPlaylist {
        id,
        name,
        description,
        public,
    }) = &mut app.dialog
    else {
        return;
    };
    theme::text(
        ui,
        catalog.get("dialog.edit_details"),
        theme::bold(20.0),
        palette.text,
    );
    ui.add_space(12.0);
    theme::text(
        ui,
        catalog.get("common.name"),
        theme::medium(13.0),
        palette.secondary,
    );
    text_field(
        ui,
        &palette,
        "edit-name",
        name,
        catalog.get("dialog.playlist_name_hint_edit"),
        true,
    );
    ui.add_space(10.0);
    theme::text(
        ui,
        catalog.get("common.description"),
        theme::medium(13.0),
        palette.secondary,
    );
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(description)
                    .id(egui::Id::new("edit-description"))
                    .hint_text(
                        egui::RichText::new(catalog.get("dialog.description_hint"))
                            .color(palette.dim),
                    )
                    .font(theme::regular(14.0))
                    .frame(egui::Frame::NONE)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
        });
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        super::widgets::switch(ui, &palette, public);
        theme::text(
            ui,
            catalog.get("dialog.public_playlist"),
            theme::regular(14.0),
            palette.text,
        );
    });
    ui.add_space(20.0);
    let id = id.clone();
    let name_value = name.trim().to_string();
    let description_value = description.trim().to_string();
    let public_value = *public;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if busy {
            theme::spinner(ui, 18.0, palette.accent);
        } else {
            if theme::pill_button(ui, &palette, catalog.get("common.save"), true).clicked()
                && !name_value.is_empty()
            {
                app.actions.push(Action::UpdatePlaylist {
                    id: id.clone(),
                    name: name_value.clone(),
                    description: description_value.clone(),
                    public: public_value,
                });
            }
            if theme::pill_button(ui, &palette, catalog.get("common.cancel"), false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        }
    });
}

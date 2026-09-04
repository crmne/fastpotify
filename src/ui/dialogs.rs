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
                            "Delete playlist?"
                        } else {
                            "Remove from Your Library?"
                        },
                        theme::bold(20.0),
                        palette.text,
                    );
                    ui.add_space(8.0);
                    let body = if owned {
                        format!("Delete “{name}”? You can recover it from Spotify for 90 days.")
                    } else {
                        format!("“{name}” will no longer appear in Your Library.")
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
                            if owned { "Delete" } else { "Remove" },
                            true,
                        )
                        .clicked()
                        {
                            app.actions.push(Action::DeletePlaylist(id.clone()));
                        }
                        if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                            app.actions.push(Action::CloseDialog);
                        }
                    });
                }
                Dialog::ConfirmPlaylistDuplicates {
                    playlist_id,
                    playlist_name,
                    items,
                    duplicate_uris,
                } => {
                    let multiple = items.len() > 1;
                    theme::text(
                        ui,
                        if multiple {
                            "Songs already in this playlist"
                        } else {
                            "Song already in this playlist"
                        },
                        theme::bold(20.0),
                        palette.text,
                    );
                    ui.add_space(8.0);
                    let body = duplicate_message(&playlist_name, &items, &duplicate_uris);
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
                        if theme::pill_button(ui, &palette, "Add anyway", true).clicked() {
                            app.actions.push(Action::ConfirmAddToPlaylist {
                                playlist_id: playlist_id.clone(),
                                playlist_name: playlist_name.clone(),
                                items: items.clone(),
                            });
                        }
                        if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                            app.actions.push(Action::CloseDialog);
                        }
                    });
                }
                Dialog::Shortcuts => {
                    theme::text(ui, "Keyboard shortcuts", theme::bold(20.0), palette.text);
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
                    // The list is longer than a small screen is tall, so it
                    // scrolls inside the dialog rather than running off the
                    // bottom with the Done button beyond reach.
                    let room = ui.ctx().content_rect().height() - 190.0;
                    egui::ScrollArea::vertical()
                        .max_height(room.max(120.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            egui::Grid::new("shortcuts")
                                .num_columns(2)
                                .spacing([24.0, 8.0])
                                .show(ui, |ui| {
                                    for (keys, description) in super::keys::SHORTCUTS {
                                        cell(ui, keys, theme::semibold(13.0), palette.text);
                                        cell(
                                            ui,
                                            description,
                                            theme::regular(13.5),
                                            palette.secondary,
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::pill_button(ui, &palette, "Done", true).clicked() {
                            app.actions.push(Action::CloseDialog);
                        }
                    });
                }
                Dialog::PremiumNeeded => {
                    theme::text(
                        ui,
                        "This account cannot play music here",
                        theme::bold(20.0),
                        palette.text,
                    );
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                "Playback needs Spotify Premium. Free accounts can browse \
                                 and search, but cannot play music through Fastpotify.",
                            )
                            .font(theme::regular(14.0))
                            .color(palette.secondary),
                        )
                        .wrap(),
                    );
                    ui.add_space(20.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::pill_button(ui, &palette, "OK", true).clicked() {
                            app.actions.push(Action::CloseDialog);
                        }
                    });
                }
            }
        });
    app.dialog_rect = Some(response.response.rect);
    if response.should_close() {
        app.actions.push(Action::CloseDialog);
    }
}

fn duplicate_message(
    playlist_name: &str,
    items: &[crate::api::models::PlayableItem],
    duplicate_uris: &[String],
) -> String {
    let mut seen = std::collections::HashSet::new();
    let names: Vec<&str> = items
        .iter()
        .filter(|item| duplicate_uris.iter().any(|uri| uri == item.uri()))
        .map(crate::api::models::PlayableItem::name)
        .filter(|name| seen.insert(*name))
        .collect();
    let named = match names.as_slice() {
        [] => "This song".to_string(),
        [name] => format!("“{name}”"),
        [first, second] => format!("“{first}” and “{second}”"),
        [first, second, rest @ ..] => {
            format!("“{first}”, “{second}”, and {} more", rest.len())
        }
    };
    let verb = if names.len() <= 1 { "is" } else { "are" };
    let question = if items.len() == 1 {
        "Add it again?"
    } else {
        "Add all selected songs anyway?"
    };
    format!("{named} {verb} already in “{playlist_name}”. {question}")
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
    let busy = app.playlist_busy;
    let Some(Dialog::CreatePlaylist {
        name,
        public,
        add_uris,
    }) = &mut app.dialog
    else {
        return;
    };
    theme::text(ui, "New playlist", theme::bold(20.0), palette.text);
    ui.add_space(12.0);
    theme::text(ui, "Name", theme::medium(13.0), palette.secondary);
    let field = text_field(ui, &palette, "playlist-name", name, "My playlist", true);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        super::widgets::switch(ui, &palette, public);
        theme::text(ui, "Public playlist", theme::regular(14.0), palette.text);
    });
    if !add_uris.is_empty() {
        ui.add_space(6.0);
        let count = add_uris.len();
        theme::text(
            ui,
            format!(
                "{count} song{} will be added.",
                if count == 1 { "" } else { "s" }
            ),
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
            let create = theme::pill_button(ui, &palette, "Create", true).clicked() || submit;
            if create && !name_value.is_empty() {
                app.actions.push(Action::CreatePlaylist {
                    name: name_value.clone(),
                    public: public_value,
                    add_uris: uris.clone(),
                });
            }
            if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        }
    });
}

fn edit_playlist(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
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
    theme::text(ui, "Edit details", theme::bold(20.0), palette.text);
    ui.add_space(12.0);
    theme::text(ui, "Name", theme::medium(13.0), palette.secondary);
    text_field(ui, &palette, "edit-name", name, "Playlist name", true);
    ui.add_space(10.0);
    theme::text(ui, "Description", theme::medium(13.0), palette.secondary);
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(description)
                    .id(egui::Id::new("edit-description"))
                    .hint_text(egui::RichText::new("Optional description").color(palette.dim))
                    .font(theme::regular(14.0))
                    .frame(egui::Frame::NONE)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
        });
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        super::widgets::switch(ui, &palette, public);
        theme::text(ui, "Public playlist", theme::regular(14.0), palette.text);
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
            if theme::pill_button(ui, &palette, "Save", true).clicked() && !name_value.is_empty() {
                app.actions.push(Action::UpdatePlaylist {
                    id: id.clone(),
                    name: name_value.clone(),
                    description: description_value.clone(),
                    public: public_value,
                });
            }
            if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                app.actions.push(Action::CloseDialog);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::duplicate_message;
    use crate::api::models::{PlayableItem, Track};

    fn song(uri: &str, name: &str) -> PlayableItem {
        PlayableItem::Track(Track {
            uri: uri.into(),
            name: name.into(),
            ..Default::default()
        })
    }

    #[test]
    fn duplicate_dialog_names_the_song() {
        let items = vec![song("spotify:track:honey", "Honey")];
        let message = duplicate_message(
            "The best music ever",
            &items,
            &["spotify:track:honey".into()],
        );

        assert_eq!(
            message,
            "“Honey” is already in “The best music ever”. Add it again?"
        );
    }

    #[test]
    fn duplicate_dialog_names_only_the_duplicates_in_a_selection() {
        let items = vec![
            song("spotify:track:honey", "Honey"),
            song("spotify:track:new", "New song"),
        ];
        let message = duplicate_message(
            "The best music ever",
            &items,
            &["spotify:track:honey".into()],
        );

        assert_eq!(
            message,
            "“Honey” is already in “The best music ever”. Add all selected songs anyway?"
        );
    }
}

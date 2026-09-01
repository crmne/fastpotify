//! The sign-in screen.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::app::App;
use crate::backend::AuthStatus;
use crate::model::Action;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui, connecting: bool) {
    let palette = app.palette;
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            super::titlebar_drag(ui, rect);
            let top = super::blend(palette.window, palette.accent, 0.10);
            super::widgets::paint_vertical_gradient(ui, rect, top, palette.window);
            let card_width = 440.0;
            let card_height = 380.0;
            let card = egui::Rect::from_center_size(rect.center() - Vec2::new(0.0, 20.0), Vec2::new(card_width, card_height));
            let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(card).layout(Layout::top_down(Align::Center)));
            Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS + 8))
                .inner_margin(Margin::same(36))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 16],
                    blur: 48,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(&mut card_ui, |ui| {
                    ui.set_width(card_width - 72.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(72.0), egui::Sense::hover());
                    theme::logo(ui, logo.center(), 72.0, palette.accent, palette.on_accent);
                    ui.add_space(6.0);
                    theme::text(ui, "Fastpotify", theme::bold(30.0), palette.text);
                    theme::text(ui, "A native Spotify client.", theme::regular(14.5), palette.secondary);
                    ui.add_space(22.0);
                    match &app.auth {
                        AuthStatus::WaitingForBrowser { url } => {
                            let url = url.clone();
                            ui.horizontal(|ui| {
                                ui.add_space((ui.available_width() - 250.0).max(0.0) / 2.0);
                                theme::spinner(ui, 18.0, palette.accent);
                                theme::text(ui, "Waiting for Spotify in your browser…", theme::medium(14.0), palette.text);
                            });
                            ui.add_space(6.0);
                            if theme::link(ui, "Didn't open? Open the sign-in page again", theme::regular(13.0), palette.secondary).clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(url));
                            }
                            ui.add_space(14.0);
                            if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                                app.actions.push(Action::CancelSignIn);
                            }
                        }
                        _ if connecting => {
                            ui.horizontal(|ui| {
                                ui.add_space((ui.available_width() - 200.0).max(0.0) / 2.0);
                                theme::spinner(ui, 18.0, palette.accent);
                                theme::text(ui, "Connecting to Spotify…", theme::medium(14.0), palette.text);
                            });
                        }
                        AuthStatus::Failed(message) => {
                            let message = message.clone();
                            ui.add(
                                egui::Label::new(egui::RichText::new(message).font(theme::regular(13.0)).color(palette.danger)).wrap(),
                            );
                            ui.add_space(12.0);
                            if big_button(ui, app, "Try again") {
                                app.actions.push(Action::SignIn);
                            }
                            if app.settings.web_client_id.is_some() {
                                ui.add_space(10.0);
                                if theme::pill_button(
                                    ui,
                                    &palette,
                                    "Use the shared Spotify app instead",
                                    false,
                                )
                                .clicked()
                                {
                                    // A wrong personal Client ID trapped the
                                    // user here with Settings out of reach.
                                    app.settings.web_client_id = None;
                                    app.mark_settings_dirty();
                                    app.actions.push(Action::ConfigurePersonalWebApp);
                                }
                            }
                        }
                        _ => {
                            if big_button(ui, app, "Sign in with Spotify") {
                                app.actions.push(Action::SignIn);
                            }
                            ui.add_space(10.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new("Sign in through your browser. Fastpotify never sees your password. Local playback needs Spotify Premium.")
                                        .font(theme::regular(12.5))
                                        .color(palette.secondary),
                                )
                                .wrap(),
                            );
                            if app.settings.web_client_id.is_some() {
                                // A wrong personal Client ID dead-ends in the
                                // browser on Spotify's side, so the app never
                                // hears it failed; the way out has to stand
                                // here, not only on the failure screen.
                                ui.add_space(10.0);
                                if theme::pill_button(
                                    ui,
                                    &palette,
                                    "Use the shared Spotify app instead",
                                    false,
                                )
                                .clicked()
                                {
                                    app.settings.web_client_id = None;
                                    app.mark_settings_dirty();
                                    app.actions.push(Action::ConfigurePersonalWebApp);
                                }
                            }
                        }
                    }
                });
            ui.painter().text(
                egui::pos2(rect.center().x, rect.bottom() - 24.0),
                egui::Align2::CENTER_BOTTOM,
                format!("Fastpotify {} • not affiliated with Spotify", env!("CARGO_PKG_VERSION")),
                theme::regular(11.5),
                palette.dim,
            );
        });
}

fn big_button(ui: &mut egui::Ui, app: &App, label: &str) -> bool {
    let palette = app.palette;
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), theme::bold(15.0), palette.on_accent);
    let size = Vec2::new(ui.available_width().min(300.0), 46.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if response.hovered() {
        palette.accent_hover
    } else {
        palette.accent
    };
    ui.painter().rect_filled(rect, 23.0, fill);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        palette.on_accent,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

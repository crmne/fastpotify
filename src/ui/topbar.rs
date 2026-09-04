//! Navigation arrows, search, and the account menu above every page.

use egui::{Align, CornerRadius, Layout, Sense, Vec2, pos2, vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Page};
use crate::theme::{self, Icon, Palette};

fn nav_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(32.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if ui.is_rect_visible(rect) {
        let fill = if palette.dark {
            egui::Color32::from_black_alpha(90)
        } else {
            egui::Color32::from_black_alpha(20)
        };
        ui.painter().circle_filled(rect.center(), 16.0, fill);
        let color = if !enabled {
            palette.dim
        } else if response.hovered() {
            palette.text
        } else {
            palette.secondary
        };
        theme::paint_icon(ui, icon, rect, 20.0, color);
    }
    if enabled {
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tooltip)
    } else {
        response
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let width = ui.available_width();
    let window_controls = super::window_controls_reservation(
        ui.ctx(),
        app.show_queue_panel,
        app.show_lyrics_panel,
        width,
    );
    // Where the titlebar used to be: the bar grows upwards into that space and
    // its empty parts drag the window.
    let inset = theme::titlebar_inset(ui.ctx());
    let content_height = theme::TOP_BAR_HEIGHT + inset;
    let height = content_height + window_controls.topbar_top;
    super::titlebar_drag(
        ui,
        egui::Rect::from_min_size(ui.cursor().min, vec2(width, height)),
    );
    ui.add_space(window_controls.topbar_top);
    ui.allocate_ui_with_layout(
        vec2(width, content_height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(super::widgets::PAGE_PADDING);
            ui.spacing_mut().item_spacing.x = 8.0;
            if !app.settings.sidebar_visible {
                if nav_button(
                    ui,
                    &palette,
                    Icon::PanelLeft,
                    true,
                    super::keys::platform_shortcut("Show sidebar (Ctrl+B)", "Show sidebar (Cmd+B)"),
                )
                .clicked()
                {
                    app.actions.push(Action::ToggleSidebar);
                }
                ui.add_space(2.0);
            }
            if !app.settings.sidebar_visible
                && nav_button(ui, &palette, Icon::House, true, "Home").clicked()
            {
                app.actions.push(Action::Open(Page::Home));
            }
            if nav_button(ui, &palette, Icon::ChevronLeft, app.can_go_back(), "Back").clicked() {
                app.actions.push(Action::Back);
            }
            if nav_button(
                ui,
                &palette,
                Icon::ChevronRight,
                app.can_go_forward(),
                "Forward",
            )
            .clicked()
            {
                app.actions.push(Action::Forward);
            }
            ui.add_space(8.0);

            let search_room = (ui.available_width() - window_controls.topbar_width).max(0.0);
            let search_width = (search_room * 0.5).clamp(200.0, 440.0);
            let id = egui::Id::new("global-search");
            let before = app.search.query.clone();
            let response = super::widgets::search_field(
                ui,
                &palette,
                id,
                &mut app.search.query,
                "What do you want to play?",
                search_width,
            );
            if app.search.focus_requested {
                app.search.focus_requested = false;
                response.request_focus();
            }
            if response.gained_focus() && !matches!(app.page(), Page::Search) {
                app.actions.push(Action::Open(Page::Search));
            }
            if app.search.query != before {
                app.search.typed_at = Some(std::time::Instant::now());
                if !matches!(app.page(), Page::Search) {
                    app.actions.push(Action::Open(Page::Search));
                }
            }
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                let query = app.search.query.clone();
                app.actions.push(Action::Search(query));
            }
            if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                response.surrender_focus();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(window_controls.topbar_width);
                ui.add_space(super::widgets::PAGE_PADDING);
                // Account.
                let (name, avatar) = app
                    .user
                    .as_ref()
                    .map(|user| {
                        (
                            user.name().to_string(),
                            pick_image(&user.images, 64).map(str::to_string),
                        )
                    })
                    .unwrap_or_default();
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
                if ui.is_rect_visible(rect) {
                    let fill = if response.hovered() {
                        palette.surface_hover
                    } else {
                        palette.surface
                    };
                    ui.painter().circle_filled(rect.center(), 18.0, fill);
                    let inner = egui::Rect::from_center_size(rect.center(), Vec2::splat(28.0));
                    match avatar.as_deref() {
                        Some(url) => super::widgets::paint_cover(
                            ui,
                            &palette,
                            Some(url),
                            inner,
                            14.0,
                            Icon::User,
                        ),
                        None => {
                            let initial = name
                                .chars()
                                .next()
                                .unwrap_or('?')
                                .to_uppercase()
                                .to_string();
                            ui.painter()
                                .circle_filled(inner.center(), 14.0, palette.accent);
                            ui.painter().text(
                                inner.center(),
                                egui::Align2::CENTER_CENTER,
                                initial,
                                theme::bold(13.0),
                                palette.on_accent,
                            );
                        }
                    }
                }
                let response = response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(&name);
                egui::Popup::menu(&response)
                    .frame(super::widgets::menu_frame(&palette))
                    .align(egui::RectAlign::BOTTOM_END)
                    .show(|ui| {
                        ui.set_width(200.0);
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            theme::text(ui, &name, theme::semibold(14.0), palette.text);
                        });
                        if let Some(product) =
                            app.user.as_ref().and_then(|user| user.product.clone())
                        {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                theme::text(
                                    ui,
                                    capitalize(&product),
                                    theme::regular(12.0),
                                    palette.secondary,
                                );
                            });
                        }
                        super::widgets::menu_separator(ui, &palette);
                        if super::widgets::menu_item(ui, &palette, Some(Icon::Settings), "Settings")
                        {
                            app.actions.push(Action::Open(Page::Settings));
                        }
                        if super::widgets::menu_item(
                            ui,
                            &palette,
                            Some(Icon::Info),
                            "Keyboard shortcuts",
                        ) {
                            app.actions
                                .push(Action::ShowDialog(crate::model::Dialog::Shortcuts));
                        }
                        super::widgets::menu_separator(ui, &palette);
                        if super::widgets::menu_item(ui, &palette, Some(Icon::LogOut), "Sign out") {
                            app.actions.push(Action::SignOut);
                        }
                    });
                ui.add_space(4.0);
                if theme::icon_button(
                    ui,
                    Icon::Settings,
                    19.0,
                    palette.secondary,
                    palette.text,
                    "Settings",
                )
                .clicked()
                {
                    app.actions.push(Action::Open(Page::Settings));
                }
                if theme::icon_button(
                    ui,
                    Icon::AudioLines,
                    19.0,
                    if app.settings.milkdrop_open {
                        palette.accent
                    } else {
                        palette.secondary
                    },
                    palette.text,
                    super::keys::platform_shortcut(
                        "MilkDrop visualiser (Ctrl+Shift+K)",
                        "MilkDrop visualiser (Cmd+Shift+K)",
                    ),
                )
                .clicked()
                {
                    app.actions.push(Action::ToggleWinampMilkdrop);
                }
                if theme::icon_button(
                    ui,
                    Icon::Shrink,
                    19.0,
                    palette.secondary,
                    palette.text,
                    super::keys::platform_shortcut(
                        "Winamp mini player (Ctrl+M)",
                        "Winamp mini player (Cmd+Shift+M)",
                    ),
                )
                .clicked()
                {
                    app.actions.push(Action::ToggleWinampWindow);
                }
                // A quiet spinner once the app has been talking to Spotify for a
                // while, long enough that fast requests never flash it.
                if app
                    .backend
                    .activity()
                    .busy(std::time::Duration::from_millis(1000))
                {
                    theme::spinner(ui, 15.0, palette.secondary)
                        .on_hover_text("Waiting for Spotify…");
                }
                // Where playback is.
                if let Some(now) = app.now_playing()
                    && !now.local
                {
                    let label = format!(
                        "Playing on {}",
                        now.device_name.unwrap_or_else(|| "another device".into())
                    );
                    let galley =
                        ui.painter()
                            .layout_no_wrap(label, theme::medium(12.5), palette.accent);
                    let size = galley.size() + vec2(28.0, 12.0);
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(14),
                        palette.accent.gamma_multiply(0.16),
                    );
                    let icon_rect = egui::Rect::from_center_size(
                        pos2(rect.left() + 14.0, rect.center().y),
                        Vec2::splat(13.0),
                    );
                    Icon::Speaker
                        .image(palette.accent, 13.0)
                        .paint_at(ui, icon_rect);
                    ui.painter().galley(
                        pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        palette.accent,
                    );
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        app.actions.push(Action::ToggleDevicesPopup);
                    }
                }
                // A newer release. Most people never visit a releases page,
                // so the app says so, quietly, until they do.
                if let Some(update) = app.update.clone() {
                    let label = format!("Update to {}", update.version);
                    let galley =
                        ui.painter()
                            .layout_no_wrap(label, theme::medium(12.5), palette.accent);
                    let size = galley.size() + vec2(28.0, 12.0);
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(14),
                        palette.accent.gamma_multiply(0.16),
                    );
                    let icon_rect = egui::Rect::from_center_size(
                        pos2(rect.left() + 14.0, rect.center().y),
                        Vec2::splat(13.0),
                    );
                    Icon::Info
                        .image(palette.accent, 13.0)
                        .paint_at(ui, icon_rect);
                    ui.painter().galley(
                        pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        palette.accent,
                    );
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(format!(
                            "Version {} is available. Open the download page.",
                            update.version
                        ))
                        .clicked()
                    {
                        app.actions.push(Action::OpenUrl(update.url));
                    }
                }
            });
        },
    );
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

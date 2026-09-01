//! Navigation arrows, search, and the account menu above every page.

use egui::{Align, CornerRadius, Layout, Sense, Stroke, Vec2, pos2, vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Page};
use crate::theme::{self, Icon, Palette};

const COMPACT_BREAKPOINT: f32 = 900.0;
const SPACIOUS_BREAKPOINT: f32 = 1180.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TopbarDensity {
    Compact,
    Standard,
    Spacious,
}

impl TopbarDensity {
    fn for_width(width: f32) -> Self {
        if width < COMPACT_BREAKPOINT {
            Self::Compact
        } else if width < SPACIOUS_BREAKPOINT {
            Self::Standard
        } else {
            Self::Spacious
        }
    }

    fn search_cap(self) -> f32 {
        match self {
            Self::Compact => 280.0,
            Self::Standard => 360.0,
            Self::Spacious => 440.0,
        }
    }
}

fn chrome_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    enabled: bool,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(36.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            palette.surface_active
        } else if response.hovered() || response.has_focus() {
            palette.surface_hover
        } else if active {
            palette
                .accent
                .gamma_multiply(if palette.dark { 0.16 } else { 0.10 })
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(0.5),
                CornerRadius::same(10),
                Stroke::new(1.0, palette.accent.gamma_multiply(0.8)),
                egui::StrokeKind::Inside,
            );
        }
        let color = if !enabled {
            palette.dim
        } else if active {
            palette.accent
        } else if response.hovered() || response.has_focus() {
            palette.text
        } else {
            palette.secondary
        };
        theme::paint_icon(ui, icon, rect, 19.0, color);
    }
    if enabled {
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tooltip)
    } else {
        response
    }
}

fn nav_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    chrome_button(ui, palette, icon, enabled, false, tooltip)
}

fn status_chip(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    label: &str,
    accent: bool,
) -> egui::Response {
    let color = if accent {
        palette.accent
    } else {
        palette.secondary
    };
    let fill = if accent {
        palette
            .accent
            .gamma_multiply(if palette.dark { 0.15 } else { 0.10 })
    } else {
        palette.surface
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), theme::medium(12.5), color);
    const HORIZONTAL_PADDING: f32 = 12.0;
    const ICON_SIZE: f32 = 13.0;
    const ICON_GAP: f32 = 8.0;
    let size = vec2(
        HORIZONTAL_PADDING * 2.0 + ICON_SIZE + ICON_GAP + galley.size().x,
        36.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let shown_fill = if response.hovered() || response.has_focus() {
        if accent {
            palette
                .accent
                .gamma_multiply(if palette.dark { 0.23 } else { 0.16 })
        } else {
            palette.surface_hover
        }
    } else {
        fill
    };
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(10), shown_fill);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(0.5),
                CornerRadius::same(10),
                Stroke::new(1.0, palette.accent.gamma_multiply(0.8)),
                egui::StrokeKind::Inside,
            );
        }
        let icon_rect = egui::Rect::from_center_size(
            pos2(
                rect.left() + HORIZONTAL_PADDING + ICON_SIZE / 2.0,
                rect.center().y,
            ),
            Vec2::splat(ICON_SIZE),
        );
        icon.image(color, ICON_SIZE).paint_at(ui, icon_rect);
        ui.painter().galley(
            pos2(
                rect.left() + HORIZONTAL_PADDING + ICON_SIZE + ICON_GAP,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let inset = theme::titlebar_inset(ui.ctx());
    let height = theme::TOP_BAR_HEIGHT + inset;
    egui::Panel::top("top-bar")
        .exact_size(height)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let bar_rect = ui.max_rect();
            ui.painter().hline(
                bar_rect.x_range(),
                bar_rect.bottom() - 0.5,
                Stroke::new(1.0, palette.outline.gamma_multiply(0.45)),
            );
            super::titlebar_drag(ui, bar_rect);

            // A full-size macOS content view puts the traffic lights in this
            // first strip. The top rail owns it so no panel below compensates
            // a second time.
            ui.add_space(inset);
            topbar_contents(app, ui, palette);
        });
}

fn topbar_contents(app: &mut App, ui: &mut egui::Ui, palette: Palette) {
    let width = ui.available_width();
    let density = TopbarDensity::for_width(width);
    ui.allocate_ui_with_layout(
        vec2(width, theme::TOP_BAR_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(super::widgets::PAGE_PADDING);
            ui.spacing_mut().item_spacing.x = 6.0;
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
            ui.add_space(10.0);

            let search_width = (ui.available_width() * 0.46).clamp(190.0, density.search_cap());
            let id = egui::Id::new("global-search");
            let before = app.search.query.clone();
            let response = super::widgets::search_field_with_radius(
                ui,
                &palette,
                id,
                &mut app.search.query,
                if density == TopbarDensity::Compact {
                    "Search"
                } else {
                    "What do you want to play?"
                },
                search_width,
                10.0,
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
                let account_tooltip = if name.is_empty() { "Account" } else { &name };
                let response = response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(account_tooltip);
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
                let update = app.update.clone();
                let more_tooltip = if app.settings.milkdrop_open {
                    "More (MilkDrop is open)"
                } else {
                    "More"
                };
                let more = chrome_button(
                    ui,
                    &palette,
                    Icon::Ellipsis,
                    true,
                    app.settings.milkdrop_open,
                    more_tooltip,
                );
                egui::Popup::menu(&more)
                    .frame(super::widgets::menu_frame(&palette))
                    .align(egui::RectAlign::BOTTOM_END)
                    .show(|ui| {
                        ui.set_width(220.0);
                        if density != TopbarDensity::Spacious
                            && let Some(update) = update.as_ref()
                        {
                            let label = format!("Update to {}", update.version);
                            if super::widgets::menu_item(ui, &palette, Some(Icon::Info), &label) {
                                app.actions.push(Action::OpenUrl(update.url.clone()));
                            }
                            super::widgets::menu_separator(ui, &palette);
                        }
                        if super::widgets::menu_item(ui, &palette, Some(Icon::Settings), "Settings")
                        {
                            app.actions.push(Action::Open(Page::Settings));
                        }
                        super::widgets::menu_separator(ui, &palette);
                        let milkdrop_label = if app.settings.milkdrop_open {
                            super::keys::platform_shortcut(
                                "Close MilkDrop visualiser (Ctrl+Shift+K)",
                                "Close MilkDrop visualiser (Cmd+Shift+K)",
                            )
                        } else {
                            super::keys::platform_shortcut(
                                "Open MilkDrop visualiser (Ctrl+Shift+K)",
                                "Open MilkDrop visualiser (Cmd+Shift+K)",
                            )
                        };
                        if super::widgets::menu_item(
                            ui,
                            &palette,
                            Some(Icon::AudioLines),
                            milkdrop_label,
                        ) {
                            app.actions.push(Action::ToggleWinampMilkdrop);
                        }
                        if super::widgets::menu_item(
                            ui,
                            &palette,
                            Some(Icon::Shrink),
                            super::keys::platform_shortcut(
                                "Winamp mini player (Ctrl+M)",
                                "Winamp mini player (Cmd+Shift+M)",
                            ),
                        ) {
                            app.actions.push(Action::ToggleWinampWindow);
                        }
                    });
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
                    let response = if density == TopbarDensity::Spacious {
                        status_chip(ui, &palette, Icon::Speaker, &label, true)
                    } else {
                        chrome_button(ui, &palette, Icon::Speaker, true, true, &label)
                    };
                    if response.clicked() {
                        app.actions.push(Action::ToggleDevicesPopup);
                    }
                }
                // A newer release. Most people never visit a releases page,
                // so the app says so, quietly, until they do.
                if density == TopbarDensity::Spacious
                    && let Some(update) = update
                {
                    let label = format!("Update to {}", update.version);
                    if status_chip(ui, &palette, Icon::Info, &label, false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_tracks_supported_window_widths() {
        assert_eq!(TopbarDensity::for_width(760.0), TopbarDensity::Compact);
        assert_eq!(TopbarDensity::for_width(899.0), TopbarDensity::Compact);
        assert_eq!(TopbarDensity::for_width(900.0), TopbarDensity::Standard);
        assert_eq!(TopbarDensity::for_width(1_179.0), TopbarDensity::Standard);
        assert_eq!(TopbarDensity::for_width(1_180.0), TopbarDensity::Spacious);
    }

    #[test]
    fn search_caps_leave_more_room_as_the_window_grows() {
        assert_eq!(TopbarDensity::Compact.search_cap(), 280.0);
        assert_eq!(TopbarDensity::Standard.search_cap(), 360.0);
        assert_eq!(TopbarDensity::Spacious.search_cap(), 440.0);
    }
}

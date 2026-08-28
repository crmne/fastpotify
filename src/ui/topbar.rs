//! Navigation arrows, search, and the account menu above every page.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, CornerRadius, Layout, Sense, Vec2, pos2, vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Page};
use crate::theme::{self, Icon, Palette};

pub(crate) const AVATAR_SIZE: f32 = 36.0;
const SETTINGS_HIT: f32 = 31.0;
const CONTROL_GAP: f32 = 4.0;
const SPINNER_SIZE: f32 = 15.0;
const SOURCE_MIN: f32 = 48.0;

/// Right-edge cluster: inset, avatar, gap, settings, optional spinner.
/// Search and the source label must yield before this width is stolen.
pub(crate) fn topbar_right_reserved(spinner: bool) -> f32 {
    let spinner_w = if spinner { SPINNER_SIZE + 8.0 } else { 0.0 };
    super::widgets::PAGE_PADDING + AVATAR_SIZE + CONTROL_GAP + SETTINGS_HIT + spinner_w
}

pub(crate) fn topbar_search_width(available_after_nav: f32, spinner: bool) -> f32 {
    let reserved = topbar_right_reserved(spinner);
    let cap = (available_after_nav - reserved).max(0.0);
    let preferred = (available_after_nav * 0.5).clamp(160.0, 440.0);
    preferred.min(cap)
}

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
    ui.allocate_ui_with_layout(
        vec2(width, theme::TOP_BAR_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(super::widgets::PAGE_PADDING);
            ui.spacing_mut().item_spacing.x = 8.0;
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

            let spinner = app
                .backend
                .activity()
                .busy(std::time::Duration::from_millis(1000));
            let search_width = topbar_search_width(ui.available_width(), spinner);
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
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(AVATAR_SIZE), Sense::click());
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
                        ui.set_min_width(200.0);
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
                if spinner {
                    theme::spinner(ui, SPINNER_SIZE, palette.secondary)
                        .on_hover_text("Talking to Spotify…");
                }
                if let Some(now) = app.now_playing()
                    && (!now.local || now.source_label.is_some())
                {
                    let label = if now.local {
                        now.source_label
                            .clone()
                            .unwrap_or_else(|| "Alternate local audio".into())
                    } else {
                        format!(
                            "Playing on {}",
                            now.device_name.unwrap_or_else(|| "another device".into())
                        )
                    };
                    let max_w = ui.available_width();
                    if max_w >= SOURCE_MIN {
                        source_chip(ui, &palette, &label, max_w, &mut app.actions);
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
                            "Fastpotify {} is out. Opens the download page.",
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

fn source_chip(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    max_width: f32,
    actions: &mut Vec<Action>,
) {
    let text_max = (max_width - 28.0).max(8.0);
    let mut job = LayoutJob::single_section(
        label.to_string(),
        TextFormat {
            font_id: theme::medium(12.5),
            color: palette.accent,
            ..Default::default()
        },
    );
    job.wrap.max_width = text_max;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
    let size = vec2(
        (galley.size().x + 28.0).min(max_width),
        galley.size().y + 12.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(14),
        palette.accent.gamma_multiply(0.16),
    );
    let icon_rect =
        egui::Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), Vec2::splat(13.0));
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
        actions.push(Action::ToggleDevicesPopup);
    }
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
    fn right_cluster_keeps_avatar_inset() {
        let reserved = topbar_right_reserved(false);
        assert!(reserved >= super::super::widgets::PAGE_PADDING + AVATAR_SIZE);
        let total = 760.0;
        let nav = super::super::widgets::PAGE_PADDING + 32.0 + 8.0 + 32.0 + 8.0;
        let after_nav = total - nav;
        let search = topbar_search_width(after_nav, false);
        let right = after_nav - search;
        assert!(
            right + 0.5 >= topbar_right_reserved(false),
            "search stole the avatar inset: search={search} right={right} reserved={reserved}"
        );
    }

    #[test]
    fn narrow_width_shrinks_search_before_avatar() {
        let nav = super::super::widgets::PAGE_PADDING + 32.0 + 8.0 + 32.0 + 8.0;
        let total = 500.0;
        let after_nav = total - nav;
        let search = topbar_search_width(after_nav, true);
        let right = after_nav - search;
        assert!(right + 0.5 >= topbar_right_reserved(true));
        assert!(
            search < 200.0,
            "search should yield on a narrow bar, got {search}"
        );
        let avatar_right = total - super::super::widgets::PAGE_PADDING;
        let avatar_left = avatar_right - AVATAR_SIZE;
        assert!(avatar_left >= 0.0);
        assert!(avatar_right <= total);
    }

    #[test]
    fn long_source_does_not_reduce_right_reserved() {
        let reserved = topbar_right_reserved(false);
        let after_nav = 400.0;
        let search = topbar_search_width(after_nav, false);
        let leftover = after_nav - search;
        assert!(leftover + 0.5 >= reserved);
        let source_max = leftover - reserved;
        assert!(source_max < 280.0 || leftover >= reserved);
    }
}

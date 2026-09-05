//! The words of the playing track, in a side panel that follows the song.

use egui::{Align, Color32, Frame, Layout, Margin, Rect, Sense, UiBuilder, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Loadable};
use crate::theme::{self, Icon};

use super::widgets;

const LINE_SIZE: f32 = 22.0;
const LINE_GAP: f32 = 18.0;
/// How long a line takes to light up or fade.
const LIGHT_UP_SECONDS: f32 = 0.3;

fn blend(from: egui::Color32, to: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    egui::Color32::from(egui::Rgba::from(from) * (1.0 - t) + egui::Rgba::from(to) * t)
}

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let panel = egui::Panel::right("lyrics-panel")
        .resizable(true)
        .default_size(app.settings.lyrics_width)
        .size_range(theme::SIDE_PANEL_MIN_WIDTH..=640.0)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(theme::Palette::dark().window)
                .inner_margin(Margin::symmetric(16, 12)),
        );
    let response = panel.show(ui, |ui| {
        background(app, ui, ui.max_rect().expand2(vec2(16.0, 12.0)));
        let window_controls = super::window_controls_reservation(
            ui.ctx(),
            app.show_queue_panel,
            app.show_lyrics_panel,
            ui.available_width(),
        );
        ui.add_space(window_controls.lyrics_top);
        header(app, ui, false);
        ui.add_space(14.0);
        track_heading(app, ui, false);
        ui.add_space(12.0);
        contents(app, ui, false);
    });
    let current_width = response.response.rect.width();
    if (app.settings.lyrics_width - current_width).abs() > 1.0 {
        app.settings.lyrics_width = current_width;
        app.actions.push(Action::SettingsChanged);
    }
}

pub fn fullscreen(app: &mut App, ui: &mut egui::Ui) {
    egui::CentralPanel::default()
        .frame(Frame::new().fill(theme::Palette::dark().window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            background(app, ui, rect);
            let width = fullscreen_content_width(rect.width());
            let top = theme::titlebar_inset(ui.ctx()) + 24.0;
            let region = Rect::from_min_max(
                pos2(rect.center().x - width / 2.0, rect.top() + top),
                pos2(rect.center().x + width / 2.0, rect.bottom()),
            );
            let mut content = ui.new_child(UiBuilder::new().max_rect(region));
            header(app, &mut content, true);
            content.add_space(20.0);
            track_heading(app, &mut content, true);
            content.add_space(16.0);
            contents(app, &mut content, true);
        });
}

fn fullscreen_content_width(viewport_width: f32) -> f32 {
    let available = (viewport_width - 48.0).max(0.0);
    (viewport_width * 0.72).clamp(400.0, 960.0).min(available)
}

fn preferred_backdrop_art(small: Option<String>, large: Option<String>) -> Option<String> {
    small.or(large)
}

fn background(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    theme::apply_local(ui, &theme::Palette::dark());
    let art = app
        .now_playing()
        .and_then(|now| preferred_backdrop_art(now.art_small, now.art_url));
    let painter = ui.painter().with_clip_rect(rect);
    if let Some(texture) = app
        .lyrics_backdrop
        .texture(ui.ctx(), app.backend.art(), art.as_deref())
    {
        painter.image(
            texture.id(),
            rect,
            cover_uv(rect.size(), texture.size_vec2()),
            Color32::from_gray(180),
        );
    }
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(120));
    widgets::paint_vertical_gradient(
        ui,
        rect,
        Color32::from_black_alpha(0),
        Color32::from_black_alpha(95),
    );
}

fn cover_uv(view: egui::Vec2, image: egui::Vec2) -> Rect {
    let ratio = (view.x / view.y.max(1.0)) / (image.x / image.y.max(1.0));
    let size = if ratio > 1.0 {
        vec2(1.0, 1.0 / ratio)
    } else {
        vec2(ratio, 1.0)
    };
    Rect::from_center_size(pos2(0.5, 0.5), size)
}

fn header(app: &mut App, ui: &mut egui::Ui, fullscreen: bool) {
    let palette = theme::Palette::dark();
    ui.horizontal(|ui| {
        theme::text(ui, "Lyrics", theme::bold(18.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (icon, hint, action) = if fullscreen {
                (
                    Icon::Shrink,
                    "Leave full screen (Esc)",
                    Action::SetLyricsFullscreen(false),
                )
            } else {
                (Icon::X, "Close lyrics", Action::ToggleLyricsPanel)
            };
            if theme::icon_button(ui, icon, 18.0, palette.text, palette.text, hint).clicked() {
                app.actions.push(action);
            }
            if !fullscreen
                && theme::icon_button(
                    ui,
                    Icon::Expand,
                    18.0,
                    palette.text,
                    palette.text,
                    "Full screen lyrics",
                )
                .clicked()
            {
                app.actions.push(Action::SetLyricsFullscreen(true));
            }
            let hint = if app.lyrics_reduce_motion {
                "Enable lyrics motion"
            } else {
                "Reduce lyrics motion"
            };
            if theme::icon_button(
                ui,
                Icon::AudioLines,
                18.0,
                if app.lyrics_reduce_motion {
                    palette.secondary
                } else {
                    palette.text
                },
                palette.text,
                hint,
            )
            .clicked()
            {
                app.actions.push(Action::ToggleLyricsMotion);
            }
            let loaded = matches!(&app.lyrics, Loadable::Loaded(Some(_)));
            if loaded
                && !app.lyrics_following
                && theme::pill_button(ui, &palette, "Follow", false).clicked()
            {
                app.actions.push(Action::FollowLyrics);
            }
        });
    });
}

fn track_heading(app: &App, ui: &mut egui::Ui, fullscreen: bool) {
    if let Some(now) = app.now_playing() {
        ui.horizontal(|ui| {
            let size = if fullscreen { 52.0 } else { 36.0 };
            let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
            widgets::paint_cover(
                ui,
                &theme::Palette::dark(),
                now.art_small.as_deref().or(now.art_url.as_deref()),
                rect,
                4.0,
                Icon::Music,
            );
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&now.title)
                            .font(theme::semibold(if fullscreen { 22.0 } else { 15.0 }))
                            .color(Color32::WHITE),
                    )
                    .truncate(),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&now.subtitle)
                            .font(theme::regular(13.0))
                            .color(Color32::from_gray(235)),
                    )
                    .truncate(),
                );
            });
        });
    }
}

fn contents(app: &mut App, ui: &mut egui::Ui, fullscreen: bool) {
    let palette = theme::Palette::dark();
    let Some(now) = app.now_playing() else {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Mic,
            "Nothing playing",
            "Play a song to see its lyrics.",
        );
        return;
    };
    let lyrics = match &app.lyrics {
        Loadable::NotLoaded | Loadable::Loading => {
            widgets::loading_row(ui, &palette);
            return;
        }
        Loadable::Failed(error) => {
            let message = format!("Couldn't fetch the lyrics: {error}");
            ui.add_space(8.0);
            theme::text(ui, message, theme::regular(13.0), palette.text);
            ui.add_space(8.0);
            if theme::pill_button(ui, &palette, "Try again", false).clicked() {
                app.actions.push(Action::RetryLyrics);
            }
            return;
        }
        Loadable::Loaded(None) => {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Mic,
                "No lyrics",
                "No lyrics found for this track.",
            );
            return;
        }
        Loadable::Loaded(Some(lyrics)) if lyrics.instrumental => {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Music,
                "Instrumental",
                "No timed lyrics for this track.",
            );
            return;
        }
        Loadable::Loaded(Some(lyrics)) => lyrics.clone(),
    };

    let active = lyrics.active_line(now.position_ms);
    let viewport = ui.available_rect_before_wrap();
    let manual_scroll = ui.rect_contains_pointer(viewport)
        && ui.input(|input| {
            input.smooth_scroll_delta.y != 0.0
                || (input.pointer.primary_down() && input.pointer.delta().y != 0.0)
        });
    let following = app.lyrics_following && !manual_scroll;
    let follow = following && app.lyrics_line_shown != Some(active);
    let animation =
        egui::style::ScrollAnimation::duration(if app.lyrics_reduce_motion { 0.0 } else { 0.45 });
    let size = if fullscreen {
        (ui.available_width() * 0.046).clamp(28.0, 42.0)
    } else {
        LINE_SIZE
    };
    // The line being sung brightens; all lines keep the same font metrics
    // so highlighting cannot rewrap the words during a transition.
    // A line takes 300 ms to light up or fade, unless motion is reduced.
    let quiet = palette.text.gamma_multiply(0.68);
    ui.spacing_mut().scroll.fade.strength = 0.0;
    egui::ScrollArea::vertical()
        .id_salt(("lyrics-scroll", &now.uri, fullscreen))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Before the first line there is nothing to highlight, so the
            // panel sits at the top rather than wherever it was left.
            if follow && lyrics.synced && active.is_none() {
                let top = ui.cursor().min;
                ui.scroll_to_rect_animation(
                    egui::Rect::from_min_size(top, egui::vec2(1.0, 1.0)),
                    Some(Align::Min),
                    animation,
                );
            }
            let padding = if lyrics.synced {
                (viewport.height() * 0.5 - size).max(12.0)
            } else {
                12.0
            };
            ui.add_space(padding);
            for (index, line) in lyrics.lines.iter().enumerate() {
                let is_active = active == Some(index);
                let lit = ui.ctx().animate_bool_with_time(
                    egui::Id::new("lyric-line").with((&now.uri, fullscreen, index)),
                    is_active,
                    if app.lyrics_reduce_motion {
                        0.0
                    } else {
                        LIGHT_UP_SECONDS
                    },
                );
                let color = if lyrics.synced {
                    blend(quiet, palette.text, lit)
                } else {
                    palette.text
                };
                let font = theme::bold(size);
                // A timed line with no words is the band playing on.
                let text = if line.text.is_empty() && lyrics.synced {
                    "\u{266a}"
                } else {
                    line.text.as_str()
                };
                let sense = if lyrics.synced {
                    Sense::click()
                } else {
                    Sense::hover()
                };
                let galley = crate::bidi::layout(
                    ui.painter(),
                    text,
                    font,
                    color,
                    ui.available_width(),
                    usize::MAX,
                    None,
                );
                let center = ui.cursor().top() + galley.size().y * 0.5;
                let edge = ((center - viewport.top()).min(viewport.bottom() - center)
                    / (size * 2.0))
                    .clamp(0.0, 1.0);
                let response = ui
                    .scope(|ui| {
                        ui.multiply_opacity(edge * edge * (3.0 - 2.0 * edge));
                        ui.add(egui::Label::new(galley).sense(sense))
                    })
                    .inner;
                let rect = response.rect;
                if lyrics.synced {
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked()
                        && let Some(at_ms) = line.at_ms
                    {
                        app.actions.push(Action::Seek(at_ms));
                        app.actions.push(Action::FollowLyrics);
                    }
                }
                if is_active && follow {
                    ui.scroll_to_rect_animation(rect, Some(Align::Center), animation);
                }
                ui.add_space(if fullscreen { LINE_GAP * 1.5 } else { LINE_GAP });
            }
            // Words without timing can only be followed by the clock: sit
            // at the part of the text the song is probably at.
            if following && !lyrics.synced && now.duration_ms > 0 {
                let fraction =
                    (f64::from(now.position_ms) / f64::from(now.duration_ms)).clamp(0.0, 1.0);
                let content = ui.min_rect();
                let y = content.top() + content.height() * fraction as f32;
                ui.scroll_to_rect_animation(
                    egui::Rect::from_min_max(
                        egui::pos2(content.left(), y),
                        egui::pos2(content.right(), y + 1.0),
                    ),
                    Some(Align::Center),
                    animation,
                );
            }
            ui.add_space(padding.max(60.0));
        });
    // Scrolling by hand means the reader wants to look elsewhere; the
    // Follow button in the header picks the song back up.
    if manual_scroll && app.lyrics_following {
        app.actions.push(Action::PauseLyricsFollow);
    }
    app.lyrics_line_shown = Some(active);
    if now.playing
        && lyrics.synced
        && let Some(next) = lyrics
            .lines
            .iter()
            .filter_map(|line| line.at_ms)
            .find(|at| *at > now.position_ms)
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(u64::from(
                next - now.position_ms,
            )));
    }
}

#[cfg(test)]
mod tests {
    use super::{fullscreen_content_width, preferred_backdrop_art};

    #[test]
    fn fullscreen_backdrop_prefers_small_art_with_large_art_as_fallback() {
        let small = "small".to_string();
        let large = "large".to_string();
        assert_eq!(
            preferred_backdrop_art(Some(small.clone()), Some(large.clone())),
            Some(small)
        );
        assert_eq!(
            preferred_backdrop_art(None, Some(large.clone())),
            Some(large)
        );
        assert_eq!(preferred_backdrop_art(None, None), None);
    }

    #[test]
    fn fullscreen_content_width_never_inverts_a_narrow_viewport() {
        for viewport_width in [0.0, 24.0, 47.0, 48.0, 64.0, 760.0, 2_000.0] {
            let width = fullscreen_content_width(viewport_width);
            assert!(width >= 0.0);
            assert!(width <= (viewport_width - 48.0).max(0.0));
        }
        assert_eq!(fullscreen_content_width(47.0), 0.0);
        assert_eq!(fullscreen_content_width(2_000.0), 960.0);
    }
}

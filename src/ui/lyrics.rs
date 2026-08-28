//! The words of the playing track, in a side panel that follows the song.

use egui::{Align, Frame, Layout, Margin, Sense};

use crate::app::App;
use crate::model::{Action, Loadable};
use crate::theme::{self, Icon};

use super::widgets;

const LINE_SIZE: f32 = 19.0;
const LINE_GAP: f32 = 10.0;
/// How long a line takes to light up or fade.
const LIGHT_UP_SECONDS: f32 = 0.22;

fn blend(from: egui::Color32, to: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    egui::Color32::from(egui::Rgba::from(from) * (1.0 - t) + egui::Rgba::from(to) * t)
}

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    egui::Panel::right("lyrics-panel")
        .exact_size(360.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                theme::text(ui, "Lyrics", theme::bold(18.0), palette.text);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Close",
                    )
                    .clicked()
                    {
                        app.actions.push(Action::ToggleLyricsPanel);
                    }
                    let loaded = matches!(&app.lyrics, Loadable::Loaded(Some(_)));
                    if loaded
                        && !app.lyrics_following
                        && theme::pill_button(ui, &palette, "Follow", false).clicked()
                    {
                        app.lyrics_following = true;
                        app.lyrics_line_shown = None;
                    }
                    // The accent fill of a pill is the toggle's on state;
                    // Romanize sits left of Follow, Translate left of that.
                    if loaded {
                        if theme::pill_button(
                            ui,
                            &palette,
                            "Romanize",
                            app.settings.lyrics_romanize,
                        )
                        .on_hover_text("Write the lines in Latin letters, to sing along.")
                        .clicked()
                        {
                            app.settings.lyrics_romanize = !app.settings.lyrics_romanize;
                            app.actions.push(Action::SettingsChanged);
                            app.request_translation();
                        }
                        if theme::pill_button(
                            ui,
                            &palette,
                            "Translate",
                            app.settings.lyrics_show_translation,
                        )
                        .on_hover_text("Show each line in your language.")
                        .clicked()
                        {
                            app.settings.lyrics_show_translation =
                                !app.settings.lyrics_show_translation;
                            app.actions.push(Action::SettingsChanged);
                            app.request_translation();
                        }
                    }
                });
            });
            ui.add_space(8.0);
            contents(app, ui);
        });
}

fn contents(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let Some(now) = app.now_playing() else {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Mic,
            "Nothing playing",
            "Play something and its words show up here.",
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
            theme::text(ui, message, theme::regular(13.0), palette.secondary);
            ui.add_space(8.0);
            if theme::pill_button(ui, &palette, "Try again", false).clicked() {
                app.request_lyrics();
            }
            return;
        }
        Loadable::Loaded(None) => {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Mic,
                "No lyrics",
                "No lyrics found for this one.",
            );
            return;
        }
        Loadable::Loaded(Some(lyrics)) if lyrics.instrumental => {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Music,
                "Instrumental",
                "No words to follow on this one.",
            );
            return;
        }
        Loadable::Loaded(Some(lyrics)) => lyrics.clone(),
    };

    let translation = app.current_translation().cloned();
    let active = lyrics.active_line(now.position_ms);
    let follow = app.lyrics_following && app.lyrics_line_shown != Some(active);
    // The line being sung is bold and in the accent colour; every other
    // line is quiet, regular text, the same before and after it has been
    // sung. A line takes 220 ms to light up or fade, as in omarchy-lyrics.
    let quiet = palette.text.gamma_multiply(0.45);
    // Whether the words are rewritten in Latin letters or echoed in the
    // reader's language. When that was asked for and the request failed,
    // one quiet line says so -- nothing more; flipping a toggle or moving
    // to the next track tries again.
    let romanize = app.settings.lyrics_romanize;
    let translate = app.settings.lyrics_show_translation;
    if (romanize || translate) && matches!(app.translation, Loadable::Failed(_)) {
        theme::subtle(ui, &palette, "Translation is unavailable right now.");
    }
    let scroll = egui::ScrollArea::vertical()
        .id_salt("lyrics-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Before the first line there is nothing to highlight, so the
            // panel sits at the top rather than wherever it was left.
            if follow && lyrics.synced && active.is_none() {
                let top = ui.cursor().min;
                ui.scroll_to_rect(
                    egui::Rect::from_min_size(top, egui::vec2(1.0, 1.0)),
                    Some(Align::Min),
                );
            }
            ui.add_space(12.0);
            // The translation under a line is its quiet echo, the same
            // colour whether the line is being sung or not; the accent
            // belongs to the main line alone.
            let quiet_translation = palette.secondary;
            for (index, line) in lyrics.lines.iter().enumerate() {
                let is_active = active == Some(index);
                let lit = ui.ctx().animate_bool_with_time(
                    egui::Id::new("lyric-line").with(index),
                    is_active,
                    LIGHT_UP_SECONDS,
                );
                let color = blend(quiet, palette.accent, lit);
                let font = if lit > 0.5 {
                    theme::bold(LINE_SIZE)
                } else {
                    theme::regular(LINE_SIZE)
                };
                let romanized = if romanize {
                    translation
                        .as_ref()
                        .and_then(|translation| translation.romanized.get(index))
                        .and_then(Option::as_deref)
                        .filter(|romanized| !romanized.is_empty())
                } else {
                    None
                };
                // A timed line with no words is the band playing on. The
                // romanized line stands in for the original when there is
                // one; when there is not, the original shows, silently.
                let text = if line.text.is_empty() && lyrics.synced {
                    "\u{266a}"
                } else {
                    romanized.unwrap_or(line.text.as_str())
                };
                let sense = if lyrics.synced {
                    Sense::click()
                } else {
                    Sense::hover()
                };
                let response = ui.add(
                    egui::Label::new(egui::RichText::new(text).font(font).color(color))
                        .sense(sense),
                );
                let rect = response.rect;
                if lyrics.synced {
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked()
                        && let Some(at_ms) = line.at_ms
                    {
                        app.actions.push(Action::Seek(at_ms));
                        app.lyrics_following = true;
                    }
                }
                if is_active && follow {
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
                if translate
                    && let Some(Some(translated)) = translation
                        .as_ref()
                        .and_then(|translation| translation.translated.get(index))
                    && !translated.is_empty()
                {
                    // The echo line is decoration: it takes no clicks, and
                    // the scroll still follows the main line above it.
                    ui.add_space(1.0);
                    theme::text(ui, translated, theme::regular(13.0), quiet_translation);
                    ui.add_space(2.0);
                }
                ui.add_space(LINE_GAP);
            }
            // Words without timing can only be followed by the clock: sit
            // at the part of the text the song is probably at.
            if app.lyrics_following && !lyrics.synced && now.duration_ms > 0 {
                let fraction =
                    (f64::from(now.position_ms) / f64::from(now.duration_ms)).clamp(0.0, 1.0);
                let content = ui.min_rect();
                let y = content.top() + content.height() * fraction as f32;
                ui.scroll_to_rect(
                    egui::Rect::from_min_max(
                        egui::pos2(content.left(), y),
                        egui::pos2(content.right(), y + 1.0),
                    ),
                    Some(Align::Center),
                );
            }
            ui.add_space(60.0);
        });
    // Scrolling by hand means the reader wants to look elsewhere; the
    // Follow button in the header picks the song back up.
    if ui.rect_contains_pointer(scroll.inner_rect)
        && ui.input(|input| input.smooth_scroll_delta.y != 0.0)
    {
        app.lyrics_following = false;
    }
    app.lyrics_line_shown = Some(active);
}

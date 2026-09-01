//! The playing song, in a side panel: its artwork large, who made it, and
//! what comes next.
//!
//! Spotify's own panel leads with a looping video for the song. There is no
//! video here: the Web API does not carry one, and what the panel can say
//! honestly it says. Artwork takes that place, which is what Spotify shows
//! for the many songs that have no video either.
//!
//! For the same reason "About the artist" counts followers rather than
//! monthly listeners, and carries no biography: neither is in the Web API.
//! Credits name the artists and the label, which are.

use egui::{Align, Frame, Layout, Margin, Sense, UiBuilder, Vec2};

use crate::api::models::{PlayableItem, pick_image};
use crate::app::{App, NowPlaying};
use crate::model::{Action, Loadable, Page, RowContext};
use crate::theme::{self, Icon};
use crate::util;

use super::widgets;

/// The gap between the panel's cards.
const CARD_GAP: f32 = 14.0;
/// How long a control takes to arrive once the pointer is inside, or to
/// leave once it is gone.
const REVEAL_SECONDS: f32 = 0.13;
/// How far a control slides on its way in. Enough to read as movement,
/// short enough that it is in place by the time the eye arrives.
const REVEAL_SLIDE: f32 = 9.0;

/// A control the panel keeps to itself until the pointer arrives.
///
/// It fades, and slides the last few pixels in from the edge it sits
/// against, rather than appearing between one frame and the next. `slide`
/// is where it comes from, in pixels to the right of where it lands.
///
/// Nothing is drawn, and no room is taken, once it is fully away.
fn revealed<R>(
    ui: &mut egui::Ui,
    id: &str,
    shown: bool,
    slide: f32,
    size: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let arrived = ui
        .ctx()
        .animate_bool_with_time(ui.id().with(id), shown, REVEAL_SECONDS);
    if arrived <= 0.0 {
        return None;
    }
    // The room it takes is the room it will land in, so its neighbours
    // stay put while it travels.
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size + 12.0), Sense::hover());
    let travelling = rect.translate(egui::vec2(slide * (1.0 - arrived), 0.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(travelling));
    child.set_opacity(arrived);
    Some(add(&mut child))
}

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let panel = egui::Panel::right("now-playing-panel")
        .resizable(true)
        .default_size(app.settings.now_playing_width)
        .size_range(280.0..=560.0)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        );
    let response = panel.show(ui, |ui| {
        // Spotify keeps this panel quiet until the pointer arrives, then
        // offers what can be done with the song. The menu counts as being
        // here: the pointer's trip into it leaves the panel, and a button
        // that vanished underneath would take the menu with it.
        let menu_id = ui.id().with("now-playing-menu");
        let busy = egui::Popup::is_id_open(ui.ctx(), menu_id);
        let offering = ui.rect_contains_pointer(ui.max_rect()) || busy;
        let now = app.now_playing();
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            // The song's home, the way Spotify titles the panel: the album
            // it came from, or the show for an episode.
            let heading = now
                .as_ref()
                .map(|now| {
                    if now.album_name.is_empty() {
                        "Now playing".to_owned()
                    } else {
                        now.album_name.clone()
                    }
                })
                .unwrap_or_else(|| "Now playing".to_owned());
            let open = match now.as_ref() {
                Some(now) => now
                    .album_id
                    .clone()
                    .map(Page::Album)
                    .or_else(|| now.show_id.clone().map(Page::Show)),
                None => None,
            };
            // The buttons take their room first and the heading truncates
            // into what is left, so a long album name cannot push them off
            // the edge and no gap is reserved when they are not showing.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Spotify slides its hide button in from the side it lives
                // on. That side is the right here, so it comes from the
                // right; the menu beside it only fades, having no edge of
                // its own to come from.
                let hide = revealed(ui, "hide", offering, REVEAL_SLIDE, 18.0, |ui| {
                    theme::icon_button(
                        ui,
                        Icon::PanelRightClose,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Hide the playing song",
                    )
                    .clicked()
                });
                if hide == Some(true) {
                    app.actions.push(Action::ToggleNowPlayingPanel);
                }
                // The same menu a row in any list answers with, for the
                // song this panel is about.
                if let Some(item) = app.now_playing_item() {
                    let more = revealed(ui, "more", offering, 0.0, 18.0, |ui| {
                        theme::icon_button(
                            ui,
                            Icon::Ellipsis,
                            18.0,
                            palette.secondary,
                            palette.text,
                            "More",
                        )
                    });
                    if let Some(more) = more {
                        egui::Popup::menu(&more)
                            .id(menu_id)
                            .frame(widgets::menu_frame(&palette))
                            .show(|ui| widgets::item_menu(ui, app, &item, None, None));
                    }
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| match open {
                    Some(page) => {
                        if theme::link(ui, &heading, theme::bold(18.0), palette.text).clicked() {
                            app.actions.push(Action::Open(page));
                        }
                    }
                    None => {
                        theme::text(ui, &heading, theme::bold(18.0), palette.text);
                    }
                });
            });
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("now-playing-panel-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match now {
                Some(now) => contents(app, ui, &now, offering),
                None => {
                    ui.add_space(24.0);
                    widgets::empty_state(
                        ui,
                        &palette,
                        Icon::Music,
                        "Nothing playing",
                        "Start a song and it will show up here.",
                    );
                }
            });
    });
    let width = response.response.rect.width();
    if (width - app.settings.now_playing_width).abs() > 1.0 {
        app.settings.now_playing_width = width;
        app.actions.push(Action::SettingsChanged);
    }
}

fn contents(app: &mut App, ui: &mut egui::Ui, now: &NowPlaying, offering: bool) {
    let palette = app.palette;
    let width = ui.available_width();

    // The artwork, as large as the panel is wide.
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(width), Sense::hover());
    widgets::paint_cover(
        ui,
        &palette,
        now.art_url.as_deref().or(now.art_small.as_deref()),
        rect,
        10.0,
        if now.is_episode {
            Icon::Mic
        } else {
            Icon::Music
        },
    );
    ui.add_space(12.0);

    // Title and artists, with the heart the player bar already has.
    ui.horizontal_top(|ui| {
        // Room for both the heart and the link, whether or not the link is
        // showing: the title must not reflow when the pointer arrives.
        let controls = if now.is_episode { 0.0 } else { 64.0 };
        ui.vertical(|ui| {
            ui.set_max_width((width - controls).max(40.0));
            if theme::link(ui, &now.title, theme::bold(22.0), palette.text).clicked() {
                open_song(app, now);
            }
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if now.artists.is_empty() {
                    theme::text(ui, &now.subtitle, theme::regular(14.0), palette.secondary);
                } else {
                    widgets::artist_links(
                        ui,
                        app,
                        &now.artists,
                        theme::regular(14.0),
                        palette.secondary,
                    );
                }
            });
        });
        if !now.is_episode {
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                // Whether the song is saved is worth knowing at a glance, so
                // the heart stays; the link only answers a pointer.
                let saved = app.is_saved(&now.uri).unwrap_or(false);
                let (icon, color, tooltip) = if saved {
                    (Icon::HeartFilled, palette.accent, "Remove from Liked Songs")
                } else {
                    (Icon::Heart, palette.secondary, "Save to Liked Songs")
                };
                if theme::icon_button(ui, icon, 19.0, color, palette.text, tooltip).clicked() {
                    app.actions.push(Action::ToggleSaved(now.uri.clone()));
                }
                let share = revealed(ui, "share", offering, REVEAL_SLIDE, 17.0, |ui| {
                    theme::icon_button(
                        ui,
                        Icon::Share,
                        17.0,
                        palette.secondary,
                        palette.text,
                        "Copy link to song",
                    )
                    .clicked()
                });
                if share == Some(true) {
                    app.actions.push(Action::CopyLink(now.uri.clone()));
                }
            });
        }
    });

    ui.add_space(CARD_GAP);
    about_the_artist(app, ui, now);
    credits(app, ui, now);
    next_in_queue(app, ui);
    ui.add_space(8.0);
}

/// A card, the way Spotify groups each section of this panel.
fn card(app: &mut App, ui: &mut egui::Ui, add: impl FnOnce(&mut App, &mut egui::Ui)) {
    let palette = app.palette;
    Frame::new()
        .fill(palette.surface)
        .corner_radius(10.0)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(app, ui);
        });
    ui.add_space(CARD_GAP);
}

/// Who made it: the lead artist, their picture, how many follow them, and
/// the same Follow button the artist page carries.
///
/// Spotify counts monthly listeners here and prints a biography. The Web API
/// has neither, so this counts followers and says so.
fn about_the_artist(app: &mut App, ui: &mut egui::Ui, now: &NowPlaying) {
    let Some(id) = now.artists.first().and_then(|artist| artist.id.clone()) else {
        return;
    };
    let Some(Loadable::Loaded(artist)) = app.artist_pages.get(&id).map(|page| &page.artist) else {
        // Loading, or an artist the API would not name: the card would be an
        // empty box, so it waits for the answer instead.
        return;
    };
    let artist = artist.clone();
    card(app, ui, |app, ui| {
        let palette = app.palette;
        theme::section_title(ui, &palette, "About the artist");
        ui.add_space(10.0);
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, width * 0.62), Sense::hover());
        widgets::paint_cover(
            ui,
            &palette,
            pick_image(&artist.images, 640),
            rect,
            8.0,
            Icon::User,
        );
        ui.add_space(10.0);
        // The name and the button share a row: the follower count below is
        // often missing, and an empty row left the button floating.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let following = app.is_saved(&artist.uri).unwrap_or(false);
                if theme::pill_button(
                    ui,
                    &palette,
                    if following { "Following" } else { "Follow" },
                    false,
                )
                .clicked()
                {
                    app.actions.push(Action::ToggleSaved(artist.uri.clone()));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if theme::link(ui, &artist.name, theme::bold(16.0), palette.text).clicked() {
                        app.actions
                            .push(Action::Open(Page::Artist(artist.id.clone())));
                    }
                });
            });
        });
        // Spotify counts monthly listeners here. The Web API has no such
        // number, and does not always answer with followers either.
        if let Some(followers) = &artist.followers {
            ui.add_space(4.0);
            theme::text(
                ui,
                format!("{} followers", util::format_count(followers.total)),
                theme::regular(13.0),
                palette.secondary,
            );
        }
    });
}

/// What the Web API knows about who made the song: the credited artists and
/// the album's label.
///
/// Spotify names writers, producers and mixers here. Those come from a
/// service the Web API does not expose, so rather than a heading with
/// nothing under it, this shows the two credits it can stand behind.
fn credits(app: &mut App, ui: &mut egui::Ui, now: &NowPlaying) {
    if now.is_episode || now.artists.is_empty() {
        return;
    }
    let label = now
        .album_id
        .as_ref()
        .and_then(|id| app.album_pages.get(id))
        .and_then(|page| page.album.get())
        .and_then(|album| album.label.clone())
        .filter(|label| !label.is_empty());
    let artists = now.artists.clone();
    card(app, ui, |app, ui| {
        let palette = app.palette;
        theme::section_title(ui, &palette, "Credits");
        ui.add_space(10.0);
        for (index, artist) in artists.iter().enumerate() {
            if index > 0 {
                ui.add_space(10.0);
            }
            if let Some(id) = &artist.id {
                if theme::link(ui, &artist.name, theme::medium(14.0), palette.text).clicked() {
                    app.actions.push(Action::Open(Page::Artist(id.clone())));
                }
            } else {
                theme::text(ui, &artist.name, theme::medium(14.0), palette.text);
            }
            theme::subtle(
                ui,
                &palette,
                if index == 0 { "Main artist" } else { "Artist" },
            );
        }
        if let Some(label) = &label {
            ui.add_space(10.0);
            theme::text(ui, label, theme::medium(14.0), palette.text);
            theme::subtle(ui, &palette, "Label");
        }
    });
}

/// What plays after this, and the way through to the rest of it.
fn next_in_queue(app: &mut App, ui: &mut egui::Ui) {
    let Some(next) = app
        .queue
        .get()
        .and_then(|queue| queue.queue.first())
        .cloned()
    else {
        return;
    };
    card(app, ui, |app, ui| {
        let palette = app.palette;
        ui.horizontal(|ui| {
            theme::section_title(ui, &palette, "Next in queue");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::link(ui, "Open queue", theme::medium(13.0), palette.secondary).clicked() {
                    app.actions.push(Action::ToggleNowPlayingPanel);
                    app.actions.push(Action::ToggleQueuePanel);
                }
            });
        });
        ui.add_space(10.0);
        let (title, subtitle, art, uri) = match &next {
            PlayableItem::Track(track) => (
                track.name.clone(),
                track
                    .artists
                    .iter()
                    .map(|artist| artist.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                track.image(64).map(str::to_string),
                track.uri.clone(),
            ),
            PlayableItem::Episode(episode) => (
                episode.name.clone(),
                episode
                    .show
                    .as_ref()
                    .map(|show| show.name.clone())
                    .unwrap_or_default(),
                pick_image(&episode.images, 64).map(str::to_string),
                episode.uri.clone(),
            ),
        };
        let row = ui
            .horizontal(|ui| {
                widgets::cover(ui, &palette, art.as_deref(), 40.0, 4.0, Icon::Music);
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    theme::text(ui, &title, theme::medium(14.0), palette.text);
                    if !subtitle.is_empty() {
                        theme::text(ui, &subtitle, theme::regular(12.0), palette.secondary);
                    }
                });
            })
            .response;
        if row
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .double_clicked()
        {
            // The queue's own rule: playing a row consumes the queue down
            // to it, the way pressing Next would.
            app.actions.push(Action::PlayFromRow {
                context: RowContext::Queue,
                uri,
                index: 0,
            });
        }
    });
}

/// The song's own page: the album it is on, or the show for an episode.
fn open_song(app: &mut App, now: &NowPlaying) {
    if let Some(id) = &now.album_id {
        app.actions.push(Action::Open(Page::Album(id.clone())));
    } else if let Some(id) = &now.show_id {
        app.actions.push(Action::Open(Page::Show(id.clone())));
    }
}

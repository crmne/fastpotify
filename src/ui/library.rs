//! Full-page library grids: albums, artists, podcasts, episodes.

use crate::api::models::{join_names, pick_image};
use crate::app::App;
use crate::model::{Action, Page};
use crate::theme::{self, Icon};

use super::widgets;

pub fn show(app: &mut App, ui: &mut egui::Ui, page: Page) {
    let palette = app.palette;
    ui.add_space(8.0);
    let (title, empty_title, empty_body) = match page {
        Page::Albums => ("Albums", "No saved albums", "Saved albums appear here."),
        Page::Artists => (
            "Artists",
            "No followed artists",
            "Followed artists appear here.",
        ),
        Page::Podcasts => (
            "Podcasts",
            "No podcasts yet",
            "Followed podcasts appear here.",
        ),
        _ => (
            "Episodes",
            "No saved episodes",
            "Saved episodes appear here.",
        ),
    };
    theme::text(ui, title, theme::bold(28.0), palette.text);
    ui.add_space(14.0);
    match page {
        Page::Albums => {
            let albums: Vec<_> = app
                .library
                .albums
                .items
                .iter()
                .map(|saved| saved.album.clone())
                .collect();
            widgets::grid(ui, |ui| {
                for album in &albums {
                    let subtitle =
                        join_names(album.artists.iter().map(|artist| artist.name.as_str()));
                    let card = widgets::card(
                        ui,
                        app,
                        pick_image(&album.images, 300),
                        &album.name,
                        &subtitle,
                        false,
                        true,
                    );
                    if card.play {
                        app.actions.push(Action::PlayContext {
                            uri: album.uri.clone(),
                            offset_uri: None,
                            offset_index: None,
                        });
                    }
                    if card.clicked {
                        app.actions
                            .push(Action::Open(Page::Album(album.id.clone())));
                    }
                }
            });
            let list = &app.library.albums;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                empty_title,
                empty_body,
                Icon::Disc,
            );
        }
        Page::Artists => {
            let artists = app.library.artists.items.clone();
            widgets::grid(ui, |ui| {
                for artist in &artists {
                    let card = widgets::card(
                        ui,
                        app,
                        pick_image(&artist.images, 300),
                        &artist.name,
                        "Artist",
                        true,
                        true,
                    );
                    if card.play {
                        app.actions.push(Action::PlayContext {
                            uri: artist.uri.clone(),
                            offset_uri: None,
                            offset_index: None,
                        });
                    }
                    if card.clicked {
                        app.actions
                            .push(Action::Open(Page::Artist(artist.id.clone())));
                    }
                }
            });
            let list = &app.library.artists;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                empty_title,
                empty_body,
                Icon::Users,
            );
        }
        Page::Podcasts => {
            let shows: Vec<_> = app
                .library
                .shows
                .items
                .iter()
                .map(|saved| saved.show.clone())
                .collect();
            widgets::grid(ui, |ui| {
                for show in &shows {
                    let card = widgets::card(
                        ui,
                        app,
                        pick_image(&show.images, 300),
                        &show.name,
                        &show.publisher,
                        false,
                        false,
                    );
                    if card.clicked {
                        app.actions.push(Action::Open(Page::Show(show.id.clone())));
                    }
                }
            });
            let list = &app.library.shows;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                empty_title,
                empty_body,
                Icon::Mic,
            );
        }
        _ => {
            let episodes: Vec<_> = app
                .library
                .episodes
                .items
                .iter()
                .map(|saved| saved.episode.clone())
                .collect();
            widgets::virtual_rows(
                ui,
                episodes.len(),
                super::show::EPISODE_ROW_HEIGHT,
                |ui, index| {
                    super::show::episode_row(app, ui, &episodes[index], None);
                },
            );
            let list = &app.library.episodes;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                empty_title,
                empty_body,
                Icon::Bookmark,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn footer(
    app: &mut App,
    ui: &mut egui::Ui,
    page: Page,
    loading: bool,
    error: Option<String>,
    can_load: bool,
    empty: bool,
    empty_title: &str,
    empty_body: &str,
    icon: Icon,
) {
    let palette = app.palette;
    if loading {
        ui.add_space(8.0);
        widgets::loading_row(ui, &palette);
    }
    if let Some(error) = error {
        widgets::error_row(ui, app, &error, Some(page.clone()));
    }
    if empty && !loading {
        widgets::empty_state(ui, &palette, icon, empty_title, empty_body);
    }
    widgets::load_more_when_near_end(ui, app, page, can_load && !loading);
}

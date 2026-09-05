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
            let card_height = widgets::card_row_height(ui);
            let count = app.library.albums.items.len();
            widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
                let album = app.library.albums.items[index].album.clone();
                let id = album.id.clone();
                let uri = album.uri.clone();
                let subtitle = join_names(album.artists.iter().map(|artist| artist.name.as_str()));
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
                        uri,
                        offset_uri: None,
                        offset_index: None,
                    });
                }
                if card.clicked {
                    app.actions.push(Action::Open(Page::Album(id)));
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
            let card_height = widgets::card_row_height(ui);
            let count = app.library.artists.items.len();
            widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
                let artist = app.library.artists.items[index].clone();
                let id = artist.id.clone();
                let uri = artist.uri.clone();
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
                        uri,
                        offset_uri: None,
                        offset_index: None,
                    });
                }
                if card.clicked {
                    app.actions.push(Action::Open(Page::Artist(id)));
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
            let card_height = widgets::card_row_height(ui);
            let count = app.library.shows.items.len();
            widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
                let show = app.library.shows.items[index].show.clone();
                let id = show.id.clone();
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
                    app.actions.push(Action::Open(Page::Show(id)));
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

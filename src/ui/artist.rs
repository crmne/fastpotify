//! The artist page.

use crate::api::models::{PlayableItem, pick_image};
use crate::app::App;
use crate::model::{Action, DiscographyFilter, Loadable, Page, RowContext};
use crate::theme::{self, Icon};
use crate::util;

use super::collection::{Hero, hero};
use super::widgets::{self, TrackRow};

pub fn show(app: &mut App, ui: &mut egui::Ui, id: &str) {
    let Some(page) = app.artist_pages.remove(id) else {
        app.ensure_loaded(Page::Artist(id.to_string()));
        return;
    };
    let palette = app.palette;
    match &page.artist {
        Loadable::Loaded(artist) => {
            let mut byline = Vec::new();
            if let Some(followers) = &artist.followers {
                byline.push((
                    format!("{} followers", util::format_count(followers.total)),
                    None,
                ));
            }
            if !artist.genres.is_empty() {
                byline.push((
                    artist
                        .genres
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                    None,
                ));
            }
            hero(
                app,
                ui,
                Hero {
                    image: pick_image(&artist.images, 300),
                    liked: false,
                    kind: "Artist",
                    title: &artist.name,
                    description: None,
                    byline,
                    round: true,
                },
            );
            let following = app.is_saved(&artist.uri).unwrap_or(false);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                if app.play_pending(&artist.uri) {
                    theme::circle_spinner(ui, 56.0, palette.accent, palette.on_accent, "Starting…");
                } else if theme::circle_button(
                    ui,
                    Icon::PlayFilled,
                    56.0,
                    palette.accent,
                    palette.accent_hover,
                    palette.on_accent,
                    "Play",
                )
                .clicked()
                {
                    app.actions.push(Action::PlayContext {
                        uri: artist.uri.clone(),
                        offset_uri: None,
                        offset_index: None,
                    });
                }
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
                let more = theme::icon_button(
                    ui,
                    Icon::Ellipsis,
                    26.0,
                    palette.secondary,
                    palette.text,
                    "More",
                );
                egui::Popup::menu(&more)
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| {
                        widgets::context_menu_items(ui, app, &artist.uri, &artist.name, None)
                    });
            });
            ui.add_space(20.0);

            // Popular.
            theme::section_title(ui, &palette, "Popular");
            ui.add_space(4.0);
            match &page.top_tracks {
                Loadable::Loaded(tracks) if !tracks.is_empty() => {
                    let uris: Vec<String> = tracks.iter().map(|track| track.uri.clone()).collect();
                    let context = RowContext::Uris(uris);
                    let items: Vec<PlayableItem> =
                        tracks.iter().cloned().map(PlayableItem::Track).collect();
                    let limit = if page.show_all_top { items.len() } else { 5 };
                    for (index, item) in items.iter().take(limit).enumerate() {
                        widgets::track_row(
                            ui,
                            app,
                            TrackRow {
                                index,
                                number: Some(index + 1),
                                item,
                                context: &context,
                                show_cover: !app.settings.tracklist_compact,
                                show_album: false,
                                added_at: None,
                                added_by: None,
                                show_added_by: false,
                                compact: false,
                                thin: app.settings.tracklist_compact,
                                shift: 0.0,
                                picked: false,
                                picked_songs: &[],
                            },
                        );
                    }
                    if items.len() > 5 {
                        ui.add_space(6.0);
                        if theme::soft_button(
                            ui,
                            &palette,
                            None,
                            if page.show_all_top {
                                "Show less"
                            } else {
                                "See more"
                            },
                            false,
                        )
                        .clicked()
                        {
                            app.actions.push(Action::ToggleShowAllTop(id.to_string()));
                        }
                    }
                }
                Loadable::Loaded(_) => {
                    theme::subtle(ui, &palette, "No popular songs to show.");
                }
                Loadable::Loading | Loadable::NotLoaded => widgets::loading_row(ui, &palette),
                Loadable::Failed(error) => {
                    let error = error.clone();
                    widgets::error_row(ui, app, &error, None);
                }
            }
            ui.add_space(20.0);

            // Discography.
            theme::section_title(ui, &palette, "Discography");
            ui.add_space(6.0);
            let options: Vec<(DiscographyFilter, &str)> = DiscographyFilter::ALL
                .iter()
                .map(|f| (*f, f.label()))
                .collect();
            if let Some(filter) = widgets::chips(ui, &palette, &options, page.filter) {
                app.actions.push(Action::SetDiscographyFilter {
                    artist_id: id.to_string(),
                    filter,
                });
            }
            ui.add_space(10.0);
            match page.albums.get(page.filter.groups()) {
                Some(list) => {
                    let mut seen = std::collections::HashSet::new();
                    let albums: Vec<_> = list
                        .items
                        .iter()
                        .filter(|album| seen.insert(album.name.to_lowercase()))
                        .collect();
                    widgets::grid(ui, |ui| {
                        for album in &albums {
                            let subtitle =
                                format!("{} • {}", album.year().unwrap_or(""), album.kind_label());
                            let card = widgets::card(
                                ui,
                                app,
                                pick_image(&album.images, 300),
                                &album.name,
                                subtitle.trim_start_matches(" • "),
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
                    if list.loading {
                        widgets::loading_row(ui, &palette);
                    } else if let Some(error) = &list.error {
                        let error = error.clone();
                        widgets::error_row(ui, app, &error, None);
                    } else if list.items.is_empty() {
                        theme::subtle(ui, &palette, "Nothing in this category.");
                    } else if list.can_load_more() {
                        ui.add_space(8.0);
                        if theme::soft_button(ui, &palette, None, "Load more", false).clicked() {
                            app.actions
                                .push(Action::LoadMoreArtistAlbums(id.to_string()));
                        }
                    }
                }
                None => widgets::loading_row(ui, &palette),
            }
            ui.add_space(20.0);

            // Related.
            if let Loadable::Loaded(related) = &page.related
                && !related.is_empty()
            {
                widgets::shelf(ui, &palette, "related", "Fans also like", |ui| {
                    for artist in related {
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
            }
        }
        Loadable::Loading | Loadable::NotLoaded => {
            ui.add_space(40.0);
            widgets::loading_row(ui, &palette);
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            ui.add_space(40.0);
            widgets::error_row(ui, app, &error, Some(Page::Artist(id.to_string())));
        }
    }
    app.artist_pages.insert(id.to_string(), page);
}

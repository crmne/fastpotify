//! Navigation and asynchronous state for song-radio pages.

use super::*;

impl App {
    pub(super) fn load_radio(&mut self, id: &str) {
        if self
            .radio_pages
            .get(id)
            .is_some_and(|page| !page.station.needs_load())
        {
            return;
        }
        self.load_generation += 1;
        let page = self.radio_pages.entry(id.to_owned()).or_default();
        page.generation = self.load_generation;
        page.station = Loadable::Loading;
        self.backend.send(Command::Radio {
            id: id.to_owned(),
            generation: page.generation,
        });
    }

    pub(super) fn receive_radio(
        &mut self,
        id: String,
        generation: u64,
        result: Result<crate::radio::Station, String>,
    ) {
        let Some(page) = self
            .radio_pages
            .get_mut(&id)
            .filter(|page| page.generation == generation)
        else {
            return;
        };
        match result {
            Ok(station) => {
                let tracks = std::iter::once(&station.seed)
                    .chain(&station.tracks)
                    .cloned()
                    .collect::<Vec<_>>();
                page.station = Loadable::Loaded(station);
                let uris = tracks.iter().map(|track| track.uri.clone()).collect();
                for track in tracks {
                    self.remember_track_recording(&track);
                    if let Some(id) = &track.id {
                        self.track_cache.insert(id.clone(), track);
                    }
                }
                self.request_contains(uris);
            }
            Err(error) => page.station = Loadable::Failed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let root =
            std::env::temp_dir().join(format!("fastpotify-radio-state-{}", std::process::id()));
        let mut app = App::new(
            &Waker::default(),
            AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.backend.set_offline(true);
        app.auth = AuthStatus::Connected {
            username: "test".into(),
        };
        app.local_ready = true;
        app
    }

    #[test]
    fn reloading_radio_ignores_the_previous_request_and_preserves_playback() {
        let mut app = app();
        app.open(Page::Radio("seed".into()));
        let first = app.radio_pages["seed"].generation;
        app.reload(Page::Radio("seed".into()));
        let second = app.radio_pages["seed"].generation;
        assert_ne!(first, second);
        app.receive_radio("seed".into(), first, Err("old request".into()));
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
        app.receive_radio(
            "seed".into(),
            second,
            Ok(crate::radio::Station {
                seed: Track {
                    name: "Seed song".into(),
                    ..Default::default()
                },
                tracks: vec![Track {
                    uri: "spotify:track:next".into(),
                    name: "Next song".into(),
                    ..Default::default()
                }],
            }),
        );
        assert_eq!(
            app.radio_pages["seed"].station.get().unwrap().tracks[0].name,
            "Next song"
        );
        assert!(app.optimistic_playing.is_none());
        assert!(app.assumed_context.is_none());
        assert!(app.queued_play.is_none());
        assert!(matches!(app.queue, Loadable::NotLoaded));
    }

    #[test]
    fn refreshing_radio_clears_selection_from_the_previous_songs() {
        let mut app = app();
        let page = Page::Radio("seed".into());
        app.open(page.clone());
        app.pick_row(&page, "original|50", 2, RowPick::Only, 50);
        assert!(app.picked_rows(&page).is_some());
        app.reload(page.clone());
        assert!(app.picked_rows(&page).is_none());
    }

    #[test]
    fn radio_recognizes_a_saved_recording_from_another_release() {
        let mut app = app();
        let original = Track {
            uri: "spotify:track:original".into(),
            external_ids: crate::api::models::ExternalIds {
                isrc: Some("GBUM71029604".into()),
            },
            ..Default::default()
        };
        app.remember_track_recording(&original);
        app.set_saved_state(original.uri.clone(), true);
        app.open(Page::Radio("seed".into()));
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio(
            "seed".into(),
            generation,
            Ok(crate::radio::Station {
                seed: Track::default(),
                tracks: vec![Track {
                    uri: "spotify:track:radio-release".into(),
                    ..original.clone()
                }],
            }),
        );
        assert_eq!(app.is_saved("spotify:track:radio-release"), Some(true));
        assert_eq!(
            app.saved_toggle_targets("spotify:track:radio-release"),
            vec![original.uri]
        );
    }

    #[test]
    fn restored_radio_retries_when_the_playback_session_becomes_ready() {
        let mut app = app();
        app.local_ready = false;
        app.open(Page::Radio("seed".into()));
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio(
            "seed".into(),
            generation,
            Err("Playback is connecting".into()),
        );
        app.handle_playback(LocalPlayback::Ready {
            device_id: "local".into(),
        });
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
        assert!(app.radio_pages["seed"].generation > generation);
        assert!(app.queued_play.is_none());
    }

    #[test]
    fn a_radio_error_is_visible_and_retry_can_load_it() {
        let mut app = app();
        app.open(Page::Radio("seed".into()));
        let generation = app.radio_pages["seed"].generation;
        app.receive_radio("seed".into(), generation, Err("Set up playback".into()));
        assert!(
            matches!(&app.radio_pages["seed"].station, Loadable::Failed(error) if error == "Set up playback")
        );
        app.reload(Page::Radio("seed".into()));
        assert!(matches!(app.radio_pages["seed"].station, Loadable::Loading));
        assert!(app.radio_pages["seed"].generation > generation);
    }

    #[test]
    fn a_radio_response_after_sign_out_is_discarded() {
        let mut app = app();
        app.open(Page::Radio("seed".into()));
        let generation = app.radio_pages["seed"].generation;
        app.handle_auth(AuthStatus::SignedOut);
        app.receive_radio(
            "seed".into(),
            generation,
            Ok(crate::radio::Station::default()),
        );
        assert!(app.radio_pages.is_empty());
    }

    #[test]
    fn radio_controls_browse_play_and_refresh_independently() {
        let mut app = app();
        let item = PlayableItem::Track(Track {
            id: Some("seed".into()),
            uri: "spotify:track:seed".into(),
            name: "Seed song".into(),
            ..Default::default()
        });
        for label in ["Go to song radio", "Start song radio", "Refresh radio"] {
            app.radio_pages.insert(
                "seed".into(),
                crate::radio::RadioPage {
                    station: Loadable::Loaded(crate::radio::Station {
                        seed: match &item {
                            PlayableItem::Track(track) => track.clone(),
                            _ => unreachable!(),
                        },
                        tracks: vec![],
                    }),
                    generation: 1,
                },
            );
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            let mut draw = |events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(500.0, 800.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        if label == "Refresh radio" {
                            crate::ui::radio::show(&mut app, ui, "seed");
                        } else {
                            crate::ui::widgets::item_menu(ui, &mut app, &item, None, None);
                        }
                    },
                );
                output.textures_delta.clear();
                output
            };
            let output = draw(vec![]);
            fn text_position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
                match shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + text.galley.size() / 2.0)
                    }
                    egui::Shape::Vec(shapes) => {
                        shapes.iter().find_map(|shape| text_position(shape, label))
                    }
                    _ => None,
                }
            }
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| text_position(&shape.shape, label))
                .expect("the menu item is present");
            for pressed in [true, false] {
                draw(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            let actions = std::mem::take(&mut app.actions);
            assert_eq!(actions.len(), 1);
            if label == "Refresh radio" {
                assert!(matches!(&actions[0], Action::Reload(Page::Radio(id)) if id == "seed"));
            } else if label == "Go to song radio" {
                assert!(matches!(&actions[0], Action::Open(Page::Radio(id)) if id == "seed"));
            } else {
                assert!(
                    matches!(&actions[0], Action::PlayTrackRadio(uri) if uri == "spotify:track:seed")
                );
            }
        }
    }
}

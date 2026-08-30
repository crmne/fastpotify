//! The Settings page.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Dialog};
use crate::settings::{Language, ThemeChoice};
use crate::theme::{self, Icon, Palette};

use super::widgets;

const PLAYBACK_DIRTY_ID: &str = "playback-settings-dirty";

fn section(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(10.0);
    theme::text(ui, title, theme::bold(18.0), palette.text);
    ui.add_space(8.0);
    Frame::new()
        .fill(
            palette
                .surface
                .gamma_multiply(if palette.dark { 0.7 } else { 1.0 }),
        )
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 2))
        .inner_margin(Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(760.0));
            add_contents(ui);
        });
    ui.add_space(8.0);
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(8.0);
    theme::text(ui, app.t("settings.title"), theme::bold(28.0), palette.text);
    ui.add_space(4.0);
    let dirty_id = egui::Id::new(PLAYBACK_DIRTY_ID);
    let mut playback_dirty = ui
        .data(|data| data.get_temp::<bool>(dirty_id))
        .unwrap_or(false);
    let mut changed = false;

    section(ui, &palette, app.t("settings.section.account"), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let avatar = app
                .user
                .as_ref()
                .and_then(|user| pick_image(&user.images, 64).map(str::to_string));
            widgets::cover(ui, &palette, avatar.as_deref(), 56.0, 28.0, Icon::User);
            ui.vertical(|ui| {
                let name = app
                    .user
                    .as_ref()
                    .map(|user| user.name().to_string())
                    .unwrap_or_default();
                theme::text(ui, name, theme::semibold(16.0), palette.text);
                let product = app
                    .user
                    .as_ref()
                    .and_then(|user| user.product.clone())
                    .map(|product| match product.as_str() {
                        "premium" => app.t("settings.product.premium").to_string(),
                        "free" | "open" => app.t("settings.product.free").to_string(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                theme::text(ui, product, theme::regular(13.0), palette.secondary);
                if let Some(username) = app.local.connected.then(|| app.local.username.clone())
                    && !username.is_empty()
                {
                    theme::text(
                        ui,
                        app.tf("settings.connected_as", &[("username", username.as_str())]),
                        theme::regular(12.0),
                        palette.dim,
                    );
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::pill_button(ui, &palette, app.t("settings.sign_out"), false).clicked() {
                    app.actions.push(Action::SignOut);
                }
            });
        });
        ui.add_space(10.0);
        let mut client_id = app.settings.web_client_id.clone().unwrap_or_default();
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.acceleration.title"),
            app.t("settings.acceleration.description"),
            |ui| {
                let response = Frame::new()
                    .fill(palette.surface)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut client_id)
                                .hint_text(
                                    egui::RichText::new(app.t("settings.client_id_hint"))
                                        .color(palette.dim),
                                )
                                .font(theme::regular(13.0))
                                .frame(egui::Frame::NONE)
                                .desired_width(200.0),
                        )
                    })
                    .inner;
                if response.changed() {
                    let trimmed = client_id.trim().to_string();
                    app.settings.web_client_id = (!trimmed.is_empty()).then_some(trimmed);
                    changed = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.no_client.title"),
            app.t("settings.no_client.description"),
            |ui| {
                if theme::pill_button(ui, &palette, app.t("common.show_me_how"), false).clicked() {
                    app.actions.push(Action::OpenUrl(
                        "https://fastpotify.rocks/make-it-even-faster/".into(),
                    ));
                }
            },
        );
        let wanted = app
            .settings
            .web_client_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let in_use = wanted
            .as_deref()
            .is_some_and(|wanted| app.web_app.as_deref() == Some(wanted));
        if in_use {
            widgets::setting_row(
                ui,
                &palette,
                app.t("settings.personal_ready.title"),
                app.t("settings.personal_ready.description"),
                |ui| {
                    if theme::pill_button(ui, &palette, app.t("common.remove"), false).clicked() {
                        app.settings.web_client_id = None;
                        app.actions.push(Action::ConfigurePersonalWebApp);
                    }
                },
            );
        } else if wanted.is_some() {
            widgets::setting_row(
                ui,
                &palette,
                app.t("settings.authorize_personal.title"),
                app.t("settings.authorize_personal.description"),
                |ui| {
                    if theme::pill_button(ui, &palette, app.t("common.authorize"), true).clicked() {
                        app.actions.push(Action::ConfigurePersonalWebApp);
                    }
                },
            );
        } else if app.web_app.is_some() {
            widgets::setting_row(
                ui,
                &palette,
                app.t("settings.remove_personal.title"),
                app.t("settings.remove_personal.description"),
                |ui| {
                    if theme::pill_button(ui, &palette, app.t("common.remove"), false).clicked() {
                        app.actions.push(Action::ConfigurePersonalWebApp);
                    }
                },
            );
        }
    });

    section(ui, &palette, app.t("settings.section.playback"), |ui| {
        let (status, detail, action) = match &app.local_playback {
            crate::backend::LocalPlayback::Ready { .. } => (
                app.t("settings.playback.status.ready"),
                app.t("settings.playback.ready_detail").to_string(),
                None,
            ),
            crate::backend::LocalPlayback::Authorizing => (
                app.t("settings.playback.status.authorizing"),
                app.t("settings.playback.authorizing_detail").to_string(),
                None,
            ),
            crate::backend::LocalPlayback::Connecting => (
                app.t("settings.playback.status.connecting"),
                app.t("settings.playback.connecting_detail").to_string(),
                None,
            ),
            crate::backend::LocalPlayback::Failed(message) => (
                app.t("settings.playback.status.unavailable"),
                message.clone(),
                Some(app.t("common.try_again")),
            ),
            crate::backend::LocalPlayback::Unavailable => (
                app.t("settings.playback.status.not_setup"),
                app.t("settings.playback.not_setup_detail").to_string(),
                Some(app.t("settings.playback.enable")),
            ),
        };
        widgets::setting_row(
            ui,
            &palette,
            &app.tf("settings.playback.status_label", &[("status", status)]),
            &detail,
            |ui| {
                if let Some(label) = action {
                    if theme::pill_button(ui, &palette, label, true).clicked() {
                        app.actions.push(Action::EnablePlayback);
                    }
                } else if app.local_ready
                    && theme::soft_button(
                        ui,
                        &palette,
                        Some(Icon::Refresh),
                        app.t("settings.reconnect"),
                        false,
                    )
                    .clicked()
                {
                    app.actions.push(Action::RestartEngine);
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.device_name.title"),
            app.t("settings.device_name.description"),
            |ui| {
                let response = Frame::new()
                    .fill(palette.surface)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut app.settings.device_name)
                                .font(theme::regular(14.0))
                                .frame(egui::Frame::NONE)
                                .desired_width(200.0),
                        )
                    })
                    .inner;
                if response.changed() {
                    changed = true;
                    playback_dirty = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.quality.title"),
            app.t("settings.quality.description"),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (kbps, label) in [
                        (320u16, app.t("settings.quality.very_high")),
                        (160, app.t("settings.quality.high")),
                        (96, app.t("settings.quality.normal")),
                    ] {
                        if theme::soft_button(
                            ui,
                            &palette,
                            None,
                            label,
                            app.settings.bitrate == kbps,
                        )
                        .clicked()
                            && app.settings.bitrate != kbps
                        {
                            app.settings.bitrate = kbps;
                            changed = true;
                            playback_dirty = true;
                        }
                    }
                });
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.normalize.title"),
            app.t("settings.normalize.description"),
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.normalisation).changed() {
                    changed = true;
                    playback_dirty = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.autoplay.title"),
            app.t("settings.autoplay.description"),
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.autoplay).changed() {
                    changed = true;
                    playback_dirty = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.gapless.title"),
            app.t("settings.gapless.description"),
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.gapless).changed() {
                    changed = true;
                    playback_dirty = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.background.title"),
            app.t("settings.background.description"),
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.keep_playing_in_background)
                    .changed()
                {
                    changed = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.updates.title"),
            app.t("settings.updates.description"),
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.check_for_updates).changed() {
                    changed = true;
                }
            },
        );
        if cfg!(target_os = "linux") {
            widgets::setting_row(
                ui,
                &palette,
                app.t("settings.audio_output.title"),
                app.t("settings.audio_output.description"),
                |ui| {
                    let current = app
                        .settings
                        .platform_backend()
                        .unwrap_or_else(|| "rodio".into());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for backend in ["rodio", "pulseaudio"] {
                            let label = if backend == "pulseaudio" {
                                app.t("settings.audio_output.pulse")
                            } else {
                                app.t("settings.audio_output.alsa")
                            };
                            if theme::soft_button(ui, &palette, None, label, current == backend)
                                .clicked()
                                && current != backend
                            {
                                app.settings.audio_backend = Some(backend.to_string());
                                changed = true;
                                playback_dirty = true;
                            }
                        }
                    });
                },
            );
        }
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.audio_cache.title"),
            app.t("settings.audio_cache.description"),
            |ui| {
                // The control area lays out right-to-left: add the rightmost item first.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::switch(ui, &palette, &mut app.settings.audio_cache).changed() {
                        changed = true;
                        playback_dirty = true;
                    }
                    if app.settings.audio_cache {
                        ui.add_space(6.0);
                        for (mb, label) in [
                            (4096u64, app.t("settings.cache_size.4gb")),
                            (1024, app.t("settings.cache_size.1gb")),
                            (512, app.t("settings.cache_size.512mb")),
                        ] {
                            if theme::soft_button(
                                ui,
                                &palette,
                                None,
                                label,
                                app.settings.audio_cache_mb == mb,
                            )
                            .clicked()
                                && app.settings.audio_cache_mb != mb
                            {
                                app.settings.audio_cache_mb = mb;
                                changed = true;
                                playback_dirty = true;
                            }
                        }
                    }
                });
            },
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if playback_dirty {
                if theme::pill_button(ui, &palette, app.t("settings.apply_restart"), true).clicked()
                {
                    app.actions.push(Action::RestartEngine);
                    playback_dirty = false;
                }
                theme::subtle(ui, &palette, app.t("settings.playback_restart_note"));
            } else {
                theme::subtle(ui, &palette, app.t("settings.playback_applied"));
            }
        });
    });

    section(ui, &palette, app.t("settings.section.appearance"), |ui| {
        widgets::setting_row(ui, &palette, app.t("settings.theme.title"), "", |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for choice in ThemeChoice::ALL {
                    if theme::soft_button(
                        ui,
                        &palette,
                        None,
                        choice.label(app.catalog),
                        app.settings.theme == choice,
                    )
                    .clicked()
                        && app.settings.theme != choice
                    {
                        app.settings.theme = choice;
                        changed = true;
                    }
                }
            });
        });
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.language.title"),
            app.t("settings.language.description"),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for choice in Language::ALL {
                        if theme::soft_button(
                            ui,
                            &palette,
                            None,
                            choice.label(app.catalog),
                            app.settings.language == choice,
                        )
                        .clicked()
                            && app.settings.language != choice
                        {
                            app.settings.language = choice;
                            changed = true;
                        }
                    }
                });
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.accent_art.title"),
            app.t("settings.accent_art.description"),
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.accent_from_art).changed() {
                    changed = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.sidebar_compact.title"),
            app.t("settings.sidebar_compact.description"),
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.sidebar_compact).changed() {
                    changed = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.tracklist_compact.title"),
            app.t("settings.tracklist_compact.description"),
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.tracklist_compact).changed() {
                    changed = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.zoom.title"),
            app.t("settings.zoom.description"),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mut zoom = app.settings.zoom;
                    if theme::soft_button(ui, &palette, None, "-", false).clicked() {
                        zoom = (zoom - 0.1).max(0.5);
                    }
                    theme::text(
                        ui,
                        app.tf(
                            "settings.zoom.percent",
                            &[("percent", &format!("{:.0}", zoom * 100.0))],
                        ),
                        theme::medium(13.5),
                        palette.text,
                    );
                    if theme::soft_button(ui, &palette, None, "+", false).clicked() {
                        zoom = (zoom + 0.1).min(2.5);
                    }
                    if (zoom - app.settings.zoom).abs() > 0.001 {
                        app.settings.zoom = zoom;
                        ui.ctx().set_zoom_factor(zoom);
                        app.mark_settings_dirty();
                    }
                });
            },
        );
    });

    section(ui, &palette, app.t("settings.section.winamp"), |ui| {
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.winamp.mini.title"),
            app.t("settings.winamp.mini.description"),
            |ui| {
                if theme::pill_button(ui, &palette, app.t("settings.winamp.switch"), true).clicked()
                {
                    app.actions.push(Action::ToggleWinampWindow);
                }
            },
        );
        let folder = app.dirs.skins_dir();
        app.winamp.refresh_choices(&folder);
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.winamp.skin.title"),
            &app.tf(
                "settings.winamp.skin.description",
                &[("folder", &folder.display().to_string())],
            ),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if theme::soft_button(
                        ui,
                        &palette,
                        Some(Icon::Globe),
                        app.t("settings.winamp.skin_museum"),
                        false,
                    )
                    .clicked()
                    {
                        app.actions
                            .push(Action::OpenUrl("https://skins.webamp.org/".into()));
                    }
                    if theme::soft_button(
                        ui,
                        &palette,
                        Some(Icon::ExternalLink),
                        app.t("settings.winamp.open_folder"),
                        false,
                    )
                    .clicked()
                    {
                        app.actions.push(Action::OpenSkinsFolder);
                    }
                });
            },
        );
        let choices = app.winamp.choices.clone();
        let mut options: Vec<(usize, &str)> = vec![(0, app.t("settings.winamp.skin_default"))];
        options.extend(
            choices
                .iter()
                .enumerate()
                .map(|(index, choice)| (index + 1, choice.label())),
        );
        let current = app
            .settings
            .skin
            .as_deref()
            .and_then(|name| choices.iter().position(|choice| choice.name == name))
            .map_or(0, |index| index + 1);
        if let Some(picked) = widgets::chips(ui, &palette, &options, current)
            && picked != current
        {
            let name = picked
                .checked_sub(1)
                .map(|index| choices[index].name.clone());
            app.actions.push(Action::SetSkin(name));
        }
        ui.add_space(4.0);
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.winamp.size.title"),
            app.t("settings.winamp.size.description"),
            |ui| {
                let scale =
                    crate::winamp::WinampState::scale(&app.settings, ui.ctx().pixels_per_point());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for candidate in 1..=crate::winamp::MAX_SCALE {
                        let label = app.tf(
                            "settings.winamp.size.scale",
                            &[("n", &candidate.to_string())],
                        );
                        if theme::soft_button(ui, &palette, None, &label, candidate == scale)
                            .clicked()
                            && candidate != scale
                        {
                            app.actions.push(Action::SetSkinScale(candidate as u8));
                        }
                    }
                });
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.winamp.always_on_top.title"),
            app.t("settings.winamp.always_on_top.description"),
            |ui| {
                let mut on_top = app.settings.winamp_on_top;
                if widgets::switch(ui, &palette, &mut on_top).changed() {
                    app.actions.push(Action::ToggleWinampOnTop);
                }
            },
        );
    });

    section(ui, &palette, app.t("settings.section.equalizer"), |ui| {
        widgets::setting_row(
            ui,
            &palette,
            app.t("settings.eq.title"),
            app.t("settings.eq.description"),
            |ui| {
                let mut on = app.settings.eq_on;
                if widgets::switch(ui, &palette, &mut on).changed() {
                    app.actions.push(Action::ToggleEq);
                }
            },
        );
        let names: Vec<(usize, &str)> = crate::eq::PRESETS
            .iter()
            .enumerate()
            .map(|(index, preset)| (index, app.t(preset.label_key())))
            .collect();
        let current = crate::eq::PRESETS
            .iter()
            .position(|preset| preset.bands_db == app.settings.eq_bands_db)
            .unwrap_or(usize::MAX);
        if let Some(picked) = widgets::chips(ui, &palette, &names, current) {
            app.actions.push(Action::ApplyEqPreset(picked));
        }
        ui.add_space(10.0);
        eq_curve(ui, &palette, &crate::app::eq_settings(&app.settings));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let on = app.settings.eq_on;
            let mut preamp = app.settings.eq_preamp_db;
            if eq_slider(ui, &palette, "Pre", &mut preamp, 0.0, on) {
                app.actions.push(Action::SetEqPreamp(preamp));
            }
            for (band, hz) in crate::eq::BANDS.iter().enumerate() {
                let mut gain = app.settings.eq_bands_db[band];
                if eq_slider(
                    ui,
                    &palette,
                    &hertz(*hz),
                    &mut gain,
                    crate::eq::RANGE_DB,
                    on,
                ) {
                    app.actions.push(Action::SetEqBand(band, gain));
                }
            }
        });
    });

    section(ui, &palette, "Storage", |ui| {
        widgets::setting_row(
            ui,
            &palette,
            "Artwork cache",
            &format!("Covers are kept in {}", app.dirs.art_cache_dir().display()),
            |ui| {
                if theme::soft_button(ui, &palette, Some(Icon::Trash), "Clear artwork", false)
                    .clicked()
                {
                    app.actions.push(Action::ClearArtCache);
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            "Audio cache",
            &format!("Audio is kept in {}", app.dirs.audio_cache_dir().display()),
            |_| {},
        );
        widgets::setting_row(
            ui,
            &palette,
            "Sign-in",
            &format!(
                "Credentials are kept in {}",
                app.dirs.credentials_dir().display()
            ),
            |_| {},
        );
    });

    section(ui, &palette, "About", |ui| {
        ui.horizontal(|ui| {
            let (logo, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
            theme::logo(ui, logo.center(), 40.0, palette.accent, palette.on_accent);
            ui.vertical(|ui| {
                theme::text(
                    ui,
                    format!("Fastpotify {}", env!("CARGO_PKG_VERSION")),
                    theme::semibold(15.0),
                    palette.text,
                );
                theme::text(
                    ui,
                    "Built with Rust, egui, and librespot. Not affiliated with Spotify.",
                    theme::regular(13.0),
                    palette.secondary,
                );
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if theme::soft_button(ui, &palette, Some(Icon::Info), "Keyboard shortcuts", false)
                .clicked()
            {
                app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
            }
            if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), "Source code", false)
                .clicked()
            {
                ui.ctx()
                    .open_url(egui::OpenUrl::new_tab(env!("CARGO_PKG_REPOSITORY")));
            }
        });
    });

    ui.data_mut(|data| data.insert_temp(dirty_id, playback_dirty));
    if changed {
        app.actions.push(Action::SettingsChanged);
    }
}

/// A band's frequency the short way: 60, 170, 1K, 16K.
fn hertz(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{}K", (hz / 1000.0).round() as u32)
    } else {
        format!("{}", hz.round() as u32)
    }
}

/// One vertical slider in the app's own style: the track filled from
/// 0 dB, the handle in the middle when flat, a double-click to put it
/// back there. Returns whether it moved.
fn eq_slider(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    value: &mut f32,
    ceiling: f32,
    on: bool,
) -> bool {
    use egui::{Rect, Stroke, pos2, vec2};
    let range = crate::eq::RANGE_DB;
    ui.vertical(|ui| {
        let (rect, response) =
            ui.allocate_exact_size(vec2(30.0, 118.0), egui::Sense::click_and_drag());
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let track = Rect::from_center_size(rect.center(), vec2(4.0, rect.height() - 20.0));
        let y_of = |db: f32| track.bottom() - (db + range) / (2.0 * range) * track.height();
        let mut changed = false;
        if response.double_clicked() {
            *value = 0.0;
            changed = true;
        } else if (response.dragged() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let db = (track.bottom() - pos.y) / track.height() * 2.0 * range - range;
            let db = (db.clamp(-range, ceiling) * 10.0).round() / 10.0;
            if db != *value {
                *value = db;
                changed = true;
            }
        }
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            painter.rect_filled(track, 2.0, palette.surface_active);
            let fill = if on { palette.accent } else { palette.dim };
            let (top, bottom) = (y_of(value.max(0.0)), y_of(value.min(0.0)));
            painter.rect_filled(
                Rect::from_min_max(pos2(track.left(), top), pos2(track.right(), bottom)),
                2.0,
                fill,
            );
            painter.hline(
                (track.left() - 3.0)..=(track.right() + 3.0),
                y_of(0.0),
                Stroke::new(1.0, palette.dim),
            );
            let handle = pos2(track.center().x, y_of(*value));
            painter.circle_filled(handle, 7.0, palette.text);
            if response.hovered() || response.dragged() {
                painter.text(
                    pos2(track.center().x, rect.top() + 2.0),
                    egui::Align2::CENTER_TOP,
                    format!("{value:+.1}"),
                    theme::regular(11.0),
                    palette.secondary,
                );
            }
        }
        theme::text(ui, label, theme::regular(11.5), palette.secondary);
        changed
    })
    .inner
}

/// The equalizer's response over the audible range, the bands marked on
/// it: the shape says what a row of numbers cannot.
fn eq_curve(ui: &mut egui::Ui, palette: &Palette, settings: &crate::eq::EqSettings) {
    use egui::{Shape, Stroke, pos2, vec2};
    let width = ui.available_width().min(720.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 120.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, theme::RADIUS as f32, palette.surface);
    let plot = rect.shrink2(vec2(10.0, 12.0));
    let (low, high) = (20f32.log10(), 20_000f32.log10());
    let x_of = |hz: f32| plot.left() + (hz.log10() - low) / (high - low) * plot.width();
    let y_of = |db: f32| {
        plot.center().y
            - db.clamp(-crate::eq::RANGE_DB, crate::eq::RANGE_DB) / crate::eq::RANGE_DB
                * plot.height()
                / 2.0
    };
    for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let color = if db == 0.0 {
            palette.dim
        } else {
            palette.outline
        };
        painter.hline(plot.x_range(), y_of(db), Stroke::new(1.0, color));
    }
    for hz in crate::eq::BANDS {
        painter.vline(x_of(hz), plot.y_range(), Stroke::new(1.0, palette.outline));
    }
    let points: Vec<egui::Pos2> = (0..=240)
        .map(|step| {
            let t = step as f32 / 240.0;
            let hz = 10f32.powf(low + t * (high - low));
            pos2(
                plot.left() + t * plot.width(),
                y_of(settings.response_db(hz)),
            )
        })
        .collect();
    let color = if settings.on {
        palette.accent
    } else {
        palette.dim
    };
    painter.add(Shape::line(points, Stroke::new(2.0, color)));
    for (hz, db) in crate::eq::BANDS.iter().zip(settings.bands_db) {
        painter.circle_filled(pos2(x_of(*hz), y_of(db + settings.preamp_db)), 3.0, color);
    }
}

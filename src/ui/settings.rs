//! The Settings page.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Dialog};
use crate::settings::{PlaybackBackend, ThemeChoice};
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
    theme::text(ui, "Settings", theme::bold(28.0), palette.text);
    ui.add_space(4.0);
    let dirty_id = egui::Id::new(PLAYBACK_DIRTY_ID);
    let mut playback_dirty = ui
        .data(|data| data.get_temp::<bool>(dirty_id))
        .unwrap_or(false);
    let mut changed = false;

    section(ui, &palette, "Account", |ui| {
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
                    .map(|product| match product.to_ascii_lowercase().as_str() {
                        "premium" => "Spotify Premium".to_string(),
                        "free" | "open" => {
                            "Spotify Free · bundled alternate local audio".to_string()
                        }
                        _ => "Spotify account · Premium not confirmed".to_string(),
                    })
                    .unwrap_or_else(|| {
                        if app.user.is_some() {
                            "Spotify account · Premium not confirmed".to_string()
                        } else {
                            String::new()
                        }
                    });
                theme::text(ui, product, theme::regular(13.0), palette.secondary);
                if let Some(username) = app.local.connected.then(|| app.local.username.clone())
                    && !username.is_empty()
                {
                    theme::text(
                        ui,
                        format!("Connected as {username}"),
                        theme::regular(12.0),
                        palette.dim,
                    );
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::pill_button(ui, &palette, "Sign out", false).clicked() {
                    app.actions.push(Action::SignOut);
                }
            });
        });
        ui.add_space(10.0);
        let mut client_id = app.settings.web_client_id.clone().unwrap_or_default();
        widgets::setting_row(
            ui,
            &palette,
            "Make it even faster",
            "Spotify limits each app, and everyone shares this one. An app of your own has its own limit, but opens only playlists you own. Paste its Client ID here.",
            |ui| {
                let response = Frame::new()
                    .fill(palette.surface)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut client_id)
                                .hint_text(egui::RichText::new("Client ID").color(palette.dim))
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
            "Don't have one?",
            "It's free and takes five minutes in Spotify's developer dashboard.",
            |ui| {
                if theme::pill_button(ui, &palette, "Show me how", false).clicked() {
                    app.actions.push(Action::OpenUrl(
                        "https://fastpotify.rocks/make-it-even-faster/".into(),
                    ));
                }
            },
        );
        // Whether the app named above is the one signed in with. Switching
        // means one more trip through the browser, so it is a button, not a
        // side effect of typing.
        let wanted = app
            .settings
            .web_client_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or(crate::auth::DEFAULT_WEB_CLIENT_ID)
            .to_string();
        let own = wanted != crate::auth::DEFAULT_WEB_CLIENT_ID;
        let in_use = app.web_app.as_deref() == Some(wanted.as_str());
        if in_use && own {
            widgets::setting_row(
                ui,
                &palette,
                "Your app is in use",
                "Requests go through your own limit.",
                |ui| {
                    theme::text(ui, "In use", theme::medium(13.0), palette.accent);
                },
            );
        } else if !in_use && app.web_app.is_some() {
            let (title, detail) = if own {
                (
                    "Ready to switch to your app",
                    "Fastpotify signs in again with it; your browser opens once.",
                )
            } else {
                (
                    "Back to the shared app?",
                    "Fastpotify signs in again with it; your browser opens once.",
                )
            };
            widgets::setting_row(ui, &palette, title, detail, |ui| {
                if theme::pill_button(ui, &palette, "Switch now", true).clicked() {
                    app.actions.push(Action::SwitchWebApp);
                }
            });
        }
    });

    section(ui, &palette, "Playback on this computer", |ui| {
        let audio_source_help = if app.is_confirmed_premium() {
            "Spotify Connect is the default and needs Premium. Alternate mode keeps Spotify metadata and plays a third-party match on this computer. It is not Spotify audio and not Spotify Connect."
        } else if app.is_free_account() {
            "Spotify Free cannot use Spotify Connect or Premium player controls. This computer stays on bundled alternate local audio. It is not Spotify audio and not Spotify Connect."
        } else {
            "Spotify Connect needs a confirmed Premium account. This computer uses bundled alternate local audio. It is not Spotify audio and not Spotify Connect."
        };
        widgets::setting_row(ui, &palette, "Audio source", audio_source_help, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let connect_locked = !app.is_confirmed_premium();
                for choice in PlaybackBackend::ALL {
                    let locked = connect_locked && choice == PlaybackBackend::Spotify;
                    if theme::soft_button(
                        ui,
                        &palette,
                        None,
                        choice.label(),
                        app.settings.playback_backend == choice,
                    )
                    .clicked()
                        && app.settings.playback_backend != choice
                        && !locked
                    {
                        app.settings.playback_backend = choice;
                        changed = true;
                        playback_dirty = true;
                    }
                }
            });
        });
        let (status, detail, action) = match (&app.local_playback, app.settings.playback_backend) {
            (crate::backend::LocalPlayback::AlternateReady { source }, _) => (
                "Ready",
                format!("This computer plays matched third-party audio locally ({source}). Not Spotify Connect."),
                None,
            ),
            (crate::backend::LocalPlayback::Ready { .. }, PlaybackBackend::Spotify) => (
                "Ready",
                "This computer is a Spotify Connect device.".to_string(),
                None,
            ),
            (crate::backend::LocalPlayback::Authorizing, _) => (
                "Setting up",
                "Finish authorizing in your browser.".to_string(),
                None,
            ),
            (crate::backend::LocalPlayback::Connecting, _) => {
                ("Connecting", "Connecting…".to_string(), None)
            }
            (crate::backend::LocalPlayback::Failed(message), _) => {
                ("Unavailable", message.clone(), Some("Try again"))
            }
            (crate::backend::LocalPlayback::Unavailable, PlaybackBackend::Alternate)
            | (crate::backend::LocalPlayback::Ready { .. }, PlaybackBackend::Alternate) => (
                "Not applied",
                if crate::alternate::has_bundled_ytdlp() {
                    "Apply playback settings to start alternate local audio. Uses the official pinned yt-dlp extracted locally, or a Piped API URL."
                        .to_string()
                } else {
                    "Apply playback settings to start alternate local audio. Needs a Piped API URL or yt-dlp."
                        .to_string()
                },
                None,
            ),
            (crate::backend::LocalPlayback::Unavailable, _) => (
                "Not set up",
                "Play music on this computer. Needs Spotify Premium and a one-time browser sign-in."
                    .to_string(),
                Some("Enable playback here"),
            ),
        };
        widgets::setting_row(ui, &palette, &format!("Status: {status}"), &detail, |ui| {
            if let Some(label) = action {
                if theme::pill_button(ui, &palette, label, true).clicked() {
                    if app.settings.playback_backend == PlaybackBackend::Alternate {
                        app.actions.push(Action::RestartEngine);
                    } else {
                        app.actions.push(Action::EnablePlayback);
                    }
                }
            } else if app.local_ready
                && theme::soft_button(ui, &palette, Some(Icon::Refresh), "Reconnect", false)
                    .clicked()
            {
                app.actions.push(Action::RestartEngine);
            }
        });
        widgets::setting_row(
            ui,
            &palette,
            "Device name",
            if app.settings.playback_backend == PlaybackBackend::Spotify {
                "How this computer appears in Spotify Connect."
            } else {
                "How this computer is labelled in the device list. Not a Spotify Connect name."
            },
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
        if app.settings.playback_backend == PlaybackBackend::Alternate {
            let piped_error =
                crate::alternate::validate_piped_base(&app.settings.piped_api_base).err();
            let ytdlp_error = crate::alternate::ytdlp_path_notice(&app.settings.ytdlp_path);
            widgets::setting_row(
                ui,
                &palette,
                "Piped API base URL",
                piped_error.as_deref().unwrap_or(
                    "Your Piped-compatible instance, for example https://piped.example. No public instance is bundled. Do not put secrets in this URL.",
                ),
                |ui| {
                    let response = Frame::new()
                        .fill(palette.surface)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut app.settings.piped_api_base)
                                    .hint_text(
                                        egui::RichText::new("https://piped.example")
                                            .color(palette.dim),
                                    )
                                    .font(theme::regular(13.0))
                                    .frame(egui::Frame::NONE)
                                    .desired_width(220.0),
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
                "yt-dlp path",
                ytdlp_error.as_deref().unwrap_or(
                    if crate::alternate::has_bundled_ytdlp() {
                        "Optional. Fastpotify extracts an official pinned yt-dlp locally and never downloads it at runtime. A strictly newer installed yt-dlp (this path or PATH) wins. Spotify tokens are never sent to it."
                    } else {
                        "Optional. Blank uses yt-dlp on PATH. This build has no bundled yt-dlp. Fastpotify never downloads yt-dlp at runtime and never sends Spotify tokens to it."
                    },
                ),
                |ui| {
                    let response = Frame::new()
                        .fill(palette.surface)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut app.settings.ytdlp_path)
                                    .hint_text(
                                        egui::RichText::new("yt-dlp on PATH").color(palette.dim),
                                    )
                                    .font(theme::regular(13.0))
                                    .frame(egui::Frame::NONE)
                                    .desired_width(220.0),
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
                "Minimum match score",
                "Reject weak matches. Higher is stricter. A miss never plays the low-confidence hit.",
                |ui| {
                    let mut score = app.settings.alternate_min_score;
                    let response = ui.add(
                        egui::Slider::new(&mut score, 0.30..=0.90)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    theme::text(
                        ui,
                        format!("{:.0}%", score * 100.0),
                        theme::medium(13.0),
                        palette.secondary,
                    );
                    if response.changed() {
                        app.settings.alternate_min_score = score;
                        changed = true;
                        playback_dirty = true;
                    }
                },
            );
            widgets::setting_row(
                ui,
                &palette,
                "Skip on miss",
                "If no match meets the score, skip to the next track. Network and decode failures stop; they do not skip.",
                |ui| {
                    if widgets::switch(ui, &palette, &mut app.settings.alternate_skip_on_miss)
                        .changed()
                    {
                        changed = true;
                        playback_dirty = true;
                    }
                },
            );
            ui.add_space(4.0);
            theme::subtle(
                ui,
                &palette,
                if crate::alternate::has_bundled_ytdlp() {
                    "You are responsible for the Piped instance and any yt-dlp you run, and for following their terms. The bundled yt-dlp is the official pinned build, extracted into this computer's state directory, never downloaded at runtime. Matches are not Spotify audio. Podcasts are not supported. Playback starts after buffering; an M4A file with moov at the end may wait. Network and decode failures stop rather than skip. Apply restarts the local player; Spotify Connect is stopped while this mode is on."
                } else {
                    "You are responsible for the Piped instance and yt-dlp you run, and for following their terms. Matches are not Spotify audio. Podcasts are not supported. Playback starts after buffering; an M4A file with moov at the end may wait. Network and decode failures stop rather than skip. Apply restarts the local player; Spotify Connect is stopped while this mode is on."
                },
            );
            ui.add_space(8.0);
        }
        if app.settings.playback_backend == PlaybackBackend::Spotify {
            widgets::setting_row(
                ui,
                &palette,
                "Audio quality",
                "Higher bitrates use more data and cache space.",
                |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for (kbps, label) in [(320u16, "Very high"), (160, "High"), (96, "Normal")]
                        {
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
                "Normalize volume",
                "Keep loud and quiet tracks at a similar level.",
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
                "Autoplay",
                "Keep playing similar songs when your music ends.",
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
                "Gapless playback",
                "Run tracks into each other without silence.",
                |ui| {
                    if widgets::switch(ui, &palette, &mut app.settings.gapless).changed() {
                        changed = true;
                        playback_dirty = true;
                    }
                },
            );
        }
        widgets::setting_row(
            ui,
            &palette,
            "Keep music playing when the window closes",
            "Fastpotify hides to the system tray. Quit from the tray menu or with Ctrl+Q.",
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
            "Tell me when a new version is out",
            "Asks GitHub once a day. Nothing about you is sent.",
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.check_for_updates).changed() {
                    changed = true;
                }
            },
        );
        if app.settings.playback_backend == PlaybackBackend::Spotify && cfg!(target_os = "linux") {
            widgets::setting_row(
                ui,
                &palette,
                "Audio output",
                "PulseAudio also covers PipeWire. Rodio talks to ALSA directly.",
                |ui| {
                    let current = app
                        .settings
                        .platform_backend()
                        .unwrap_or_else(|| "rodio".into());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for backend in ["rodio", "pulseaudio"] {
                            let label = if backend == "pulseaudio" {
                                "PulseAudio / PipeWire"
                            } else {
                                "ALSA (rodio)"
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
        if app.settings.playback_backend == PlaybackBackend::Spotify {
            widgets::setting_row(
                ui,
                &palette,
                "Audio cache",
                "Keep downloaded audio so replays don't stream again.",
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
                            for (mb, label) in [(4096u64, "4 GB"), (1024, "1 GB"), (512, "512 MB")]
                            {
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
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if playback_dirty {
                if theme::pill_button(ui, &palette, "Apply and restart playback", true).clicked() {
                    app.actions.push(Action::RestartEngine);
                    playback_dirty = false;
                }
                theme::subtle(
                    ui,
                    &palette,
                    "Playback settings take effect after a restart of the local player.",
                );
            } else {
                theme::subtle(ui, &palette, "Playback settings are applied.");
            }
        });
    });

    section(ui, &palette, "Appearance", |ui| {
        widgets::setting_row(ui, &palette, "Theme", "", |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for choice in ThemeChoice::ALL {
                    if theme::soft_button(
                        ui,
                        &palette,
                        None,
                        choice.label(),
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
            "Colour from album art",
            "Tint pages and the player with the playing cover.",
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.accent_from_art).changed() {
                    changed = true;
                }
            },
        );
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
            ui.painter()
                .circle_filled(logo.center(), 20.0, palette.accent);
            let icon_rect = egui::Rect::from_center_size(
                logo.center() + Vec2::new(2.0, 0.0),
                Vec2::splat(18.0),
            );
            Icon::PlayFilled
                .image(palette.on_accent, 18.0)
                .paint_at(ui, icon_rect);
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

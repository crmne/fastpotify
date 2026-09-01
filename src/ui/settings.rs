//! The Settings page.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Dialog};
use crate::settings::ThemeChoice;
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
                    .map(|product| match product.as_str() {
                        "premium" => "Spotify Premium".to_string(),
                        "free" | "open" => "Spotify Free, local playback needs Premium".to_string(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
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
            "Add your own Spotify Development Mode app as optional acceleration. Fastpotify keeps the shared app for catalog coverage and external playlists.",
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
                "Personal acceleration is ready",
                "Supported requests use your app. Shared catalog coverage stays available.",
                |ui| {
                    if theme::pill_button(ui, &palette, "Remove", false).clicked() {
                        app.settings.web_client_id = None;
                        app.actions.push(Action::ConfigurePersonalWebApp);
                    }
                },
            );
        } else if wanted.is_some() {
            widgets::setting_row(
                ui,
                &palette,
                "Authorize your personal app",
                "Spotify opens once to verify that both sessions belong to this account.",
                |ui| {
                    if theme::pill_button(ui, &palette, "Authorize", true).clicked() {
                        app.actions.push(Action::ConfigurePersonalWebApp);
                    }
                },
            );
        } else if app.web_app.is_some() {
            widgets::setting_row(
                ui,
                &palette,
                "Remove personal app",
                "Shared access remains signed in.",
                |ui| {
                    if theme::pill_button(ui, &palette, "Remove", false).clicked() {
                        app.actions.push(Action::ConfigurePersonalWebApp);
                    }
                },
            );
        }
    });

    section(ui, &palette, "Playback on this computer", |ui| {
        let (status, detail, action) = match &app.local_playback {
            crate::backend::LocalPlayback::Ready { .. } => (
                "Ready",
                "This computer is a Spotify Connect device.".to_string(),
                None,
            ),
            crate::backend::LocalPlayback::Authorizing => (
                "Setting up",
                "Finish authorizing in your browser.".to_string(),
                None,
            ),
            crate::backend::LocalPlayback::Connecting => {
                ("Connecting", "Connecting to Spotify…".to_string(), None)
            }
            crate::backend::LocalPlayback::Failed(message) => {
                ("Unavailable", message.clone(), Some("Try again"))
            }
            crate::backend::LocalPlayback::Unavailable => (
                "Not set up",
                "Play music on this computer. Needs Spotify Premium and a one-time browser sign-in."
                    .to_string(),
                Some("Enable playback here"),
            ),
        };
        widgets::setting_row(ui, &palette, &format!("Status: {status}"), &detail, |ui| {
            if let Some(label) = action {
                if theme::pill_button(ui, &palette, label, true).clicked() {
                    app.actions.push(Action::EnablePlayback);
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
            "How this computer appears in Spotify Connect.",
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
            "Audio quality",
            "Higher bitrates use more data and cache space.",
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (kbps, label) in [
                        (320u16, "Very high · 320 kbps"),
                        (160, "High · 160 kbps"),
                        (96, "Normal · 96 kbps"),
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
        widgets::setting_row(
            ui,
            &palette,
            "Keep music playing when the window closes",
            super::keys::platform_shortcut(
                "Fastpotify hides to the system tray. Quit from the tray menu or with Ctrl+Q.",
                "Fastpotify hides to the system tray. Quit from the tray menu or with Cmd+Q.",
            ),
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
        if cfg!(target_os = "linux") {
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
        widgets::setting_row(
            ui,
            &palette,
            "Output buffer",
            "How much sound the speaker is given at a time. More rides out a busy machine without clicking; less answers the controls sooner.",
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let current = app.settings.audio_buffer_ms;
                    for ms in [50u32, 100, 200] {
                        let label = format!("{ms} ms");
                        if theme::soft_button(ui, &palette, None, &label, current == ms).clicked()
                            && current != ms
                        {
                            app.settings.audio_buffer_ms = ms;
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
                        for (mb, label) in [(4096u64, "4 GB"), (1024, "1 GB"), (512, "512 MB")] {
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
        widgets::setting_row(
            ui,
            &palette,
            "Compact library sidebar",
            "Show names only in the sidebar without cover artwork.",
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.sidebar_compact).changed() {
                    changed = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            "Compact track list",
            "Show track names and artists in a single line without cover artwork.",
            |ui| {
                if widgets::switch(ui, &palette, &mut app.settings.tracklist_compact).changed() {
                    changed = true;
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            "Interface zoom",
            super::keys::platform_shortcut(
                "Ctrl+Plus and Ctrl+Minus work anywhere; Ctrl+0 resets.",
                "Cmd+Plus and Cmd+Minus work anywhere; Cmd+0 resets.",
            ),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mut zoom = app.settings.zoom;
                    if theme::soft_button(ui, &palette, None, "-", false).clicked() {
                        zoom = (zoom - 0.1).max(0.5);
                    }
                    theme::text(
                        ui,
                        format!("{:.0}%", zoom * 100.0),
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

    section(ui, &palette, "Winamp skins", |ui| {
        widgets::setting_row(
            ui,
            &palette,
            "Mini player",
            super::keys::platform_shortcut(
                "Fastpotify becomes a small player that wears classic Winamp skins (.wsz files); the logo in the skin, or Ctrl+M, brings this window back. Drop a skin on either window to add it.",
                "Fastpotify becomes a small player that wears classic Winamp skins (.wsz files); the logo in the skin, or Cmd+Shift+M, brings this window back. Drop a skin on either window to add it.",
            ),
            |ui| {
                if theme::pill_button(ui, &palette, "Switch to it", true).clicked() {
                    app.actions.push(Action::ToggleWinampWindow);
                }
            },
        );
        let folder = app.dirs.skins_dir();
        app.winamp.refresh_choices(&folder);
        widgets::setting_row(
            ui,
            &palette,
            "Skin",
            &format!(
                "Skins in {} are listed here. Thousands more are at the Winamp Skin Museum.",
                folder.display()
            ),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if theme::soft_button(ui, &palette, Some(Icon::Globe), "Skin Museum", false)
                        .clicked()
                    {
                        app.actions
                            .push(Action::OpenUrl("https://skins.webamp.org/".into()));
                    }
                    if theme::soft_button(
                        ui,
                        &palette,
                        Some(Icon::ExternalLink),
                        "Open folder",
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
        let mut options: Vec<(usize, &str)> = vec![(0, "Fastpotify")];
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
            "Size",
            "Screen pixels per skin pixel, always a whole number so the pixels stay crisp.",
            |ui| {
                let scale =
                    crate::winamp::WinampState::scale(&app.settings, ui.ctx().pixels_per_point());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for candidate in 1..=crate::winamp::MAX_SCALE {
                        let label = format!("{candidate}x");
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
            "Always on top",
            "Keep the Winamp window above everything else.",
            |ui| {
                let mut on_top = app.settings.winamp_on_top;
                if widgets::switch(ui, &palette, &mut on_top).changed() {
                    app.actions.push(Action::ToggleWinampOnTop);
                }
            },
        );
    });

    section(ui, &palette, "MilkDrop", |ui| {
        widgets::setting_row(
            ui,
            &palette,
            "MilkDrop window",
            super::keys::platform_shortcut(
                "The classic Winamp visualiser, drawn by projectM in a window of its own. Open it here, with the vis button in the top bar, with Ctrl+Shift+K, or with the V on the mini player. Its keys are MilkDrop's own; press ? or F1 inside it to see them. It draws the music played on this computer.",
                "The classic Winamp visualiser, drawn by projectM in a window of its own. Open it here, with the vis button in the top bar, with Cmd+Shift+K, or with the V on the mini player. Its keys are MilkDrop's own; press ? or F1 inside it to see them. It draws the music played on this computer.",
            ),
            |ui| {
                let mut open = app.settings.milkdrop_open;
                if widgets::switch(ui, &palette, &mut open).changed() {
                    app.actions.push(Action::ToggleWinampMilkdrop);
                }
            },
        );
        let folder = app.dirs.milkdrop_dir();
        app.winamp.presets.refresh(&folder);
        let count = app.winamp.presets.count();
        let downloading = app.winamp.presets.downloading();
        widgets::setting_row(
            ui,
            &palette,
            "Presets",
            &format!(
                "{} in {}. Presets are .milk files; put your own there. An empty folder fills itself the first time MilkDrop opens.",
                match count {
                    0 => "None yet".to_string(),
                    1 => "One preset".to_string(),
                    n => format!("{n} presets"),
                },
                folder.display(),
            ),
            |_ui| {},
        );
        // Three buttons are wider than a row's control slot; they get a
        // line of their own under the words.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (index, pack) in crate::milkdrop::PACKS.iter().enumerate() {
                let label = match downloading {
                    Some(name) if name == pack.name => "Fetching...".to_string(),
                    _ => format!("Get {}", pack.name),
                };
                if theme::soft_button(ui, &palette, Some(Icon::Globe), &label, false)
                    .on_hover_text(pack.note)
                    .clicked()
                    && downloading.is_none()
                {
                    app.actions.push(Action::DownloadMilkdropPack(index));
                }
            }
            if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), "Open folder", false)
                .clicked()
            {
                app.actions.push(Action::OpenMilkdropFolder);
            }
        });
        ui.add_space(10.0);
        widgets::setting_row(
            ui,
            &palette,
            "Time per preset",
            "How long each preset plays before the next fades in.",
            |ui| {
                let mut seconds = app.settings.milkdrop_seconds.clamp(2, 300);
                let slider = egui::Slider::new(&mut seconds, 2..=300)
                    .logarithmic(true)
                    .suffix(" s");
                if ui.add(slider).changed() {
                    app.actions.push(Action::SetMilkdropSeconds(seconds));
                }
            },
        );
        let screen_hz = app.settings.milkdrop_screen_hz;
        widgets::setting_row(
            ui,
            &palette,
            "Frame rate",
            &match screen_hz {
                0 => "How often the window draws. Fewer frames leave more of the machine for everything else; uncapped draws as fast as it can.".to_string(),
                hz => format!(
                    "How often the window draws. Your screen refreshes {hz} times a second, and drawing more often than that shows nobody anything; uncapped draws as fast as it can."
                ),
            },
            |ui| {
                let fps = app.settings.milkdrop_fps;
                // The dial stops at the rates worth having and passes
                // through nothing in between, the way a gear lever does.
                let stops = crate::milkdrop::fps_stops(screen_hz, fps);
                let last = stops.len().saturating_sub(1);
                let mut at = stops.iter().position(|rate| *rate == fps).unwrap_or(1);
                let labels: Vec<String> = stops
                    .iter()
                    .map(|rate| crate::milkdrop::fps_label(*rate, screen_hz))
                    .collect();
                let shown = labels.clone();
                let typed = stops.clone();
                let slider = egui::Slider::new(&mut at, 0..=last)
                    .step_by(1.0)
                    .custom_formatter(move |value, _| {
                        shown
                            .get((value.round().max(0.0) as usize).min(shown.len() - 1))
                            .cloned()
                            .unwrap_or_default()
                    })
                    .custom_parser(move |text| {
                        // A rate typed in lands on the nearest stop, since
                        // the stops are all this dial can hold.
                        let text = text.trim().to_lowercase();
                        if text.starts_with("un") {
                            return Some(typed.len().saturating_sub(1) as f64);
                        }
                        let wanted: u32 = text
                            .trim_end_matches("fps")
                            .trim()
                            .split(',')
                            .next()?
                            .trim()
                            .parse()
                            .ok()?;
                        typed
                            .iter()
                            .enumerate()
                            .filter(|(_, rate)| **rate > 0)
                            .min_by_key(|(_, rate)| rate.abs_diff(wanted))
                            .map(|(index, _)| index as f64)
                    });
                if ui.add(slider).changed()
                    && let Some(rate) = stops.get(at)
                {
                    app.actions.push(Action::SetMilkdropFps(*rate));
                }
            },
        );
        widgets::setting_row(
            ui,
            &palette,
            "Resolution",
            "The picture's inner resolution. Half or a quarter eases a slower machine; the window stretches it back up with hard pixels.",
            |ui| {
                let current = app.settings.milkdrop_scale.max(1);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (scale, label) in [(1u32, "Full"), (2, "Half"), (4, "Quarter")] {
                        if theme::soft_button(ui, &palette, None, label, scale == current).clicked()
                            && scale != current
                        {
                            app.actions.push(Action::SetMilkdropScale(scale));
                        }
                    }
                });
            },
        );
    });

    section(ui, &palette, "Equalizer", |ui| {
        widgets::setting_row(
            ui,
            &palette,
            "Equalizer",
            "Ten bands over the music played on this computer. Speakers and phones across the room play what Spotify sends them.",
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
            .map(|(index, preset)| (index, preset.name))
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
            if eq_slider(ui, &palette, "Pre", &mut preamp, on) {
                app.actions.push(Action::SetEqPreamp(preamp));
            }
            for (band, hz) in crate::eq::BANDS.iter().enumerate() {
                let mut gain = app.settings.eq_bands_db[band];
                if eq_slider(ui, &palette, &hertz(*hz), &mut gain, on) {
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
            "Play history",
            &format!(
                "Spotify is never told what plays here, so the Recent tab keeps its own list, in {}. It is never sent anywhere.",
                app.dirs.history_file().display()
            ),
            |ui| {
                if theme::soft_button(ui, &palette, Some(Icon::Trash), "Clear history", false)
                    .clicked()
                {
                    app.actions.push(Action::ClearPlayHistory);
                }
            },
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
fn eq_slider(ui: &mut egui::Ui, palette: &Palette, label: &str, value: &mut f32, on: bool) -> bool {
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
            let db = (db.clamp(-range, range) * 10.0).round() / 10.0;
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
    let curve = settings.curve();
    let points: Vec<egui::Pos2> = (0..=240)
        .map(|step| {
            let t = step as f32 / 240.0;
            let hz = 10f32.powf(low + t * (high - low));
            pos2(plot.left() + t * plot.width(), y_of(curve.db_at(hz)))
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

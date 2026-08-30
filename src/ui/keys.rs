//! Keyboard shortcuts.

use egui::{Key, Modifiers};

use crate::app::App;
use crate::model::{Action, Dialog, Page};

pub fn handle(app: &mut App, ctx: &egui::Context) {
    let typing = ctx.memory(|memory| memory.focused().is_some());
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        let mut key = |modifiers: Modifiers, key: Key, action: Action| {
            if input.consume_key(modifiers, key) {
                actions.push(action);
            }
        };
        key(Modifiers::COMMAND, Key::F, Action::FocusSearch);
        key(Modifiers::COMMAND, Key::B, Action::ToggleSidebar);
        key(Modifiers::COMMAND, Key::Comma, Action::Open(Page::Settings));
        key(Modifiers::COMMAND, Key::Q, Action::Quit);
        // The platform's close key. macOS only closes a window from its
        // menu, which winit does not install, and the mini player has no
        // title bar for the system to close it by.
        key(Modifiers::COMMAND, Key::W, Action::CloseWindow);
        // winit installs its own macOS app menu, whose Hide item owns Cmd+H
        // before the window is offered the key.
        if cfg!(target_os = "macos") {
            key(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::H,
                Action::Open(Page::Home),
            );
        } else {
            key(Modifiers::COMMAND, Key::H, Action::Open(Page::Home));
        }
        key(Modifiers::COMMAND, Key::L, Action::Open(Page::LikedSongs));
        // Cmd+M minimises on macOS.
        if cfg!(target_os = "macos") {
            key(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::M,
                Action::ToggleWinampWindow,
            );
        } else {
            key(Modifiers::COMMAND, Key::M, Action::ToggleWinampWindow);
        }
        key(
            Modifiers::COMMAND,
            Key::Slash,
            Action::ShowDialog(Dialog::Shortcuts),
        );
        key(Modifiers::ALT, Key::ArrowLeft, Action::Back);
        key(Modifiers::ALT, Key::ArrowRight, Action::Forward);
        key(Modifiers::COMMAND, Key::ArrowLeft, Action::Previous);
        key(Modifiers::COMMAND, Key::ArrowRight, Action::Next);
        key(Modifiers::COMMAND, Key::ArrowUp, Action::VolumeBy(5));
        key(Modifiers::COMMAND, Key::ArrowDown, Action::VolumeBy(-5));
        key(
            Modifiers::COMMAND | Modifiers::SHIFT,
            Key::A,
            Action::OpenUri("artist".into()),
        );
        key(
            Modifiers::COMMAND | Modifiers::SHIFT,
            Key::B,
            Action::OpenUri("album".into()),
        );
        // Cmd+Shift+Q is Log Out, taken by the window server.
        if cfg!(target_os = "macos") {
            key(Modifiers::COMMAND, Key::U, Action::ToggleQueuePanel);
        } else {
            key(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::Q,
                Action::ToggleQueuePanel,
            );
        }
        if !typing {
            key(
                Modifiers::NONE,
                Key::Questionmark,
                Action::ShowDialog(Dialog::Shortcuts),
            );
            key(
                Modifiers::SHIFT,
                Key::Questionmark,
                Action::ShowDialog(Dialog::Shortcuts),
            );
            key(Modifiers::SHIFT, Key::ArrowLeft, Action::SeekBy(-10_000));
            key(Modifiers::SHIFT, Key::ArrowRight, Action::SeekBy(10_000));
            key(Modifiers::NONE, Key::Space, Action::TogglePlay);
            key(Modifiers::NONE, Key::M, Action::ToggleMute);
            key(Modifiers::NONE, Key::S, Action::ToggleShuffle);
            key(Modifiers::NONE, Key::R, Action::CycleRepeat);
            key(Modifiers::NONE, Key::Q, Action::ToggleQueuePanel);
            key(Modifiers::NONE, Key::L, Action::ToggleLyricsPanel);
            key(Modifiers::NONE, Key::Slash, Action::FocusSearch);
        }
    });
    // Resolve the "open current artist/album" placeholders.
    for action in actions {
        match action {
            Action::OpenUri(kind) if kind == "artist" => {
                if let Some(id) = app
                    .now_playing()
                    .and_then(|now| now.artists.first().and_then(|artist| artist.id.clone()))
                {
                    app.actions.push(Action::Open(Page::Artist(id)));
                }
            }
            Action::OpenUri(kind) if kind == "album" => {
                if let Some(now) = app.now_playing() {
                    if let Some(id) = now.album_id {
                        app.actions.push(Action::Open(Page::Album(id)));
                    } else if let Some(id) = now.show_id {
                        app.actions.push(Action::Open(Page::Show(id)));
                    }
                }
            }
            other => app.actions.push(other),
        }
    }
    // A mouse's back and forward buttons, the way a browser takes them.
    let (back, forward) = ctx.input(|input| {
        (
            input.pointer.button_pressed(egui::PointerButton::Extra1),
            input.pointer.button_pressed(egui::PointerButton::Extra2),
        )
    });
    if back {
        app.actions.push(Action::Back);
    }
    if forward {
        app.actions.push(Action::Forward);
    }
    if ctx.input(|input| input.key_pressed(Key::Escape)) {
        if app.dialog.is_some() {
            app.actions.push(Action::CloseDialog);
        } else if app.show_devices {
            app.show_devices = false;
        }
    }
}

pub fn shortcuts(app: &App) -> Vec<(String, &str)> {
    let mod_key = app.catalog.mod_key();
    let home_keys = if cfg!(target_os = "macos") {
        app.t("shortcuts.keys.home_mac").to_string()
    } else {
        app.t("shortcuts.keys.home_other").to_string()
    };
    let winamp_keys = if cfg!(target_os = "macos") {
        app.t("shortcuts.keys.winamp_mac").to_string()
    } else {
        app.t("shortcuts.keys.winamp_other").to_string()
    };
    vec![
        ("Space".into(), app.t("shortcuts.play_pause")),
        (
            format!("{mod_key}+←  /  {mod_key}+→"),
            app.t("shortcuts.prev_next"),
        ),
        ("Shift+←  /  Shift+→".into(), app.t("shortcuts.seek")),
        (
            format!("{mod_key}+↑  /  {mod_key}+↓"),
            app.t("shortcuts.volume"),
        ),
        ("M".into(), app.t("shortcuts.mute")),
        ("S".into(), app.t("shortcuts.shuffle")),
        ("R".into(), app.t("shortcuts.repeat")),
        ("Q".into(), app.t("shortcuts.queue")),
        ("L".into(), app.t("shortcuts.lyrics")),
        (format!("{mod_key}+F  or  /"), app.t("shortcuts.search")),
        (format!("{mod_key}+B"), app.t("shortcuts.sidebar")),
        ("Alt+←  /  Alt+→".into(), app.t("shortcuts.history")),
        (home_keys, app.t("shortcuts.home")),
        (format!("{mod_key}+L"), app.t("shortcuts.liked")),
        (format!("{mod_key}+Shift+A"), app.t("shortcuts.go_artist")),
        (format!("{mod_key}+Shift+B"), app.t("shortcuts.go_album")),
        (winamp_keys, app.t("shortcuts.winamp")),
        (format!("{mod_key}+,"), app.t("shortcuts.settings")),
        (format!("{mod_key}+/ or ?"), app.t("shortcuts.help")),
        (format!("{mod_key}+W"), app.t("shortcuts.close")),
        (format!("{mod_key}+Q"), app.t("shortcuts.quit")),
    ]
}

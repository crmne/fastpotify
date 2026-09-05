//! Keyboard shortcuts.

use egui::{Key, Modifiers};

use crate::app::App;
use crate::model::{Action, Dialog, Page};

pub(super) const fn platform_shortcut(ctrl: &'static str, cmd: &'static str) -> &'static str {
    if cfg!(target_os = "macos") { cmd } else { ctrl }
}

pub(super) const SIDEBAR_SHORTCUT: &str = platform_shortcut("Ctrl+B", "Cmd+B");
pub(super) const QUIT_SHORTCUT: &str = platform_shortcut("Ctrl+Q", "Cmd+Q");
pub(super) const WINAMP_SHORTCUT: &str = platform_shortcut("Ctrl+M", "Cmd+Shift+M");
pub(super) const MILKDROP_SHORTCUT: &str = platform_shortcut("Ctrl+Shift+K", "Cmd+Shift+K");

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
        // Winamp's key for starting and stopping the visualisation plug-in.
        key(
            Modifiers::COMMAND | Modifiers::SHIFT,
            Key::K,
            Action::ToggleWinampMilkdrop,
        );
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
    if !typing
        && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::B))
        && let Some(now) = app.now_playing().filter(|now| !now.is_episode)
    {
        actions.push(Action::ToggleSaved(now.uri));
    }
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
    // Map mouse back and forward buttons to navigation.
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
        } else if app.lyrics_fullscreen.is_some() {
            app.actions.push(Action::SetLyricsFullscreen(false));
        }
    }
}

pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Space", "Play or pause"),
    (
        platform_shortcut("Ctrl+←  /  Ctrl+→", "Cmd+←  /  Cmd+→"),
        "Previous or next",
    ),
    ("Shift+←  /  Shift+→", "Seek 10 seconds"),
    (
        platform_shortcut("Ctrl+↑  /  Ctrl+↓", "Cmd+↑  /  Cmd+↓"),
        "Volume up or down",
    ),
    ("M", "Mute or unmute"),
    ("B", "Like or unlike the playing song"),
    ("S", "Toggle shuffle"),
    ("R", "Cycle repeat"),
    ("Q", "Show the queue"),
    ("L", "Show the lyrics"),
    (platform_shortcut("Ctrl+F  or  /", "Cmd+F  or  /"), "Search"),
    (SIDEBAR_SHORTCUT, "Show or hide the sidebar"),
    ("Alt+←  /  Alt+→", "Back or forward"),
    (platform_shortcut("Ctrl+H", "Cmd+Shift+H"), "Home"),
    (platform_shortcut("Ctrl+L", "Cmd+L"), "Liked Songs"),
    (
        platform_shortcut("Ctrl+Shift+A", "Cmd+Shift+A"),
        "Go to the playing artist",
    ),
    (
        platform_shortcut("Ctrl+Shift+B", "Cmd+Shift+B"),
        "Go to the playing album",
    ),
    (WINAMP_SHORTCUT, "Winamp mini player"),
    (MILKDROP_SHORTCUT, "MilkDrop, under the mini player"),
    ("F  or  double-click", "MilkDrop: fill the screen"),
    ("→  /  N", "MilkDrop: next preset"),
    ("←  /  P", "MilkDrop: previous preset"),
    ("L", "MilkDrop: keep this preset"),
    ("Esc", "MilkDrop: leave full screen, or close"),
    (platform_shortcut("Ctrl+,", "Cmd+,"), "Settings"),
    (
        platform_shortcut("Ctrl+/ or ?", "Cmd+/ or ?"),
        "Keyboard shortcuts",
    ),
    (platform_shortcut("Ctrl+W", "Cmd+W"), "Close the window"),
    (QUIT_SHORTCUT, "Quit"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppOptions;
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    #[test]
    fn shortcut_constants_name_the_platform_modifier() {
        let expected = if cfg!(target_os = "macos") {
            ["Cmd+B", "Cmd+Q", "Cmd+Shift+M", "Cmd+Shift+K"]
        } else {
            ["Ctrl+B", "Ctrl+Q", "Ctrl+M", "Ctrl+Shift+K"]
        };
        assert_eq!(
            [
                SIDEBAR_SHORTCUT,
                QUIT_SHORTCUT,
                WINAMP_SHORTCUT,
                MILKDROP_SHORTCUT,
            ],
            expected
        );
    }

    #[test]
    fn shortcut_dialog_never_names_the_other_command_modifier() {
        let other = if cfg!(target_os = "macos") {
            "Ctrl+"
        } else {
            "Cmd+"
        };
        for (keys, _) in SHORTCUTS {
            assert!(!keys.contains(other), "wrong modifier in {keys}");
        }
    }

    #[test]
    fn shortcut_dialog_names_platform_reserved_alternatives() {
        let label = |description| {
            SHORTCUTS
                .iter()
                .find(|(_, candidate)| *candidate == description)
                .map(|(keys, _)| *keys)
                .unwrap()
        };
        if cfg!(target_os = "macos") {
            assert_eq!(label("Home"), "Cmd+Shift+H");
            assert_eq!(label("Winamp mini player"), "Cmd+Shift+M");
        } else {
            assert_eq!(label("Home"), "Ctrl+H");
            assert_eq!(label("Winamp mini player"), "Ctrl+M");
        }
    }

    #[test]
    fn b_toggles_the_playing_song_in_liked_songs() {
        let root = std::env::temp_dir().join(format!(
            "fastpotify-like-shortcut-test-{}",
            std::process::id()
        ));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let mut app = App::new(
            &crate::backend::Waker::default(),
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        crate::demo::populate(&mut app);

        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::B,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |_ui| handle(&mut app, &ctx));
        output.textures_delta.clear();

        assert!(matches!(
            app.actions.as_slice(),
            [Action::ToggleSaved(uri)] if uri == "spotify:track:trk0"
        ));
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }
}

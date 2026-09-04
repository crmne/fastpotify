//! A status-notifier tray item (Linux), so closing the window can leave the
//! music playing.
//!
//! The tray runs on its own thread and exchanges bounded messages with the
//! interface, exactly like MPRIS: a missing or broken status-notifier host
//! must never take audio or the window down with it. When no host is
//! present, spawning fails and the app simply quits on close as before.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use ksni::blocking::TrayMethods;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    ShowHide,
    PlayPause,
    Next,
    Previous,
    Quit,
}

struct FastTray {
    commands: Sender<TrayCommand>,
    wake: Arc<dyn Fn() + Send + Sync>,
    playing: bool,
}

impl FastTray {
    fn send(&self, command: TrayCommand) {
        if self.commands.send(command).is_ok() {
            (self.wake)();
        }
    }
}

impl ksni::Tray for FastTray {
    fn id(&self) -> String {
        "fastpotify".into()
    }

    fn title(&self) -> String {
        "Fastpotify".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let size = 64usize;
        let rgba = crate::util::app_icon_rgba(size);
        // ksni wants ARGB32 in network byte order.
        let mut data = Vec::with_capacity(rgba.len());
        let (pixels, _) = rgba.as_chunks::<4>();
        for [r, g, b, a] in pixels {
            data.extend_from_slice(&[*a, *r, *g, *b]);
        }
        vec![ksni::Icon {
            width: size as i32,
            height: size as i32,
            data,
        }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::ShowHide);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Show or hide Fastpotify".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::ShowHide)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: if self.playing {
                    "Pause".into()
                } else {
                    "Play".into()
                },
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::PlayPause)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Next".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Next)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Previous".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Previous)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub struct TrayService {
    handle: ksni::blocking::Handle<FastTray>,
    commands: Receiver<TrayCommand>,
    playing: bool,
}

impl TrayService {
    /// Registers the tray item. `None` when no status-notifier host exists.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        let tray = FastTray {
            commands: sender,
            wake: Arc::new(wake),
            playing: false,
        };
        match tray.spawn() {
            Ok(handle) => Some(Self {
                handle,
                commands,
                playing: false,
            }),
            Err(error) => {
                log::info!("no system tray available: {error}");
                None
            }
        }
    }

    pub fn drain_commands(&self) -> Vec<TrayCommand> {
        self.commands.try_iter().collect()
    }

    /// Keeps the menu's Play/Pause label matching reality.
    pub fn set_playing(&mut self, playing: bool) {
        if self.playing != playing {
            self.playing = playing;
            self.handle.update(|tray| tray.playing = playing);
        }
    }

    /// Nothing to do: the item lives on its own thread from the start.
    pub fn attach(&mut self) {}

    /// Nothing to do either; see `attach`.
    pub fn hidden(&mut self) {}
}

/// Waits while the app lives in the tray without a window. Linux has
/// nothing to pump here: the tray and MPRIS run on their own threads.
pub fn idle(duration: Duration) {
    std::thread::sleep(duration);
}

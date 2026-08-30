//! The tray item on Windows and macOS: the same menu as on Linux, so closing
//! the window leaves the music playing on every desktop.
//!
//! Windows runs the item on its own thread with a message loop, like the
//! Linux one. macOS allows status items on the main thread only, and only
//! while its event loop runs, so there the item is created with the first
//! window and, while no window exists, the headless loop in `main` pumps
//! the application's events itself. Its Dock icon remains available and a
//! Dock activation recreates the window.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::i18n::TrayLabels;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    ShowHide,
    PlayPause,
    Next,
    Previous,
    Quit,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

const SHOW: &str = "show";
const PLAY_PAUSE: &str = "play-pause";
const NEXT: &str = "next";
const PREVIOUS: &str = "previous";
const QUIT: &str = "quit";

fn command_for(id: &MenuId) -> Option<TrayCommand> {
    match id.0.as_str() {
        SHOW => Some(TrayCommand::ShowHide),
        PLAY_PAUSE => Some(TrayCommand::PlayPause),
        NEXT => Some(TrayCommand::Next),
        PREVIOUS => Some(TrayCommand::Previous),
        QUIT => Some(TrayCommand::Quit),
        _ => None,
    }
}

fn play_pause_label(labels: &TrayLabels, playing: bool) -> String {
    if playing {
        labels.pause.clone()
    } else {
        labels.play.clone()
    }
}

/// The item, and the menu entries whose labels follow playback or language.
struct Item {
    _icon: TrayIcon,
    labels: TrayLabels,
    playing: bool,
    show_hide: MenuItem,
    play_pause: MenuItem,
    next: MenuItem,
    previous: MenuItem,
    quit: MenuItem,
}

impl Item {
    fn apply_labels(&self, labels: &TrayLabels, playing: bool) {
        self.show_hide.set_text(labels.show_hide.clone());
        self.play_pause.set_text(play_pause_label(labels, playing));
        self.next.set_text(labels.next.clone());
        self.previous.set_text(labels.previous.clone());
        self.quit.set_text(labels.quit.clone());
        self._icon.set_tooltip(Some(labels.tooltip.clone()));
    }
}

/// Builds the item on the current thread and routes its events to `sender`.
fn build(
    sender: Sender<TrayCommand>,
    wake: Wake,
    labels: TrayLabels,
    playing: bool,
) -> Result<Item, Box<dyn std::error::Error>> {
    let size = 32u32;
    #[cfg(not(target_os = "macos"))]
    let icon = Icon::from_rgba(crate::util::app_icon_rgba(size as usize), size, size)?;
    // macOS draws template images itself, black or white to match the menu
    // bar, so the item looks native in either theme.
    #[cfg(target_os = "macos")]
    let icon = Icon::from_rgba(crate::util::tray_template_rgba(size as usize), size, size)?;
    let menu = Menu::new();
    let show_hide = MenuItem::with_id(SHOW, labels.show_hide.clone(), true, None);
    let play_pause = MenuItem::with_id(PLAY_PAUSE, play_pause_label(&labels, playing), true, None);
    let next = MenuItem::with_id(NEXT, labels.next.clone(), true, None);
    let previous = MenuItem::with_id(PREVIOUS, labels.previous.clone(), true, None);
    let quit = MenuItem::with_id(QUIT, labels.quit.clone(), true, None);
    menu.append_items(&[
        &show_hide,
        &PredefinedMenuItem::separator(),
        &play_pause,
        &next,
        &previous,
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let builder = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip(labels.tooltip.clone())
        .with_menu(Box::new(menu));
    // A plain click shows or hides the window on every platform; the menu
    // stays on right click.
    #[cfg(target_os = "macos")]
    let builder = builder
        .with_icon_as_template(true)
        .with_menu_on_left_click(false);
    let icon = builder.build()?;

    let menu_sender = sender.clone();
    let menu_wake = Arc::clone(&wake);
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(command) = command_for(&event.id)
            && menu_sender.send(command).is_ok()
        {
            menu_wake();
        }
    }));
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event: tray_icon::TrayIconEvent| {
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
            && sender.send(TrayCommand::ShowHide).is_ok()
        {
            wake();
        }
    }));

    Ok(Item {
        _icon: icon,
        labels,
        playing,
        show_hide,
        play_pause,
        next,
        previous,
        quit,
    })
}

#[cfg(windows)]
mod host {
    use super::*;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP,
    };

    /// Runs the item on its own thread. Answers with the thread's id once
    /// the item exists, or with why it could not be made.
    pub fn start(
        sender: Sender<TrayCommand>,
        wake: Wake,
        playing: Receiver<bool>,
        labels: Receiver<TrayLabels>,
    ) -> Result<u32, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("fastpotify-tray".to_owned())
            .spawn(move || {
                let initial_labels = labels
                    .recv()
                    .unwrap_or_else(|_| TrayLabels::from_catalog(crate::i18n::Catalog::default()));
                let mut item = match build(sender, wake, initial_labels, false) {
                    Ok(item) => item,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
                let mut message: MSG = unsafe { std::mem::zeroed() };
                while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
                    if message.message == WM_APP {
                        while let Ok(playing) = playing.try_recv() {
                            item.playing = playing;
                            item.play_pause
                                .set_text(play_pause_label(&item.labels, playing));
                        }
                        while let Ok(labels) = labels.try_recv() {
                            item.labels = labels;
                            item.apply_labels(&item.labels, item.playing);
                        }
                        continue;
                    }
                    unsafe {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            });
        if let Err(error) = spawned {
            return Err(error.to_string());
        }
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "the tray thread did not answer".to_string())?
    }

    /// Wakes the thread's message loop to read what was sent to it.
    pub fn poke(thread_id: u32) {
        unsafe {
            PostThreadMessageW(thread_id, WM_APP, 0, 0);
        }
    }
}

#[cfg(windows)]
pub struct TrayService {
    commands: Receiver<TrayCommand>,
    playing: bool,
    playing_tx: Sender<bool>,
    labels_tx: Sender<TrayLabels>,
    thread_id: u32,
}

#[cfg(windows)]
impl TrayService {
    /// Registers the tray item. `None` when it cannot be made.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static, labels: TrayLabels) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        let (playing_tx, playing_rx) = std::sync::mpsc::channel();
        let (labels_tx, labels_rx) = std::sync::mpsc::channel();
        let _ = labels_tx.send(labels);
        match host::start(sender, Arc::new(wake), playing_rx, labels_rx) {
            Ok(thread_id) => Some(Self {
                commands,
                playing: false,
                playing_tx,
                labels_tx,
                thread_id,
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
            if self.playing_tx.send(playing).is_ok() {
                host::poke(self.thread_id);
            }
        }
    }

    pub fn set_labels(&mut self, labels: TrayLabels) {
        if self.labels_tx.send(labels).is_ok() {
            host::poke(self.thread_id);
        }
    }

    /// Nothing to do: the item lives on its own thread from the start.
    pub fn attach(&mut self) {}

    /// Nothing to do either; see `attach`.
    pub fn hidden(&mut self) {}
}

/// Waits while the app lives in the tray without a window. Windows has
/// nothing to pump here: the item runs on its own thread.
#[cfg(windows)]
pub fn idle(duration: Duration) {
    std::thread::sleep(duration);
}

#[cfg(target_os = "macos")]
mod host {
    use std::cell::RefCell;
    use std::ffi::CString;

    use objc2::runtime::{AnyClass, AnyObject, Bool, MethodImplementation, Sel};
    use objc2::{Encode, MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};

    use super::*;

    thread_local! {
        /// The status item, which only the main thread may touch.
        pub static ITEM: RefCell<Option<Item>> = const { RefCell::new(None) };
        pub(super) static REOPEN: RefCell<Option<Sender<TrayCommand>>> = const { RefCell::new(None) };
    }

    pub(super) fn request_reopen(has_visible_windows: bool) -> Bool {
        if !has_visible_windows {
            REOPEN.with(|slot| {
                if let Some(sender) = slot.borrow().as_ref() {
                    let _ = sender.send(TrayCommand::Show);
                }
            });
        }
        Bool::YES
    }

    extern "C-unwind" fn application_should_handle_reopen(
        _delegate: *mut AnyObject,
        _selector: Sel,
        _application: *mut NSApplication,
        has_visible_windows: Bool,
    ) -> Bool {
        request_reopen(has_visible_windows.as_bool())
    }

    fn install_reopen_handler(app: &NSApplication) {
        let Some(delegate) = app.delegate() else {
            log::warn!("the macOS application delegate is unavailable");
            return;
        };
        let delegate: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
        let class = delegate.class();
        let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
        if class.responds_to(selector) {
            return;
        }
        let implementation: extern "C-unwind" fn(
            *mut AnyObject,
            Sel,
            *mut NSApplication,
            Bool,
        ) -> Bool = application_should_handle_reopen;
        let types = CString::new(format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING))
            .expect("valid Objective-C type encoding");
        let installed = unsafe {
            objc2::ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                selector,
                implementation.__imp(),
                types.as_ptr(),
            )
        };
        if !installed.as_bool() {
            log::warn!("the macOS Dock reopen handler could not be installed");
        }
    }

    /// Creates the item, once, on the main thread.
    pub fn create(sender: Sender<TrayCommand>, wake: Wake, playing: bool, labels: TrayLabels) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("the status item can only be made on the main thread");
            return;
        };
        REOPEN.with(|slot| *slot.borrow_mut() = Some(sender.clone()));
        install_reopen_handler(&NSApplication::sharedApplication(mtm));
        match build(sender, wake, labels.clone(), playing) {
            Ok(item) => {
                item.apply_labels(&labels, playing);
                ITEM.with(|slot| *slot.borrow_mut() = Some(item));
            }
            Err(error) => log::info!("no status item: {error}"),
        }
    }

    pub fn exists() -> bool {
        ITEM.with(|slot| slot.borrow().is_some())
    }

    pub fn set_playing(playing: bool) {
        ITEM.with(|slot| {
            if let Some(item) = slot.borrow_mut().as_mut() {
                item.playing = playing;
                item.play_pause
                    .set_text(play_pause_label(&item.labels, playing));
            }
        });
    }

    pub fn set_labels(labels: TrayLabels) {
        ITEM.with(|slot| {
            if let Some(item) = slot.borrow_mut().as_mut() {
                item.labels = labels.clone();
                item.apply_labels(&labels, item.playing);
            }
        });
    }

    pub fn activate() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }

    /// Runs the application's event loop for `duration`, so the status
    /// item and the media controls keep answering without a window.
    pub fn pump(duration: Duration) {
        let Some(mtm) = MainThreadMarker::new() else {
            std::thread::sleep(duration);
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let deadline = NSDate::dateWithTimeIntervalSinceNow(duration.as_secs_f64());
        // Safety: reading an extern static AppKit defines and never changes.
        let mode = unsafe { NSDefaultRunLoopMode };
        loop {
            let event = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&deadline),
                mode,
                true,
            );
            match event {
                Some(event) => app.sendEvent(&event),
                None => break,
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub struct TrayService {
    commands: Receiver<TrayCommand>,
    playing: bool,
    labels: TrayLabels,
    /// What the item needs, until the first window lets it be made.
    pending: Option<(Sender<TrayCommand>, Wake)>,
}

#[cfg(target_os = "macos")]
impl TrayService {
    /// Prepares the item. It is made with the first window, when AppKit's
    /// event loop is running, which it must be.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static, labels: TrayLabels) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        Some(Self {
            commands,
            playing: false,
            labels,
            pending: Some((sender, Arc::new(wake))),
        })
    }

    pub fn drain_commands(&self) -> Vec<TrayCommand> {
        self.commands.try_iter().collect()
    }

    /// Keeps the menu's Play/Pause label matching reality.
    pub fn set_playing(&mut self, playing: bool) {
        if self.playing != playing {
            self.playing = playing;
            host::set_playing(playing);
        }
    }

    pub fn set_labels(&mut self, labels: TrayLabels) {
        self.labels = labels.clone();
        host::set_labels(labels);
    }

    /// A window exists: make the item if this is the first one and bring the
    /// application forward.
    pub fn attach(&mut self) {
        if let Some((sender, wake)) = self.pending.take() {
            host::create(sender, wake, self.playing, self.labels.clone());
        }
        if host::exists() {
            host::activate();
        }
    }

    /// No window: the status item and Dock icon both remain available.
    pub fn hidden(&mut self) {}
}

/// Waits while the app lives in the tray without a window, keeping AppKit
/// served meanwhile.
#[cfg(target_os = "macos")]
pub fn idle(duration: Duration) {
    host::pump(duration);
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn dock_reopen_requests_a_window_only_when_none_is_visible() {
        let (sender, commands) = std::sync::mpsc::channel();
        host::REOPEN.with(|slot| *slot.borrow_mut() = Some(sender));

        assert!(host::request_reopen(true).as_bool());
        assert!(commands.try_recv().is_err());

        assert!(host::request_reopen(false).as_bool());
        assert_eq!(commands.try_recv(), Ok(TrayCommand::Show));
        host::REOPEN.with(|slot| *slot.borrow_mut() = None);
    }
}

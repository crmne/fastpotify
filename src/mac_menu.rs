//! Native macOS application menu bar (File, Edit, View, Playback, Window, Help).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    PlayPause,
    Next,
    Previous,
    SeekForward,
    SeekBackward,
    ToggleShuffle,
    CycleRepeat,
    VolumeUp,
    VolumeDown,
    ToggleMute,
    Home,
    Search,
    LikedSongs,
    Sidebar,
    Queue,
    Settings,
    Shortcuts,
    Back,
    Forward,
    OpenRepo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[cfg(not(target_os = "macos"))]
pub fn init(_catalog: crate::i18n::Catalog) {}

#[cfg(not(target_os = "macos"))]
pub fn refresh(_catalog: crate::i18n::Catalog) {}

#[cfg(not(target_os = "macos"))]
pub fn set_waker(_wake: impl Fn() + Send + Sync + 'static) {}

#[cfg(not(target_os = "macos"))]
pub fn drain_commands() -> Vec<MenuCommand> {
    Vec::new()
}

#[cfg(target_os = "macos")]
pub use mac_impl::*;

#[cfg(target_os = "macos")]
mod mac_impl {
    use objc2::rc::Retained;
    use objc2::runtime::Sel;
    use objc2::{MainThreadOnly, define_class, sel};
    use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
    use objc2_foundation::{MainThreadMarker, NSObject, NSString, ns_string};
    use std::sync::Mutex;

    use super::MenuCommand;

    static COMMANDS: Mutex<Vec<MenuCommand>> = Mutex::new(Vec::new());
    static WAKER: Mutex<Option<Box<dyn Fn() + Send + Sync>>> = Mutex::new(None);
    static LABELED_ITEMS: Mutex<Vec<(Retained<NSMenuItem>, &'static str)>> = Mutex::new(Vec::new());

    fn label(catalog: crate::i18n::Catalog, key: &'static str) -> Retained<NSString> {
        NSString::from_str(catalog.get(key))
    }

    fn remember(item: &Retained<NSMenuItem>, key: &'static str) {
        if let Ok(mut list) = LABELED_ITEMS.lock() {
            list.push((item.retain(), key));
        }
    }

    pub fn refresh(catalog: crate::i18n::Catalog) {
        if let Ok(list) = LABELED_ITEMS.lock() {
            for (item, key) in list.iter() {
                item.setTitle(&label(catalog, key));
            }
        }
    }

    pub fn set_waker(wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut w) = WAKER.lock() {
            *w = Some(Box::new(wake));
        }
    }

    fn push_command(cmd: MenuCommand) {
        if let Ok(mut list) = COMMANDS.lock() {
            list.push(cmd);
        }
        if let Ok(w) = WAKER.lock()
            && let Some(wake) = w.as_ref()
        {
            wake();
        }
    }

    pub fn drain_commands() -> Vec<MenuCommand> {
        if let Ok(mut list) = COMMANDS.lock() {
            std::mem::take(&mut *list)
        } else {
            Vec::new()
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "FastpotifyMenuHandler"]
        pub struct FastpotifyMenuHandler;

        impl FastpotifyMenuHandler {
            #[unsafe(method(openSettings:))]
            fn open_settings(&self, _sender: &NSObject) {
                push_command(MenuCommand::Settings);
            }

            #[unsafe(method(playPause:))]
            fn play_pause(&self, _sender: &NSObject) {
                push_command(MenuCommand::PlayPause);
            }

            #[unsafe(method(nextTrack:))]
            fn next_track(&self, _sender: &NSObject) {
                push_command(MenuCommand::Next);
            }

            #[unsafe(method(previousTrack:))]
            fn previous_track(&self, _sender: &NSObject) {
                push_command(MenuCommand::Previous);
            }

            #[unsafe(method(seekForward:))]
            fn seek_forward(&self, _sender: &NSObject) {
                push_command(MenuCommand::SeekForward);
            }

            #[unsafe(method(seekBackward:))]
            fn seek_backward(&self, _sender: &NSObject) {
                push_command(MenuCommand::SeekBackward);
            }

            #[unsafe(method(toggleShuffle:))]
            fn toggle_shuffle(&self, _sender: &NSObject) {
                push_command(MenuCommand::ToggleShuffle);
            }

            #[unsafe(method(cycleRepeat:))]
            fn cycle_repeat(&self, _sender: &NSObject) {
                push_command(MenuCommand::CycleRepeat);
            }

            #[unsafe(method(volumeUp:))]
            fn volume_up(&self, _sender: &NSObject) {
                push_command(MenuCommand::VolumeUp);
            }

            #[unsafe(method(volumeDown:))]
            fn volume_down(&self, _sender: &NSObject) {
                push_command(MenuCommand::VolumeDown);
            }

            #[unsafe(method(toggleMute:))]
            fn toggle_mute(&self, _sender: &NSObject) {
                push_command(MenuCommand::ToggleMute);
            }

            #[unsafe(method(openHome:))]
            fn open_home(&self, _sender: &NSObject) {
                push_command(MenuCommand::Home);
            }

            #[unsafe(method(focusSearch:))]
            fn focus_search(&self, _sender: &NSObject) {
                push_command(MenuCommand::Search);
            }

            #[unsafe(method(openLikedSongs:))]
            fn open_liked_songs(&self, _sender: &NSObject) {
                push_command(MenuCommand::LikedSongs);
            }

            #[unsafe(method(toggleSidebar:))]
            fn toggle_sidebar(&self, _sender: &NSObject) {
                push_command(MenuCommand::Sidebar);
            }

            #[unsafe(method(toggleQueue:))]
            fn toggle_queue(&self, _sender: &NSObject) {
                push_command(MenuCommand::Queue);
            }

            #[unsafe(method(goBack:))]
            fn go_back(&self, _sender: &NSObject) {
                push_command(MenuCommand::Back);
            }

            #[unsafe(method(goForward:))]
            fn go_forward(&self, _sender: &NSObject) {
                push_command(MenuCommand::Forward);
            }

            #[unsafe(method(showShortcuts:))]
            fn show_shortcuts(&self, _sender: &NSObject) {
                push_command(MenuCommand::Shortcuts);
            }

            #[unsafe(method(openRepo:))]
            fn open_repo(&self, _sender: &NSObject) {
                push_command(MenuCommand::OpenRepo);
            }

            // The Edit items answer to this handler rather than to the
            // responder chain: winit's view implements none of the standard
            // editing selectors, so a menu item aimed there does nothing,
            // while its key equivalent still takes the chord away from the
            // window. Routed through egui, the same item and chord work.
            #[unsafe(method(editCut:))]
            fn edit_cut(&self, _sender: &NSObject) {
                push_command(MenuCommand::Cut);
            }

            #[unsafe(method(editCopy:))]
            fn edit_copy(&self, _sender: &NSObject) {
                push_command(MenuCommand::Copy);
            }

            #[unsafe(method(editPaste:))]
            fn edit_paste(&self, _sender: &NSObject) {
                push_command(MenuCommand::Paste);
            }

            #[unsafe(method(editSelectAll:))]
            fn edit_select_all(&self, _sender: &NSObject) {
                push_command(MenuCommand::SelectAll);
            }
        }
    );

    fn create_item(
        mtm: MainThreadMarker,
        title: &NSString,
        action: Option<Sel>,
        key: &NSString,
        masks: Option<NSEventModifierFlags>,
        target: Option<&NSObject>,
    ) -> Retained<NSMenuItem> {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), title, action, key)
        };
        if let Some(masks) = masks {
            item.setKeyEquivalentModifierMask(masks);
        }
        if let Some(target) = target {
            unsafe { item.setTarget(Some(target)) };
        }
        item
    }

    fn create_menu(
        mtm: MainThreadMarker,
        title: &NSString,
    ) -> (Retained<NSMenuItem>, Retained<NSMenu>) {
        let container_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), title, None, ns_string!(""))
        };
        let menu = NSMenu::initWithTitle(mtm.alloc(), title);
        menu.setAutoenablesItems(false);
        container_item.setSubmenu(Some(&menu));
        (container_item, menu)
    }

    pub fn init(catalog: crate::i18n::Catalog) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let Some(menubar) = app.mainMenu() else {
            return;
        };

        static INITIALIZED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if INITIALIZED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }

        let handler: Retained<FastpotifyMenuHandler> =
            unsafe { objc2::msg_send![mtm.alloc::<FastpotifyMenuHandler>(), init] };
        let target: &NSObject = &handler;

        // 1. Settings item in app menu (first menu)
        if let Some(app_menu_item) = menubar.itemAtIndex(0)
            && let Some(app_menu) = app_menu_item.submenu()
        {
            let settings_item = create_item(
                mtm,
                &label(catalog, "mac.menu.settings"),
                Some(sel!(openSettings:)),
                ns_string!(","),
                None,
                Some(target),
            );
            remember(&settings_item, "mac.menu.settings");
            let sep = NSMenuItem::separatorItem(mtm);
            app_menu.insertItem_atIndex(&settings_item, 1);
            app_menu.insertItem_atIndex(&sep, 2);
        }

        // 2. File menu
        let (file_item, file_menu) = create_menu(mtm, &label(catalog, "mac.menu.file"));
        remember(&file_item, "mac.menu.file");
        let close_window = create_item(
            mtm,
            &label(catalog, "mac.menu.close_window"),
            Some(sel!(performClose:)),
            ns_string!("w"),
            None,
            None,
        );
        remember(&close_window, "mac.menu.close_window");
        file_menu.addItem(&close_window);
        menubar.addItem(&file_item);

        // 3. Edit menu. No Undo and Redo: egui's text fields handle Cmd+Z
        // themselves, and a menu item holding that chord would take it
        // from them.
        let (edit_item, edit_menu) = create_menu(mtm, &label(catalog, "mac.menu.edit"));
        remember(&edit_item, "mac.menu.edit");
        for (key, action, shortcut) in [
            ("mac.menu.cut", sel!(editCut:), "x"),
            ("mac.menu.copy", sel!(editCopy:), "c"),
            ("mac.menu.paste", sel!(editPaste:), "v"),
            ("mac.menu.select_all", sel!(editSelectAll:), "a"),
        ] {
            let item = create_item(
                mtm,
                &label(catalog, key),
                Some(action),
                &NSString::from_str(shortcut),
                None,
                Some(target),
            );
            remember(&item, key);
            edit_menu.addItem(&item);
        }
        menubar.addItem(&edit_item);

        // 4. Playback menu
        let (playback_item, playback_menu) = create_menu(mtm, &label(catalog, "mac.menu.playback"));
        remember(&playback_item, "mac.menu.playback");
        for (key, action, shortcut, masks) in [
            ("mac.menu.play_pause", sel!(playPause:), "", None),
            (
                "mac.menu.next_track",
                sel!(nextTrack:),
                "\u{F703}",
                Some(NSEventModifierFlags::Command),
            ),
            (
                "mac.menu.previous_track",
                sel!(previousTrack:),
                "\u{F702}",
                Some(NSEventModifierFlags::Command),
            ),
        ] {
            let item = create_item(
                mtm,
                &label(catalog, key),
                Some(action),
                &NSString::from_str(shortcut),
                masks,
                Some(target),
            );
            remember(&item, key);
            playback_menu.addItem(&item);
        }
        playback_menu.addItem(&NSMenuItem::separatorItem(mtm));
        for (key, action) in [
            ("mac.menu.seek_forward", sel!(seekForward:)),
            ("mac.menu.seek_backward", sel!(seekBackward:)),
        ] {
            let item = create_item(
                mtm,
                &label(catalog, key),
                Some(action),
                ns_string!(""),
                None,
                Some(target),
            );
            remember(&item, key);
            playback_menu.addItem(&item);
        }
        playback_menu.addItem(&NSMenuItem::separatorItem(mtm));
        for (key, action) in [
            ("mac.menu.shuffle", sel!(toggleShuffle:)),
            ("mac.menu.repeat", sel!(cycleRepeat:)),
        ] {
            let item = create_item(
                mtm,
                &label(catalog, key),
                Some(action),
                ns_string!(""),
                None,
                Some(target),
            );
            remember(&item, key);
            playback_menu.addItem(&item);
        }
        playback_menu.addItem(&NSMenuItem::separatorItem(mtm));
        for (key, action, shortcut, masks) in [
            (
                "mac.menu.volume_up",
                sel!(volumeUp:),
                "\u{F700}",
                Some(NSEventModifierFlags::Command),
            ),
            (
                "mac.menu.volume_down",
                sel!(volumeDown:),
                "\u{F701}",
                Some(NSEventModifierFlags::Command),
            ),
            ("mac.menu.mute", sel!(toggleMute:), "", None),
        ] {
            let item = create_item(
                mtm,
                &label(catalog, key),
                Some(action),
                &NSString::from_str(shortcut),
                masks,
                Some(target),
            );
            remember(&item, key);
            playback_menu.addItem(&item);
        }
        menubar.addItem(&playback_item);

        // 5. View menu
        let (view_item, view_menu) = create_menu(mtm, &label(catalog, "mac.menu.view"));
        remember(&view_item, "mac.menu.view");
        for (key, action, shortcut, masks) in [
            (
                "mac.menu.back",
                sel!(goBack:),
                "[",
                Some(NSEventModifierFlags::Command),
            ),
            (
                "mac.menu.forward",
                sel!(goForward:),
                "]",
                Some(NSEventModifierFlags::Command),
            ),
            (
                "mac.menu.home",
                sel!(openHome:),
                "H",
                Some(NSEventModifierFlags::Command | NSEventModifierFlags::Shift),
            ),
            (
                "mac.menu.search",
                sel!(focusSearch:),
                "f",
                Some(NSEventModifierFlags::Command),
            ),
            (
                "mac.menu.liked",
                sel!(openLikedSongs:),
                "l",
                Some(NSEventModifierFlags::Command),
            ),
            (
                "mac.menu.toggle_sidebar",
                sel!(toggleSidebar:),
                "b",
                Some(NSEventModifierFlags::Command),
            ),
            (
                "mac.menu.queue",
                sel!(toggleQueue:),
                "u",
                Some(NSEventModifierFlags::Command),
            ),
        ] {
            if key == "mac.menu.home" {
                view_menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
            let item = create_item(
                mtm,
                &label(catalog, key),
                Some(action),
                &NSString::from_str(shortcut),
                masks,
                Some(target),
            );
            remember(&item, key);
            view_menu.addItem(&item);
        }
        view_menu.addItem(&NSMenuItem::separatorItem(mtm));
        let fullscreen = create_item(
            mtm,
            &label(catalog, "mac.menu.fullscreen"),
            Some(sel!(toggleFullScreen:)),
            ns_string!("f"),
            Some(NSEventModifierFlags::Control | NSEventModifierFlags::Command),
            None,
        );
        remember(&fullscreen, "mac.menu.fullscreen");
        view_menu.addItem(&fullscreen);
        menubar.addItem(&view_item);

        // 6. Window menu
        let (window_item, window_menu) = create_menu(mtm, &label(catalog, "mac.menu.window"));
        remember(&window_item, "mac.menu.window");
        for (key, action, shortcut, masks) in [
            (
                "mac.menu.minimize",
                sel!(performMiniaturize:),
                "m",
                Some(NSEventModifierFlags::Command),
            ),
            ("mac.menu.zoom", sel!(performZoom:), "", None),
            ("mac.menu.bring_all_front", sel!(arrangeInFront:), "", None),
        ] {
            if key == "mac.menu.bring_all_front" {
                window_menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
            let item = create_item(
                mtm,
                &label(catalog, key),
                Some(action),
                &NSString::from_str(shortcut),
                masks,
                None,
            );
            remember(&item, key);
            window_menu.addItem(&item);
        }
        menubar.addItem(&window_item);

        // 7. Help menu
        let (help_item, help_menu) = create_menu(mtm, &label(catalog, "mac.menu.help"));
        remember(&help_item, "mac.menu.help");
        for (key, action, shortcut, masks) in [
            (
                "mac.menu.shortcuts",
                sel!(showShortcuts:),
                "/",
                Some(NSEventModifierFlags::Command),
            ),
            ("mac.menu.github", sel!(openRepo:), "", None),
        ] {
            let item = create_item(
                mtm,
                &label(catalog, key),
                Some(action),
                &NSString::from_str(shortcut),
                masks,
                Some(target),
            );
            remember(&item, key);
            help_menu.addItem(&item);
        }
        menubar.addItem(&help_item);
        // for as long as the menu bar exists. It is a single process-wide
        // object, so leaking it is the whole lifetime story.
        std::mem::forget(handler);
    }
}

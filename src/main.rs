//! Desktop entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use fastpotify::{app, backend, paths, settings, single_instance, util};

use clap::Parser;

/// A fast, native Spotify client.
#[derive(Debug, Parser)]
#[command(name = "fastpotify", version, about)]
struct Cli {
    /// Spotify Connect device name for this session.
    #[arg(long)]
    device_name: Option<String>,

    /// Log more from librespot and the Web API client.
    #[arg(short, long)]
    verbose: bool,

    /// Start with sample data and no Spotify connection (for screenshots).
    #[cfg(feature = "demo")]
    #[arg(long)]
    demo: bool,

    /// Page to open in demo mode, e.g. `home`, `playlist:pl1`, `artist:art0`.
    #[cfg(feature = "demo")]
    #[arg(long)]
    demo_page: Option<String>,

    /// Extra demo surfaces: a comma-separated list of `queue`, `devices`,
    /// `shortcuts`, `create`, `light`.
    #[cfg(feature = "demo")]
    #[arg(long)]
    demo_show: Option<String>,

    /// Write a PNG of the demo window to this path and exit. Implies
    /// `--demo`. The shot is the window's own frame buffer, so it is however
    /// large the window is: full screen where that request is honoured, and
    /// the size of the tile under a tiling window manager, which decides for
    /// itself.
    #[cfg(feature = "demo")]
    #[arg(long, value_name = "PATH")]
    demo_shot: Option<std::path::PathBuf>,

    /// How long to let cover art download before the shot is taken.
    #[cfg(feature = "demo")]
    #[arg(long, value_name = "MS", default_value_t = 6000)]
    demo_shot_delay: u64,
}

fn main() -> eframe::Result<()> {
    let cli = Cli::parse();
    let default_filter = if cli.verbose {
        "info,librespot=info,fastpotify=debug"
    } else {
        "warn,fastpotify=info"
    };
    let dirs = paths::AppDirs::discover();
    let dirs_ready = dirs.ensure();
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter));
    // Launched from a desktop, stderr goes nowhere; keep the run's log where
    // a bug report can find it.
    match std::fs::File::create(dirs.log_file()) {
        Ok(file) => {
            logger.target(env_logger::Target::Pipe(Box::new(Tee(file))));
        }
        Err(error) => eprintln!("not keeping a log file: {error}"),
    }
    logger.init();
    if let Err(error) = dirs_ready {
        log::warn!("unable to create the application directories: {error}");
    }
    log_panics(dirs.panic_log());
    let mut settings = settings::Settings::load(&dirs.settings_file());
    if let Some(name) = cli.device_name {
        settings.device_name = name;
    }

    // The application (audio engine, Web API, MPRIS, tray) outlives any
    // window. Closing to the tray destroys the window and this loop creates
    // a new one when the tray or MPRIS asks for it. Plain window lifecycle,
    // portable across desktops.
    let waker = backend::Waker::default();

    // A second launch surfaces the instance already running instead of
    // starting a rival one. Held for the lifetime of the process.
    #[cfg(feature = "demo")]
    let demo = cli.demo || cli.demo_shot.is_some();
    #[cfg(feature = "demo")]
    let guarded = !demo;
    #[cfg(not(feature = "demo"))]
    let guarded = true;
    let instance = if guarded {
        match single_instance::acquire(&waker) {
            single_instance::Outcome::Only(guard) => Some(guard),
            single_instance::Outcome::Surfaced => {
                log::info!("Fastpotify is already running; asked it to show its window");
                return Ok(());
            }
        }
    } else {
        None
    };

    // A capture run is a throwaway process next to the real one: no tray
    // icon of its own, and no second MPRIS service to fight over media keys.
    #[allow(unused_mut)]
    let mut options = app::AppOptions::default();
    #[cfg(feature = "demo")]
    if cli.demo_shot.is_some() {
        options = app::AppOptions {
            mpris: false,
            tray: false,
        };
    }
    #[allow(unused_mut)]
    let mut app = app::App::new(&waker, dirs, settings, options);
    if let Some(guard) = &instance {
        app.set_show_requests(guard.show_requests());
    }
    #[cfg(feature = "demo")]
    if demo {
        fastpotify::demo::populate(&mut app);
        fastpotify::demo::apply_flags(&mut app, cli.demo_page.as_deref(), cli.demo_show.as_deref());
    }
    #[cfg(feature = "demo")]
    let shot = cli.demo_shot.clone().map(|path| Shot {
        path,
        due: std::time::Instant::now() + std::time::Duration::from_millis(cli.demo_shot_delay),
        asked: false,
    });
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(app)));

    loop {
        let creator_slot = std::sync::Arc::clone(&slot);
        let creator_waker = waker.clone();
        #[cfg(feature = "demo")]
        let creator_shot = shot.clone();
        #[cfg(feature = "demo")]
        let options = native_options(shot.is_some());
        #[cfg(not(feature = "demo"))]
        let options = native_options(false);
        eframe::run_native(
            "Fastpotify",
            options,
            Box::new(move |cc| {
                creator_waker.attach(&cc.egui_ctx);
                let mut app = creator_slot
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take()
                    .expect("application state present");
                // Built once per window, before the first frame; the handler
                // wakes the loop so a menu pick is not held until the next
                // repaint.
                #[cfg(target_os = "macos")]
                {
                    fastpotify::mac_menu::init();
                    let ctx = cc.egui_ctx.clone();
                    fastpotify::mac_menu::set_waker(move || ctx.request_repaint());
                }
                app.attach(&cc.egui_ctx);
                Ok(Box::new(Shell {
                    app: Some(app),
                    slot: std::sync::Arc::clone(&creator_slot),
                    #[cfg(feature = "demo")]
                    shot: creator_shot.clone(),
                }))
            }),
        )?;
        waker.detach();

        let hide = {
            let guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            let app = guard.as_ref().expect("application state present");
            !app.quit_requested && app.hide_intent
        };
        if !hide {
            break;
        }

        // Tray life: no window, but audio, MPRIS, the tray, and polling all
        // keep running until Show or Quit.
        let headless = egui::Context::default();
        slot.lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
            .expect("application state present")
            .window_gone();
        loop {
            {
                let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
                let app = guard.as_mut().expect("application state present");
                app.background_frame(&headless);
                if app.quit_requested || app.wants_show {
                    break;
                }
            }
            fastpotify::tray::idle(std::time::Duration::from_millis(150));
        }
        let quit = {
            let guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .as_ref()
                .expect("application state present")
                .quit_requested
        };
        if quit {
            break;
        }
    }

    if let Some(mut app) = slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
        app.shutdown();
    }
    Ok(())
}

/// Every log line goes to stderr and to the log file.
struct Tee(std::fs::File);

impl std::io::Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        self.0.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        self.0.flush()
    }
}

/// Records every panic in `path` before the process dies of it.
///
/// Release builds abort on panic and, on Windows, have no console, so a
/// crash would otherwise leave nothing behind to put in a bug report.
fn log_panics(path: std::path::PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        let thread = std::thread::current();
        let entry = format!(
            "{} fastpotify {} on thread {:?}: {info}\n",
            jiff::Timestamp::now(),
            env!("CARGO_PKG_VERSION"),
            thread.name().unwrap_or("unnamed"),
        );
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path);
        if let Ok(mut file) = file {
            use std::io::Write;
            let _ = file.write_all(entry.as_bytes());
        }
    }));
}

fn native_options(fullscreen: bool) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fastpotify")
            .with_app_id("fastpotify")
            .with_inner_size([1240.0, 800.0])
            .with_min_inner_size([760.0, 520.0])
            .with_fullscreen(fullscreen)
            // macOS takes the dock icon from the bundle's .icns, which is the
            // 1024px drawing with the platform's rounding. Setting a window
            // icon there replaces it with this flat 128px square.
            .with_icon(if cfg!(target_os = "macos") {
                egui::IconData::default()
            } else {
                app_icon()
            }),
        // A Wayland compositor stops sending frame callbacks to a hidden
        // window; waiting for vsync there would block the event loop.
        // Repaints are event-driven, so nothing spins.
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The eframe adapter around the long-lived [`app::App`]: delegates frames
/// and, when the window goes away, hands the state back for the next window.
struct Shell {
    app: Option<app::App>,
    slot: std::sync::Arc<std::sync::Mutex<Option<app::App>>>,
    /// A pending `--demo-shot` capture, if this is a screenshot run.
    #[cfg(feature = "demo")]
    shot: Option<Shot>,
}

/// A screenshot the window still owes us.
///
/// Cover art arrives over the network, so the capture waits for `due` before
/// asking egui for the frame buffer. The image comes back as an input event
/// on a later frame, which is where it gets written and the window closed.
#[cfg(feature = "demo")]
#[derive(Clone)]
struct Shot {
    path: std::path::PathBuf,
    due: std::time::Instant,
    asked: bool,
}

#[cfg(feature = "demo")]
impl Shell {
    fn drive_shot(&mut self, ctx: &egui::Context) {
        let Some(shot) = self.shot.as_mut() else {
            return;
        };

        // Nothing here is driven by user input, so the frames have to be
        // asked for: art still has to load and the request has to be issued.
        ctx.request_repaint();

        if !shot.asked && std::time::Instant::now() >= shot.due {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            shot.asked = true;
        }

        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else {
            return;
        };

        let [width, height] = [image.size[0] as u32, image.size[1] as u32];
        let pixels: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_srgba_unmultiplied())
            .collect();
        match image::RgbaImage::from_raw(width, height, pixels) {
            Some(buffer) => match buffer.save(&shot.path) {
                Ok(()) => log::info!("wrote {}x{} to {}", width, height, shot.path.display()),
                Err(error) => log::error!("could not write {}: {error}", shot.path.display()),
            },
            None => log::error!("the frame buffer did not match {width}x{height}"),
        }
        self.shot = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(app) = self.app.as_mut() {
            #[cfg(target_os = "macos")]
            for command in fastpotify::mac_menu::drain_commands() {
                use fastpotify::mac_menu::MenuCommand;
                use fastpotify::model::{Action, Dialog, Page};
                let action = match command {
                    MenuCommand::PlayPause => Action::TogglePlay,
                    MenuCommand::Next => Action::Next,
                    MenuCommand::Previous => Action::Previous,
                    MenuCommand::SeekForward => Action::SeekBy(10_000),
                    MenuCommand::SeekBackward => Action::SeekBy(-10_000),
                    MenuCommand::ToggleShuffle => Action::ToggleShuffle,
                    MenuCommand::CycleRepeat => Action::CycleRepeat,
                    MenuCommand::VolumeUp => Action::VolumeBy(5),
                    MenuCommand::VolumeDown => Action::VolumeBy(-5),
                    MenuCommand::ToggleMute => Action::ToggleMute,
                    MenuCommand::Home => Action::Open(Page::Home),
                    MenuCommand::Search => Action::FocusSearch,
                    MenuCommand::LikedSongs => Action::Open(Page::LikedSongs),
                    MenuCommand::Queue => Action::ToggleQueuePanel,
                    MenuCommand::Settings => Action::Open(Page::Settings),
                    MenuCommand::Shortcuts => Action::ShowDialog(Dialog::Shortcuts),
                    MenuCommand::Back => Action::Back,
                    MenuCommand::Forward => Action::Forward,
                    MenuCommand::OpenRepo => {
                        ctx.open_url(egui::OpenUrl::new_tab(
                            "https://github.com/crmne/fastpotify",
                        ));
                        continue;
                    }
                };
                app.actions.push(action);
            }
            app.background_frame(ctx);
        }
        #[cfg(feature = "demo")]
        self.drive_shot(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(app) = self.app.as_mut() {
            app.frame_ui(ui);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(app) = self.app.as_mut() {
            app.save_state();
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        *self.slot.lock().unwrap_or_else(|p| p.into_inner()) = self.app.take();
    }
}

/// The window icon, from the shared runtime drawing.
fn app_icon() -> egui::IconData {
    const SIZE: usize = 128;
    egui::IconData {
        rgba: util::app_icon_rgba(SIZE),
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

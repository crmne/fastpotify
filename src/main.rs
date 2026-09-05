//! Desktop entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use fastpotify::{app, backend, paths, settings, single_instance, util};

use clap::Parser;

/// A fast, native Spotify client.
#[derive(Debug, Parser)]
#[command(name = "fastpotify", version, about)]
struct Cli {
    /// A command for the running instance; without one, the app starts.
    #[command(subcommand)]
    control: Option<Control>,

    /// A Spotify link to open: spotify:track:…, or an open.spotify.com
    /// address. The running Fastpotify opens it when there is one, which
    /// is how the desktop hands links over.
    #[arg(value_name = "LINK")]
    link: Option<String>,

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
    /// `shortcuts`, `create`, `lyrics`, `lyrics-fullscreen`, `light`, `focus`.
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

/// Remote control of the running instance, for Raycast scripts, launchers,
/// and hands on keyboards.
#[derive(Debug, clap::Subcommand)]
enum Control {
    /// Toggle play/pause
    PlayPause,
    /// Start playback if paused
    Play,
    /// Pause playback if playing
    Pause,
    /// Skip to the next track
    Next,
    /// Return to the previous track
    Previous,
    /// Seek by this many seconds; negative seeks backwards
    Seek {
        #[arg(allow_negative_numbers = true)]
        seconds: i64,
    },
    /// Seek to a position, in seconds from the start
    SeekTo { seconds: u32 },
    /// Set the volume to a percentage
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Raise the volume
    VolumeUp {
        #[arg(default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=100))]
        percent: u8,
    },
    /// Lower the volume
    VolumeDown {
        #[arg(default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=100))]
        percent: u8,
    },
    /// Toggle mute
    Mute,
    /// Toggle shuffle, or set it outright
    Shuffle { state: Option<OnOff> },
    /// Cycle the repeat mode, or set it outright
    Repeat { mode: Option<Repeat> },
    /// Save the playing track to your library, or take it back out
    Like,
    /// Play a Spotify URI: a track, album, playlist, artist, or show
    PlayUri { uri: String },
    /// List the Spotify Connect devices
    Devices {
        /// Print the JSON the running instance sent instead.
        #[arg(long)]
        raw: bool,
    },
    /// Move playback to a device, by the id `devices` prints
    Transfer { device_id: String },
    /// Print the playing track
    NowPlaying {
        /// Print the fields tab-separated instead: state, title, artists,
        /// album, position_ms, duration_ms, volume, shuffle, repeat,
        /// art_url, saved, device.
        #[arg(long)]
        raw: bool,
    },
    /// Bring the window of the running instance forward
    Show,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum Repeat {
    /// Play through and stop
    Off,
    /// Repeat the album, playlist, or queue
    Context,
    /// Repeat this track
    Track,
}

/// Sends one control verb to the running instance. Speaks over the
/// single-instance loopback socket, which Linux does not have.
#[cfg(not(target_os = "linux"))]
fn run_control(control: Control) -> i32 {
    let raw = matches!(
        control,
        Control::NowPlaying { raw: true } | Control::Devices { raw: true }
    );
    let verb = match control {
        Control::PlayPause => "playpause".to_owned(),
        Control::Play => "play".to_owned(),
        Control::Pause => "pause".to_owned(),
        Control::Next => "next".to_owned(),
        Control::Previous => "previous".to_owned(),
        Control::Seek { seconds } => format!("seek-by {}", seconds.saturating_mul(1000)),
        Control::SeekTo { seconds } => format!("seek-to {}", u64::from(seconds) * 1000),
        Control::Volume { percent } => format!("volume-set {percent}"),
        Control::VolumeUp { percent } => format!("volume-by {percent}"),
        Control::VolumeDown { percent } => format!("volume-by -{percent}"),
        Control::Mute => "mute".to_owned(),
        Control::Shuffle { state: None } => "shuffle".to_owned(),
        Control::Shuffle { state: Some(state) } => {
            let state = match state {
                OnOff::On => "on",
                OnOff::Off => "off",
            };
            format!("shuffle-set {state}")
        }
        Control::Repeat { mode: None } => "repeat".to_owned(),
        Control::Repeat { mode: Some(mode) } => {
            let mode = match mode {
                Repeat::Off => "off",
                Repeat::Context => "context",
                Repeat::Track => "track",
            };
            format!("repeat-set {mode}")
        }
        Control::Like => "save-toggle".to_owned(),
        Control::PlayUri { uri } => format!("play-uri {uri}"),
        Control::Devices { .. } => "devices".to_owned(),
        Control::Transfer { device_id } => format!("transfer {device_id}"),
        Control::NowPlaying { .. } => "nowplaying".to_owned(),
        Control::Show => "show".to_owned(),
    };
    match single_instance::send(&verb) {
        Ok(single_instance::Reply::Ok) => 0,
        Ok(single_instance::Reply::NowPlaying(snapshot)) => {
            if raw {
                println!("{snapshot}");
            } else {
                println!("{}", format_now_playing(&snapshot));
            }
            0
        }
        Ok(single_instance::Reply::Devices(snapshot)) => {
            if raw {
                println!("{snapshot}");
            } else {
                print!("{}", format_devices(&snapshot));
            }
            0
        }
        Err(error) => {
            eprintln!("Fastpotify is not running or does not support remote control: {error}");
            1
        }
    }
}

#[cfg(target_os = "linux")]
fn run_control(_control: Control) -> i32 {
    eprintln!(
        "On Linux the running instance speaks MPRIS instead; use e.g. \
         `playerctl --player=fastpotify play-pause`."
    );
    2
}

/// The `nowplaying` snapshot as one human-readable line.
#[cfg(not(target_os = "linux"))]
fn format_now_playing(snapshot: &str) -> String {
    let mut fields = snapshot.split('\t');
    let state = fields.next().unwrap_or_default();
    let title = fields.next().unwrap_or_default();
    let artists = fields.next().unwrap_or_default();
    let _album = fields.next();
    let position_ms: u32 = fields.next().and_then(|ms| ms.parse().ok()).unwrap_or(0);
    let duration_ms: u32 = fields.next().and_then(|ms| ms.parse().ok()).unwrap_or(0);
    let clock = |ms: u32| format!("{}:{:02}", ms / 60_000, ms % 60_000 / 1000);
    match state {
        "playing" | "paused" => {
            let mark = if state == "playing" { "▶" } else { "⏸" };
            format!(
                "{mark} {title} — {artists}  [{} / {}]",
                clock(position_ms),
                clock(duration_ms)
            )
        }
        _ => "Nothing playing".to_owned(),
    }
}

/// The `devices` snapshot as one line per device, the active one marked.
/// The id comes first because `fastpotify transfer` is what it is for.
#[cfg(not(target_os = "linux"))]
fn format_devices(snapshot: &str) -> String {
    let Ok(devices) = serde_json::from_str::<Vec<serde_json::Value>>(snapshot) else {
        return String::new();
    };
    let field =
        |device: &serde_json::Value, key: &str| device[key].as_str().unwrap_or_default().to_owned();
    devices
        .iter()
        .map(|device| {
            format!(
                "{}{}\t{}\t{}\n",
                if device["active"].as_bool().unwrap_or(false) {
                    "* "
                } else {
                    "  "
                },
                field(device, "id"),
                field(device, "name"),
                field(device, "kind"),
            )
        })
        .collect()
}

fn main() -> eframe::Result<()> {
    // A MilkDrop child launch is a bare visualiser window, not the app: it has
    // its own event loop and OpenGL context, reads the sound from a shared
    // buffer, and never touches the app's state. Handle it before anything
    // else, including the argument parser, which does not know its flags.
    #[cfg(feature = "milkdrop")]
    if let Some(args) = fastpotify::milkdrop::child::Args::parse() {
        std::process::exit(fastpotify::milkdrop::child::run(args));
    }

    let cli = Cli::parse();
    // A control launch is a client, not a second app: talk to the running
    // instance and exit before touching the log file it is writing to.
    if let Some(control) = cli.control {
        std::process::exit(run_control(control));
    }
    // A link is read before anything starts: one that is not a Spotify
    // link ends the launch here rather than reaching the running instance.
    let link = cli
        .link
        .as_deref()
        .map(|text| match fastpotify::link::parse(text) {
            Some(uri) => uri,
            None => {
                eprintln!("not a Spotify link: {text}");
                std::process::exit(2);
            }
        });
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
        match single_instance::acquire(&waker, link.as_deref()) {
            single_instance::Outcome::Only(guard) => Some(guard),
            single_instance::Outcome::Surfaced => {
                log::info!("Fastpotify is already running; asked it to show its window");
                return Ok(());
            }
        }
    } else {
        None
    };
    // macOS hands links to an app as Apple Events, the one it was launched
    // for included, so the handler is in place before the event loop that
    // delivers them starts.
    #[cfg(target_os = "macos")]
    if let Some(guard) = &instance {
        fastpotify::mac_links::install(guard.commands(), waker.clone());
    }

    // A capture run is a throwaway process next to the real one: no tray
    // icon of its own, and no second MPRIS service to fight over media keys.
    #[allow(unused_mut)]
    let mut options = app::AppOptions::default();
    #[cfg(feature = "demo")]
    if cli.demo_shot.is_some() {
        options = app::AppOptions {
            media_controls: false,
            tray: false,
        };
    }
    #[allow(unused_mut)]
    let mut app = app::App::new(&waker, dirs, settings, options);
    if let Some(guard) = &instance {
        app.set_remote_control(guard);
    }
    if let Some(uri) = link {
        app.open_link(uri);
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
        let mini = {
            let guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            MiniWindow::wanted(guard.as_ref().expect("application state present"))
        };
        #[cfg(feature = "demo")]
        let options = native_options(shot.is_some() && mini.is_none(), mini);
        #[cfg(not(feature = "demo"))]
        let options = native_options(false, mini);
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

        let (switch, hide) = {
            let guard = slot.lock().unwrap_or_else(|p| p.into_inner());
            let app = guard.as_ref().expect("application state present");
            (
                !app.quit_requested && app.switch_intent,
                !app.quit_requested && app.hide_intent,
            )
        };
        if switch {
            // Straight back round: the other kind of window opens.
            continue;
        }
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

/// The Winamp mini player's window, when that is the window to open.
struct MiniWindow {
    /// A first size; the window corrects it once it knows the display.
    size: egui::Vec2,
    position: Option<[f32; 2]>,
    on_top: bool,
}

impl MiniWindow {
    fn wanted(app: &app::App) -> Option<Self> {
        app.settings.winamp_window.then(|| Self {
            size: fastpotify::ui::winamp::initial_size(&app.settings),
            position: app.winamp.restore_pos,
            on_top: app.settings.winamp_on_top,
        })
    }
}

const fn main_window_decorated(on_windows: bool) -> bool {
    !on_windows
}

fn native_options(fullscreen: bool, mini: Option<MiniWindow>) -> eframe::NativeOptions {
    let icon = if cfg!(target_os = "macos") {
        // macOS takes the dock icon from the bundle's .icns, which is the
        // 1024px drawing with the platform's rounding. Setting a window
        // icon there replaces it with this flat 128px square.
        egui::IconData::default()
    } else {
        app_icon()
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title("Fastpotify")
        .with_app_id("fastpotify")
        .with_icon(icon);
    let viewport = match mini {
        Some(mini) => {
            let level = app::on_top_window_level(mini.on_top);
            // See-through, for skins that are not rectangles; the skin
            // paints every pixel that is the window. MilkDrop runs in its own
            // process, so nothing else shares this window's surface.
            let viewport = viewport
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_maximize_button(false)
                .with_inner_size(mini.size)
                .with_min_inner_size(mini.size)
                .with_max_inner_size(mini.size)
                .with_window_level(level);
            match mini.position {
                Some([x, y]) => viewport.with_position([x, y]),
                None => viewport,
            }
        }
        None => viewport
            // macOS: no title bar strip above the app. The content runs to
            // the top edge and the traffic lights float over it, the way
            // every other music player on the platform looks; the interface
            // leaves room for them with `theme::titlebar_inset`.
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            // Windows has no equivalent to macOS's floating traffic lights.
            // Removing its decorations lets the app surface fill the window.
            .with_decorations(main_window_decorated(cfg!(windows)))
            .with_inner_size([1240.0, 800.0])
            .with_min_inner_size([760.0, 520.0])
            .with_fullscreen(fullscreen),
    };
    eframe::NativeOptions {
        viewport,
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

#[cfg(test)]
mod native_window_tests {
    use super::*;

    #[test]
    fn main_window_uses_the_platform_decoration_policy() {
        let options = native_options(false, None);
        assert_eq!(options.viewport.decorations, Some(!cfg!(windows)));
        assert_eq!(options.viewport.fullsize_content_view, Some(true));
        assert_eq!(options.viewport.titlebar_shown, Some(false));
        assert_eq!(options.viewport.title_shown, Some(false));
    }

    #[test]
    fn only_windows_removes_the_native_frame() {
        assert!(!main_window_decorated(true));
        assert!(main_window_decorated(false));
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
                    MenuCommand::Sidebar => Action::ToggleSidebar,
                    MenuCommand::Queue => Action::ToggleQueuePanel,
                    MenuCommand::Settings => Action::Open(Page::Settings),
                    MenuCommand::CheckForUpdates => Action::CheckForUpdates,
                    MenuCommand::Shortcuts => Action::ShowDialog(Dialog::Shortcuts),
                    MenuCommand::Back => Action::Back,
                    MenuCommand::Forward => Action::Forward,
                    MenuCommand::OpenRepo => {
                        ctx.open_url(egui::OpenUrl::new_tab(
                            "https://github.com/crmne/fastpotify",
                        ));
                        continue;
                    }
                    // Editing goes through egui, which owns the text field
                    // and the clipboard.
                    MenuCommand::Cut => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut);
                        continue;
                    }
                    MenuCommand::Copy => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy);
                        continue;
                    }
                    MenuCommand::Paste => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                        continue;
                    }
                    MenuCommand::SelectAll => {
                        ctx.input_mut(|input| {
                            input.events.push(egui::Event::Key {
                                key: egui::Key::A,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: egui::Modifiers::COMMAND,
                            });
                        });
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

    /// The mini player's window is see-through where the skin leaves it
    /// out; the big window paints itself over eframe's own ground.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self
            .app
            .as_ref()
            .is_some_and(|app| app.settings.winamp_window)
        {
            [0.0; 4]
        } else {
            egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A link on the command line is a link, and a control verb is still a
    /// verb: the two do not get in each other's way.
    #[test]
    fn a_link_and_a_verb_are_told_apart() {
        // #given / #when / #then
        let launch = Cli::try_parse_from(["fastpotify", "spotify:track:4uLU6hMCjMI75M1A2tKUQC"])
            .expect("a link parses");
        assert_eq!(
            launch.link.as_deref(),
            Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC")
        );
        assert!(launch.control.is_none());

        let launch = Cli::try_parse_from([
            "fastpotify",
            "https://open.spotify.com/album/1DFixLWuPkv3KT3TnV35m3?si=x",
            "--verbose",
        ])
        .expect("a web address parses");
        assert!(launch.link.is_some());
        assert!(launch.verbose);

        let verb = Cli::try_parse_from(["fastpotify", "next"]).expect("a verb parses");
        assert!(matches!(verb.control, Some(Control::Next)));
        assert!(verb.link.is_none());

        let bare = Cli::try_parse_from(["fastpotify"]).expect("a plain launch parses");
        assert!(bare.link.is_none() && bare.control.is_none());
    }
}

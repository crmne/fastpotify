//! The MilkDrop child process: a window of its own, with its own OpenGL
//! context and its own winit event loop.
//!
//! winit allows one event loop per process and eframe owns the app's, so
//! MilkDrop runs as a separate process: the app spawns this same binary with
//! `--milkdrop-child` (see `super::host`). It reads the sound from the
//! shared-memory ring the app fills, takes its settings and a close request
//! on stdin as JSON lines, and reports where it sits and when it closes on
//! stdout. Everything projectM makes belongs to this process, so it cannot
//! touch the app's windows.

use std::io::{BufRead, Write};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::glow::{self, HasContext};
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use serde::{Deserialize, Serialize};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, ResizeDirection, Window, WindowId};

use super::engine::Engine;
use super::overlay::{Overlay, Place, TextLine};
use super::shm::Ring;
use super::{DEFAULT_FPS, DEFAULT_SECONDS, LAG, MIN_SIZE, Presets, Request};

/// What the child is told to do, on stdin, as one JSON object per line.
#[derive(Debug, Default, Deserialize)]
struct Control {
    fps: Option<u32>,
    seconds: Option<u32>,
    scale: Option<u32>,
    /// The playing song, as lines to overlay when it changes.
    song: Option<Vec<String>>,
    close: Option<bool>,
}

/// What the child reports back, on stdout, as one JSON object per line.
#[derive(Debug, Default, Serialize)]
struct Event {
    #[serde(skip_serializing_if = "Option::is_none")]
    closed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pos: Option<[f32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<[f32; 2]>,
}

/// How the child was started, from the command line.
pub struct Args {
    pub shm: PathBuf,
    pub presets: PathBuf,
    pub size: [f32; 2],
    pub pos: Option<[f32; 2]>,
    pub fullscreen: bool,
    pub fps: u32,
    pub seconds: u32,
    pub scale: u32,
    /// A framebuffer to write the picture to for a skin, instead of (or
    /// along with) showing a window of its own. `None` is a window only.
    pub frames: Option<PathBuf>,
    /// No visible window: render only to write frames for a skin.
    pub hidden: bool,
}

impl Args {
    /// Reads the child's arguments from the command line; `None` when this is
    /// not a child process.
    pub fn parse() -> Option<Self> {
        let mut all = std::env::args().skip(1);
        if !std::env::args().any(|arg| arg == "--milkdrop-child") {
            return None;
        }
        let mut shm = None;
        let mut presets = None;
        let mut size = super::DEFAULT_SIZE;
        let mut pos = None;
        let mut fullscreen = false;
        let mut fps = DEFAULT_FPS;
        let mut seconds = DEFAULT_SECONDS;
        let mut scale = 1u32;
        let mut frames = None;
        let mut hidden = false;
        while let Some(arg) = all.next() {
            match arg.as_str() {
                "--milkdrop-shm" => shm = all.next().map(PathBuf::from),
                "--milkdrop-presets" => presets = all.next().map(PathBuf::from),
                "--milkdrop-size" => {
                    if let Some(pair) = all.next().and_then(|value| parse_pair(&value)) {
                        size = pair;
                    }
                }
                "--milkdrop-pos" => pos = all.next().and_then(|value| parse_pair(&value)),
                "--milkdrop-fullscreen" => fullscreen = true,
                "--milkdrop-fps" => fps = all.next().and_then(|v| v.parse().ok()).unwrap_or(fps),
                "--milkdrop-seconds" => {
                    seconds = all.next().and_then(|v| v.parse().ok()).unwrap_or(seconds);
                }
                "--milkdrop-scale" => {
                    scale = all.next().and_then(|v| v.parse().ok()).unwrap_or(scale);
                }
                "--milkdrop-frames" => frames = all.next().map(PathBuf::from),
                "--milkdrop-hidden" => hidden = true,
                _ => {}
            }
        }
        Some(Self {
            shm: shm?,
            presets: presets?,
            size,
            pos,
            fullscreen,
            fps,
            seconds,
            scale,
            frames,
            hidden,
        })
    }
}

fn parse_pair(value: &str) -> Option<[f32; 2]> {
    let (a, b) = value.split_once([',', 'x'])?;
    Some([a.trim().parse().ok()?, b.trim().parse().ok()?])
}

/// Runs the child process to the end; returns its exit code.
pub fn run(args: Args) -> i32 {
    let ring = match Ring::open(&args.shm) {
        Ok(ring) => ring,
        Err(error) => {
            eprintln!("MilkDrop: no audio ring: {error}");
            return 1;
        }
    };
    let event_loop = match EventLoop::<Control>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("MilkDrop: no event loop: {error}");
            return 1;
        }
    };
    // A thread reads control lines from the app and wakes the loop with them;
    // end of input (the app is gone) closes the window.
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let control: Control = serde_json::from_str(&line).unwrap_or_default();
            if proxy.send_event(control).is_err() {
                break;
            }
        }
        let _ = proxy.send_event(Control {
            close: Some(true),
            ..Default::default()
        });
    });

    let frames = args.frames.as_deref().and_then(|path| {
        super::frame::Frame::open(path)
            .map_err(|error| eprintln!("MilkDrop: no frame buffer: {error}"))
            .ok()
    });
    let mut app = Child::new(args, ring, frames);
    let _ = event_loop.run_app(&mut app);
    0
}

/// The live window and everything on its graphics context. Fields drop in
/// order and the engine frees projectM with GL calls, so it comes first.
struct Live {
    engine: Engine,
    gl: Arc<glow::Context>,
    overlay: Option<Overlay>,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    window: Window,
    fullscreen: bool,
}

struct Child {
    args: Args,
    ring: Ring,
    /// The framebuffer a skin reads, when the app asked for frames.
    frames: Option<super::frame::Frame>,
    /// Scratch for the readback, reused so a frame does not allocate.
    frame_buf: Vec<u8>,
    cursor: u64,
    presets: Presets,
    live: Option<Live>,
    song: Option<Vec<String>>,
    fps: u32,
    scale: u32,
    seconds: u32,
    pointer: PhysicalPosition<f64>,
    last_click: Option<Instant>,
    next_frame: Instant,
    reported: Option<([f32; 2], [f32; 2])>,
}

impl Child {
    fn new(args: Args, ring: Ring, frames: Option<super::frame::Frame>) -> Self {
        let fps = args.fps;
        let scale = args.scale;
        let seconds = args.seconds;
        Self {
            args,
            ring,
            frames,
            frame_buf: Vec::new(),
            cursor: 0,
            presets: Presets::new(),
            live: None,
            song: None,
            fps,
            scale,
            seconds,
            pointer: PhysicalPosition::new(0.0, 0.0),
            last_click: None,
            next_frame: Instant::now(),
            reported: None,
        }
    }

    fn frame_interval(&self) -> Option<Duration> {
        (self.fps != 0).then(|| Duration::from_secs_f32(1.0 / self.fps as f32))
    }

    fn create(&mut self, event_loop: &ActiveEventLoop) {
        match build(event_loop, &self.args, self.seconds) {
            Ok(live) => {
                live.window.request_redraw();
                self.live = Some(live);
            }
            Err(error) => {
                eprintln!("MilkDrop: {error}");
                report(&Event {
                    closed: Some(true),
                    ..Default::default()
                });
                event_loop.exit();
            }
        }
    }

    fn set_fullscreen(&mut self, on: bool) {
        if let Some(live) = &mut self.live {
            live.window
                .set_fullscreen(on.then(|| Fullscreen::Borderless(None)));
            live.fullscreen = on;
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop) {
        report(&Event {
            closed: Some(true),
            ..Default::default()
        });
        self.live = None;
        event_loop.exit();
    }

    fn render(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let folder = self.args.presets.clone();
        self.presets.refresh(&folder);
        if self.presets.current().is_none() && self.presets.count() > 0 {
            self.presets.next(false);
        }
        if let Some(hard) = live.engine.switch_wanted()
            && !self.presets.locked
        {
            self.presets.next(hard);
        }
        live.engine.set_seconds(self.seconds);
        live.engine.set_locked(self.presets.locked);
        if let Some(Request::Load { path, smooth }) = self.presets.take_request() {
            live.engine.load(&path, smooth);
        }
        let frames = self.ring.since(&mut self.cursor, LAG);
        live.engine.feed_frames(&frames);
        let size = live.window.inner_size();
        live.engine.render(
            0,
            0,
            size.width.max(1),
            size.height.max(1),
            self.scale.max(1),
        );
        if let Some(overlay) = &mut live.overlay {
            let scale = live.window.scale_factor() as f32;
            overlay.draw(&live.gl, (size.width, size.height), scale);
        }
        if self.frames.is_some() {
            let size = live.window.inner_size();
            let (width, height) = (size.width.max(1), size.height.max(1));
            if let Some(frames) = &self.frames {
                Self::write_frame(&live.gl, frames, &mut self.frame_buf, width, height);
            }
        }
        // A hidden child feeding a skin presents to nothing.
        if !self.args.hidden
            && let Err(error) = live.surface.swap_buffers(&live.context)
        {
            eprintln!("MilkDrop: present failed: {error}");
        }
        self.report_geometry();
    }

    /// Reads the composed picture back and hands it to a skin: RGBA,
    /// flipped top-down the way a texture wants it.
    fn write_frame(
        gl: &glow::Context,
        frames: &super::frame::Frame,
        buf: &mut Vec<u8>,
        width: u32,
        height: u32,
    ) {
        use super::frame::{MAX_HEIGHT, MAX_WIDTH};
        if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
            return;
        }
        let bytes = width as usize * height as usize * 4;
        buf.resize(bytes, 0);
        // SAFETY: the context is current; reading the default framebuffer
        // the engine just drew into.
        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.read_pixels(
                0,
                0,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(buf)),
            );
        }
        // GL rows run bottom-up; the framebuffer wants top-down.
        let stride = width as usize * 4;
        let mut top_down = vec![0u8; bytes];
        for row in 0..height as usize {
            let from = row * stride;
            let to = (height as usize - 1 - row) * stride;
            top_down[to..to + stride].copy_from_slice(&buf[from..from + stride]);
        }
        frames.write(width, height, &top_down);
    }

    /// Tells the app where the window is, when it has moved or resized.
    fn report_geometry(&mut self) {
        let Some(live) = &self.live else {
            return;
        };
        let size = live.window.inner_size();
        let size = [size.width as f32, size.height as f32];
        let pos = live
            .window
            .outer_position()
            .map(|pos| [pos.x as f32, pos.y as f32])
            .unwrap_or([0.0, 0.0]);
        if self.reported != Some((pos, size)) {
            self.reported = Some((pos, size));
            report(&Event {
                pos: Some(pos),
                size: Some(size),
                ..Default::default()
            });
        }
    }

    fn over_grip(&self) -> bool {
        let Some(live) = &self.live else {
            return false;
        };
        let size = live.window.inner_size();
        self.pointer.x >= size.width as f64 - 16.0 && self.pointer.y >= size.height as f64 - 16.0
    }
}

impl ApplicationHandler<Control> for Child {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_none() {
            self.create(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, control: Control) {
        if let Some(scale) = control.scale {
            self.scale = scale.clamp(1, 4);
        }
        if let Some(song) = control.song
            && self.song.as_ref() != Some(&song)
        {
            // The way MilkDrop showed the title when the song turned over.
            self.song = Some(song);
            self.show_song();
        }
        if let Some(fps) = control.fps {
            self.fps = fps;
        }
        if let Some(seconds) = control.seconds {
            self.seconds = seconds;
        }
        if control.close == Some(true) {
            self.close(event_loop);
        } else if let Some(live) = &self.live {
            live.window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.close(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(live) = &self.live
                    && let (Some(width), Some(height)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    live.surface.resize(&live.context, width, height);
                    live.window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.pointer = position,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => self.on_press(button),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.on_key(event.logical_key, event_loop);
            }
            WindowEvent::RedrawRequested => {
                self.render();
                self.next_frame = Instant::now() + self.frame_interval().unwrap_or(Duration::ZERO);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_none() {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            return;
        }
        match self.frame_interval() {
            None => {
                if let Some(live) = &self.live {
                    live.window.request_redraw();
                }
                event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
            }
            Some(_) => {
                if Instant::now() >= self.next_frame {
                    if self.frames.is_some() {
                        // A hidden child feeding a skin renders on the
                        // timer itself; a window never shown gets no
                        // redraws to answer to.
                        self.render();
                        self.next_frame =
                            Instant::now() + self.frame_interval().unwrap_or(Duration::ZERO);
                    } else if let Some(live) = &self.live {
                        live.window.request_redraw();
                    }
                } else {
                    event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                        self.next_frame,
                    ));
                }
            }
        }
    }
}

impl Child {
    fn on_press(&mut self, button: MouseButton) {
        let Some(live) = &self.live else {
            return;
        };
        match button {
            MouseButton::Left => {
                if self.over_grip() {
                    let _ = live.window.drag_resize_window(ResizeDirection::SouthEast);
                    return;
                }
                let now = Instant::now();
                let double = self
                    .last_click
                    .is_some_and(|last| now.duration_since(last) < Duration::from_millis(350));
                if double {
                    self.last_click = None;
                    let on = !live.fullscreen;
                    self.set_fullscreen(on);
                } else {
                    self.last_click = Some(now);
                    let _ = live.window.drag_window();
                }
            }
            MouseButton::Right => self.presets.next(false),
            _ => {}
        }
    }

    fn on_key(&mut self, key: Key, event_loop: &ActiveEventLoop) {
        let fullscreen = self.live.as_ref().is_some_and(|live| live.fullscreen);
        match key.as_ref() {
            Key::Named(NamedKey::Escape) => {
                if fullscreen {
                    self.set_fullscreen(false);
                } else {
                    self.close(event_loop);
                }
            }
            Key::Named(NamedKey::Space) | Key::Named(NamedKey::ArrowRight) => {
                self.presets.next(false);
            }
            Key::Named(NamedKey::ArrowLeft) => self.presets.previous(),
            Key::Character("f") | Key::Character("F") => self.set_fullscreen(!fullscreen),
            Key::Character("n") | Key::Character("N") => self.presets.next(false),
            Key::Character("p") | Key::Character("P") => self.presets.previous(),
            Key::Character("l") | Key::Character("L") => {
                self.presets.locked = !self.presets.locked;
            }
            Key::Character("?") | Key::Named(NamedKey::F1) => self.show_keys(),
            _ => {}
        }
    }

    /// The keys, over the picture, the way MilkDrop answered F1.
    fn show_keys(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(overlay) = &mut live.overlay else {
            return;
        };
        let line = |text: &str| TextLine {
            text: text.into(),
            px: 17.0,
        };
        let lines = [
            TextLine {
                text: "Keys".into(),
                px: 24.0,
            },
            line("N, Space, the right arrow, or a right-click \u{2014} next preset"),
            line("P or the left arrow \u{2014} previous preset"),
            line("L \u{2014} keep this preset"),
            line("F or a double-click \u{2014} full screen"),
            line("Esc \u{2014} leave full screen, or close"),
            line("? \u{2014} these keys"),
        ];
        overlay.show(
            &live.gl,
            &lines,
            Place::TopLeft,
            Duration::from_secs(8),
            live.window.scale_factor() as f32,
        );
        live.window.request_redraw();
    }

    /// The playing song, over the picture, fading away again.
    fn show_song(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let (Some(overlay), Some(song)) = (&mut live.overlay, &self.song) else {
            return;
        };
        let lines: Vec<TextLine> = song
            .iter()
            .take(2)
            .enumerate()
            .map(|(index, text)| TextLine {
                text: text.clone(),
                px: if index == 0 { 26.0 } else { 18.0 },
            })
            .collect();
        if lines.is_empty() {
            return;
        }
        overlay.show(
            &live.gl,
            &lines,
            Place::BottomLeft,
            Duration::from_secs(4),
            live.window.scale_factor() as f32,
        );
        live.window.request_redraw();
    }
}

/// Writes one event line to the app and flushes it. The line is tagged so
/// the app can tell it from libprojectM's own chatter on the same stream.
fn report(event: &Event) {
    if let Ok(line) = serde_json::to_string(event) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{}{line}", super::host::EVENT_PREFIX);
        let _ = stdout.flush();
    }
}

/// Builds the window, a GL context and surface for it, and projectM.
fn build(event_loop: &ActiveEventLoop, args: &Args, seconds: u32) -> Result<Live, String> {
    let mut attributes = Window::default_attributes()
        .with_title("MilkDrop")
        .with_decorations(false)
        .with_resizable(true)
        .with_inner_size(LogicalSize::new(args.size[0], args.size[1]));
    // The standalone window has a usable minimum; a hidden frame source
    // renders small on purpose, so it takes the size it is given.
    if !args.hidden {
        attributes = attributes.with_min_inner_size(LogicalSize::new(MIN_SIZE[0], MIN_SIZE[1]));
    }
    // A skin's frames need no window of their own; the picture goes to
    // the framebuffer a skin reads instead.
    if args.hidden {
        attributes = attributes.with_visible(false);
    }
    if let Some(pos) = args.pos {
        attributes = attributes.with_position(LogicalPosition::new(pos[0], pos[1]));
    }

    // Opaque: no alpha, so the window is never see-through.
    let template = ConfigTemplateBuilder::new().with_alpha_size(0);
    let (window, config) = DisplayBuilder::new()
        .with_window_attributes(Some(attributes))
        .build(event_loop, template, |mut configs| {
            configs.next().expect("a GL config for MilkDrop")
        })
        .map_err(|error| format!("no GL config: {error}"))?;
    let window = window.ok_or_else(|| "the window was not created".to_string())?;

    let raw = window
        .window_handle()
        .map_err(|error| format!("no window handle: {error}"))?
        .as_raw();
    let context_attributes = ContextAttributesBuilder::new().build(Some(raw));
    let not_current = unsafe {
        config
            .display()
            .create_context(&config, &context_attributes)
    }
    .map_err(|error| format!("no GL context: {error}"))?;

    let inner = window.inner_size();
    let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw,
        NonZeroU32::new(inner.width).unwrap_or(NonZeroU32::MIN),
        NonZeroU32::new(inner.height).unwrap_or(NonZeroU32::MIN),
    );
    let surface = unsafe {
        config
            .display()
            .create_window_surface(&config, &surface_attributes)
    }
    .map_err(|error| format!("no GL surface: {error}"))?;
    let context = not_current
        .make_current(&surface)
        .map_err(|error| format!("could not make the context current: {error}"))?;
    let _ = surface.set_swap_interval(&context, SwapInterval::DontWait);

    let gl = Arc::new(unsafe {
        glow::Context::from_loader_function_cstr(|symbol| config.display().get_proc_address(symbol))
    });
    let texture_dirs = [args.presets.clone(), args.presets.join("textures")];
    let dirs: Vec<&std::path::Path> = texture_dirs.iter().map(|dir| dir.as_path()).collect();
    let engine = Engine::new(Arc::clone(&gl), &dirs, seconds)
        .ok_or_else(|| "needs OpenGL 3.3".to_string())?;
    // Text over the picture; the picture goes on without it if the
    // shaders will not take.
    let overlay = Overlay::new(&gl);

    let mut live = Live {
        engine,
        gl,
        overlay,
        context,
        surface,
        window,
        fullscreen: false,
    };
    if args.fullscreen {
        live.window
            .set_fullscreen(Some(Fullscreen::Borderless(None)));
        live.fullscreen = true;
    }
    Ok(live)
}

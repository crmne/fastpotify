//! Shared palette, typography, icons, and base widgets.
//!
//! Inter provides real font weights, and Lucide provides a consistent icon set.
//! All colors use [`Palette`] so light, dark, and album-art-tinted themes stay
//! consistent.

use egui::{Color32, CornerRadius, Response, Sense, Stroke, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub window: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub surface_active: Color32,
    pub outline: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub overlay: Color32,
    pub shadow: Color32,
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            dark: true,
            window: Color32::from_rgb(0x0f, 0x11, 0x14),
            panel: Color32::from_rgb(0x15, 0x18, 0x1c),
            surface: Color32::from_rgb(0x1d, 0x21, 0x27),
            surface_hover: Color32::from_rgb(0x26, 0x2b, 0x33),
            surface_active: Color32::from_rgb(0x2f, 0x35, 0x3f),
            outline: Color32::from_rgb(0x2a, 0x30, 0x38),
            text: Color32::from_rgb(0xf2, 0xf4, 0xf6),
            secondary: Color32::from_rgb(0xa9, 0xb1, 0xbc),
            dim: Color32::from_rgb(0x6e, 0x77, 0x84),
            accent: Color32::from_rgb(0x1e, 0xd7, 0x60),
            accent_hover: Color32::from_rgb(0x3c, 0xe8, 0x7a),
            on_accent: Color32::from_rgb(0x0a, 0x14, 0x0e),
            danger: Color32::from_rgb(0xf5, 0x71, 0x7f),
            warning: Color32::from_rgb(0xf2, 0xb8, 0x5c),
            overlay: Color32::from_rgb(0x22, 0x27, 0x2e),
            shadow: Color32::from_black_alpha(140),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            window: Color32::from_rgb(0xf8, 0xf9, 0xfb),
            panel: Color32::from_rgb(0xff, 0xff, 0xff),
            surface: Color32::from_rgb(0xee, 0xf0, 0xf3),
            surface_hover: Color32::from_rgb(0xe3, 0xe6, 0xeb),
            surface_active: Color32::from_rgb(0xd7, 0xdb, 0xe1),
            outline: Color32::from_rgb(0xdd, 0xe1, 0xe6),
            text: Color32::from_rgb(0x14, 0x17, 0x1a),
            secondary: Color32::from_rgb(0x53, 0x5b, 0x66),
            dim: Color32::from_rgb(0x8b, 0x93, 0x9e),
            accent: Color32::from_rgb(0x15, 0xa6, 0x4a),
            accent_hover: Color32::from_rgb(0x12, 0x8f, 0x40),
            on_accent: Color32::WHITE,
            danger: Color32::from_rgb(0xd6, 0x3b, 0x4c),
            warning: Color32::from_rgb(0xb8, 0x7a, 0x14),
            overlay: Color32::from_rgb(0xff, 0xff, 0xff),
            shadow: Color32::from_black_alpha(50),
        }
    }

    /// Follows the OS light/dark setting, and on Linux also the active
    /// desktop palette when Omarchy publishes one. Missing or unreadable
    /// palette files use the built-in light or dark colours. A half-written
    /// file keeps the last complete palette; deleting the file drops back
    /// to the built-in colours.
    pub fn from_system(dark: bool) -> Self {
        #[cfg(not(target_os = "linux"))]
        {
            if dark { Self::dark() } else { Self::light() }
        }
        #[cfg(target_os = "linux")]
        {
            Self::from_linux_desktop(dark)
        }
    }

    #[cfg(target_os = "linux")]
    fn from_linux_desktop(dark: bool) -> Self {
        use std::sync::{Mutex, OnceLock};
        use std::time::SystemTime;

        struct Cache {
            mtime: Option<SystemTime>,
            last_good: Option<Palette>,
        }

        static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| {
            Mutex::new(Cache {
                mtime: None,
                last_good: None,
            })
        });
        let mut cached = cache.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(path) = linux_palette_file() else {
            cached.mtime = None;
            cached.last_good = None;
            return if dark { Self::dark() } else { Self::light() };
        };
        let mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|meta| meta.modified().ok());
        if mtime.is_none() {
            cached.mtime = None;
            cached.last_good = None;
            return if dark { Self::dark() } else { Self::light() };
        }
        if cached.mtime == mtime
            && let Some(palette) = cached.last_good
        {
            return palette;
        }
        cached.mtime = mtime;
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|colors| Self::from_desktop_colors(&colors))
        {
            Some(palette) => {
                cached.last_good = Some(palette);
                palette
            }
            None => cached
                .last_good
                .unwrap_or_else(|| if dark { Self::dark() } else { Self::light() }),
        }
    }

    fn from_desktop_colors(colors: &str) -> Option<Self> {
        let color = |name: &str| color_from_toml(colors, name);
        let window = color("background")?;
        let text = color("selection_foreground").or_else(|| color("foreground"))?;
        let accent = color("accent")?;
        let dark = luminance(window) < luminance(text);
        let panel = blend(window, text, 0.035);
        let surface = blend(window, text, 0.075);
        Some(Self {
            dark,
            window,
            panel,
            surface,
            surface_hover: blend(window, text, 0.13),
            surface_active: color("selection_background").unwrap_or(blend(window, accent, 0.20)),
            outline: blend(window, text, 0.18),
            text,
            secondary: blend(window, text, 0.80),
            dim: blend(window, text, 0.68),
            accent,
            accent_hover: blend(
                accent,
                if dark { Color32::WHITE } else { Color32::BLACK },
                0.10,
            ),
            on_accent: if luminance(accent) > 0.179 {
                Color32::BLACK
            } else {
                Color32::WHITE
            },
            danger: color("color1").unwrap_or(accent),
            warning: color("color3").unwrap_or(accent),
            overlay: window,
            shadow: Color32::from_black_alpha(if dark { 140 } else { 50 }),
        })
    }

    /// A colour derived from album art, softened so it can sit behind text.
    pub fn tint_from_art(&self, rgb: [u8; 3]) -> Color32 {
        let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let lightness = (max + min) / 2.0;
        let target = if self.dark { 0.30 } else { 0.72 };
        let (r, g, b) = if lightness < 0.01 {
            (target, target, target)
        } else {
            let scale = target / lightness;
            (
                (r * scale).min(1.0),
                (g * scale).min(1.0),
                (b * scale).min(1.0),
            )
        };
        Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
    }
}

#[cfg(target_os = "linux")]
fn linux_palette_file() -> Option<std::path::PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    Some(config.join("omarchy/current/theme/colors.toml"))
}

fn color_from_toml(colors: &str, name: &str) -> Option<Color32> {
    colors.lines().find_map(|line| {
        let line = toml_without_comment(line).trim();
        let (key, value) = line.split_once('=')?;
        (key.trim() == name).then(|| parse_hex_color(toml_string(value)))?
    })
}

fn toml_without_comment(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return "";
    }
    let mut in_single = false;
    let mut in_double = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => return &line[..index],
            _ => {}
        }
    }
    line
}

fn toml_string(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|rest| rest.split('"').next())
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.split('\'').next())
        })
        .unwrap_or(value)
}

fn parse_hex_color(value: &str) -> Option<Color32> {
    let hex = value.trim().strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let pair = |start: usize| u8::from_str_radix(&hex[start..start + 2], 16).ok();
    match hex.len() {
        3 => {
            let digit = |start: usize| {
                let value = u8::from_str_radix(&hex[start..start + 1], 16).ok()?;
                Some(value * 17)
            };
            Some(Color32::from_rgb(digit(0)?, digit(1)?, digit(2)?))
        }
        6 => Some(Color32::from_rgb(pair(0)?, pair(2)?, pair(4)?)),
        8 => Some(Color32::from_rgb(pair(0)?, pair(2)?, pair(4)?)),
        _ => None,
    }
}

fn blend(base: Color32, top: Color32, amount: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(
        mix(base.r(), top.r()),
        mix(base.g(), top.g()),
        mix(base.b(), top.b()),
    )
}

fn luminance(color: Color32) -> f32 {
    let linear = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

pub const RADIUS: u8 = 8;
pub const RADIUS_SMALL: u8 = 4;
pub const ROW_HEIGHT: f32 = 56.0;
pub const COMPACT_ROW_HEIGHT: f32 = 48.0;
/// The compact track list: one line, no cover.
pub const THIN_ROW_HEIGHT: f32 = 36.0;
pub const PLAYER_BAR_HEIGHT: f32 = 88.0;
/// The narrowest either right-hand panel goes. The queue and the lyrics
/// take the same edge and swap places there, so a width that suits one
/// has to suit the other, or the window would jump on the swap.
pub const SIDE_PANEL_MIN_WIDTH: f32 = 280.0;
pub const TOP_BAR_HEIGHT: f32 = 56.0;

/// macOS hides the titlebar and draws the window content all the way to the
/// top edge, so whatever sits at the top of the window has to leave room for
/// the traffic lights. Zero everywhere else, and in fullscreen, where the
/// buttons are gone.
pub fn titlebar_inset(ctx: &egui::Context) -> f32 {
    if cfg!(target_os = "macos") && !ctx.input(|input| input.viewport().fullscreen.unwrap_or(false))
    {
        28.0
    } else {
        0.0
    }
}

const INTER_MEDIUM: &str = "inter-medium";
const INTER_SEMIBOLD: &str = "inter-semibold";
const INTER_BOLD: &str = "inter-bold";

pub fn regular(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Proportional)
}

pub fn medium(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(INTER_MEDIUM.into()))
}

pub fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(INTER_SEMIBOLD.into()))
}

pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(INTER_BOLD.into()))
}

/// Install fonts, icons, and the base style once.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    register_icons(ctx);
    egui_extras::install_image_loaders(ctx);
}

/// Applies the palette to egui's own widgets so dialogs, menus, and text
/// fields agree with the custom views.
pub fn apply(ctx: &egui::Context, palette: &Palette) {
    let mut style = (*ctx.global_style()).clone();
    let visuals = &mut style.visuals;
    *visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.dark_mode = palette.dark;
    visuals.panel_fill = palette.panel;
    visuals.window_fill = palette.overlay;
    visuals.extreme_bg_color = palette.surface;
    visuals.faint_bg_color = palette.surface;
    visuals.code_bg_color = palette.surface;
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.secondary);
    visuals.hyperlink_color = palette.text;
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.window_stroke = Stroke::new(1.0, palette.outline);
    visuals.window_corner_radius = CornerRadius::same(RADIUS + 2);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 0,
        color: palette.shadow,
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: palette.shadow,
    };
    let corner = CornerRadius::same(RADIUS_SMALL + 2);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = corner;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0, palette.text);
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.corner_radius = corner;
    visuals.widgets.noninteractive.bg_fill = palette.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.outline);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.inactive.bg_fill = palette.surface;
    visuals.widgets.inactive.weak_bg_fill = palette.surface;
    visuals.widgets.hovered.bg_fill = palette.surface_hover;
    visuals.widgets.hovered.weak_bg_fill = palette.surface_hover;
    visuals.widgets.active.bg_fill = palette.surface_active;
    visuals.widgets.active.weak_bg_fill = palette.surface_active;
    visuals.widgets.open.bg_fill = palette.surface_hover;
    visuals.widgets.open.weak_bg_fill = palette.surface_hover;
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);
    visuals.striped = false;
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = egui::style::HandleShape::Circle;

    use egui::FontFamily::{Monospace, Proportional};
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::new(11.5, Proportional)),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Heading, FontId::new(22.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, Monospace)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size = Vec2::new(40.0, 28.0);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 8.0,
        floating_width: 6.0,
        floating_allocated_width: 0.0,
        handle_min_length: 28.0,
        bar_inner_margin: 3.0,
        bar_outer_margin: 2.0,
        dormant_background_opacity: 0.0,
        dormant_handle_opacity: 0.0,
        active_background_opacity: 0.0,
        active_handle_opacity: 0.55,
        interact_handle_opacity: 0.85,
        foreground_color: true,
        ..egui::style::ScrollStyle::floating()
    };
    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = 0.4;
    style.animation_time = 0.12;
    style.url_in_tooltip = false;
    ctx.set_global_style(style);
}

fn install_fonts(ctx: &egui::Context) {
    use egui::epaint::text::VariationCoords;
    use egui::{FontData, FontDefinitions, FontFamily};
    use std::sync::Arc;

    let mut fonts = FontDefinitions::default();
    let inter = include_bytes!("../assets/fonts/InterVariable.ttf");
    let weighted = |weight: f32| {
        let mut data = FontData::from_static(inter);
        data.tweak.coords = VariationCoords::new([(b"wght", weight)]);
        Arc::new(data)
    };
    fonts.font_data.insert("inter".to_owned(), weighted(400.0));
    fonts
        .font_data
        .insert(INTER_MEDIUM.to_owned(), weighted(500.0));
    fonts
        .font_data
        .insert(INTER_SEMIBOLD.to_owned(), weighted(600.0));
    fonts
        .font_data
        .insert(INTER_BOLD.to_owned(), weighted(700.0));

    let noto_emoji = include_bytes!("../assets/fonts/NotoEmoji.ttf");
    fonts.font_data.insert(
        "noto_emoji".to_owned(),
        Arc::new(FontData::from_static(noto_emoji)),
    );

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());
    // Right behind the text face, ahead of the emoji subset and the icon
    // font egui bundles, so every emoji comes from the one full face and
    // wears the same style; egui's pair still serves what Noto lacks.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(1, "noto_emoji".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(1, "noto_emoji".to_owned());
    let fallbacks: Vec<String> = fonts.families[&FontFamily::Proportional]
        .iter()
        .skip(1)
        .cloned()
        .collect();
    for name in [INTER_MEDIUM, INTER_SEMIBOLD, INTER_BOLD] {
        let mut family = vec![name.to_owned()];
        family.extend(fallbacks.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }

    // Add installed fallbacks for scripts Inter does not cover. Keep them after
    // Inter and the emoji font to preserve Latin shapes and color emoji.
    for font in crate::system_fonts::fallbacks() {
        // Reuse cached font bytes to avoid copying large collections whenever
        // epaint rebuilds the glyph atlas.
        let mut data = FontData::from_static(&font.bytes);
        data.index = font.index;
        fonts.font_data.insert(font.name.clone(), Arc::new(data));
        for family in fonts.families.values_mut() {
            family.push(font.name.clone());
        }
    }

    ctx.set_fonts(fonts);
}

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        &[$((
            Icon::$variant,
            concat!("bytes://fastpotify-icon-", $file, ".svg"),
            include_bytes!(concat!("../assets/icons/", $file, ".svg")).as_slice(),
        )),*]
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Icon {
    ArrowLeft,
    ArrowRight,
    AudioLines,
    BadgeCheck,
    Bookmark,
    BookmarkFilled,
    Car,
    Cast,
    Check,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    CircleAlert,
    CircleCheck,
    CirclePlay,
    CirclePlus,
    CircleX,
    Clock,
    Compass,
    Copy,
    Disc,
    Ellipsis,
    ExternalLink,
    Gamepad,
    Globe,
    GripVertical,
    Headphones,
    Heart,
    HeartFilled,
    House,
    Info,
    Laptop,
    Library,
    ListEnd,
    ListMusic,
    ListPlus,
    ListVideo,
    Loader,
    Lock,
    LogOut,
    Mic,
    Minus,
    Monitor,
    Moon,
    Music,
    Pause,
    PauseFilled,
    PanelLeft,
    Pin,
    PinOff,
    Pencil,
    Play,
    PlayFilled,
    Plus,
    Radio,
    Refresh,
    Repeat,
    Repeat1,
    Search,
    Settings,
    Shrink,
    Shuffle,
    SkipBack,
    SkipBackFilled,
    SkipForward,
    SkipForwardFilled,
    Smartphone,
    Sparkles,
    Speaker,
    Square,
    SquarePen,
    Sun,
    Tablet,
    Trash,
    TrendingUp,
    Tv,
    User,
    Users,
    Volume,
    Volume1,
    Volume2,
    VolumeX,
    Watch,
    X,
    Zap,
}

const ICONS: &[(Icon, &str, &[u8])] = icons! {
    ArrowLeft => "arrow-left",
    ArrowRight => "arrow-right",
    AudioLines => "audio-lines",
    BadgeCheck => "badge-check",
    Bookmark => "bookmark",
    BookmarkFilled => "bookmark-filled",
    Car => "car",
    Cast => "cast",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    CirclePlay => "circle-play",
    CirclePlus => "circle-plus",
    CircleX => "circle-x",
    Clock => "clock",
    Compass => "compass",
    Copy => "copy",
    Disc => "disc-3",
    Ellipsis => "ellipsis",
    ExternalLink => "external-link",
    Gamepad => "gamepad-2",
    Globe => "globe",
    GripVertical => "grip-vertical",
    Headphones => "headphones",
    Heart => "heart",
    HeartFilled => "heart-filled",
    House => "house",
    Info => "info",
    Laptop => "laptop",
    Library => "library",
    ListEnd => "list-end",
    ListMusic => "list-music",
    ListPlus => "list-plus",
    ListVideo => "list-video",
    Loader => "loader-circle",
    Lock => "lock",
    LogOut => "log-out",
    Mic => "mic",
    Minus => "minus",
    Monitor => "monitor",
    Moon => "moon",
    Music => "music",
    Pause => "pause",
    PauseFilled => "pause-filled",
    PanelLeft => "panel-left",
    Pin => "pin",
    PinOff => "pin-off",
    Pencil => "pencil",
    Play => "play",
    PlayFilled => "play-filled",
    Plus => "plus",
    Radio => "radio",
    Refresh => "refresh-cw",
    Repeat => "repeat",
    Repeat1 => "repeat-1",
    Search => "search",
    Settings => "settings",
    Shrink => "shrink",
    Shuffle => "shuffle",
    SkipBack => "skip-back",
    SkipBackFilled => "skip-back-filled",
    SkipForward => "skip-forward",
    SkipForwardFilled => "skip-forward-filled",
    Smartphone => "smartphone",
    Sparkles => "sparkles",
    Speaker => "speaker",
    Square => "square",
    SquarePen => "square-pen",
    Sun => "sun",
    Tablet => "tablet",
    Trash => "trash-2",
    TrendingUp => "trending-up",
    Tv => "tv",
    User => "user",
    Users => "users",
    Volume => "volume",
    Volume1 => "volume-1",
    Volume2 => "volume-2",
    VolumeX => "volume-x",
    Watch => "watch",
    X => "x",
    Zap => "zap",
};

impl Icon {
    pub fn uri(self) -> &'static str {
        ICONS
            .iter()
            .find(|(icon, _, _)| *icon == self)
            .map_or("", |(_, uri, _)| *uri)
    }

    pub fn image(self, color: Color32, size: f32) -> egui::Image<'static> {
        egui::Image::new(self.uri())
            .tint(color)
            .fit_to_exact_size(Vec2::splat(size))
    }
}

fn register_icons(ctx: &egui::Context) {
    for (_, uri, bytes) in ICONS {
        ctx.include_bytes(*uri, *bytes);
    }
}

/// A static icon.
pub fn icon(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> Response {
    ui.add(icon.image(color, size))
}

/// Paints an icon centred in `rect` without allocating space.
pub fn paint_icon(ui: &egui::Ui, icon: Icon, rect: egui::Rect, size: f32, color: Color32) {
    let icon_rect = egui::Rect::from_center_size(
        rect.center() + play_glyph_offset(icon, size),
        Vec2::splat(size),
    );
    icon.image(color, size).paint_at(ui, icon_rect);
}

/// Make keyboard focus visible without changing the control's layout.
pub fn focus_ring(ui: &egui::Ui, response: &Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            4.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Outside,
        );
    }
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
}

/// A frameless icon control whose colour lifts on hover.
pub fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    color: Color32,
    hover: Color32,
    tooltip: &str,
) -> Response {
    let edge = size + 12.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(edge), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    if ui.is_rect_visible(rect) {
        let tint = if response.hovered() || response.has_focus() {
            hover
        } else {
            color
        };
        let scale = if response.is_pointer_button_down_on() {
            0.92
        } else {
            1.0
        };
        paint_icon(ui, icon, rect, size * scale, tint);
    }
    focus_ring(ui, &response);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Horizontal offset that optically centers play triangles.
///
/// Lucide includes a 1/24-width shift; a measured 3% shift centers the icon at
/// Fastpotify's sizes. Use this everywhere instead of per-call adjustments.
pub fn play_glyph_offset(icon: Icon, icon_size: f32) -> Vec2 {
    if matches!(icon, Icon::PlayFilled | Icon::Play) {
        Vec2::new(icon_size * (0.03 - 1.0 / 24.0), 0.0)
    } else {
        Vec2::ZERO
    }
}

/// The app's mark, the accent disc with the play triangle, drawn the same
/// wherever it appears.
pub fn logo(ui: &egui::Ui, center: egui::Pos2, diameter: f32, disc: Color32, glyph: Color32) {
    ui.painter().circle_filled(center, diameter / 2.0, disc);
    let icon_size = diameter * 0.45;
    let icon_rect = egui::Rect::from_center_size(
        center + play_glyph_offset(Icon::PlayFilled, icon_size),
        Vec2::splat(icon_size),
    );
    Icon::PlayFilled
        .image(glyph, icon_size)
        .paint_at(ui, icon_rect);
}

pub fn circle_button(
    ui: &mut egui::Ui,
    icon: Icon,
    diameter: f32,
    fill: Color32,
    fill_hover: Color32,
    icon_color: Color32,
    tooltip: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let grow = if hovered { 1.05 } else { 1.0 };
        let radius = diameter / 2.0 * grow;
        let fill = if hovered { fill_hover } else { fill };
        ui.painter().circle_filled(rect.center(), radius, fill);
        let icon_size = diameter * 0.46;
        let offset = play_glyph_offset(icon, icon_size);
        let icon_rect =
            egui::Rect::from_center_size(rect.center() + offset, Vec2::splat(icon_size));
        icon.image(icon_color, icon_size).paint_at(ui, icon_rect);
    }
    focus_ring(ui, &response);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// A disc the size of a [`circle_button`] whose icon is replaced by a
/// spinner: the pressed play button itself shows that Spotify is reacting.
pub fn circle_spinner(
    ui: &mut egui::Ui,
    diameter: f32,
    fill: Color32,
    spin: Color32,
    tooltip: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ProgressIndicator,
            ui.is_enabled(),
            tooltip,
        )
    });
    if ui.is_rect_visible(rect) {
        ui.painter()
            .circle_filled(rect.center(), diameter / 2.0, fill);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        ));
        spinner(&mut child, diameter * 0.55, spin);
    }
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// A pill-shaped text button: filled for the primary action, outlined otherwise.
pub fn pill_button(ui: &mut egui::Ui, palette: &Palette, label: &str, primary: bool) -> Response {
    let font = semibold(13.0);
    let color = if primary {
        palette.on_accent
    } else {
        palette.text
    };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let padding = Vec2::new(18.0, 8.0);
    let size = galley.size() + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let radius = rect.height() / 2.0;
        if primary {
            let fill = if hovered {
                palette.accent_hover
            } else {
                palette.accent
            };
            ui.painter().rect_filled(rect, radius, fill);
        } else {
            let stroke_color = if hovered { palette.text } else { palette.dim };
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, stroke_color),
                egui::StrokeKind::Inside,
            );
        }
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, color);
    }
    focus_ring(ui, &response);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A muted button with an icon and label, for row and header actions.
pub fn soft_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Option<Icon>,
    label: &str,
    active: bool,
) -> Response {
    let font = medium(13.0);
    let color = if active { palette.window } else { palette.text };
    let galley =
        ui.painter()
            .layout_no_wrap(crate::bidi::display_text(label).into_owned(), font, color);
    let icon_size = 15.0;
    let icon_width = if icon.is_some() { icon_size + 6.0 } else { 0.0 };
    let padding = Vec2::new(12.0, 7.0);
    let size = Vec2::new(galley.size().x + icon_width, galley.size().y) + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if active {
            palette.text
        } else if hovered {
            palette.surface_hover
        } else {
            palette.surface
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        let mut x = rect.left() + padding.x;
        if let Some(icon) = icon {
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(x + icon_size / 2.0, rect.center().y),
                Vec2::splat(icon_size),
            );
            icon.image(color, icon_size).paint_at(ui, icon_rect);
            x += icon_width;
        }
        let pos = egui::pos2(x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(pos, galley, color);
    }
    focus_ring(ui, &response);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An animated busy indicator paced independently of the graphics driver.
pub fn spinner(ui: &mut egui::Ui, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
        let radius = size / 2.0 - 2.0;
        let start = ui.input(|input| input.time) * std::f64::consts::TAU * 1.2;
        let sweep = 250_f64.to_radians();
        let points = (0..20)
            .map(|index| {
                let angle = start + sweep * f64::from(index) / 19.0;
                let (sin, cos) = angle.sin_cos();
                rect.center() + radius * egui::vec2(cos as f32, sin as f32)
            })
            .collect();
        ui.painter()
            .add(egui::Shape::line(points, Stroke::new(2.0, color)));
    }
    response
}

/// Truncated single-line text in a given font and colour.
pub fn text(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
) -> Response {
    let text = text.into();
    if crate::bidi::is_rtl(&text) {
        // Laid out here so a cut lands at the reading end, on the left.
        let galley = crate::bidi::layout(
            ui.painter(),
            &text,
            font,
            color,
            ui.available_width(),
            1,
            Some(crate::bidi::ELLIPSIS),
        );
        return ui.add(egui::Label::new(galley).selectable(false));
    }
    ui.add(
        egui::Label::new(egui::RichText::new(text).font(font).color(color))
            .truncate()
            .selectable(false),
    )
}

/// Single-line text that acts like a link: underlines on hover, clickable.
pub fn link(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
) -> Response {
    let text = text.into();
    let response = if crate::bidi::is_rtl(&text) {
        let galley = crate::bidi::layout(
            ui.painter(),
            &text,
            font,
            color,
            ui.available_width(),
            1,
            Some(crate::bidi::ELLIPSIS),
        );
        ui.add(
            egui::Label::new(galley)
                .selectable(false)
                .sense(Sense::click()),
        )
    } else {
        ui.add(
            egui::Label::new(egui::RichText::new(text.clone()).font(font).color(color))
                .truncate()
                .selectable(false)
                .sense(Sense::click()),
        )
    };
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, ui.is_enabled(), &text));
    focus_ring(ui, &response);
    if response.hovered() {
        let rect = response.rect;
        ui.painter()
            .hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(1.0, color));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn section_title(ui: &mut egui::Ui, palette: &Palette, label: &str) -> Response {
    text(ui, label, bold(17.0), palette.text)
}

pub fn subtle(ui: &mut egui::Ui, palette: &Palette, label: &str) -> Response {
    text(ui, label, regular(13.0), palette.secondary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omarchy_palette_uses_the_system_theme_colours() {
        let palette = Palette::from_desktop_colors(
            r##"
                background = "#0d0b1a"
                foreground = "#c4c0e0"
                selection_foreground = "#e6e2f5"
                selection_background = "#2a2340"
                accent = "#a97bf0"
                color0 = "#1b1730"
                color1 = "#f7768e"
                color3 = "#e0c068"
                color7 = "#b8b2d8"
                color8 = "#453d63"
                color13 = "#c49bff"
            "##,
        )
        .expect("valid Omarchy palette");

        assert_eq!(palette.window, Color32::from_rgb(0x0d, 0x0b, 0x1a));
        assert_eq!(palette.accent, Color32::from_rgb(0xa9, 0x7b, 0xf0));
        assert_eq!(palette.surface_active, Color32::from_rgb(0x2a, 0x23, 0x40));
        assert_eq!(palette.danger, Color32::from_rgb(0xf7, 0x76, 0x8e));
    }

    #[test]
    fn desktop_palette_accepts_comments_quotes_and_short_hex() {
        let palette = Palette::from_desktop_colors(
            r##"
                # a comment
                background = '#abc' # trailing
                foreground = "#e6e2f5"
                accent = "#A97BF0FF"
            "##,
        )
        .expect("tolerant desktop palette");
        assert_eq!(palette.window, Color32::from_rgb(0xaa, 0xbb, 0xcc));
        assert_eq!(palette.accent, Color32::from_rgb(0xa9, 0x7b, 0xf0));
        assert!(Palette::from_desktop_colors("background = invalid").is_none());
        assert_eq!(parse_hex_color("#éabcx"), None);
        assert_eq!(parse_hex_color("#12"), None);
    }

    #[test]
    fn system_palette_is_readable_and_rejects_invalid_hex() {
        let contrast = |a: Color32, b: Color32| {
            let lum = |c: Color32| {
                let linear = |v: u8| {
                    let v = v as f32 / 255.0;
                    if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                };
                0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
            };
            (lum(a).max(lum(b)) + 0.05) / (lum(a).min(lum(b)) + 0.05)
        };
        let p = Palette::from_desktop_colors(
            "background = \"#0d0b1a\"\nforeground = \"#c4c0e0\"\naccent = \"#a97bf0\"",
        )
        .unwrap();
        assert_eq!(p.on_accent, Color32::BLACK);
        assert_ne!(p.panel, p.window);
        assert_ne!(p.surface, p.panel);
        assert_ne!(p.surface_hover, p.surface);
        assert!(contrast(p.secondary, p.panel) >= 4.5);
        assert!(contrast(p.dim, p.panel) >= 4.5);
        let light = Palette::from_desktop_colors(
            "background = \"#fafafa\"\nforeground = \"#202020\"\naccent = \"#8050c0\"",
        )
        .unwrap();
        assert!(!light.dark);
        assert!(contrast(light.on_accent, light.accent) >= 4.5);
    }

    #[test]
    fn fonts_install_and_layout_emojis() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let galley = ui.painter().layout_no_wrap(
                "Rosewood 🔥 Otomo 🎵 ❤️ 🚀".to_string(),
                regular(14.0),
                Color32::WHITE,
            );
            assert!(galley.rows[0].glyphs.len() >= 5);
        });
        output.textures_delta.clear();
    }
}

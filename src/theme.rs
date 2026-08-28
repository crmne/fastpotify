//! Fastpotify's visual language: palette, typography, icons, base widgets.
//!
//! Inter carries the interface with real weights (egui's `strong()` only
//! brightens), IBM-free monospace is unnecessary here, and one Lucide icon
//! vocabulary replaces Unicode lookalikes. Everything colour-related goes
//! through a [`Palette`] so light and dark stay coherent and album-art tints
//! can be blended in without hunting for hard-coded colours.

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

pub const RADIUS: u8 = 8;
pub const RADIUS_SMALL: u8 = 4;
pub const ROW_HEIGHT: f32 = 56.0;
pub const COMPACT_ROW_HEIGHT: f32 = 48.0;
pub const PLAYER_BAR_HEIGHT: f32 = 88.0;
pub const TOP_BAR_HEIGHT: f32 = 56.0;

const INTER_MEDIUM: &str = "inter-medium";
const INTER_SEMIBOLD: &str = "inter-semibold";
const INTER_BOLD: &str = "inter-bold";
const CJK: &str = "cjk";

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

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());
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

    // Inter covers no CJK, and neither do the faces egui bundles, so Chinese,
    // Japanese, and Korean titles arrive as rows of tofu boxes. Shipping a CJK
    // face would add more than ten megabytes to a binary that is a couple, and
    // every desktop that displays those scripts already carries one: borrow it
    // and append it to each family, after Inter so Latin text keeps its shape
    // and after the emoji faces so emoji keep their colour.
    if let Some(data) = system_cjk_font() {
        fonts.font_data.insert(CJK.to_owned(), Arc::new(data));
        for family in fonts.families.values_mut() {
            family.push(CJK.to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

/// Font files that cover CJK, best first, each with the face to open inside it.
///
/// Names are matched case-insensitively against file names, so only one
/// platform's entries can ever match. Faces cut for Simplified Chinese come
/// first: the pan-CJK collections open on their Japanese face, which draws a
/// handful of shared characters (直, 骨, 令) in shapes a Chinese reader sees as
/// wrong rather than missing.
const CJK_FONT_FILES: &[(&str, u32)] = &[
    // Linux, in package-manager order of likelihood.
    ("notosanssc-regular.otf", 0),
    ("notosanssc-regular.ttf", 0),
    ("notosanssc-vf.ttf", 0),
    ("notosanscjksc-regular.otf", 0),
    ("sourcehansanssc-regular.otf", 0),
    // Noto Sans CJK collections hold JP, KR, SC, TC, HK in that order.
    ("notosanscjk-regular.ttc", 2),
    ("notosanscjk-vf.otf.ttc", 2),
    ("notosanscjk-vf.ttf.ttc", 2),
    ("sourcehansans-regular.ttc", 2),
    ("wqy-microhei.ttc", 0),
    ("wqy-zenhei.ttc", 0),
    ("droidsansfallbackfull.ttf", 0),
    ("droidsansfallback.ttf", 0),
    // macOS.
    ("pingfang.ttc", 0),
    ("hiragino sans gb.ttc", 0),
    ("stheiti light.ttc", 0),
    // Windows.
    ("msyh.ttc", 0),
    ("msyh.ttf", 0),
    ("simhei.ttf", 0),
    ("simsun.ttc", 0),
];

/// How deep to walk each font directory. Distributions nest a level or two
/// (`/usr/share/fonts/truetype/noto`); nothing legitimate goes deeper, and the
/// bound also ends any symlink loop.
const FONT_SCAN_DEPTH: usize = 4;

/// The best CJK font installed on this machine, ready to use as a fallback.
///
/// Returns `None` when the system has no CJK font at all, which is the normal
/// state of a desktop that never displays those scripts; text that needs one
/// then renders exactly as it did before.
///
/// The search and the read happen once per process. [`install`] runs again
/// for every window the app creates, and a Noto CJK collection is twenty
/// megabytes to walk to and read.
fn system_cjk_font() -> Option<egui::FontData> {
    static FONT: std::sync::OnceLock<Option<(Vec<u8>, u32)>> = std::sync::OnceLock::new();
    let (bytes, index) = FONT.get_or_init(read_system_cjk_font).as_ref()?;
    // Lending epaint the cached bytes rather than handing it owned ones saves
    // a twenty-megabyte copy into its own blob, paid again every time it
    // rebuilds the glyph atlas -- which CJK text, filling the atlas fast, is
    // exactly what provokes.
    let mut data = egui::FontData::from_static(bytes);
    data.index = *index;
    Some(data)
}

/// Reads the best-ranked font of [`CJK_FONT_FILES`] found on disk, with the
/// face to open inside it.
fn read_system_cjk_font() -> Option<(Vec<u8>, u32)> {
    let (rank, path) = font_dirs()
        .iter()
        .filter_map(|dir| scan_fonts(dir, 0))
        .min_by_key(|(rank, _)| *rank)?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("cannot read CJK font {}: {error}", path.display());
            return None;
        }
    };
    // A collection that ships fewer faces than expected still covers CJK on
    // its first one, and asking for a face that is not there would panic.
    let wanted = CJK_FONT_FILES[rank].1;
    let index = if wanted < face_count(&bytes) {
        wanted
    } else {
        0
    };
    log::debug!("CJK fallback font: {} (face {index})", path.display());
    Some((bytes, index))
}

/// Where the platform keeps installed fonts.
fn font_dirs() -> Vec<std::path::PathBuf> {
    use std::path::{Path, PathBuf};

    let user = directories::UserDirs::new();
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
    } else if cfg!(target_os = "windows") {
        dirs.push(
            std::env::var_os("SystemRoot")
                .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
                .join("Fonts"),
        );
        dirs.extend(
            std::env::var_os("LOCALAPPDATA")
                .map(|local| PathBuf::from(local).join(r"Microsoft\Windows\Fonts")),
        );
    } else {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        // What a Flatpak sees of the host's fonts.
        dirs.push(PathBuf::from("/run/host/fonts"));
        // The pre-XDG per-user directory, which fontconfig still honours.
        dirs.extend(user.as_ref().map(|user| user.home_dir().join(".fonts")));
    }
    // `~/Library/Fonts` on macOS and `$XDG_DATA_HOME/fonts` on Linux: where a
    // font the user installed by hand lands. Windows keeps none, and its
    // per-user store is the `LOCALAPPDATA` path above.
    dirs.extend(
        user.as_ref()
            .and_then(|user| user.font_dir())
            .map(Path::to_path_buf),
    );
    dirs
}

/// The best-ranked entry of [`CJK_FONT_FILES`] below `dir`. Directories that
/// cannot be read are simply skipped.
fn scan_fonts(dir: &std::path::Path, depth: usize) -> Option<(usize, std::path::PathBuf)> {
    if depth >= FONT_SCAN_DEPTH {
        return None;
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            // The kind comes back with the directory listing, so only a
            // symlink (Debian nests its font tree behind them, as does a
            // Flatpak's `/run/host/fonts`) costs a look at the target.
            let kind = entry.file_type().ok()?;
            if kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()) {
                return scan_fonts(&entry.path(), depth + 1);
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let rank = CJK_FONT_FILES
                .iter()
                .position(|(file, _)| *file == name.as_str())?;
            Some((rank, entry.path()))
        })
        .min_by_key(|(rank, _)| *rank)
}

/// How many faces a font file holds: collections start with `ttcf` and a
/// count, everything else is a single face.
fn face_count(bytes: &[u8]) -> u32 {
    if !bytes.starts_with(b"ttcf") {
        return 1;
    }
    bytes
        .get(8..12)
        .and_then(|count| count.try_into().ok())
        .map_or(1, u32::from_be_bytes)
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
    let icon_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(size));
    icon.image(color, size).paint_at(ui, icon_rect);
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
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// A round, filled control such as the main play button.
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
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let grow = if hovered { 1.05 } else { 1.0 };
        let radius = diameter / 2.0 * grow;
        let fill = if hovered { fill_hover } else { fill };
        ui.painter().circle_filled(rect.center(), radius, fill);
        let icon_size = diameter * 0.46;
        // A right-pointing triangle's visual mass sits left of its bounding
        // box, so a geometrically centred glyph reads as pushed left and a
        // full optical shift reads as pushed right. Lucide bakes about one
        // viewBox unit (1/24) of right shift into the artwork; replace it
        // with a measured 3% of the icon size, which lands the triangle
        // visually centred in the disc at every size used here.
        let offset = if matches!(icon, Icon::PlayFilled | Icon::Play) {
            Vec2::new(icon_size * (0.03 - 1.0 / 24.0), 0.0)
        } else {
            Vec2::ZERO
        };
        let icon_rect =
            egui::Rect::from_center_size(rect.center() + offset, Vec2::splat(icon_size));
        icon.image(icon_color, icon_size).paint_at(ui, icon_rect);
    }
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
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let icon_size = 15.0;
    let icon_width = if icon.is_some() { icon_size + 6.0 } else { 0.0 };
    let padding = Vec2::new(12.0, 7.0);
    let size = Vec2::new(galley.size().x + icon_width, galley.size().y) + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
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
    let response = ui.add(
        egui::Label::new(egui::RichText::new(text).font(font).color(color))
            .truncate()
            .selectable(false)
            .sense(Sense::click()),
    );
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
    fn face_count_reads_collection_headers() {
        assert_eq!(face_count(b"OTTO"), 1);
        assert_eq!(face_count(b""), 1);
        assert_eq!(face_count(b"ttcf"), 1, "a truncated header is not a count");
        let mut collection = b"ttcf".to_vec();
        collection.extend_from_slice(&[0, 1, 0, 0]);
        collection.extend_from_slice(&5u32.to_be_bytes());
        assert_eq!(face_count(&collection), 5);
    }

    #[test]
    fn scan_finds_the_best_ranked_font_below_a_directory() {
        let root = std::env::temp_dir().join(format!("fastpotify-fonts-{}", std::process::id()));
        let nested = root.join("truetype/noto");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("InterVariable.ttf"), b"").unwrap();
        std::fs::write(nested.join("wqy-microhei.ttc"), b"").unwrap();
        std::fs::write(nested.join("NotoSansCJK-Regular.ttc"), b"").unwrap();

        let (rank, path) = scan_fonts(&root, 0).expect("a CJK font below the root");
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(CJK_FONT_FILES[rank].0, "notosanscjk-regular.ttc");
        assert_eq!(CJK_FONT_FILES[rank].1, 2, "the Simplified Chinese face");
        assert!(path.ends_with("NotoSansCJK-Regular.ttc"));
    }
}

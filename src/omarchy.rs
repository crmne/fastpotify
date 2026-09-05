//! Omarchy desktop theme integration.
//!
//! Omarchy resolves the active theme into one small palette file. Reading it
//! keeps Fastpotify independent of individual theme repositories and watching
//! it means `omarchy theme set` can retint an open window without a hook.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use egui::Color32;

use crate::theme::{FontFace, InterfaceFont, Palette};

const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The active, fully resolved Omarchy palette.
fn theme_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| {
        dirs.home_dir()
            .join(".local/state/omarchy/current/theme/colors.toml")
    })
}

/// Omarchy writes its selected interface font here. Reading the declared
/// family directly avoids a package-owned Fontconfig default masking the
/// later user rule, while watching it still catches `omarchy font set`
/// without requiring an application-specific hook.
fn fontconfig_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".config/fontconfig/fonts.conf"))
}

/// Reads the active Omarchy palette once.
pub(crate) fn load_palette() -> Result<Palette, String> {
    let path =
        theme_path().ok_or_else(|| "Omarchy themes are only available on Linux".to_string())?;
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
    parse_palette(&source).map_err(|error| format!("{}: {error}", path.display()))
}

/// Resolves the active Omarchy font through Fontconfig, including the closest
/// installed face for each weight Fastpotify uses.
pub(crate) fn load_font() -> Result<InterfaceFont, String> {
    let path =
        fontconfig_path().ok_or_else(|| "Omarchy fonts are only available on Linux".to_string())?;
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
    load_font_source(&source).map_err(|error| format!("{}: {error}", path.display()))
}

fn load_font_source(source: &str) -> Result<InterfaceFont, String> {
    let configured_family = parse_font_family(source)?;
    let regular = match_font(&configured_family, 80)?;
    let medium = match_font(&configured_family, 100)?;
    let semibold = match_font(&configured_family, 180)?;
    let bold = match_font(&configured_family, 200)?;
    let family = regular.family.clone();
    let mut files = HashMap::new();
    Ok(InterfaceFont {
        family,
        regular: read_face(regular, &mut files)?,
        medium: read_face(medium, &mut files)?,
        semibold: read_face(semibold, &mut files)?,
        bold: read_face(bold, &mut files)?,
    })
}

struct MatchedFont {
    family: String,
    path: PathBuf,
    index: u32,
}

fn match_font(family: &str, weight: u16) -> Result<MatchedFont, String> {
    let pattern = format!("{family}:weight={weight}");
    let output = Command::new("fc-match")
        .args(["-f", "%{family[0]}\n%{file}\n%{index}\n", &pattern])
        .output()
        .map_err(|error| format!("unable to run fc-match: {error}"))?;
    if !output.status.success() {
        return Err(format!("fc-match exited with {}", output.status));
    }
    parse_font_match(&output.stdout)
}

fn parse_font_family(source: &str) -> Result<String, String> {
    let (_, after_monospace) = source
        .split_once("<string>monospace</string>")
        .ok_or_else(|| "missing the monospace font rule".to_string())?;
    let edit_start = after_monospace
        .find("<edit")
        .ok_or_else(|| "missing the monospace family edit".to_string())?;
    let edit = &after_monospace[edit_start..];
    let edit_end = edit
        .find("</edit>")
        .ok_or_else(|| "incomplete monospace family edit".to_string())?;
    let edit = &edit[..edit_end];
    let (_, family) = edit
        .split_once("<string>")
        .ok_or_else(|| "missing the selected font family".to_string())?;
    let (family, _) = family
        .split_once("</string>")
        .ok_or_else(|| "incomplete selected font family".to_string())?;
    let family = family.trim();
    if family.is_empty() {
        return Err("selected font family is empty".to_string());
    }
    Ok(family.to_string())
}

fn parse_font_match(output: &[u8]) -> Result<MatchedFont, String> {
    let output = std::str::from_utf8(output)
        .map_err(|error| format!("fc-match returned invalid UTF-8: {error}"))?;
    let mut lines = output.lines();
    let family = lines.next().unwrap_or_default().trim();
    let path = lines.next().unwrap_or_default().trim();
    let index = lines.next().unwrap_or_default().trim();
    if family.is_empty() || path.is_empty() || index.is_empty() {
        return Err("fc-match returned an incomplete font description".to_string());
    }
    let index = index
        .parse()
        .map_err(|error| format!("fc-match returned an invalid face index: {error}"))?;
    Ok(MatchedFont {
        family: family.to_string(),
        path: PathBuf::from(path),
        index,
    })
}

fn read_face(
    matched: MatchedFont,
    files: &mut HashMap<PathBuf, Arc<[u8]>>,
) -> Result<FontFace, String> {
    let bytes = if let Some(bytes) = files.get(&matched.path) {
        Arc::clone(bytes)
    } else {
        let bytes: Arc<[u8]> = std::fs::read(&matched.path)
            .map_err(|error| format!("unable to read {}: {error}", matched.path.display()))?
            .into();
        files.insert(matched.path.clone(), Arc::clone(&bytes));
        bytes
    };
    Ok(FontFace {
        bytes,
        index: matched.index,
    })
}

/// Background reader for changes to Omarchy's theme and Fontconfig state.
pub(crate) struct ThemeWatcher {
    palette_updates: Receiver<Result<Palette, String>>,
    font_updates: Receiver<Result<InterfaceFont, String>>,
    stop: Arc<AtomicBool>,
}

impl ThemeWatcher {
    pub(crate) fn spawn(ctx: egui::Context) -> Option<Self> {
        let path = theme_path()?;
        Some(Self::spawn_at(path, fontconfig_path(), ctx, POLL_INTERVAL))
    }

    fn spawn_at(
        path: PathBuf,
        font_path: Option<PathBuf>,
        ctx: egui::Context,
        interval: Duration,
    ) -> Self {
        let (palette_send, palette_updates) = mpsc::channel();
        let (font_send, font_updates) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut last_source = None;
            let mut last_font_source = None;
            while !thread_stop.load(Ordering::Relaxed) {
                let read = std::fs::read_to_string(&path);
                let source_key = match &read {
                    Ok(source) => source.clone(),
                    Err(error) => format!("read-error:{:?}:{error}", error.kind()),
                };
                if last_source.as_ref() != Some(&source_key) {
                    last_source = Some(source_key);
                    let update = read
                        .map_err(|error| format!("unable to read {}: {error}", path.display()))
                        .and_then(|source| {
                            parse_palette(&source)
                                .map_err(|error| format!("{}: {error}", path.display()))
                        });
                    if palette_send.send(update).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
                if let Some(font_path) = &font_path {
                    let read = std::fs::read_to_string(font_path);
                    let source_key = match &read {
                        Ok(source) => source.clone(),
                        Err(error) => format!("read-error:{:?}:{error}", error.kind()),
                    };
                    if last_font_source.as_ref() != Some(&source_key) {
                        last_font_source = Some(source_key);
                        let update = read
                            .map_err(|error| {
                                format!("unable to read {}: {error}", font_path.display())
                            })
                            .and_then(|source| {
                                load_font_source(&source)
                                    .map_err(|error| format!("{}: {error}", font_path.display()))
                            });
                        if font_send.send(update).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                }
                std::thread::sleep(interval);
            }
        });
        Self {
            palette_updates,
            font_updates,
            stop,
        }
    }

    /// Returns only the newest update when several theme changes arrived
    /// between frames.
    pub(crate) fn latest_palette(&self) -> Option<Result<Palette, String>> {
        let mut latest = None;
        while let Ok(update) = self.palette_updates.try_recv() {
            latest = Some(update);
        }
        latest
    }

    /// Returns only the newest font update when several changes arrived
    /// between frames.
    pub(crate) fn latest_font(&self) -> Option<Result<InterfaceFont, String>> {
        let mut latest = None;
        while let Ok(update) = self.font_updates.try_recv() {
            latest = Some(update);
        }
        latest
    }
}

impl Drop for ThemeWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn parse_palette(source: &str) -> Result<Palette, String> {
    let values: HashMap<_, _> = source
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            let value = quoted(value.trim())?;
            Some((key, value))
        })
        .collect();

    let background = required_color(&values, "background")?;
    let dark = match values.get("mode").copied() {
        Some("dark") => true,
        Some("light") => false,
        Some(other) => return Err(format!("unsupported mode {other:?}")),
        None => relative_luminance(background) < 0.5,
    };
    let foreground = required_color(&values, "foreground")?;
    let accent = required_color(&values, "accent")?;
    let selection =
        optional_color(&values, "selection")?.unwrap_or_else(|| mix(background, foreground, 0.16));
    let muted =
        optional_color(&values, "muted")?.unwrap_or_else(|| mix(background, foreground, 0.24));
    let panel = optional_color(&values, "dark_background")?.unwrap_or(background);
    let surface = optional_color(&values, "lighter_background")?
        .unwrap_or_else(|| mix(background, foreground, 0.08));
    let text = optional_color(&values, "bright_foreground")?.unwrap_or(foreground);
    let secondary = optional_color(&values, "light_foreground")?.unwrap_or(foreground);
    let dim = optional_color(&values, "dark_foreground")?.unwrap_or(muted);

    Ok(Palette {
        dark,
        window: background,
        panel,
        surface,
        surface_hover: selection,
        surface_active: muted,
        outline: selection,
        text,
        secondary,
        dim,
        accent,
        accent_hover: mix(
            accent,
            if dark { Color32::WHITE } else { Color32::BLACK },
            0.16,
        ),
        on_accent: contrasting_text(accent),
        danger: optional_color(&values, "red")?.unwrap_or(accent),
        warning: optional_color(&values, "yellow")?.unwrap_or(accent),
        overlay: surface,
        shadow: Color32::from_black_alpha(if dark { 140 } else { 50 }),
    })
}

fn quoted(value: &str) -> Option<&str> {
    let value = value.strip_prefix('"')?;
    Some(&value[..value.find('"')?])
}

fn required<'a>(values: &HashMap<&str, &'a str>, key: &str) -> Result<&'a str, String> {
    values
        .get(key)
        .copied()
        .ok_or_else(|| format!("missing {key:?}"))
}

fn required_color(values: &HashMap<&str, &str>, key: &str) -> Result<Color32, String> {
    parse_color(required(values, key)?).map_err(|()| format!("invalid color for {key:?}"))
}

fn optional_color(values: &HashMap<&str, &str>, key: &str) -> Result<Option<Color32>, String> {
    values
        .get(key)
        .map(|value| parse_color(value).map_err(|()| format!("invalid color for {key:?}")))
        .transpose()
}

fn parse_color(value: &str) -> Result<Color32, ()> {
    let hex = value.strip_prefix('#').ok_or(())?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(());
    }
    let channel = |offset| u8::from_str_radix(&hex[offset..offset + 2], 16).map_err(|_| ());
    Ok(Color32::from_rgb(channel(0)?, channel(2)?, channel(4)?))
}

fn mix(from: Color32, to: Color32, amount: f32) -> Color32 {
    let channel = |from: u8, to: u8| {
        (f32::from(from) * (1.0 - amount) + f32::from(to) * amount).round() as u8
    };
    Color32::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
}

fn contrasting_text(background: Color32) -> Color32 {
    let luminance = relative_luminance(background);
    let black_contrast = (luminance + 0.05) / 0.05;
    let white_contrast = 1.05 / (luminance + 0.05);
    if black_contrast >= white_contrast {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

fn relative_luminance(color: Color32) -> f32 {
    let linear = |channel: u8| {
        let channel = f32::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO_NIGHT: &str = r##"
mode = "dark"
accent = "#7aa2f7"
selection = "#292e42"
muted = "#414868"
background = "#1a1b26"
dark_background = "#13141c"
lighter_background = "#24283b"
foreground = "#a9b1d6"
dark_foreground = "#565f89"
light_foreground = "#b4bee6"
bright_foreground = "#c0caf5"
red = "#f7768e"
yellow = "#e0af68"
"##;

    #[test]
    fn maps_omarchy_tokens_to_fastpotify_surfaces() {
        let palette = parse_palette(TOKYO_NIGHT).unwrap();
        assert!(palette.dark);
        assert_eq!(palette.window, Color32::from_rgb(0x1a, 0x1b, 0x26));
        assert_eq!(palette.panel, Color32::from_rgb(0x13, 0x14, 0x1c));
        assert_eq!(palette.surface, Color32::from_rgb(0x24, 0x28, 0x3b));
        assert_eq!(palette.surface_hover, Color32::from_rgb(0x29, 0x2e, 0x42));
        assert_eq!(palette.text, Color32::from_rgb(0xc0, 0xca, 0xf5));
        assert_eq!(palette.accent, Color32::from_rgb(0x7a, 0xa2, 0xf7));
        assert_eq!(palette.on_accent, Color32::BLACK);
        assert_eq!(palette.danger, Color32::from_rgb(0xf7, 0x76, 0x8e));
    }

    #[test]
    fn accepts_a_minimal_light_palette_and_derives_optional_surfaces() {
        let palette = parse_palette(
            r##"mode = "light"
accent = "#205ea6"
background = "#fffcf0"
foreground = "#100f0f"
"##,
        )
        .unwrap();
        assert!(!palette.dark);
        assert_eq!(palette.panel, palette.window);
        assert_ne!(palette.surface, palette.window);
        assert_eq!(palette.danger, palette.accent);
        assert_eq!(palette.on_accent, Color32::WHITE);
    }

    #[test]
    fn infers_legacy_palette_mode_from_its_background() {
        let mars = parse_palette(
            r##"accent = "#7b534e"
selection = "#4a2c2c"
background = "#000000"
foreground = "#d9afa7"
"##,
        )
        .unwrap();
        assert!(mars.dark);

        let light = parse_palette(
            r##"accent = "#205ea6"
background = "#fffcf0"
foreground = "#100f0f"
"##,
        )
        .unwrap();
        assert!(!light.dark);
    }

    #[test]
    fn rejects_unknown_modes_and_malformed_colors() {
        assert!(parse_palette(&TOKYO_NIGHT.replace("dark", "sepia")).is_err());
        for color in ["blue", "#12345g", "#aé123"] {
            assert!(parse_palette(&TOKYO_NIGHT.replace("#7aa2f7", color)).is_err());
        }
    }

    #[test]
    fn parses_fontconfig_face_descriptions() {
        let matched = parse_font_match(
            b"JetBrainsMono Nerd Font\n/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf\n2\n",
        )
        .unwrap();
        assert_eq!(matched.family, "JetBrainsMono Nerd Font");
        assert_eq!(
            matched.path,
            PathBuf::from("/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf")
        );
        assert_eq!(matched.index, 2);
        assert!(parse_font_match(b"missing fields\n").is_err());
    }

    #[test]
    fn reads_the_family_declared_by_omarchy_instead_of_a_masked_alias() {
        let source = r#"<?xml version="1.0"?>
<fontconfig>
  <match target="pattern">
    <test name="family" qual="any">
      <string>monospace</string>
    </test>
    <edit name="family" mode="prepend_first" binding="strong">
      <string>Liberation Mono</string>
    </edit>
  </match>
</fontconfig>"#;
        assert_eq!(parse_font_family(source).unwrap(), "Liberation Mono");
        assert!(parse_font_family("<fontconfig/>").is_err());
    }

    #[test]
    fn watcher_publishes_palette_changes() {
        let path = std::env::temp_dir().join(format!(
            "fastpotify-omarchy-theme-{}-{}.toml",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::write(&path, TOKYO_NIGHT).unwrap();
        let watcher = ThemeWatcher::spawn_at(
            path.clone(),
            None,
            egui::Context::default(),
            Duration::from_millis(10),
        );

        let wait_for = |dark| {
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                if let Some(Ok(palette)) = watcher.latest_palette()
                    && palette.dark == dark
                {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        };
        assert!(wait_for(true));
        std::fs::write(&path, TOKYO_NIGHT.replacen("dark", "light", 1)).unwrap();
        assert!(wait_for(false));

        drop(watcher);
        let _ = std::fs::remove_file(path);
    }
}

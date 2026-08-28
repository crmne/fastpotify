//! System fonts used as glyph fallbacks.
//!
//! Inter has no Hangul, kana, or CJK ideographs, so Korean, Japanese, and
//! Chinese titles would otherwise render as empty boxes. Shipping Noto CJK
//! would add tens of megabytes to the binary; the operating system's UI
//! fonts already cover those scripts, so they are loaded at startup and
//! appended after Inter.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{FontData, FontDefinitions};

/// Append installed CJK (and, if nothing else is found, Arial Unicode)
/// fonts to every family. Missing glyphs then fall through from Inter to a
/// font that actually has them.
pub fn add_system_fallbacks(fonts: &mut FontDefinitions) {
    let loaded = load_fallbacks(fonts);
    if loaded.is_empty() {
        log::warn!("no system CJK font found; Korean, Japanese, and Chinese text may not render");
        return;
    }
    for family in fonts.families.values_mut() {
        for name in &loaded {
            if !family.contains(name) {
                family.push(name.clone());
            }
        }
    }
}

fn load_fallbacks(fonts: &mut FontDefinitions) -> Vec<String> {
    let files = list_system_font_files();
    let mut loaded = Vec::new();

    if let Some(path) = pick(&files, PAN_CJK) {
        push_font(fonts, &mut loaded, "cjk", &path);
        return loaded;
    }

    for (key, needles) in [
        ("cjk-kr", KOREAN),
        ("cjk-jp", JAPANESE),
        ("cjk-sc", CHINESE_SIMPLIFIED),
        ("cjk-tc", CHINESE_TRADITIONAL),
    ] {
        if let Some(path) = pick(&files, needles) {
            push_font(fonts, &mut loaded, key, &path);
        }
    }

    if loaded.is_empty()
        && let Some(path) = pick(&files, UNICODE_FALLBACK)
    {
        push_font(fonts, &mut loaded, "unicode-fallback", &path);
    }

    loaded
}

fn push_font(fonts: &mut FontDefinitions, loaded: &mut Vec<String>, key: &str, path: &Path) {
    match fs::read(path) {
        Ok(bytes) if bytes.len() > 100 => {
            log::info!("using {} as a CJK fallback", path.display());
            fonts
                .font_data
                .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
            loaded.push(key.to_owned());
        }
        Ok(_) => log::debug!("ignoring tiny font file {}", path.display()),
        Err(error) => log::debug!("unable to read {}: {error}", path.display()),
    }
}

/// Substrings matched against the filename, in preference order. ASCII
/// needles are compared case-insensitively; the rest are taken as-is so
/// both NFC and NFD forms of Hiragino's filename can be listed.
const PAN_CJK: &[&str] = &[
    "notosanscjk-regular.ttc",
    "notosanscjk-regular.otf",
    "notosanscjk-otc",
    "sourcehansans-regular.ttc",
    "sourcehansans-regular.otf",
    "sourcehansans-regular.otc",
];

const KOREAN: &[&str] = &[
    "applegothic.ttf",
    "malgun.ttf",
    "notosanskr-regular",
    "notosanscjkkr-regular",
    "nanumbarungothic.ttf",
    "nanumgothic.ttf",
    "sourcehansanskr-regular",
    "applesdgothicneo",
];

const JAPANESE: &[&str] = &[
    // Hiragino Kaku Gothic W4/W3; macOS stores the name decomposed (NFD).
    "角ゴシック W4.ttc",
    "角ゴシック W4.ttc",
    "角ゴシック W3.ttc",
    "角ゴシック W3.ttc",
    "yugothr.ttc",
    "yugothr.ttf",
    "meiryo.ttc",
    "notosansjp-regular",
    "notosanscjkjp-regular",
    "sourcehansansjp-regular",
    "msgothic.ttc",
];

const CHINESE_SIMPLIFIED: &[&str] = &[
    "hiragino sans gb",
    "msyh.ttc",
    "msyh.ttf",
    "notosanssc-regular",
    "notosanscjksc-regular",
    "sourcehansanssc-regular",
];

const CHINESE_TRADITIONAL: &[&str] = &[
    "msjh.ttc",
    "msjh.ttf",
    "notosanstc-regular",
    "notosanscjktc-regular",
    "sourcehansanstc-regular",
];

const UNICODE_FALLBACK: &[&str] = &["arial unicode"];

fn pick(files: &[PathBuf], needles: &[&str]) -> Option<PathBuf> {
    for needle in needles {
        if let Some(path) = files.iter().find(|path| file_matches(path, needle)) {
            return Some(path.clone());
        }
    }
    None
}

fn file_matches(path: &Path, needle: &str) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if needle.is_ascii() {
        name.to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    } else {
        name.contains(needle)
    }
}

fn list_system_font_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in font_dirs() {
        collect_font_files(&dir, 3, &mut files);
    }
    files
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
    }
    if cfg!(target_os = "windows")
        && let Some(root) = std::env::var_os("SystemRoot")
    {
        dirs.push(PathBuf::from(root).join("Fonts"));
    }
    if cfg!(target_os = "linux") {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join("Library/Fonts"));
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
    }
    dirs
}

fn collect_font_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 || !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_files(&path, depth.saturating_sub(1), out);
            continue;
        }
        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if matches!(
            ext.to_ascii_lowercase().as_str(),
            "ttf" | "otf" | "ttc" | "otc"
        ) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::FontId;

    fn installed_context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        ctx
    }

    fn has_glyph(ctx: &egui::Context, c: char) -> bool {
        let font = FontId::proportional(14.0);
        ctx.fonts_mut(|fonts| fonts.has_glyph(&font, c))
    }

    #[test]
    fn macos_and_windows_find_a_korean_font() {
        if !(cfg!(target_os = "macos") || cfg!(target_os = "windows")) {
            return;
        }
        let files = list_system_font_files();
        assert!(
            pick(&files, KOREAN).is_some() || pick(&files, PAN_CJK).is_some(),
            "expected a system Korean or pan-CJK font, searched {} files",
            files.len()
        );
    }

    #[test]
    fn hangul_is_available_when_a_korean_font_is_installed() {
        let files = list_system_font_files();
        if pick(&files, KOREAN).is_none() && pick(&files, PAN_CJK).is_none() {
            return;
        }
        let ctx = installed_context();
        assert!(
            has_glyph(&ctx, '한'),
            "a CJK font was found on disk but Hangul still has no glyph"
        );
    }

    #[test]
    fn kana_and_han_are_available_when_cjk_fonts_are_installed() {
        let files = list_system_font_files();
        let has_jp = pick(&files, JAPANESE).is_some() || pick(&files, PAN_CJK).is_some();
        let has_sc = pick(&files, CHINESE_SIMPLIFIED).is_some()
            || pick(&files, PAN_CJK).is_some()
            || pick(&files, UNICODE_FALLBACK).is_some()
            || has_jp;
        if !has_jp && !has_sc {
            return;
        }
        let ctx = installed_context();
        if has_jp {
            assert!(
                has_glyph(&ctx, 'あ'),
                "a Japanese font was found on disk but hiragana still has no glyph"
            );
        }
        if has_sc {
            assert!(
                has_glyph(&ctx, '文'),
                "a CJK font was found on disk but han still has no glyph"
            );
        }
    }
}

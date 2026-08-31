//! Holds the currently active custom palette, if any, and resolves it
//! against dark/light mode on request. Thread-safe so it can sit on `App`
//! and still be reachable from wherever a hot-reload command arrives.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::theme::Palette;

use super::theme_file::ThemeFile;
use super::{Result, resolve};

/// A snapshot of the manager's state: enough to render, and to show in
/// Settings.
#[derive(Clone, Debug)]
pub struct PaletteState {
    /// The active theme's name, or `None` when using the built-in default.
    pub name: Option<String>,
    pub palette: Palette,
    /// Where the active theme file lives on disk, if it came from one
    /// (built-in palettes and the default have no file to reload).
    pub source_file: Option<PathBuf>,
}

struct Inner {
    theme: Option<ThemeFile>,
    source_file: Option<PathBuf>,
}

pub struct PaletteManager {
    inner: RwLock<Inner>,
}

impl PaletteManager {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner {
                theme: None,
                source_file: None,
            }),
        }
    }

    fn locked(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn locked_mut(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The resolved palette and metadata for the given mode.
    pub fn current(&self, dark: bool) -> PaletteState {
        let inner = self.locked();
        let base = if dark { Palette::dark() } else { Palette::light() };
        let palette = match &inner.theme {
            Some(theme) => {
                let colors = if dark { &theme.dark } else { &theme.light };
                resolve(base, colors).unwrap_or(base)
            }
            None => base,
        };
        PaletteState {
            name: inner.theme.as_ref().map(|theme| theme.name.clone()),
            palette,
            source_file: inner.source_file.clone(),
        }
    }

    /// Loads a theme file from disk and makes it active. `~` at the start
    /// of the path is expanded to the home directory, since the path
    /// usually comes from a text field or command line rather than a
    /// shell that would have expanded it already.
    pub fn load_from_file(&self, path: &Path) -> Result<()> {
        let path = expand_tilde(path);
        let theme = read_theme_file(&path)?;
        let mut inner = self.locked_mut();
        inner.theme = Some(theme);
        inner.source_file = Some(path);
        Ok(())
    }

    /// Makes one of the built-in palettes active.
    pub fn load_builtin(&self, name: &str) -> Result<()> {
        let make = super::BUILTIN_PALETTES
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, make)| *make)
            .ok_or_else(|| super::PaletteError::NotFound(name.to_string()))?;
        let mut inner = self.locked_mut();
        inner.theme = Some(make());
        inner.source_file = None;
        Ok(())
    }

    /// Drops the active theme, back to the built-in default.
    pub fn reset(&self) {
        let mut inner = self.locked_mut();
        inner.theme = None;
        inner.source_file = None;
    }

    /// Re-reads the active theme's file from disk. No-op if the active
    /// theme isn't file-backed (built-in, or none).
    pub fn reload(&self) -> Result<()> {
        let path = self.locked().source_file.clone();
        match path {
            Some(path) => self.load_from_file(&path),
            None => Ok(()),
        }
    }

    /// Built-in palette names, in listing order.
    pub fn list_builtins(&self) -> Vec<&'static str> {
        super::BUILTIN_PALETTES.iter().map(|(name, _)| *name).collect()
    }
}

impl Default for PaletteManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Expands a leading `~` (or `~/...`) to the home directory. Anything else
/// passes through unchanged; a `~` that isn't at the start, or that isn't
/// followed by a path separator, is left as a literal character rather
/// than guessed at.
fn expand_tilde(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    let Some(rest) = text.strip_prefix('~') else {
        return path.to_path_buf();
    };
    if !rest.is_empty() && !rest.starts_with('/') && !rest.starts_with(std::path::MAIN_SEPARATOR) {
        return path.to_path_buf();
    }
    let Some(home) = std::env::var_os("HOME") else {
        return path.to_path_buf();
    };
    let mut expanded = PathBuf::from(home);
    if let Some(rest) = rest.strip_prefix(['/', std::path::MAIN_SEPARATOR]) {
        expanded.push(rest);
    }
    expanded
}

fn read_theme_file(path: &Path) -> Result<ThemeFile> {
    let text = std::fs::read_to_string(path).map_err(|source| super::PaletteError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| super::PaletteError::ParseFile {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_the_built_in_palette() {
        let manager = PaletteManager::new();
        let state = manager.current(true);
        assert_eq!(state.name, None);
        assert_eq!(state.palette, Palette::dark());
    }

    #[test]
    fn loads_a_theme_file_and_overlays_only_named_colors() {
        let dir = std::env::temp_dir().join(format!("fastpotify-palette-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("theme.json");
        std::fs::write(
            &path,
            r##"{"name":"Test","dark":{"accent":"#ff0000"}}"##,
        )
        .unwrap();

        let manager = PaletteManager::new();
        manager.load_from_file(&path).unwrap();
        let state = manager.current(true);
        assert_eq!(state.name.as_deref(), Some("Test"));
        assert_eq!(state.palette.accent, egui::Color32::from_rgb(255, 0, 0));
        // Untouched field keeps the default.
        assert_eq!(state.palette.text, Palette::dark().text);
        assert_eq!(state.source_file, Some(path.clone()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reset_returns_to_the_default() {
        let manager = PaletteManager::new();
        manager.load_builtin("Nord").unwrap();
        assert!(manager.current(true).name.is_some());
        manager.reset();
        assert_eq!(manager.current(true).name, None);
    }

    #[test]
    fn unknown_builtin_is_an_error() {
        let manager = PaletteManager::new();
        assert!(manager.load_builtin("Nonexistent").is_err());
    }

    #[test]
    fn supports_transparency_in_a_theme_color() {
        let dir = std::env::temp_dir().join(format!("fastpotify-palette-test-alpha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("theme.json");
        std::fs::write(&path, r##"{"name":"T","dark":{"overlay":"#00000080"}}"##).unwrap();

        let manager = PaletteManager::new();
        manager.load_from_file(&path).unwrap();
        let alpha = manager.current(true).palette.overlay.a();
        assert_eq!(alpha, 0x80);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expands_a_leading_tilde_against_home() {
        if std::env::var_os("HOME").is_none() {
            return; // nothing to expand against on this runner
        }
        let dir = tempdir_under_home("fastpotify-palette-test-tilde");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("theme.json");
        std::fs::write(&path, r##"{"name":"Tilde"}"##).unwrap();

        let home = std::env::var("HOME").unwrap();
        let relative = path.strip_prefix(&home).unwrap();
        let tilde_path = PathBuf::from("~").join(relative);

        let manager = PaletteManager::new();
        manager.load_from_file(&tilde_path).unwrap();
        assert_eq!(manager.current(true).name.as_deref(), Some("Tilde"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A directory under `$HOME` for the tilde-expansion test, so the test
    /// path and `HOME` agree regardless of the platform's temp dir.
    fn tempdir_under_home(name: &str) -> PathBuf {
        let home = std::env::var("HOME").expect("HOME is set");
        PathBuf::from(home).join(format!("{name}-{}", std::process::id()))
    }
}

//! Custom colour palettes: hex-defined themes loaded from a JSON file or
//! picked from the built-in set, with transparency support via the alpha
//! channel. See [`PaletteManager`] for the entry point the app holds.

mod builtin;
mod colors;
mod manager;
mod theme_file;

pub use builtin::BUILTIN_PALETTES;
pub use colors::{ColorError, parse_hex_color};
pub use manager::{PaletteManager, PaletteState};
pub use theme_file::{ThemeColors, ThemeFile};

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, PaletteError>;

#[derive(Debug, thiserror::Error)]
pub enum PaletteError {
    #[error("could not read palette file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse palette file {path}: {source}")]
    ParseFile {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("invalid color '{0}'")]
    InvalidColor(String),
    #[error("palette '{0}' not found")]
    NotFound(String),
}

impl From<ColorError> for PaletteError {
    fn from(error: ColorError) -> Self {
        PaletteError::InvalidColor(error.0)
    }
}

/// Overlays `colors` onto `base`, one field at a time; a `None` field keeps
/// `base`'s value. This is how a custom theme file only needs to name the
/// colours it wants to change.
fn resolve(base: crate::theme::Palette, colors: &ThemeColors) -> Result<crate::theme::Palette> {
    use crate::theme::Palette;

    macro_rules! field {
        ($name:ident) => {
            match &colors.$name {
                Some(hex) => parse_hex_color(hex)?,
                None => base.$name,
            }
        };
    }

    Ok(Palette {
        dark: base.dark,
        window: field!(window),
        panel: field!(panel),
        surface: field!(surface),
        surface_hover: field!(surface_hover),
        surface_active: field!(surface_active),
        outline: field!(outline),
        text: field!(text),
        secondary: field!(secondary),
        dim: field!(dim),
        accent: field!(accent),
        accent_hover: field!(accent_hover),
        on_accent: field!(on_accent),
        danger: field!(danger),
        warning: field!(warning),
        overlay: field!(overlay),
        shadow: field!(shadow),
    })
}

//! The on-disk shape of a custom palette: a name and up to two colour sets,
//! one per mode. Every colour is optional; anything left out falls back to
//! the built-in dark or light palette.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThemeFile {
    pub name: String,
    #[serde(default)]
    pub dark: ThemeColors,
    #[serde(default)]
    pub light: ThemeColors,
}

/// Hex colour strings (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`), one field
/// per [`crate::theme::Palette`] colour. `dark`/`bool` isn't here: it's
/// which set applies, not a colour.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeColors {
    pub window: Option<String>,
    pub panel: Option<String>,
    pub surface: Option<String>,
    pub surface_hover: Option<String>,
    pub surface_active: Option<String>,
    pub outline: Option<String>,
    pub text: Option<String>,
    pub secondary: Option<String>,
    pub dim: Option<String>,
    pub accent: Option<String>,
    pub accent_hover: Option<String>,
    pub on_accent: Option<String>,
    pub danger: Option<String>,
    pub warning: Option<String>,
    pub overlay: Option<String>,
    pub shadow: Option<String>,
}

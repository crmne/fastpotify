//! Palettes shipped with the app, in the same shape as a user's theme file
//! so both go through the same resolution path.

use super::theme_file::{ThemeColors, ThemeFile};

/// A built-in palette's name and its factory function.
type BuiltinEntry = (&'static str, fn() -> ThemeFile);

/// Built-in palettes, by name.
pub const BUILTIN_PALETTES: &[BuiltinEntry] = &[("Catppuccin Mocha", catppuccin_mocha), ("Nord", nord)];

fn hex(value: &str) -> Option<String> {
    Some(value.to_string())
}

fn catppuccin_mocha() -> ThemeFile {
    // https://github.com/catppuccin/catppuccin - Mocha palette.
    ThemeFile {
        name: "Catppuccin Mocha".to_string(),
        dark: ThemeColors {
            window: hex("#1e1e2e"),
            panel: hex("#181825"),
            surface: hex("#313244"),
            surface_hover: hex("#45475a"),
            surface_active: hex("#585b70"),
            outline: hex("#45475a"),
            text: hex("#cdd6f4"),
            secondary: hex("#bac2de"),
            dim: hex("#a6adc8"),
            accent: hex("#cba6f7"),
            accent_hover: hex("#b4befe"),
            on_accent: hex("#1e1e2e"),
            danger: hex("#f38ba8"),
            warning: hex("#f9e2af"),
            overlay: hex("#11111b"),
            shadow: None,
        },
        // Catppuccin doesn't define a light Mocha; light mode falls back to
        // the app default (every field left unset).
        light: ThemeColors::default(),
    }
}

fn nord() -> ThemeFile {
    // https://www.nordtheme.com/docs/colors-and-palettes
    ThemeFile {
        name: "Nord".to_string(),
        dark: ThemeColors {
            window: hex("#2e3440"),
            panel: hex("#242933"),
            surface: hex("#3b4252"),
            surface_hover: hex("#434c5e"),
            surface_active: hex("#4c566a"),
            outline: hex("#4c566a"),
            text: hex("#eceff4"),
            secondary: hex("#d8dee9"),
            dim: hex("#8fbcbb"),
            accent: hex("#88c0d0"),
            accent_hover: hex("#8fbcbb"),
            on_accent: hex("#2e3440"),
            danger: hex("#bf616a"),
            warning: hex("#ebcb8b"),
            overlay: hex("#242933"),
            shadow: None,
        },
        light: ThemeColors::default(),
    }
}

//! No-op Omarchy theme integration for non-Linux platforms.

use crate::theme::{InterfaceFont, Palette};

pub(crate) fn load_palette() -> Result<Palette, String> {
    Err("Omarchy themes are only available on Linux".to_string())
}

pub(crate) fn load_font() -> Result<InterfaceFont, String> {
    Err("Omarchy fonts are only available on Linux".to_string())
}

pub(crate) struct ThemeWatcher;

impl ThemeWatcher {
    pub(crate) fn spawn(_ctx: egui::Context) -> Option<Self> {
        None
    }

    pub(crate) fn latest_palette(&self) -> Option<Result<Palette, String>> {
        None
    }

    pub(crate) fn latest_font(&self) -> Option<Result<InterfaceFont, String>> {
        None
    }
}

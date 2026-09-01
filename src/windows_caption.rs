//! Native Windows caption styling.
//!
//! The standard frame keeps real system window controls and resize behavior,
//! while DWM's supported color attributes let it read as part of Fastpotify
//! instead of an unrelated accent-colored strip.

use fastpotify::theme::Palette;
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DwmSetWindowAttribute,
    },
};

pub struct Caption {
    hwnd: HWND,
    applied: Option<CaptionColors>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CaptionColors {
    dark: bool,
    caption: u32,
    text: u32,
    border: u32,
}

impl Caption {
    pub fn new(context: &eframe::CreationContext<'_>) -> Option<Self> {
        let handle = context.window_handle().ok()?.as_raw();
        let RawWindowHandle::Win32(handle) = handle else {
            return None;
        };
        Some(Self {
            hwnd: handle.hwnd.get() as HWND,
            applied: None,
        })
    }

    pub fn apply(&mut self, palette: Palette) {
        let colors = CaptionColors {
            dark: palette.dark,
            caption: colorref(palette.window),
            text: colorref(palette.text),
            border: colorref(palette.outline),
        };
        if self.applied == Some(colors) {
            return;
        }
        self.applied = Some(colors);

        let dark = i32::from(colors.dark);
        // SAFETY: `hwnd` comes from eframe's live root window, and every
        // attribute points to the documented value type for the duration of
        // the call. Unsupported attributes simply return a failed HRESULT.
        unsafe {
            set_attribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark);
            set_attribute(self.hwnd, DWMWA_CAPTION_COLOR, &colors.caption);
            set_attribute(self.hwnd, DWMWA_TEXT_COLOR, &colors.text);
            set_attribute(self.hwnd, DWMWA_BORDER_COLOR, &colors.border);
        }
    }
}

unsafe fn set_attribute<T>(hwnd: HWND, attribute: i32, value: &T) {
    // SAFETY: The caller guarantees that `value` has the type DWM documents
    // for `attribute`, and the pointer remains valid through this call.
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute as u32,
            std::ptr::from_ref(value).cast(),
            size_of::<T>() as u32,
        )
    };
    if result < 0 {
        log::debug!("DWM window attribute {attribute} is unavailable: {result:#x}");
    }
}

/// Win32 `COLORREF` stores bytes as `0x00BBGGRR`.
fn colorref(color: egui::Color32) -> u32 {
    u32::from(color.r()) | (u32::from(color.g()) << 8) | (u32::from(color.b()) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_srgb_to_windows_colorref_order() {
        assert_eq!(
            colorref(egui::Color32::from_rgb(0x12, 0x34, 0x56)),
            0x563412
        );
    }
}

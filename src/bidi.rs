//! RTL (right-to-left) helpers for Arabic, Hebrew, and other RTL scripts.
//!
//! `egui`/`epaint` 0.36 does not implement the Unicode Bidirectional Algorithm:
//! it shapes each font run left-to-right and concatenates runs in logical
//! order.  For a paragraph whose base direction is RTL that leaves the words
//! in logical order while the letters inside each word are already mirrored by
//! HarfBuzz, so `غيوم في السماء` ("clouds in the sky") appears as
//! `مويغ يف ءامسلا` – words left-to-right, letters mirrored – and an RTL
//! reader sees `السماء في غيوم` (reversed word order).
//!
//! The fix is intentionally narrow – only song titles, album titles and lyrics
//! need it (see #37) – and stays lightweight: detect the paragraph's base
//! direction from its first strong directional character and, when it is RTL,
//! reorder the *words* for display.  HarfBuzz then mirrors the letters inside
//! each word, so the final glyphs left-to-right are the full character-wise
//! reversal that a RTL paragraph needs when rendered by a LTR engine.  Mixed
//! LTR/RTL paragraphs are left untouched.

use egui::{Align, Color32, FontId};
use unicode_bidi::BidiInfo;

/// Is the paragraph's base direction RTL?
///
/// Uses the first strong directional character, as suggested in #37 and as
/// `unicode-bidi` does for `BidiInfo::new(text, None)`.  This is more
/// accurate than a simple code-point range check and stays cheap.
pub fn is_rtl(text: &str) -> bool {
    // Fast path for empty or whitespace-only.
    if text.trim().is_empty() {
        return false;
    }
    let info = BidiInfo::new(text, None);
    if info.paragraphs.is_empty() {
        return false;
    }
    info.paragraphs[0].level.is_rtl()
}

/// For display in a LTR engine, return the text that should be fed to
/// `egui` so that the final glyphs appear correctly.
///
/// `egui` 0.36 shapes each font run left-to-right and concatenates runs in
/// logical order.  For an RTL paragraph that leaves words in logical order
/// while HarfBuzz already mirrors letters inside each word.  The visual order
/// for RTL (as computed by `unicode-bidi`) is the full character-wise
/// reversal, but feeding that directly to the engine would make it mirror
/// again.  Instead we feed the *word-reversed* logical order: the visual
/// order with each RTL word's characters reversed back to logical.  HarfBuzz
/// then mirrors them again, so the final glyphs left-to-right are the full
/// reversal that RTL needs.  LTR paragraphs are returned unchanged.
pub fn display_text(text: &str) -> std::borrow::Cow<'_, str> {
    if !is_rtl(text) {
        return std::borrow::Cow::Borrowed(text);
    }
    // Fast path for single word: no reordering needed beyond HarfBuzz's
    // intra-word mirroring.
    if !text.contains(' ') && !text.contains('\t') {
        return std::borrow::Cow::Borrowed(text);
    }
    let info = BidiInfo::new(text, None);
    if info.paragraphs.is_empty() {
        return std::borrow::Cow::Borrowed(text);
    }
    let para = &info.paragraphs[0];
    // `reorder_line` gives the visual order with RTL words already
    // character-reversed and LTR runs kept logical, e.g. "غيوم Hello"
    // -> "Hello مويغ".
    let reordered = info.reorder_line(para, para.range.clone());
    // To compensate for HarfBuzz's upcoming intra-word mirroring, reverse
    // each RTL word's characters back to logical.
    let mut out = String::with_capacity(reordered.len());
    // We need to work on the reordered string's words. Splitting on
    // whitespace and reversing each RTL word's characters is sufficient for
    // titles/lyrics (which are single lines without complex whitespace).
    let words: Vec<&str> = reordered.split_whitespace().collect();
    if words.is_empty() {
        return std::borrow::Cow::Borrowed(text);
    }
    for (idx, word) in words.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        if is_rtl(word) {
            // Reverse characters to go from visual (reversed) back to logical,
            // so HarfBuzz will reverse again to visual.
            for c in word.chars().rev() {
                out.push(c);
            }
        } else {
            out.push_str(word);
        }
    }
    // If the reordering did not change the text (e.g. pure LTR), avoid
    // allocation.
    if out == text {
        return std::borrow::Cow::Borrowed(text);
    }
    std::borrow::Cow::Owned(out)
}

/// Horizontal alignment that matches the paragraph direction.
pub fn halign_for(text: &str) -> Align {
    if is_rtl(text) {
        Align::RIGHT
    } else {
        Align::LEFT
    }
}

/// Create a `LayoutJob` that respects the paragraph direction.
///
/// The job's `halign` is set to `RIGHT` for RTL so that the text hugs the
/// right edge of its container, and the text itself is reordered via
/// `display_text` when needed.
pub fn layout_job(text: &str, font: FontId, color: Color32, wrap_width: f32) -> egui::text::LayoutJob {
    let display = display_text(text);
    let mut job = egui::text::LayoutJob::simple(display.into_owned(), font, color, wrap_width);
    job.halign = halign_for(text);
    job
}

/// Like `layout_job` but for truncated single-line text (ellipsis).
pub fn layout_job_singleline(text: &str, font: FontId, color: Color32) -> egui::text::LayoutJob {
    let display = display_text(text);
    let mut job = egui::text::LayoutJob::simple_singleline(display.into_owned(), font, color);
    job.halign = halign_for(text);
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rtl() {
        assert!(is_rtl("غيوم في السماء"));
        assert!(is_rtl("مرحبا بالعالم"));
        assert!(is_rtl("السماء"));
        assert!(!is_rtl("Hello world"));
        assert!(!is_rtl("Hello غيوم")); // base LTR
        assert!(is_rtl("غيوم Hello")); // base RTL
        assert!(!is_rtl(""));
        assert!(!is_rtl("   "));
    }

    #[test]
    fn reorders_pure_rtl() {
        assert_eq!(display_text("غيوم في السماء"), "السماء في غيوم");
        assert_eq!(display_text("مرحبا بالعالم"), "بالعالم مرحبا");
        assert_eq!(display_text("واحد اثنان ثلاثة"), "ثلاثة اثنان واحد");
        // single word unchanged
        assert_eq!(display_text("غيوم"), "غيوم");
        // mixed LTR base left untouched
        assert_eq!(display_text("Hello غيوم"), "Hello غيوم");
        // mixed RTL base reorders
        assert_eq!(display_text("غيوم Hello"), "Hello غيوم");
    }

    #[test]
    fn halign() {
        assert_eq!(halign_for("غيوم"), Align::RIGHT);
        assert_eq!(halign_for("Hello"), Align::LEFT);
    }
}

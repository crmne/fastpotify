//! Right-to-left text support for egui's left-to-right layout.
//!
//! epaint shapes letters within Arabic and Hebrew words but does not reorder
//! the words or align the line to the right.
//!
//! [`display_text`] applies visual word order while preserving logical letter
//! order for shaping. [`layout`] handles wrapped and truncated text one row at
//! a time and places ellipses at the left reading edge.

use std::borrow::Cow;
use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Align, Align2, Color32, FontId, Galley, Painter, Pos2, Rect, pos2};
use unicode_bidi::{BidiClass, BidiInfo, bidi_class};

/// The character that marks a cut.
pub const ELLIPSIS: char = '\u{2026}';

/// Whether the text reads right to left: decided by its first strong
/// character, the way the bidi algorithm decides a paragraph's direction.
/// ASCII never does, and most text is ASCII, so that check comes first and
/// costs nothing per frame.
pub fn is_rtl(text: &str) -> bool {
    if text.is_ascii() {
        return false;
    }
    for character in text.chars() {
        match bidi_class(character) {
            BidiClass::L => return false,
            BidiClass::R | BidiClass::AL => return true,
            _ => {}
        }
    }
    false
}

/// The edge the text should hug.
pub fn halign_for(text: &str) -> Align {
    if is_rtl(text) {
        Align::RIGHT
    } else {
        Align::LEFT
    }
}

/// `text` as the engine should receive it: every right-to-left line with
/// its words reordered. Text that needs no change comes back borrowed.
pub fn display_text(text: &str) -> Cow<'_, str> {
    if !text.contains('\n') {
        return display_line(text);
    }
    if !text.split('\n').any(is_rtl) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.split('\n')
            .map(display_line)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The reordered text, or `None` when `text` can be used as it is. Saves a
/// copy where the caller already owns a `String`.
pub fn reorder(text: &str) -> Option<String> {
    match display_text(text) {
        Cow::Owned(owned) => Some(owned),
        Cow::Borrowed(_) => None,
    }
}

/// One line. Punctuation, digits, and an ellipsis marking a cut keep the
/// places the bidi algorithm gives them; only the letters of each
/// right-to-left run go back to logical order, for the shaper to mirror.
fn display_line(line: &str) -> Cow<'_, str> {
    if !is_rtl(line) || line.chars().all(is_rtl_letter) {
        return Cow::Borrowed(line);
    }
    let line = line.trim_end_matches('\r');
    let info = BidiInfo::new(line, None);
    let Some(paragraph) = info.paragraphs.first() else {
        return Cow::Borrowed(line);
    };
    let visual = info.reorder_line(paragraph, paragraph.range.clone());
    let mut out = String::with_capacity(line.len());
    let mut letters: Vec<char> = Vec::new();
    for (index, word) in visual.split_whitespace().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        for character in word.chars() {
            if is_rtl_letter(character) {
                letters.push(character);
            } else {
                out.extend(letters.drain(..).rev());
                out.push(character);
            }
        }
        out.extend(letters.drain(..).rev());
    }
    Cow::Owned(out)
}

/// A letter of a right-to-left script, or a mark that rides on one.
fn is_rtl_letter(character: char) -> bool {
    matches!(
        bidi_class(character),
        BidiClass::R | BidiClass::AL | BidiClass::NSM
    )
}

/// Lays `text` out within `wrap_width`, on at most `max_rows` rows, with
/// `overflow` marking a cut, in reading order for either direction. The
/// galley's `halign` says which edge to anchor it at; [`galley_pos`] does
/// that for a painter, and `egui::Label` does it on its own.
pub fn layout(
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    wrap_width: f32,
    max_rows: usize,
    overflow: Option<char>,
) -> Arc<Galley> {
    if !text.split('\n').any(is_rtl) {
        let mut job = LayoutJob::simple(text.to_owned(), font, color, wrap_width);
        job.wrap.max_rows = max_rows;
        job.wrap.break_anywhere = false;
        job.wrap.overflow_character = overflow;
        return painter.layout_job(job);
    }
    // The rows are found on the logical text, whole words at a time, then
    // each row is reordered on its own, so a paragraph's first row still
    // holds its first words. epaint shapes the words itself, so a row's
    // glyphs say nothing reliable about its characters; measuring words is
    // the dependable way to know what fits.
    let width = |piece: &str| {
        painter
            .layout_no_wrap(piece.to_owned(), font.clone(), color)
            .size()
            .x
    };
    let rows = break_rows(text, wrap_width, max_rows, overflow, width);
    let display = rows
        .iter()
        .map(|row| display_line(row))
        .collect::<Vec<_>>()
        .join("\n");
    let mut job = LayoutJob::simple(display, font, color, f32::INFINITY);
    job.halign = Align::RIGHT;
    painter.layout_job(job)
}

/// Fills rows with whole words up to `wrap_width`, at most `max_rows` of
/// them, and ends a cut with `overflow`. The rows come back in logical
/// order, one string each.
fn break_rows(
    text: &str,
    wrap_width: f32,
    max_rows: usize,
    overflow: Option<char>,
    width: impl Fn(&str) -> f32,
) -> Vec<String> {
    let max_rows = max_rows.max(1);
    let space = width(" ");
    let mut rows: Vec<(String, f32)> = Vec::new();
    let mut row = (String::new(), 0.0_f32);
    let mut cut = false;
    'paragraphs: for (index, paragraph) in text.split('\n').enumerate() {
        if index > 0 {
            if rows.len() + 1 >= max_rows {
                cut = true;
                break;
            }
            rows.push(std::mem::take(&mut row));
        }
        for word in paragraph.split_whitespace() {
            let word_width = width(word);
            if !row.0.is_empty() && row.1 + space + word_width > wrap_width {
                if rows.len() + 1 >= max_rows {
                    cut = true;
                    break 'paragraphs;
                }
                rows.push(std::mem::take(&mut row));
            }
            if !row.0.is_empty() {
                row.0.push(' ');
                row.1 += space;
            }
            row.0.push_str(word);
            row.1 += word_width;
        }
    }
    rows.push(row);
    if cut && let Some(mark) = overflow {
        let mark_width = width(mark.encode_utf8(&mut [0; 4]));
        let last = rows.last_mut().expect("one row at least");
        while last.1 + mark_width > wrap_width
            && let Some(at) = last.0.rfind(' ')
        {
            last.1 -= width(&last.0[at + 1..]) + space;
            last.0.truncate(at);
        }
        last.0.push(mark);
    }
    rows.into_iter().map(|(text, _)| text).collect()
}

/// Where to paint a galley from [`layout`] so that it sits inside `rect`:
/// its left edge, or its right edge for right-to-left text.
pub fn galley_pos(rect: Rect, galley: &Galley) -> Pos2 {
    match galley.job.halign {
        Align::RIGHT => rect.right_top(),
        Align::Center => rect.center_top(),
        _ => rect.left_top(),
    }
}

/// Paints one line of text centred on `y`, starting at `left`, or ending at
/// `right` when it reads right to left. Returns the painted rect.
pub fn paint_line(
    painter: &Painter,
    left: f32,
    right: f32,
    y: f32,
    text: &str,
    font: FontId,
    color: Color32,
) -> Rect {
    if is_rtl(text) {
        painter.text(
            pos2(right, y),
            Align2::RIGHT_CENTER,
            display_text(text),
            font,
            color,
        )
    } else {
        painter.text(pos2(left, y), Align2::LEFT_CENTER, text, font, color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rtl() {
        assert!(is_rtl("غيوم في السماء"));
        assert!(is_rtl("מה נשמע"));
        assert!(is_rtl("السماء"));
        assert!(!is_rtl("Hello world"));
        assert!(!is_rtl("Hello غيوم")); // base LTR
        assert!(is_rtl("غيوم Hello")); // base RTL
        assert!(is_rtl("2024 غيوم")); // digits are weak
        assert!(!is_rtl(""));
        assert!(!is_rtl("   "));
        assert!(!is_rtl("123"));
        assert!(!is_rtl("Ça va"));
    }

    #[test]
    fn reorders_words_of_rtl_lines() {
        assert_eq!(display_text("غيوم في السماء"), "السماء في غيوم");
        assert_eq!(display_text("مرحبا بالعالم"), "بالعالم مرحبا");
        assert_eq!(display_text("واحد اثنان ثلاثة"), "ثلاثة اثنان واحد");
        // A single word is already right.
        assert_eq!(display_text("غيوم"), "غيوم");
        assert!(matches!(display_text("غيوم"), Cow::Borrowed(_)));
        // Left-to-right text, even with a right-to-left word in it, stays.
        assert_eq!(display_text("Hello غيوم"), "Hello غيوم");
        assert!(matches!(display_text("Hello world"), Cow::Borrowed(_)));
        // A Latin run inside a right-to-left line keeps its own order.
        assert_eq!(display_text("غيوم Hello"), "Hello غيوم");
    }

    #[test]
    fn ellipsis_moves_to_the_reading_end() {
        assert_eq!(display_text("غيوم في\u{2026}"), "\u{2026}في غيوم");
        assert_eq!(display_text("غيوم\u{2026}"), "\u{2026}غيوم");
        assert_eq!(display_text("Hello\u{2026}"), "Hello\u{2026}");
    }

    /// Punctuation and numbers follow their word on its left, where a
    /// right-to-left reader expects what comes after it.
    #[test]
    fn punctuation_and_digits_stay_after_their_word() {
        assert_eq!(display_text("الحلقة الأولى: كيف"), "كيف :الأولى الحلقة");
        assert_eq!(display_text("وإدارة شركتك. يقدمه"), "يقدمه .شركتك وإدارة");
        assert_eq!(display_text("الكلمة1 الكلمة2"), "2الكلمة 1الكلمة");
        assert_eq!(display_text("شركة 37signals كل"), "كل 37signals شركة");
    }

    #[test]
    fn lines_are_reordered_one_by_one() {
        assert_eq!(
            display_text("غيوم في السماء\nHello world\nمرحبا بالعالم"),
            "السماء في غيوم\nHello world\nبالعالم مرحبا"
        );
        assert!(matches!(display_text("Hello\nworld"), Cow::Borrowed(_)));
        assert_eq!(reorder("Hello world"), None);
        assert_eq!(reorder("غيوم في").as_deref(), Some("في غيوم"));
    }

    #[test]
    fn halign() {
        assert_eq!(halign_for("غيوم"), Align::RIGHT);
        assert_eq!(halign_for("Hello"), Align::LEFT);
    }

    /// Words fill rows up to the width, and a cut ends the last row.
    #[test]
    fn rows_fill_with_whole_words() {
        let width = |piece: &str| piece.chars().count() as f32 * 10.0;
        let text = "واحد اثنان ثلاثة";
        assert_eq!(
            break_rows(text, 110.0, usize::MAX, Some(ELLIPSIS), width),
            vec!["واحد اثنان".to_string(), "ثلاثة".to_string()]
        );
        assert_eq!(
            break_rows(text, 110.0, 1, Some(ELLIPSIS), width),
            vec!["واحد اثنان\u{2026}".to_string()]
        );
        // The mark has to fit too, so a word gives way to it.
        assert_eq!(
            break_rows(text, 100.0, 1, Some(ELLIPSIS), width),
            vec!["واحد\u{2026}".to_string()]
        );
        assert_eq!(
            break_rows("واحد\nاثنان", 200.0, usize::MAX, None, width),
            vec!["واحد".to_string(), "اثنان".to_string()]
        );
        assert_eq!(
            break_rows("واحد\nاثنان", 200.0, 1, Some(ELLIPSIS), width),
            vec!["واحد\u{2026}".to_string()]
        );
    }

    /// Wrapped text keeps its first words on the first row, and a cut
    /// sits at the left end of the last row.
    #[test]
    fn wrapped_rows_keep_reading_order() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            let font = FontId::proportional(14.0);
            let text = "واحد اثنان ثلاثة أربعة خمسة ستة سبعة ثمانية";
            let galley = layout(
                painter,
                text,
                font.clone(),
                Color32::WHITE,
                90.0,
                2,
                Some(ELLIPSIS),
            );
            assert_eq!(galley.job.halign, Align::RIGHT);
            let lines: Vec<&str> = galley.text().split('\n').collect();
            assert_eq!(lines.len(), 2, "{lines:?}");
            assert!(
                lines[0].ends_with("واحد"),
                "first row ends with the first word: {lines:?}"
            );
            assert!(
                lines[1].starts_with(ELLIPSIS),
                "the cut is at the left end: {lines:?}"
            );
            let plain = layout(
                painter,
                "Hello world",
                font,
                Color32::WHITE,
                200.0,
                1,
                Some(ELLIPSIS),
            );
            assert_eq!(plain.job.halign, Align::LEFT);
            assert_eq!(plain.text(), "Hello world");
        });
        output.textures_delta.clear();
    }
}

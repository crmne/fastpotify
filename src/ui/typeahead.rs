//! Type-ahead jump in the song lists, the way file explorers do it: plain
//! letters select the first title that starts with what was typed, more
//! letters narrow the search, and a moment of silence clears it again.
//! Matching forgives what titles have and hands skip: case, accents,
//! punctuation, spaces, and a leading The.

use std::time::{Duration, Instant};

use egui::{
    Align2, Context, CornerRadius, Event, Frame, Id, ImeEvent, Margin, Order, Rect, Stroke, pos2,
    vec2,
};

use crate::app::App;
use crate::model::{Loadable, Page};
use crate::theme::{self, Palette};

/// How long the typed characters survive without another keystroke.
const TIMEOUT: Duration = Duration::from_secs(1);

/// A title as the search sees it: lowercased, accents folded, and
/// punctuation and spaces gone, plus where the title begins without a
/// leading "the", "a", or "an".
#[derive(Clone, Debug)]
pub struct NormalizedTitle {
    full: String,
    article_start: usize,
}

impl NormalizedTitle {
    pub fn full(&self) -> &str {
        &self.full
    }

    /// The title ignoring a leading article; the whole title when it
    /// starts with none.
    fn article(&self) -> &str {
        &self.full[self.article_start..]
    }
}

/// Prepares a title for the search: `"The Beatles: 1"` becomes
/// `thebeatles1`, ready to meet a typed `beatles`.
pub fn normalize_title(name: &str) -> NormalizedTitle {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for ch in name.chars() {
        for lower in ch.to_lowercase() {
            if lower.is_alphanumeric() {
                push_folded(&mut word, lower);
            } else if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    let mut full = String::with_capacity(words.iter().map(String::len).sum());
    for word in &words {
        full.push_str(word);
    }
    // Only a whole first word is an article: "The Beatles" gives up
    // "the", "Theoretical" keeps every letter.
    let article_start = match words.first().map(String::as_str) {
        Some("the" | "a" | "an") => words[0].len(),
        _ => 0,
    };
    NormalizedTitle {
        full,
        article_start,
    }
}

/// Prepares typed text to meet a title: spaces and punctuation are
/// dropped, accents fold, case goes.
fn normalize_typed(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        for lower in ch.to_lowercase() {
            if lower.is_alphanumeric() {
                push_folded(&mut out, lower);
            }
        }
    }
    out
}

/// Folds the accents of a lowercased letter, so a typed `e` meets an `é`.
/// A hand-rolled map of the common Latin accents, so no folding crate
/// joins the build; everything else passes through untouched.
fn push_folded(out: &mut String, lower: char) {
    let folded = match lower {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ǎ' | 'ǟ' | 'ǡ' | 'ǻ' | 'ȁ' | 'ȃ'
        | 'ạ' | 'ả' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ' | 'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ' => {
            'a'
        }
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' | 'ð' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'ȅ' | 'ȇ' | 'ẹ' | 'ẻ' | 'ẽ' | 'ế'
        | 'ề' | 'ể' | 'ễ' | 'ệ' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'ǐ' | 'ȉ' | 'ȋ' | 'ị' | 'ỉ' => {
            'i'
        }
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŋ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'ơ' | 'ǒ' | 'ǫ' | 'ǭ' | 'ǿ' | 'ȍ'
        | 'ȏ' | 'ọ' | 'ỏ' | 'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ' | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' => {
            'o'
        }
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'š' | 'ş' | 'ș' => 's',
        'ţ' | 'ť' | 'ŧ' | 'ț' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'ư' | 'ǔ' | 'ǖ' | 'ǘ' | 'ǚ'
        | 'ǜ' | 'ȕ' | 'ȗ' | 'ụ' | 'ủ' | 'ứ' | 'ừ' | 'ử' | 'ữ' | 'ự' => 'u',
        'ŵ' | 'ẁ' | 'ẃ' | 'ẅ' => 'w',
        'ý' | 'ÿ' | 'ŷ' | 'ỳ' | 'ỵ' | 'ỷ' | 'ỹ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        'æ' => {
            out.push_str("ae");
            return;
        }
        'œ' => {
            out.push_str("oe");
            return;
        }
        'ß' => {
            out.push_str("ss");
            return;
        }
        'þ' => {
            out.push_str("th");
            return;
        }
        plain => {
            out.push(plain);
            return;
        }
    };
    out.push(folded);
}

/// Where a match may live, in the order one is preferred: the title as
/// typed, then without its leading article; loose mode adds the letters
/// as an unbroken run, then scattered anywhere in order.
#[derive(Clone, Copy)]
enum Tier {
    PrefixFull,
    PrefixArticle,
    SubstringFull,
    SubstringArticle,
    SubsequenceFull,
    SubsequenceArticle,
}

const PREFIX_TIERS: [Tier; 2] = [Tier::PrefixFull, Tier::PrefixArticle];
const LOOSE_TIERS: [Tier; 6] = [
    Tier::PrefixFull,
    Tier::PrefixArticle,
    Tier::SubstringFull,
    Tier::SubstringArticle,
    Tier::SubsequenceFull,
    Tier::SubsequenceArticle,
];

fn tiers(loose: bool) -> &'static [Tier] {
    if loose { &LOOSE_TIERS } else { &PREFIX_TIERS }
}

fn tier_matches(title: &NormalizedTitle, needle: &str, tier: Tier) -> bool {
    match tier {
        Tier::PrefixFull => title.full().starts_with(needle),
        Tier::PrefixArticle => title.article().starts_with(needle),
        Tier::SubstringFull => title.full().contains(needle),
        Tier::SubstringArticle => title.article().contains(needle),
        Tier::SubsequenceFull => subsequence(title.full(), needle),
        Tier::SubsequenceArticle => subsequence(title.article(), needle),
    }
}

/// Whether `needle`'s letters appear in `title` in order — the loose
/// mode's way of matching.
fn subsequence(title: &str, needle: &str) -> bool {
    let mut rest = title;
    for ch in needle.chars() {
        match rest.find(ch) {
            Some(at) => rest = &rest[at + ch.len_utf8()..],
            None => return false,
        }
    }
    true
}

/// The type-ahead search of one song list, kept per page in egui's
/// temporary data.
#[derive(Clone, Default)]
pub struct Search {
    buffer: String,
    last_keystroke: Option<Instant>,
    /// The row the search currently points at, in the list's display
    /// order, as produced by the table's view cache.
    pub row: Option<usize>,
    /// Whether letters may match anywhere in a title, from the setting.
    loose: bool,
    /// An unmatched query is waiting for another page. It does not expire
    /// while the request is in flight.
    waiting_for_results: bool,
    /// IME preedit text is still being composed. The committed query must
    /// not disappear while the candidate window is open.
    ime_composing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputAction {
    TogglePlay,
    Play {
        row: usize,
        pointer_event_before: bool,
    },
}

impl Search {
    /// Sets whether letters may match anywhere in a title, from the
    /// setting; read again every frame.
    pub fn set_loose(&mut self, loose: bool) {
        self.loose = loose;
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn has_needle(&self) -> bool {
        !normalize_typed(&self.buffer).is_empty()
    }

    /// Starts or finishes a wait for another page. Once results arrive,
    /// the ordinary timeout starts from their arrival rather than from the
    /// keystroke that requested them.
    pub fn set_waiting_for_results(&mut self, waiting: bool, now: Instant) {
        if self.waiting_for_results && !waiting && !self.buffer.is_empty() {
            self.last_keystroke = Some(now);
        }
        self.waiting_for_results = waiting;
    }

    /// Drops the search after a moment of silence, at the start of a frame.
    pub fn expire(&mut self, now: Instant) {
        if !self.waiting_for_results
            && !self.ime_composing
            && let Some(elapsed) = self
                .last_keystroke
                .and_then(|keystroke| now.checked_duration_since(keystroke))
            && elapsed >= TIMEOUT
        {
            *self = Self::default();
        }
    }

    /// Wakes the otherwise idle UI exactly when this search should expire.
    pub fn schedule_expiry(&self, ctx: &Context, now: Instant) {
        if self.waiting_for_results || self.ime_composing {
            return;
        }
        if let Some(keystroke) = self.last_keystroke {
            let elapsed = now.checked_duration_since(keystroke).unwrap_or_default();
            ctx.request_repaint_after(TIMEOUT.saturating_sub(elapsed));
        }
    }

    /// A typed character. Every letter lands in the buffer, matched or
    /// not, so what was typed stays typed: a prefix that matches nothing
    /// yet can still match once the list loads further, and the hint shows
    /// every key. The same letter again only cycles when it cannot extend
    /// anything and the buffer is that one letter, the way Explorer
    /// cycles through a first letter's matches.
    pub fn type_char(&mut self, ch: char, titles: &[NormalizedTitle]) {
        let candidate = format!("{}{ch}", self.buffer);
        let needle = normalize_typed(&candidate);
        let previous_needle = normalize_typed(&self.buffer);
        let typed = normalize_typed(&ch.to_string());
        let repeated = self.buffer.chars().count() == 1 && typed == previous_needle;
        let found = find(titles, &needle, 0, false, self.loose);

        // A second copy of a one-letter query cycles only when the doubled
        // text is not itself a title prefix.
        if found.is_none()
            && repeated
            && let Some(found) = find(
                titles,
                &previous_needle,
                self.row.map_or(0, |row| row + 1),
                true,
                self.loose,
            )
        {
            self.row = Some(found);
            self.last_keystroke = Some(Instant::now());
            self.waiting_for_results = false;
            return;
        }

        self.buffer = candidate;
        self.last_keystroke = Some(Instant::now());
        self.waiting_for_results = false;

        // Punctuation and spaces stay visible in the hint but do not move a
        // match when they add nothing to the normalized query.
        if needle == previous_needle {
            if !self.row.is_some_and(|row| {
                row < titles.len() && matches_any(&titles[row], &needle, self.loose)
            }) {
                self.row = found;
            }
            return;
        }

        if let Some(found) = found {
            self.row = Some(found);
            return;
        }
        // Nothing starts with the text typed so far; the row goes until a
        // title carries the buffer again.
        self.row = None;
    }

    /// Backspace: drop the last character and search again from the top.
    pub fn backspace(&mut self, titles: &[NormalizedTitle]) {
        self.buffer.pop();
        let needle = normalize_typed(&self.buffer);
        self.row = find(titles, &needle, 0, false, self.loose);
        self.last_keystroke = Some(Instant::now());
        self.waiting_for_results = false;
    }

    /// Escape: end the search at once.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Re-checks the remembered row after the list changed under it.
    pub fn revalidate(&mut self, titles: &[NormalizedTitle]) {
        if self.buffer.is_empty() {
            self.row = None;
            return;
        }
        let needle = normalize_typed(&self.buffer);
        let still_there = self.row.is_some_and(|row| {
            row < titles.len() && matches_any(&titles[row], &needle, self.loose)
        });
        if !still_there {
            self.row = find(titles, &needle, 0, false, self.loose);
        }
    }

    /// Applies keyboard text and editing keys in their original event order.
    pub fn handle_events(
        &mut self,
        events: &[Event],
        titles: &[NormalizedTitle],
    ) -> Vec<InputAction> {
        let mut actions = Vec::new();
        let mut pointer_event = false;
        for event in events {
            match event {
                Event::Text(text) | Event::Paste(text) => self.type_text(text, titles),
                Event::Ime(ImeEvent::Commit(text)) => {
                    self.ime_composing = false;
                    self.type_text(text, titles);
                }
                Event::Ime(ImeEvent::Preedit { text, .. }) => {
                    self.ime_composing = !text.is_empty();
                    if !text.is_empty() || !self.buffer.is_empty() {
                        self.last_keystroke = Some(Instant::now());
                    }
                }
                Event::Ime(ImeEvent::DeleteSurrounding { before_chars, .. }) => {
                    for _ in 0..*before_chars {
                        self.backspace(titles);
                    }
                }
                Event::Key {
                    key: egui::Key::Backspace,
                    pressed: true,
                    ..
                } => self.backspace(titles),
                Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    repeat: false,
                    ..
                } => self.clear(),
                Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    repeat: false,
                    ..
                } => {
                    if let Some(row) = self.row {
                        actions.push(InputAction::Play {
                            row,
                            pointer_event_before: pointer_event,
                        });
                    }
                }
                Event::Key {
                    key: egui::Key::Space,
                    pressed: true,
                    repeat: false,
                    ..
                } if self.buffer.is_empty() => actions.push(InputAction::TogglePlay),
                Event::PointerButton { .. } => pointer_event = true,
                _ => {}
            }
        }
        actions
    }

    fn type_text(&mut self, text: &str, titles: &[NormalizedTitle]) {
        for ch in text.chars() {
            if ch.is_control() || (ch.is_whitespace() && self.buffer.is_empty()) {
                continue;
            }
            self.type_char(ch, titles);
        }
    }
}

/// Whether the title carries the needle under any tier the mode allows.
fn matches_any(title: &NormalizedTitle, needle: &str, loose: bool) -> bool {
    tiers(loose)
        .iter()
        .any(|tier| tier_matches(title, needle, *tier))
}

#[derive(Clone, PartialEq, Eq)]
struct ActiveTarget {
    data_revision: u64,
    page: Option<Page>,
}

fn state_id_for(data_revision: u64, page: &Page) -> Id {
    Id::new("typeahead").with(data_revision).with(page)
}

pub(crate) fn state_id(app: &App, page: &Page) -> Id {
    state_id_for(app.data_revision, page)
}

fn set_active_target(app: &App, ctx: &Context, page: Option<Page>) {
    let memory_id = Id::new("active-typeahead-target");
    let current = ActiveTarget {
        data_revision: app.data_revision,
        page,
    };
    let previous = ctx.data(|data| data.get_temp::<ActiveTarget>(memory_id));
    if previous.as_ref() == Some(&current) {
        return;
    }
    if let Some(previous) = previous {
        if let Some(page) = previous.page {
            ctx.data_mut(|data| {
                data.remove::<Search>(state_id_for(previous.data_revision, &page));
            });
        }
        ctx.memory_mut(|memory| memory.interrupt_ime());
    }
    ctx.data_mut(|data| data.insert_temp(memory_id, current));
}

pub fn enter_page(app: &App, ctx: &Context) {
    set_active_target(app, ctx, Some(app.page().clone()));
}

pub fn leave_page(app: &App, ctx: &Context) {
    set_active_target(app, ctx, None);
}

/// Whether the visible main-window page has a type-ahead listener. A failed
/// page and the Winamp window leave the ordinary playback shortcuts alone.
pub fn owns_keyboard(app: &App, ctx: &Context) -> bool {
    let page_listens = match app.page() {
        Page::TopSongs => !matches!(&app.home.top_songs, Loadable::Failed(_)),
        Page::Playlist(id) => app
            .playlist_pages
            .get(id)
            .is_none_or(|page| !matches!(&page.playlist, Loadable::Failed(_))),
        Page::Album(id) => app
            .album_pages
            .get(id)
            .is_none_or(|page| !matches!(&page.album, Loadable::Failed(_))),
        Page::LikedSongs => true,
        _ => false,
    };
    app.settings.typeahead_jump
        && !app.settings.winamp_window
        && app.is_connected()
        && app.user.is_some()
        && app.dialog.is_none()
        && !app.show_devices
        && !egui::Popup::is_any_open(ctx)
        && page_listens
}

pub fn clear_state(app: &App, ctx: &Context, page: &Page) {
    ctx.data_mut(|data| data.remove::<Search>(state_id(app, page)));
}

/// Type-ahead has no text widget of its own, so it publishes the IME target
/// directly while it owns the keyboard. Committed composition arrives as an
/// ordinary search event while the candidate window sits by the hint.
pub fn enable_ime(ctx: &Context) {
    let content = ctx.content_rect();
    let cursor_rect = Rect::from_min_size(
        pos2(
            content.center().x,
            content.top() + theme::TOP_BAR_HEIGHT + 24.0,
        ),
        vec2(1.0, 18.0),
    );
    ctx.output_mut(|output| {
        output.ime = Some(egui::output::IMEOutput {
            purpose: egui::IMEPurpose::Normal,
            rect: cursor_rect.expand(8.0),
            cursor_rect,
            should_interrupt_composition: false,
        });
    });
}

/// The first title from `from` onwards the needle fits, under the first
/// tier that has a match, wrapping back to the top when `wrap`.
fn find(
    titles: &[NormalizedTitle],
    needle: &str,
    from: usize,
    wrap: bool,
    loose: bool,
) -> Option<usize> {
    if needle.is_empty() || titles.is_empty() {
        return None;
    }
    for tier in tiers(loose) {
        let found = (0..titles.len())
            .map(|offset| (from + offset) % titles.len())
            .take(if wrap {
                titles.len()
            } else {
                titles.len().saturating_sub(from)
            })
            .find(|&index| tier_matches(&titles[index], needle, *tier));
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The transient pill above the page showing what has been typed, so the
/// letters taken over by the search are visible somewhere.
pub fn hint(ctx: &Context, palette: Palette, page: &Page, buffer: &str) {
    if buffer.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new(("typeahead-hint", page.encode())))
        .order(Order::Tooltip)
        .interactable(false)
        // Clears the top bar, whose height the pill floats beneath.
        .anchor(Align2::CENTER_TOP, vec2(0.0, theme::TOP_BAR_HEIGHT + 10.0))
        .show(ctx, |ui| {
            Frame::new()
                .fill(palette.overlay)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(Margin::symmetric(12, 7))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 4],
                    blur: 16,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;
                        theme::text(ui, "Jump", theme::medium(12.5), palette.secondary);
                        // Every key the listener typed, never cut short:
                        // the text is the search's memory.
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(buffer)
                                    .font(theme::medium(13.5))
                                    .color(palette.text),
                            )
                            .selectable(false),
                        );
                    });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles() -> Vec<NormalizedTitle> {
        [
            "Bohemian Rhapsody",
            "Cancion Animal",
            "Bob",
            "Bohemian Like You",
            "Despacito",
            "Ubermensch",
        ]
        .iter()
        .map(|title| normalize_title(title))
        .collect()
    }

    #[test]
    fn more_letters_narrow_the_first_match() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(0));
        search.type_char('o', &titles);
        assert_eq!(search.buffer(), "bo");
        // Bohemian remains the first match while the longer query still
        // matches it; only repeated one-letter queries cycle.
        assert_eq!(search.row, Some(0));
        search.type_char('h', &titles);
        assert_eq!(search.row, Some(0));
    }

    #[test]
    fn the_same_letter_cycles_through_the_matches() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(0));
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(2));
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(3));
        // It wraps back to the top.
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(0));
    }

    #[test]
    fn a_repeated_ligature_cycles_its_matches() {
        let titles = ["Æsir", "Æon"]
            .iter()
            .map(|title| normalize_title(title))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        search.type_char('æ', &titles);
        assert_eq!(search.row, Some(0));
        search.type_char('æ', &titles);
        assert_eq!(search.buffer(), "æ");
        assert_eq!(search.row, Some(1));
    }

    #[test]
    fn a_letter_that_fits_nowhere_stays_in_the_buffer() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        search.type_char('o', &titles);
        // "bod" matches nothing, yet every letter stays typed: the hint
        // shows them all, and a later title can still carry the text.
        search.type_char('d', &titles);
        assert_eq!(search.buffer(), "bod");
        assert_eq!(search.row, None);
        // Backspace walks the text back and finds again, from the top.
        search.backspace(&titles);
        assert_eq!(search.buffer(), "bo");
        assert_eq!(search.row, Some(0));
    }

    #[test]
    fn a_letter_that_matches_nothing_is_kept() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('z', &titles);
        assert_eq!(search.buffer(), "z");
        assert_eq!(search.row, None);
    }

    #[test]
    fn a_repeated_letter_beyond_the_first_extends_the_text() {
        let titles = titles();
        let mut search = Search::default();
        // "ca" matches Cancion; a second letter that cannot extend it is
        // kept as text instead of hijacking the search, so words with
        // doubled letters type as typed.
        search.type_char('c', &titles);
        search.type_char('a', &titles);
        assert_eq!(search.row, Some(1));
        search.type_char('l', &titles);
        assert_eq!(search.buffer(), "cal");
        assert_eq!(search.row, None);
        search.type_char('l', &titles);
        assert_eq!(search.buffer(), "call");
        assert_eq!(search.row, None);
    }

    #[test]
    fn matching_ignores_case() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('D', &titles);
        assert_eq!(search.row, Some(4));
        // "dE" still only fits Despacito, in any casing.
        search.type_char('E', &titles);
        assert_eq!(search.row, Some(4));
        assert_eq!(search.buffer(), "DE");
    }

    #[test]
    fn matching_ignores_punctuation_and_spaces() {
        let titles = ["Don't Stop", "Let It Be"]
            .iter()
            .map(|title| normalize_title(title))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        search.type_char('d', &titles);
        search.type_char('o', &titles);
        search.type_char('n', &titles);
        // The typed apostrophe rides along but changes nothing.
        search.type_char('\'', &titles);
        assert_eq!(search.buffer(), "don'");
        assert_eq!(search.row, Some(0));
        // "dontstop" meets "Don't Stop" without any punctuation.
        search.type_char('t', &titles);
        search.type_char('s', &titles);
        search.type_char('t', &titles);
        search.type_char('o', &titles);
        search.type_char('p', &titles);
        assert_eq!(search.row, Some(0));
        // So does "letit" meet "Let It Be", spaces and all.
        search.clear();
        search.type_char('l', &titles);
        search.type_char('e', &titles);
        search.type_char('t', &titles);
        search.type_char('i', &titles);
        search.type_char('t', &titles);
        assert_eq!(search.buffer(), "letit");
        assert_eq!(search.row, Some(1));
    }

    #[test]
    fn ignored_punctuation_does_not_move_the_match() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(0));
        search.type_char('.', &titles);
        assert_eq!(search.buffer(), "b.");
        assert_eq!(search.row, Some(0));
    }

    #[test]
    fn a_space_joins_an_active_search() {
        let titles = ["Let It Be"]
            .iter()
            .map(|t| normalize_title(t))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        search.type_char('l', &titles);
        search.type_char('e', &titles);
        search.type_char('t', &titles);
        // The space shows in the text and is ignored by the match.
        search.type_char(' ', &titles);
        assert_eq!(search.buffer(), "let ");
        assert_eq!(search.row, Some(0));
        search.type_char('i', &titles);
        assert_eq!(search.row, Some(0));
    }

    #[test]
    fn matching_folds_accents() {
        let titles = ["Canción"]
            .iter()
            .map(|t| normalize_title(t))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        for ch in "cancion".chars() {
            search.type_char(ch, &titles);
        }
        assert_eq!(search.buffer(), "cancion");
        assert_eq!(search.row, Some(0));
        assert_eq!(normalize_title("Canción").full(), "cancion");
    }

    #[test]
    fn matching_folds_modern_latin_accents_and_ligatures() {
        assert_eq!(
            normalize_title("Șapte, Ắn, Æsir & Œuvre").full(),
            "sapteanaesiroeuvre"
        );
        let titles = ["Șapte Seri", "Œuvre"]
            .iter()
            .map(|title| normalize_title(title))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        for ch in "sapte".chars() {
            search.type_char(ch, &titles);
        }
        assert_eq!(search.row, Some(0));
        assert_eq!(normalize_title("Mơ ước").full(), "mouoc");
    }

    #[test]
    fn a_leading_article_is_a_fallback_form() {
        assert_eq!(normalize_title("The Beatles").article(), "beatles");
        assert_eq!(
            normalize_title("A Day in the Life").article(),
            "dayinthelife"
        );
        // Only a whole first word is an article.
        assert_eq!(normalize_title("Theoretical").article(), "theoretical");

        let both = ["The House of Sun", "House of Night"]
            .iter()
            .map(|title| normalize_title(title))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        search.type_char('h', &both);
        search.type_char('o', &both);
        search.type_char('u', &both);
        search.type_char('s', &both);
        search.type_char('e', &both);
        // Between an article-carried and a plain title, the plain wins.
        assert_eq!(search.row, Some(1));

        // With the plain one gone, the article yields.
        let only_the = ["The House of Sun"]
            .iter()
            .map(|title| normalize_title(title))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        search.type_char('h', &only_the);
        search.type_char('o', &only_the);
        search.type_char('u', &only_the);
        search.type_char('s', &only_the);
        search.type_char('e', &only_the);
        assert_eq!(search.row, Some(0));
    }

    #[test]
    fn loose_mode_matches_letters_anywhere_in_order() {
        let titles = titles();
        let mut search = Search::default();
        // Without loose mode, "bl" after the first letter finds nothing
        // (Bohemian Rhapsody has no l).
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(0));
        search.type_char('l', &titles);
        assert_eq!(search.row, None);
        // With it, the letters may appear anywhere in order.
        search.clear();
        search.loose = true;
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(0));
        search.type_char('l', &titles);
        // Rhapsody has no l, so the search moves on to Like You.
        assert_eq!(search.row, Some(3));
        search.type_char('y', &titles);
        assert_eq!(search.buffer(), "bly");
        assert_eq!(search.row, Some(3));
    }

    #[test]
    fn loose_mode_prefers_an_unbroken_run_over_scattered_letters() {
        // "orm" is scattered through the champion but an unbroken run at
        // the end of the storm; the run wins, whoever sits first.
        let titles: Vec<_> = ["Porntstart Champion", "Sainted by the Storm"]
            .iter()
            .map(|title| normalize_title(title))
            .collect();
        let mut search = Search {
            loose: true,
            ..Default::default()
        };
        for ch in "orm".chars() {
            search.type_char(ch, &titles);
        }
        assert_eq!(search.buffer(), "orm");
        assert_eq!(search.row, Some(1));
    }

    #[test]
    fn loose_mode_reads_the_typed_text_as_one_run() {
        // "the storm" is an unbroken run inside the sainted title; the
        // darkstorm galaxy only scatters the same letters.
        let titles: Vec<_> = [
            "Goblin King of the Darkstorm Galaxy",
            "Sainted by the Storm",
        ]
        .iter()
        .map(|title| normalize_title(title))
        .collect();
        let mut search = Search {
            loose: true,
            ..Default::default()
        };
        for ch in "the storm".chars() {
            search.type_char(ch, &titles);
        }
        assert_eq!(search.buffer(), "the storm");
        assert_eq!(search.row, Some(1));
    }

    #[test]
    fn loose_mode_searches_the_whole_list_for_a_longer_text() {
        let titles = titles();
        let mut search = Search {
            loose: true,
            ..Default::default()
        };
        // "bly" lands on Bohemian Like You, row 3.
        search.type_char('b', &titles);
        search.type_char('l', &titles);
        search.type_char('y', &titles);
        assert_eq!(search.row, Some(3));
        // A new text searches the whole list again: "anc" fits Cancion
        // Animal, back at row 1.
        search.clear();
        search.loose = true;
        for ch in "anc".chars() {
            search.type_char(ch, &titles);
        }
        assert_eq!(search.row, Some(1));
    }

    #[test]
    fn backspace_shortens_and_searches_from_the_top() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        search.type_char('o', &titles);
        search.backspace(&titles);
        assert_eq!(search.buffer(), "b");
        assert_eq!(search.row, Some(0));
        search.backspace(&titles);
        assert_eq!(search.buffer(), "");
        assert_eq!(search.row, None);
    }

    fn key(key: egui::Key, repeat: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn events_are_applied_in_the_order_they_arrive() {
        let titles = ["Ab", "Ac"]
            .iter()
            .map(|title| normalize_title(title))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        search.type_char('a', &titles);
        search.type_char('b', &titles);
        search.handle_events(
            &[key(egui::Key::Backspace, false), Event::Text("c".into())],
            &titles,
        );
        assert_eq!(search.buffer(), "ac");
        assert_eq!(search.row, Some(1));

        search.handle_events(
            &[
                key(egui::Key::Backspace, true),
                key(egui::Key::Backspace, true),
            ],
            &titles,
        );
        assert_eq!(search.buffer(), "");
    }

    #[test]
    fn a_space_uses_the_query_state_at_its_place_in_the_event_stream() {
        let titles = ["A B"]
            .iter()
            .map(|title| normalize_title(title))
            .collect::<Vec<_>>();
        let mut search = Search::default();
        let actions = search.handle_events(
            &[
                Event::Text("a".into()),
                key(egui::Key::Space, false),
                Event::Text(" ".into()),
                Event::Text("b".into()),
            ],
            &titles,
        );
        assert!(actions.is_empty());
        assert_eq!(search.buffer(), "a b");
        assert_eq!(search.row, Some(0));

        search.clear();
        let actions = search.handle_events(
            &[key(egui::Key::Space, false), Event::Text(" ".into())],
            &titles,
        );
        assert_eq!(actions, vec![InputAction::TogglePlay]);
        assert!(search.buffer().is_empty());
    }

    #[test]
    fn paste_and_ime_commits_type_into_the_search() {
        let titles = titles();
        let mut search = Search::default();
        search.handle_events(
            &[
                Event::Paste("bo".into()),
                Event::Ime(ImeEvent::Commit("h".into())),
            ],
            &titles,
        );
        assert_eq!(search.buffer(), "boh");
        assert_eq!(search.row, Some(0));
    }

    #[test]
    fn ime_preedit_keeps_an_existing_query_alive() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        search.handle_events(
            &[Event::Ime(ImeEvent::Preedit {
                text: "ぼ".into(),
                active_range_chars: None,
            })],
            &titles,
        );
        let composed_at = Instant::now();
        search.expire(composed_at + TIMEOUT * 10);
        assert_eq!(search.buffer(), "b");
        search.handle_events(&[Event::Ime(ImeEvent::Commit("o".into()))], &titles);
        assert_eq!(search.buffer(), "bo");
        let committed_at = Instant::now();
        search.expire(committed_at + TIMEOUT + Duration::from_millis(1));
        assert!(search.buffer().is_empty());
    }

    #[test]
    fn repeated_enter_is_ignored() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        let actions = search.handle_events(
            &[key(egui::Key::Enter, false), key(egui::Key::Enter, true)],
            &titles,
        );
        assert_eq!(
            actions,
            vec![InputAction::Play {
                row: 0,
                pointer_event_before: false,
            }]
        );
    }

    #[test]
    fn enter_remembers_whether_a_pointer_event_preceded_it() {
        let titles = titles();
        let pointer = |pressed| Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut search = Search::default();
        search.type_char('b', &titles);
        let actions =
            search.handle_events(&[pointer(false), key(egui::Key::Enter, false)], &titles);
        assert_eq!(
            actions,
            vec![InputAction::Play {
                row: 0,
                pointer_event_before: true,
            }]
        );
        let actions = search.handle_events(&[key(egui::Key::Enter, false), pointer(true)], &titles);
        assert_eq!(
            actions,
            vec![InputAction::Play {
                row: 0,
                pointer_event_before: false,
            }]
        );
    }

    #[test]
    fn a_moment_of_silence_clears_the_search() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        assert_eq!(search.row, Some(0));
        let typed_at = Instant::now();
        search.expire(typed_at + TIMEOUT / 2);
        assert_eq!(search.row, Some(0));
        search.expire(typed_at + TIMEOUT + Duration::from_millis(1));
        assert_eq!(search.buffer(), "");
        assert_eq!(search.row, None);
    }

    #[test]
    fn an_unmatched_search_waits_for_a_loading_page() {
        let mut search = Search::default();
        search.type_char('z', &[]);
        let waiting_at = Instant::now();
        search.set_waiting_for_results(true, waiting_at);
        search.expire(waiting_at + TIMEOUT * 10);
        assert_eq!(search.buffer(), "z");

        let arrived_at = waiting_at + TIMEOUT * 10;
        search.set_waiting_for_results(false, arrived_at);
        search.expire(arrived_at + TIMEOUT / 2);
        assert_eq!(search.buffer(), "z");
        search.expire(arrived_at + TIMEOUT + Duration::from_millis(1));
        assert!(search.buffer().is_empty());
    }

    #[test]
    fn escape_clears_the_search_at_once() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('b', &titles);
        search.clear();
        assert_eq!(search.buffer(), "");
        assert_eq!(search.row, None);
    }

    #[test]
    fn a_changed_list_revalidates_the_row() {
        let titles = titles();
        let mut search = Search::default();
        search.type_char('d', &titles);
        assert_eq!(search.row, Some(4));
        // The list shrinks and Despacito is gone.
        let shorter: Vec<_> = ["Bohemian Rhapsody", "Despacito"]
            .iter()
            .map(|title| normalize_title(title))
            .collect();
        search.revalidate(&shorter);
        assert_eq!(search.row, Some(1));
        // The match is gone entirely.
        let without: Vec<_> = ["Bohemian Rhapsody"]
            .iter()
            .map(|title| normalize_title(title))
            .collect();
        search.revalidate(&without);
        assert_eq!(search.row, None);
        assert_eq!(search.buffer(), "d");
    }

    #[test]
    fn escape_and_an_empty_list_are_safe() {
        let mut search = Search::default();
        search.revalidate(&[]);
        assert_eq!(search.row, None);
        search.type_char('b', &[]);
        assert_eq!(search.buffer(), "b");
        assert_eq!(search.row, None);
        search.backspace(&[]);
        assert_eq!(search.buffer(), "");
    }
}

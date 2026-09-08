//! System fallback fonts for scripts not covered by the interface font.
//!
//! Inter covers Latin, Greek, and Cyrillic. Bundling fonts for every other
//! script would greatly increase the binary size, so Fastpotify registers one
//! suitable fallback per script from what the desktop already carries.
//!
//! macOS answers this itself, in the language the user reads, and the
//! `mac_fonts` module asks it. That module is compiled only there, so this
//! names it in prose rather than linking to it. Linux and Windows offer
//! nothing equivalent, so there the fonts are read and ranked here.

#[cfg(not(target_os = "macos"))]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use skrifa::MetadataProvider as _;
#[cfg(not(target_os = "macos"))]
use skrifa::raw::TableProvider as _;

/// Registered font name, file bytes, and face index.
pub struct Fallback {
    pub name: String,
    pub bytes: Vec<u8>,
    pub index: u32,
}

/// Scripts Inter does not cover, with a probe character and family-name hint.
///
/// One face is selected per entry in this order. A face covering multiple
/// scripts is registered once. Glyphs are rasterized only when used.
///
/// The hint ranks a face by name where the fonts are ranked here. CoreText
/// needs only the probe.
pub(crate) const FALLBACK_SCRIPTS: &[(&str, char, &str)] = &[
    ("han", '\u{4e2d}', "cjk"),
    ("kana", '\u{3042}', "cjk"),
    ("hangul", '\u{d55c}', "cjk"),
    ("arabic", '\u{0627}', "arabic"),
    ("hebrew", '\u{05d0}', "hebrew"),
    ("thai", '\u{0e01}', "thai"),
    ("lao", '\u{0e81}', "lao"),
    ("khmer", '\u{1780}', "khmer"),
    ("myanmar", '\u{1000}', "myanmar"),
    ("devanagari", '\u{0915}', "devanagari"),
    ("bengali", '\u{0995}', "bengali"),
    ("gurmukhi", '\u{0a15}', "gurmukhi"),
    ("gujarati", '\u{0a95}', "gujarati"),
    ("tamil", '\u{0ba4}', "tamil"),
    ("telugu", '\u{0c15}', "telugu"),
    ("kannada", '\u{0c95}', "kannada"),
    ("malayalam", '\u{0d15}', "malayalam"),
    ("sinhala", '\u{0d9a}', "sinhala"),
    ("armenian", '\u{0531}', "armenian"),
    ("georgian", '\u{10d0}', "georgian"),
    ("ethiopic", '\u{1200}', "ethiopic"),
    ("cherokee", '\u{13a0}', "cherokee"),
    ("symbols", '\u{2605}', "symbol"),
];

/// The regional cut of a pan-CJK font a locale should be shown, longest
/// prefix first.
#[cfg(not(target_os = "macos"))]
const HAN_REGIONS: &[(&str, &str)] = &[
    ("zh_tw", "tc"),
    ("zh_hant", "tc"),
    ("zh_hk", "hk"),
    ("zh_mo", "hk"),
    ("zh", "sc"),
    ("ja", "jp"),
    ("ko", "kr"),
];

/// How deep to walk each font directory. Distributions nest a level or two
/// (`/usr/share/fonts/truetype/noto`); nothing legitimate goes deeper, and the
/// bound also ends any symlink loop.
const FONT_SCAN_DEPTH: usize = 4;

/// A collection says how many faces it holds, and a corrupt or hostile file
/// can say billions. No real one holds more than a few dozen.
pub(crate) const MAX_FACES: u32 = 64;

/// One system fallback face per unsupported script.
///
/// The answer is found once per process: `theme::install` runs again for
/// every window the app creates, and finding it costs a walk of every font
/// on the machine, or a question per script to CoreText.
pub fn fallbacks() -> &'static [Fallback] {
    static FONTS: OnceLock<Vec<Fallback>> = OnceLock::new();
    #[cfg(target_os = "macos")]
    return FONTS.get_or_init(crate::mac_fonts::load);
    #[cfg(not(target_os = "macos"))]
    return FONTS.get_or_init(scan);
}

/// The face Winamp's playlists were drawn in, or the nearest this desktop
/// carries: Arial, then the metric twins that stand in for it on Linux,
/// then a plain sans. `None` leaves it to the interface font.
pub fn pledit_face() -> Option<&'static Fallback> {
    static FACE: OnceLock<Option<Fallback>> = OnceLock::new();
    FACE.get_or_init(|| {
        find_family(&[
            "arial",
            "liberation sans",
            "arimo",
            "helvetica",
            "helvetica neue",
            "nimbus sans",
            "dejavu sans",
        ])
    })
    .as_ref()
}

/// The regular face of the first family in `wanted` that is installed.
fn find_family(wanted: &[&str]) -> Option<Fallback> {
    let mut best: Option<(usize, PathBuf, u32)> = None;
    for dir in font_dirs() {
        walk_fonts(&dir, 0, &mut |path| rank_family(path, wanted, &mut best));
    }
    let (_, path, index) = best?;
    let bytes = std::fs::read(&path).ok()?;
    log::debug!("playlist face: {} (face {index})", path.display());
    Some(Fallback {
        name: "pledit".to_string(),
        bytes,
        index,
    })
}

/// Visits every font file below `dir`.
fn walk_fonts(dir: &Path, depth: usize, visit: &mut dyn FnMut(&Path)) {
    if depth >= FONT_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
            walk_fonts(&path, depth + 1, visit);
        } else if is_font_file(&path) {
            visit(&path);
        }
    }
}

/// Keeps the file if it holds a regular face of a wanted family that ranks
/// above the one held so far.
fn rank_family(path: &Path, wanted: &[&str], best: &mut Option<(usize, PathBuf, u32)>) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    // Safety: as in `probe_file`, a read-only mapping that lives inside
    // this call.
    let Ok(map) = (unsafe { memmap2::Mmap::map(&file) }) else {
        return;
    };
    let faces: Vec<(u32, skrifa::FontRef)> = match skrifa::raw::FileRef::new(&map) {
        Ok(skrifa::raw::FileRef::Font(font)) => vec![(0, font)],
        Ok(skrifa::raw::FileRef::Collection(collection)) => (0..collection.len().min(MAX_FACES))
            .filter_map(|index| collection.get(index).ok().map(|font| (index, font)))
            .collect(),
        Err(_) => return,
    };
    for (index, font) in faces {
        let attributes = font.attributes();
        if attributes.style != skrifa::attribute::Style::Normal
            || !(350.0..=450.0).contains(&attributes.weight.value())
        {
            continue;
        }
        let family = font
            .localized_strings(skrifa::string::StringId::FAMILY_NAME)
            .english_or_first()
            .map(|name| name.to_string())
            .unwrap_or_default()
            .to_lowercase();
        let Some(rank) = wanted.iter().position(|name| *name == family) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(held, held_path, _)| (rank, path) < (*held, held_path.as_path()))
        {
            *best = Some((rank, path.to_path_buf(), index));
        }
    }
}

/// A face that covers a script, and how well it suits the interface.
#[cfg(not(target_os = "macos"))]
struct Candidate {
    /// Drawn for another Han region than the locale's. A Japanese face draws
    /// 中 and so passes the coverage probe, but every simplified character it
    /// lacks would come from whatever fallback follows, at that font's size
    /// and baseline, so it ranks below any face from the right region and
    /// serves only when it is all there is.
    foreign: bool,
    score: u32,
    path: PathBuf,
    index: u32,
}

/// Finds the best face for each of [`FALLBACK_SCRIPTS`] and reads the files
/// they live in.
///
/// Asking every installed font what it covers is the only question that
/// survives a distribution renaming its packages, and the answer comes from
/// the parser epaint already uses to rasterize the glyphs.
#[cfg(not(target_os = "macos"))]
fn scan() -> Vec<Fallback> {
    let han = han_region(&locale());
    let started = std::time::Instant::now();
    let mut best: BTreeMap<&str, Candidate> = BTreeMap::new();
    for dir in font_dirs() {
        probe_dir(&dir, 0, han, &mut best);
    }
    log::debug!(
        "probed the system fonts in {:.1} ms, {} of {} scripts covered",
        started.elapsed().as_secs_f32() * 1e3,
        best.len(),
        FALLBACK_SCRIPTS.len()
    );

    // A face that covers several scripts -- a pan-CJK collection covers three
    // of them alone -- is read and registered once, under the first script
    // that chose it.
    let mut fonts: Vec<Fallback> = Vec::new();
    let mut taken: Vec<(PathBuf, u32)> = Vec::new();
    for (script, _, _) in FALLBACK_SCRIPTS {
        let Some(candidate) = best.get(script) else {
            log::debug!("no fallback face covers {script}");
            continue;
        };
        if taken.contains(&(candidate.path.clone(), candidate.index)) {
            continue;
        }
        let bytes = match std::fs::read(&candidate.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("cannot read {}: {error}", candidate.path.display());
                continue;
            }
        };
        log::debug!(
            "{script} fallback: {} (face {})",
            candidate.path.display(),
            candidate.index
        );
        taken.push((candidate.path.clone(), candidate.index));
        fonts.push(Fallback {
            name: format!("fallback-{script}"),
            bytes,
            index: candidate.index,
        });
    }
    fonts
}

/// Probes every font file below `dir`, keeping the best face per script.
#[cfg(not(target_os = "macos"))]
fn probe_dir(dir: &Path, depth: usize, han: &str, best: &mut BTreeMap<&str, Candidate>) {
    if depth >= FONT_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // The kind comes back with the directory listing, so only a symlink
        // (Debian nests its font tree behind them, as does a Flatpak's
        // `/run/host/fonts`) costs a look at the target.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
            probe_dir(&path, depth + 1, han, best);
        } else if is_font_file(&path) {
            probe_file(&path, han, best);
        }
    }
}

/// Whether a path names a font this can open.
fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

/// Offers every face in one font file to every script that still wants one.
#[cfg(not(target_os = "macos"))]
fn probe_file(path: &Path, han: &str, best: &mut BTreeMap<&str, Candidate>) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    // Mapping the file rather than reading it keeps this to the few pages
    // holding each font's header, names, and character map: reading every
    // font on a normal Linux tree whole costs half a second, mapping them
    // costs a tenth of that.
    //
    // Safety: the mapping is read-only and lives inside this call. A font
    // file rewritten underneath it during that window would fault, which is
    // the same bet every font enumerator on the platform makes.
    let Ok(map) = (unsafe { memmap2::Mmap::map(&file) }) else {
        return;
    };
    let faces: Vec<(u32, skrifa::FontRef)> = match skrifa::raw::FileRef::new(&map) {
        Ok(skrifa::raw::FileRef::Font(font)) => vec![(0, font)],
        Ok(skrifa::raw::FileRef::Collection(collection)) => (0..collection.len().min(MAX_FACES))
            .filter_map(|index| collection.get(index).ok().map(|font| (index, font)))
            .collect(),
        Err(_) => return,
    };
    for (index, font) in faces {
        let attributes = font.attributes();
        if attributes.style != skrifa::attribute::Style::Normal {
            continue;
        }
        let family = font
            .localized_strings(skrifa::string::StringId::FAMILY_NAME)
            .english_or_first()
            .map(|name| name.to_string())
            .unwrap_or_default()
            .to_lowercase();
        let charmap = font.charmap();
        let outlines = font.outline_glyphs();
        // A face must draw the character, not merely map it: a bitmap or
        // colour-only font passes the charmap and renders nothing.
        let draws = |character: char| {
            charmap
                .map(character)
                .is_some_and(|glyph| outlines.get(glyph).is_some())
        };
        for (script, probe, hint) in FALLBACK_SCRIPTS {
            if !draws(*probe) {
                continue;
            }
            let score = face_score(&family, attributes.weight.value(), han, hint);
            let foreign = *script == "han" && {
                let code_pages = font.os2().ok().and_then(|os2| os2.ul_code_page_range_1());
                !covers_han_region(code_pages, han)
            };
            // Ties break on the path so two machines carrying the same fonts
            // resolve the same face, whatever order their directories list.
            if best.get(script).is_none_or(|held| {
                (foreign, score, path) < (held.foreign, held.score, held.path.as_path())
            }) {
                best.insert(
                    script,
                    Candidate {
                        foreign,
                        score,
                        path: path.to_path_buf(),
                        index,
                    },
                );
            }
        }
    }
}

/// Ranks a face as interface text for the script named by `hint`, lowest
/// first.
///
/// Serif, monospace, and display cuts lose to a plain sans, and anything far
/// from regular weight loses to something near it, so the fallback sits
/// beside Inter rather than shouting over it.
#[cfg(not(target_os = "macos"))]
fn face_score(family: &str, weight: f32, han: &str, hint: &str) -> u32 {
    let mut score = ((weight - 400.0).abs() / 25.0) as u32;
    // A face that names the script was drawn for it. Liberation Sans carries
    // enough Hebrew to pass the coverage test, but Noto Sans Hebrew is the
    // one a reader wants.
    if !family.contains(hint) {
        score += 25;
    }
    // A family that calls itself sans is asking to be read at interface size.
    // The rest of a Noto set are the specialised cuts -- Nastaliq for Urdu
    // poetry, Rashi for rabbinic commentary, Kufi for display -- which are
    // correct typography for a body of text and wrong for a track title.
    // Emoji and symbol faces do not necessarily include "sans" in their family names.
    if !family.contains("sans") && hint != "emoji" && hint != "symbol" {
        score += 50;
    }
    for (fragment, penalty) in [
        ("serif", 200),
        ("mono", 120),
        ("kufi", 80),
        ("naskh", 80),
        ("looped", 80),
        ("display", 80),
        ("condensed", 60),
        ("caption", 40),
    ] {
        if family.contains(fragment) {
            score += penalty;
        }
    }
    // Han characters are unified in Unicode, so 直 and 骨 have a Japanese
    // shape and a Chinese one and the font decides which a reader sees. A
    // pan-CJK family names its regional cut last ("Noto Sans CJK SC",
    // "PingFang TC"); prefer the cut this locale reads.
    if let Some(region) = family
        .rsplit(' ')
        .next()
        .filter(|region| han_code_page(region).is_some())
        && region != han
    {
        score += 40;
    }
    score
}

/// The OS/2 `ulCodePageRange1` bit a face sets to declare it covers a
/// region's legacy character set: Shift JIS, GB 2312, Wansung, or Big5.
#[cfg(not(target_os = "macos"))]
fn han_code_page(region: &str) -> Option<u32> {
    match region {
        "jp" => Some(17),
        "sc" => Some(18),
        "kr" => Some(19),
        "tc" | "hk" => Some(20),
        _ => None,
    }
}

/// Whether a face's declared code pages include the region's character set.
/// A face too old to declare any is taken at its word: none.
#[cfg(not(target_os = "macos"))]
fn covers_han_region(code_pages: Option<u32>, han: &str) -> bool {
    han_code_page(han)
        .zip(code_pages)
        .is_some_and(|(bit, pages)| pages & (1 << bit) != 0)
}

/// The user's locale, in the shape [`han_region`] matches, or an empty
/// string when none is set. A variable that is set but empty does not
/// count, as POSIX has it.
#[cfg(not(target_os = "macos"))]
fn locale() -> String {
    let named = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()));
    // A Windows desktop sets none of those, so without asking the system
    // every listener there would be shown the default cut, whatever they
    // read. The variables still come first, so setting one overrides it.
    #[cfg(windows)]
    let named = named.or_else(windows_language);
    normalise(named.unwrap_or_default())
}

/// A language name as [`HAN_REGIONS`] writes them: lowercase, with `_`
/// between the language and its region. Windows and BCP 47 hyphenate.
#[cfg(not(target_os = "macos"))]
fn normalise(name: String) -> String {
    name.to_lowercase().replace('-', "_")
}

/// The language a Windows desktop is read in.
///
/// The display language comes first, since that is what `LANG` names on the
/// other two platforms. A machine whose display language is not one of the
/// installed ones still has a user locale, which carries the region.
#[cfg(windows)]
fn windows_language() -> Option<String> {
    use windows_sys::Win32::Globalization::{
        GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME,
    };

    /// `LOCALE_NAME_MAX_LENGTH`, which windows-sys does not carry.
    const NAME_LENGTH: usize = 85;

    let mut names = [0u16; NAME_LENGTH * 8];
    let mut count = 0u32;
    let mut length = names.len() as u32;
    // SAFETY: both calls write at most `length` (or `len`) units into the
    // buffer they are handed, which is the buffer's own length.
    let preferred = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            names.as_mut_ptr(),
            &mut length,
        )
    };
    if preferred != 0
        && count > 0
        && let Some(first) = first_name(&names)
    {
        return Some(first);
    }

    let mut name = [0u16; NAME_LENGTH];
    let read = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    (read > 0).then(|| first_name(&name)).flatten()
}

/// The first string of a null-terminated list of them.
#[cfg(windows)]
fn first_name(names: &[u16]) -> Option<String> {
    let end = names.iter().position(|unit| *unit == 0)?;
    (end > 0).then(|| String::from_utf16_lossy(&names[..end]))
}

/// The pan-CJK cut a locale reads, defaulting to Simplified Chinese -- the
/// most widely read of them, and what a desktop that never set a locale gets.
#[cfg(not(target_os = "macos"))]
fn han_region(locale: &str) -> &'static str {
    HAN_REGIONS
        .iter()
        .find(|(prefix, _)| locale.starts_with(prefix))
        .map_or("sc", |(_, region)| *region)
}

/// Where the platform keeps installed fonts.
fn font_dirs() -> Vec<PathBuf> {
    let user = directories::UserDirs::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut add = |dir: PathBuf| {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    if cfg!(target_os = "macos") {
        add(PathBuf::from("/System/Library/Fonts"));
        add(PathBuf::from("/Library/Fonts"));
    } else if cfg!(target_os = "windows") {
        add(std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
            .join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            add(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
        }
    } else {
        // Every data directory fontconfig looks in, in its order, so a
        // distribution that keeps its fonts elsewhere (NixOS, Guix) is
        // covered along with the usual two.
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
        for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
            add(PathBuf::from(dir).join("fonts"));
        }
        // What a Flatpak sees of the host's fonts.
        add(PathBuf::from("/run/host/fonts"));
        // The pre-XDG per-user directory, which fontconfig still honours.
        if let Some(user) = &user {
            add(user.home_dir().join(".fonts"));
        }
    }
    // `~/Library/Fonts` on macOS and `$XDG_DATA_HOME/fonts` on Linux: where a
    // font the user installed by hand lands. Windows keeps none, and its
    // per-user store is the `LOCALAPPDATA` path above.
    if let Some(font_dir) = user.as_ref().and_then(|user| user.font_dir()) {
        add(font_dir.to_path_buf());
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_font_files_are_probed() {
        assert!(is_font_file(Path::new("/x/NotoSans.ttf")));
        assert!(is_font_file(Path::new("/x/NotoSansCJK.TTC")));
        assert!(is_font_file(Path::new("/x/PingFang.otf")));
        assert!(!is_font_file(Path::new("/x/fonts.dir")));
        assert!(!is_font_file(Path::new("/x/README")));
    }

    #[test]
    fn asking_the_system_never_panics() {
        // Whatever fonts this machine has, including none.
        let _ = fallbacks();
    }
}

/// The ranking that serves the platforms with no answer of their own.
#[cfg(all(test, not(target_os = "macos")))]
mod ranking_tests {
    use super::*;

    #[test]
    fn locales_choose_a_pan_cjk_cut() {
        assert_eq!(han_region("zh_cn.utf-8"), "sc");
        assert_eq!(han_region("zh_tw.utf-8"), "tc");
        assert_eq!(han_region("zh_hk.utf-8"), "hk");
        assert_eq!(han_region("ja_jp.utf-8"), "jp");
        assert_eq!(han_region("ko_kr.utf-8"), "kr");
        assert_eq!(han_region("en_us.utf-8"), "sc", "the default");
        assert_eq!(han_region(""), "sc", "no locale set");
    }

    /// Windows and BCP 47 write `ko-KR` where POSIX writes `ko_KR`, and the
    /// prefixes above are POSIX-shaped.
    #[test]
    fn hyphenated_language_names_choose_a_cut_too() {
        let cut = |name: &str| han_region(&normalise(name.to_owned()));
        assert_eq!(cut("ko-KR"), "kr");
        assert_eq!(cut("ja-JP"), "jp");
        assert_eq!(cut("zh-TW"), "tc");
        assert_eq!(cut("zh-Hant-TW"), "tc");
        assert_eq!(cut("zh-HK"), "hk");
        assert_eq!(cut("zh-CN"), "sc");
        assert_eq!(cut("en-US"), "sc", "the default");
    }

    /// A Windows desktop sets none of the POSIX variables, so the language
    /// has to come from the system. An empty answer puts every Japanese and
    /// Korean listener on the Simplified Chinese cut.
    #[cfg(windows)]
    #[test]
    fn windows_names_the_language_it_is_read_in() {
        let locale = locale();
        assert!(!locale.is_empty(), "Windows named no language");
        assert!(
            !locale.contains('-'),
            "{locale} keeps a hyphen no region prefix matches"
        );
        assert_eq!(locale, locale.to_lowercase());
    }

    #[test]
    fn a_face_declares_the_regions_it_covers() {
        // Hiragino Sans covers 中 but declares only Shift JIS.
        let japanese = Some(1 << 17);
        assert!(covers_han_region(japanese, "jp"));
        assert!(!covers_han_region(japanese, "sc"));
        assert!(!covers_han_region(None, "sc"), "no OS/2 table, no claim");
        for (_, region) in HAN_REGIONS {
            assert!(han_code_page(region).is_some(), "{region} has a code page");
        }
    }

    #[test]
    fn interface_faces_outrank_display_ones() {
        let sans = face_score("noto sans arabic", 400.0, "sc", "arabic");
        assert!(sans < face_score("noto naskh arabic", 400.0, "sc", "arabic"));
        assert!(sans < face_score("noto kufi arabic", 400.0, "sc", "arabic"));
        assert!(sans < face_score("noto nastaliq urdu", 400.0, "sc", "arabic"));
        assert!(sans < face_score("noto serif arabic", 400.0, "sc", "arabic"));
        assert!(sans < face_score("noto sans arabic", 700.0, "sc", "arabic"));
    }

    #[test]
    fn a_face_drawn_for_the_script_wins() {
        // Liberation Sans covers Hebrew, but Noto Sans Hebrew is drawn for it.
        assert!(
            face_score("noto sans hebrew", 400.0, "sc", "hebrew")
                < face_score("liberation sans", 400.0, "sc", "hebrew")
        );
    }

    #[test]
    fn the_locale_picks_between_regional_cuts() {
        let simplified = face_score("noto sans cjk sc", 400.0, "sc", "cjk");
        assert!(simplified < face_score("noto sans cjk jp", 400.0, "sc", "cjk"));
        assert_eq!(
            face_score("noto sans cjk tc", 400.0, "tc", "cjk"),
            simplified,
            "each locale ranks its own cut the same"
        );
    }

    #[test]
    fn symbols_faces_are_scored_fairly() {
        let symbol = face_score("noto sans symbols", 400.0, "sc", "symbol");
        let non_symbol = face_score("noto sans arabic", 400.0, "sc", "symbol");
        assert!(symbol < non_symbol);
    }
}

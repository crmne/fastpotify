//! The fallback faces macOS itself draws each script with.
//!
//! The alternative is to read every font on the machine and rank what is
//! found, which [`crate::system_fonts`] does on the other two platforms.
//! That ranking cannot be made correct here, for two reasons a scan cannot
//! see:
//!
//! * The face macOS uses for Han is `PingFang`, which no font directory
//!   holds. It ships as an on-demand asset under `/System/Library/AssetsV2`,
//!   beside dozens of optional download faces that are not interface faces
//!   and that nothing in the asset metadata separates it from. Adding that
//!   directory to the scan lets those in, and a Japanese or Korean desktop
//!   then draws its interface in a Taiwanese or a brush face.
//! * A face declares the regions it covers in its OS/2 code pages, and a
//!   face may declare all four while being drawn for one. Nothing in the
//!   file distinguishes the interface face a reader expects from a display
//!   face that merely covers the same characters.
//!
//! CoreText already holds the answer, per script and in the language the
//! user reads, and it is the answer every other application on the desktop
//! draws with.

use std::ffi::c_void;
use std::path::PathBuf;

use skrifa::MetadataProvider as _;

use crate::system_fonts::{FALLBACK_SCRIPTS, Fallback, MAX_FACES};

/// The face CoreText resolves for each script, read into memory.
///
/// A face serving several scripts is registered once, under the first script
/// that resolved to it: `PingFang` answers Han, kana, and the symbols probe
/// on a Chinese desktop.
pub fn load() -> Vec<Fallback> {
    let started = std::time::Instant::now();
    let mut fonts: Vec<Fallback> = Vec::new();
    let mut taken: Vec<(PathBuf, u32)> = Vec::new();
    for (script, probe, _) in FALLBACK_SCRIPTS {
        let candidates = resolve(*probe);
        if candidates.is_empty() {
            log::debug!("CoreText names no face for {script}");
            continue;
        }
        // The first answer is the one macOS draws with. The rest are its
        // cascade, in the order it would fall through them, and serve when
        // the first is a face epaint cannot rasterize.
        let found = candidates.iter().find_map(|(family, path)| {
            face_index(path, family, *probe).map(|index| (family, path, index))
        });
        let Some((family, path, index)) = found else {
            log::debug!("no face CoreText offers for {script} draws {probe} readably");
            continue;
        };
        if taken.contains(&(path.clone(), index)) {
            continue;
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("cannot read {}: {error}", path.display());
                continue;
            }
        };
        log::debug!(
            "{script} fallback: {family}, {} (face {index})",
            path.display()
        );
        taken.push((path.clone(), index));
        fonts.push(Fallback {
            name: format!("fallback-{script}"),
            bytes,
            index,
        });
    }
    log::debug!(
        "asked CoreText for {} scripts in {:.1} ms, {} faces registered",
        FALLBACK_SCRIPTS.len(),
        started.elapsed().as_secs_f32() * 1e3,
        fonts.len()
    );
    fonts
}

/// The family name and file CoreText draws `probe` with, in the language the
/// user reads: the face it draws `probe` with, then the cascade behind it.
///
/// Both start from a descriptor that names no family, so the answers are
/// public faces. Starting from `.AppleSystemUIFont` instead answers with the
/// private interface cuts, whose file is `PingFangUI.ttc`: it stores outlines
/// in Apple's `hvgl` table, which skrifa cannot rasterize.
///
/// The first answer is still sometimes one of those, depending on the macOS
/// version and the language the desktop is set to, so the cascade follows it.
/// It is the list macOS would itself fall through, and it holds the same
/// families in files that carry ordinary outlines.
fn resolve(probe: char) -> Vec<(String, PathBuf)> {
    let mut utf16 = [0u16; 2];
    let encoded = probe.encode_utf16(&mut utf16);
    let mut answers: Vec<(String, PathBuf)> = Vec::new();

    // SAFETY: every handle below is created here and released here. The
    // string borrows `encoded` only for the CFStringCreate call, which
    // copies. Descriptors read from the cascade array are owned by it, so
    // they are not released.
    unsafe {
        let empty = CFDictionaryCreate(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        if empty.is_null() {
            return answers;
        }
        let descriptor = CTFontDescriptorCreateWithAttributes(empty);
        CFRelease(empty);
        if descriptor.is_null() {
            return answers;
        }
        let base = CTFontCreateWithFontDescriptor(descriptor, 14.0, std::ptr::null());
        CFRelease(descriptor);
        if base.is_null() {
            return answers;
        }

        let text = CFStringCreateWithCharacters(
            std::ptr::null(),
            encoded.as_ptr(),
            encoded.len() as isize,
        );
        if !text.is_null() {
            let face = CTFontCreateForString(
                base,
                text,
                CFRange {
                    location: 0,
                    length: encoded.len() as isize,
                },
            );
            CFRelease(text);
            if !face.is_null() {
                let family = CTFontCopyFamilyName(face);
                let url = CTFontCopyAttribute(face, kCTFontURLAttribute);
                CFRelease(face);
                if let (Some(family), Some(path)) = (string_of(family), path_of(url)) {
                    answers.push((family, path));
                }
                if !family.is_null() {
                    CFRelease(family);
                }
                if !url.is_null() {
                    CFRelease(url);
                }
            }
        }

        let cascade = CTFontCopyDefaultCascadeListForLanguages(base, std::ptr::null());
        CFRelease(base);
        if !cascade.is_null() {
            for position in 0..CFArrayGetCount(cascade) {
                let entry = CFArrayGetValueAtIndex(cascade, position);
                if entry.is_null() {
                    continue;
                }
                // Owned by the array, so copied attributes are released and
                // the entry itself is not.
                let family = CTFontDescriptorCopyAttribute(entry, kCTFontFamilyNameAttribute);
                let url = CTFontDescriptorCopyAttribute(entry, kCTFontURLAttribute);
                if let (Some(family), Some(path)) = (string_of(family), path_of(url)) {
                    answers.push((family, path));
                }
                if !family.is_null() {
                    CFRelease(family);
                }
                if !url.is_null() {
                    CFRelease(url);
                }
            }
            CFRelease(cascade);
        }
    }
    answers
}

/// A `CFString` as a Rust string.
///
/// # Safety
///
/// `handle` is a `CFStringRef` or null, and stays alive for the call.
unsafe fn string_of(handle: *const c_void) -> Option<String> {
    if handle.is_null() {
        return None;
    }
    // A character can take three bytes in UTF-8 and one UTF-16 unit, plus
    // the terminator this asks for.
    let length = unsafe { CFStringGetLength(handle) };
    let mut buffer = vec![0u8; length as usize * 3 + 1];
    // SAFETY: the buffer is as long as the call is told it is.
    let copied = unsafe {
        CFStringGetCString(
            handle,
            buffer.as_mut_ptr().cast(),
            buffer.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if !copied {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0)?;
    buffer.truncate(end);
    String::from_utf8(buffer).ok()
}

/// A file `CFURL` as a path.
///
/// # Safety
///
/// `handle` is a `CFURLRef` or null, and stays alive for the call.
unsafe fn path_of(handle: *const c_void) -> Option<PathBuf> {
    if handle.is_null() {
        return None;
    }
    let mut buffer = [0u8; 1024];
    // SAFETY: the buffer is as long as the call is told it is.
    let filled = unsafe {
        CFURLGetFileSystemRepresentation(handle, true, buffer.as_mut_ptr(), buffer.len() as isize)
    };
    if !filled {
        return None;
    }
    let end = buffer.iter().position(|byte| *byte == 0)?;
    let text = std::str::from_utf8(&buffer[..end]).ok()?;
    Some(PathBuf::from(text))
}

/// Where `family` sits in `path`, preferring the face nearest regular weight.
///
/// CoreText names a family and a file; epaint needs the index of one face
/// within it. `PingFang.ttc` holds 24, six of them the requested family at
/// six weights, and the interface wants the regular cut.
///
/// The typographic family name is checked first. A family with more than the
/// four styles that name alone can carry writes the shared name there and a
/// per-style name in the family name: every face of `Mukta Mahee` is named
/// `MuktaMahee Regular` or `MuktaMahee Bold`, and CoreText answers with the
/// typographic name that none of them carries as its family name.
///
/// The face must also draw `probe`, not merely be named right. CoreText
/// answers for the whole system, including faces epaint cannot rasterize:
/// `PingFangUI.ttc` keeps its outlines in Apple's `hvgl` table, and every
/// face in it maps the character and draws nothing. Registering one would
/// put a blank where the glyph belongs, which is worse than the mixed
/// rendering this change removes.
fn face_index(path: &std::path::Path, family: &str, probe: char) -> Option<u32> {
    let file = std::fs::File::open(path).ok()?;
    // Safety: a read-only mapping that lives inside this call, as in
    // `system_fonts::probe_file`.
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let faces: Vec<(u32, skrifa::FontRef)> = match skrifa::raw::FileRef::new(&map) {
        Ok(skrifa::raw::FileRef::Font(font)) => vec![(0, font)],
        Ok(skrifa::raw::FileRef::Collection(collection)) => (0..collection.len().min(MAX_FACES))
            .filter_map(|index| collection.get(index).ok().map(|font| (index, font)))
            .collect(),
        Err(_) => return None,
    };
    faces
        .iter()
        .filter(|(_, font)| {
            let attributes = font.attributes();
            attributes.style == skrifa::attribute::Style::Normal
                && names(font, family)
                && draws(font, probe)
        })
        .min_by(|(_, one), (_, other)| {
            let distance =
                |font: &skrifa::FontRef| (font.attributes().weight.value() - 400.0).abs();
            distance(one).total_cmp(&distance(other))
        })
        .map(|(index, _)| *index)
}

/// Whether a face carries an outline for `character`, rather than only a
/// character map entry pointing at one.
fn draws(font: &skrifa::FontRef, character: char) -> bool {
    let outlines = font.outline_glyphs();
    font.charmap()
        .map(character)
        .is_some_and(|glyph| outlines.get(glyph).is_some())
}

/// Whether a face belongs to `family`, under either name it can be written.
fn names(font: &skrifa::FontRef, family: &str) -> bool {
    [
        skrifa::string::StringId::TYPOGRAPHIC_FAMILY_NAME,
        skrifa::string::StringId::FAMILY_NAME,
    ]
    .iter()
    .any(|id| {
        font.localized_strings(*id)
            .english_or_first()
            .is_some_and(|name| name.chars().eq(family.chars()))
    })
}

#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;

    fn CFRelease(handle: *const c_void);
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFStringCreateWithCharacters(
        allocator: *const c_void,
        characters: *const u16,
        length: isize,
    ) -> *const c_void;
    fn CFStringGetLength(string: *const c_void) -> isize;
    fn CFStringGetCString(
        string: *const c_void,
        buffer: *mut i8,
        size: isize,
        encoding: u32,
    ) -> bool;
    fn CFURLGetFileSystemRepresentation(
        url: *const c_void,
        resolve_against_base: bool,
        buffer: *mut u8,
        size: isize,
    ) -> bool;
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    static kCTFontURLAttribute: *const c_void;
    static kCTFontFamilyNameAttribute: *const c_void;

    fn CTFontDescriptorCreateWithAttributes(attributes: *const c_void) -> *const c_void;
    fn CTFontCreateWithFontDescriptor(
        descriptor: *const c_void,
        size: f64,
        matrix: *const c_void,
    ) -> *const c_void;
    /// The face the cascade of `font` draws the characters in `range` with.
    fn CTFontCreateForString(
        font: *const c_void,
        string: *const c_void,
        range: CFRange,
    ) -> *const c_void;
    fn CTFontCopyFamilyName(font: *const c_void) -> *const c_void;
    fn CTFontCopyAttribute(font: *const c_void, attribute: *const c_void) -> *const c_void;
    /// The faces macOS falls through behind `font`, in its own order.
    fn CTFontCopyDefaultCascadeListForLanguages(
        font: *const c_void,
        languages: *const c_void,
    ) -> *const c_void;
    fn CTFontDescriptorCopyAttribute(
        descriptor: *const c_void,
        attribute: *const c_void,
    ) -> *const c_void;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever fonts this Mac carries, and whatever language it is read in.
    #[test]
    fn every_script_resolves_to_a_readable_face() {
        let fonts = load();
        assert!(
            !fonts.is_empty(),
            "CoreText named no face for any of the {} scripts",
            FALLBACK_SCRIPTS.len()
        );
        for font in &fonts {
            assert!(!font.bytes.is_empty(), "{} is empty", font.name);
            assert!(
                font.index < MAX_FACES,
                "{} points past the faces a collection can hold",
                font.name
            );
        }
    }

    /// A face that maps the probe and draws nothing is refused, so `load`
    /// moves on to the cascade behind it.
    ///
    /// `PingFangUI.ttc` is the case that matters: CoreText answers with it
    /// on some macOS versions and language settings, every face in it keeps
    /// its outlines in Apple's `hvgl` table, and registering one would put a
    /// blank where every Han character belongs.
    #[test]
    fn a_face_that_draws_nothing_is_refused() {
        let private = std::path::Path::new(
            "/System/Library/PrivateFrameworks/FontServices.framework/Resources/Reserved/PingFangUI.ttc",
        );
        if !private.exists() {
            return; // Not on this macOS version.
        }
        for family in ["PingFang SC", "PingFang TC", "PingFang HK"] {
            assert_eq!(
                face_index(private, family, '\u{4e2d}'),
                None,
                "{family} in PingFangUI.ttc has hvgl outlines and must be refused"
            );
        }
    }

    /// The face chosen for Han draws whole titles, not just the probe.
    ///
    /// A face covering part of the script leaves the rest to the face behind
    /// it, and a title then mixes two of them, at two weights and two
    /// baselines. That is the reported fault, and `中` alone cannot catch it:
    /// every CJK face maps it.
    ///
    /// The characters are the traditional ones that first showed the split.
    /// Every face any Mac uses for running Han text draws them, including
    /// the Japanese one. A face that carries a few thousand ideographs for
    /// the hanja in Korean names is not such a face, and a Mac holding only
    /// that has nothing to draw these titles with, so the check does not
    /// apply to it.
    #[test]
    fn the_han_face_draws_the_characters_that_split() {
        let candidates = resolve('\u{4e2d}');
        // The same choice `load` makes, so this tests the face that ships.
        let Some((family, path, index)) = candidates.iter().find_map(|(family, path)| {
            face_index(path, family, '\u{4e2d}').map(|index| (family, path, index))
        }) else {
            return; // No Han face at all is not this test's business.
        };
        let bytes = std::fs::read(path).expect("CoreText named a readable file");
        let font = match skrifa::raw::FileRef::new(&bytes) {
            Ok(skrifa::raw::FileRef::Font(font)) => font,
            Ok(skrifa::raw::FileRef::Collection(collection)) => {
                collection.get(index).expect("the resolved face")
            }
            Err(error) => panic!("skrifa cannot read {}: {error}", path.display()),
        };
        let charmap = font.charmap();
        let outlines = font.outline_glyphs();
        // Enough of the script to run text in it. Every face macOS uses for
        // Han carries ten thousand ideographs or more; a Korean face carries
        // a few thousand, for the hanja in names, and is all a Mac with no
        // Chinese or Japanese font has to offer.
        const RUNNING_TEXT: usize = 10_000;
        let covered = (0x4e00..0xa000u32)
            .filter_map(char::from_u32)
            .filter(|character| charmap.map(*character).is_some())
            .count();
        if covered < RUNNING_TEXT {
            return; // Nothing here can run Han text at all.
        }
        let where_from = format!("{family}, face {index} of {}", path.display());
        for character in "個韓劇愛蘋樂園隊紅絕".chars() {
            let glyph = charmap
                .map(character)
                .unwrap_or_else(|| panic!("{where_from} does not map {character}"));
            assert!(
                outlines.get(glyph).is_some(),
                "{where_from} maps {character} but draws nothing"
            );
        }
    }
}

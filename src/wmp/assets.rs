//! The skin's files, looked up the way the player found them.
//!
//! A skin names its art by file name alone; the reader matches without
//! regard to case or folder, since Windows ran on a case-insensitive file
//! system and skins nest their files freely. When several files share a
//! name, the last one read wins, which is what unpacking a skin over
//! itself produced.
//!
//! Decoding is capped: skin art is hundreds of kilobytes, so an image
//! wider or taller than [`MAX_DIMENSION`] or larger in area than
//! [`MAX_PIXELS`] is refused rather than allocated. Crafted archives
//! claim sizes they do not have; the refusal keeps a claimed gigabyte
//! from becoming one.

use std::collections::HashMap;
use std::io::Cursor;

use crate::skin::Bitmap;

/// The final path component of an archive entry, which is what a skin
/// names its files by.
fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The widest or tallest image a skin may carry.
pub const MAX_DIMENSION: u32 = 8192;
/// The most pixels one image may cover.
pub const MAX_PIXELS: u64 = 1 << 24;

/// The files of one skin, keyed by lower-case base name.
#[derive(Clone, Debug, Default)]
pub struct Assets {
    files: HashMap<String, Vec<u8>>,
}

impl Assets {
    /// Takes the files, by lower-case base name: the path inside the
    /// archive counts for nothing. When a name repeats, the last copy
    /// wins.
    pub fn from_files<I, N>(files: I) -> Self
    where
        I: IntoIterator<Item = (N, Vec<u8>)>,
        N: AsRef<str>,
    {
        Self {
            files: files
                .into_iter()
                .map(|(name, bytes)| (base_name(name.as_ref()).to_ascii_lowercase(), bytes))
                .collect(),
        }
    }

    /// The file names the skin carries, in no particular order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The bytes of a file, by the name the skin wrote for it.
    pub fn bytes(&self, name: &str) -> Option<&[u8]> {
        self.files
            .get(&name.to_ascii_lowercase())
            .map(Vec::as_slice)
    }

    /// A file decoded as an image, whatever its name claimed to be.
    /// Unreadable and oversized images are `None`, as the player
    /// silently skipped them.
    pub fn bitmap(&self, name: &str) -> Option<Bitmap> {
        let bytes = self.bytes(name)?;
        let dimensions = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        let (width, height) = dimensions;
        if width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || u64::from(width) * u64::from(height) > MAX_PIXELS
        {
            log::warn!("skin image {name} is {width}x{height}, which is more than a skin needs");
            return None;
        }
        let image = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .decode()
            .ok()?;
        let image = image.into_rgba8();
        Some(Bitmap {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb(color));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn files_are_found_regardless_of_case_or_folder() {
        let assets = Assets::from_files([("Some Skin/TOOTHY_BASE.BMP", b"one".to_vec())]);
        assert_eq!(assets.bytes("toothy_base.bmp"), Some(b"one".as_slice()));
        assert_eq!(assets.bytes("TOOTHY_BASE.BMP"), Some(b"one".as_slice()));
        assert_eq!(assets.bytes("missing.bmp"), None);
    }

    #[test]
    fn the_last_copy_of_a_repeated_file_wins() {
        let assets = Assets::from_files([
            ("base.bmp", b"first".to_vec()),
            ("nested/base.bmp", b"second".to_vec()),
        ]);
        assert_eq!(assets.bytes("BASE.BMP"), Some(b"second".as_slice()));
    }

    #[test]
    fn images_decode_whatever_they_are_called() {
        let assets = Assets::from_files([("odd-name.img", png(7, 5, [1, 2, 3]))]);
        let bitmap = assets.bitmap("ODD-NAME.IMG").unwrap();
        assert_eq!((bitmap.width, bitmap.height), (7, 5));
        assert_eq!(bitmap.pixel(0, 0), Some([1, 2, 3, 255]));
    }

    #[test]
    fn an_oversized_claim_is_refused_and_so_is_rubbish() {
        let assets = Assets::from_files([
            ("huge.png", png(MAX_DIMENSION + 1, 1, [0, 0, 0])),
            ("wide.png", png(1, MAX_DIMENSION + 1, [0, 0, 0])),
            ("rubbish.png", b"this is not an image".to_vec()),
        ]);
        assert!(assets.bitmap("huge.png").is_none());
        assert!(assets.bitmap("wide.png").is_none());
        assert!(assets.bitmap("rubbish.png").is_none());
        assert!(assets.bitmap("absent.png").is_none());
    }

    #[test]
    fn a_just_undersized_image_still_decodes() {
        let assets = Assets::from_files([("ok.png", png(MAX_DIMENSION, 1, [4, 5, 6]))]);
        assert!(assets.bitmap("ok.png").is_some());
    }
}

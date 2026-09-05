//! Bounded preparation of user-selected playlist artwork, off the UI thread.

use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageDecoder, ImageFormat, ImageReader};

/// Spotify limits the complete Base64 request body to 256 KB.
pub const MAX_PAYLOAD: usize = 256_000;
const MAX_FILE: usize = 20 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Cover {
    pub jpeg: Arc<[u8]>,
    pub encoded: Arc<str>,
    pub uri: String,
}

#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub selection: Option<Cover>,
    pub request: Option<u64>,
    pub uploading: bool,
    pub error: Option<String>,
}

pub fn read(path: &Path) -> Result<Cover, String> {
    let file = std::fs::File::open(path)
        .map_err(|_| "Couldn't open that image. Check that it is still available.".to_string())?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err("Choose a JPEG or PNG file.".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Couldn't read that image. Try another file.".to_string())?;
    prepare(&bytes)
}

pub fn prepare(bytes: &[u8]) -> Result<Cover, String> {
    if bytes.len() > MAX_FILE {
        return Err("Choose an image smaller than 20 MB.".into());
    }
    let format = image::guess_format(bytes).map_err(|_| "Choose a JPEG or PNG image.")?;
    if !matches!(format, ImageFormat::Jpeg | ImageFormat::Png) {
        return Err("Choose a JPEG or PNG image.".into());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| {
        "Couldn't decode this image. Choose a valid JPEG or PNG no larger than 8192 pixels per side."
    })?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| "Couldn't decode this image. Try another JPEG or PNG.")?;
    image.apply_orientation(orientation);
    // Preserve the entire image and its aspect ratio. Flatten transparency
    // onto white because JPEG has no alpha channel.
    let mut pixels = image.thumbnail(640, 640).into_rgba8();
    for pixel in pixels.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
        pixel[3] = 255;
    }
    let original = image::DynamicImage::ImageRgba8(pixels);
    for size in [640, 480, 320] {
        let rgb = original.thumbnail(size, size).into_rgb8();
        for quality in [90, 80, 70, 60] {
            let mut jpeg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
                .encode_image(&rgb)
                .map_err(|_| "Couldn't encode this image. Try another file.")?;
            let encoded = STANDARD.encode(&jpeg);
            if encoded.len() <= MAX_PAYLOAD {
                use sha2::{Digest, Sha256};
                let uri = format!("bytes://playlist-cover-{:x}.jpg", Sha256::digest(&jpeg));
                return Ok(Cover {
                    jpeg: jpeg.into(),
                    encoded: encoded.into(),
                    uri,
                });
            }
        }
    }
    Err("This image is too detailed for Spotify's upload limit. Choose a smaller image.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_unsupported_and_oversized_files() {
        assert!(prepare(b"not an image").is_err());
        assert!(prepare(b"GIF89a").is_err());
        assert!(prepare(&vec![0; MAX_FILE + 1]).is_err());
    }

    #[test]
    fn jpeg_input_and_missing_file_are_handled() {
        let image = image::RgbImage::from_pixel(24, 24, image::Rgb([30, 50, 70]));
        let mut jpeg = Cursor::new(Vec::new());
        image.write_to(&mut jpeg, ImageFormat::Jpeg).unwrap();
        assert!(prepare(jpeg.get_ref()).is_ok());
        let missing = std::env::temp_dir().join(format!(
            "fastpotify-missing-cover-{}.jpg",
            std::process::id()
        ));
        assert!(read(&missing).unwrap_err().contains("Couldn't open"));
    }

    #[test]
    fn png_is_flattened_and_encoded_as_a_bounded_jpeg() {
        let image = image::RgbaImage::from_pixel(1200, 600, image::Rgba([0, 0, 0, 0]));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        let cover = prepare(png.get_ref()).unwrap();
        assert!(cover.encoded.len() <= MAX_PAYLOAD);
        assert_eq!(
            STANDARD.decode(cover.encoded.as_bytes()).unwrap(),
            &*cover.jpeg
        );
        let decoded = image::load_from_memory(&cover.jpeg).unwrap().into_rgb8();
        assert_eq!(decoded.dimensions(), (640, 320));
        assert!(
            decoded
                .pixels()
                .all(|pixel| pixel.0.iter().all(|value| *value >= 250))
        );
    }

    #[test]
    fn detailed_image_still_fits_the_encoded_payload_limit() {
        let mut state = 17_u32;
        let image = image::RgbImage::from_fn(800, 800, |_, _| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            image::Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8])
        });
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        let cover = prepare(png.get_ref()).unwrap();
        assert!(cover.encoded.len() <= MAX_PAYLOAD);
    }
}

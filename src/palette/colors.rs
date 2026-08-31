//! Hex colour parsing for theme files: `#RGB`, `#RGBA`, `#RRGGBB`,
//! `#RRGGBBAA`. The `#` is optional. Alpha defaults to opaque.

use egui::Color32;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid color '{0}'")]
pub struct ColorError(pub String);

/// Parses a hex colour string into a [`Color32`], alpha included.
pub fn parse_hex_color(text: &str) -> Result<Color32, ColorError> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    let invalid = || ColorError(text.to_string());

    let digit = |c: u8| -> Result<u8, ColorError> {
        (c as char).to_digit(16).map(|d| d as u8).ok_or_else(invalid)
    };
    let byte = |hi: u8, lo: u8| -> Result<u8, ColorError> { Ok(digit(hi)? * 16 + digit(lo)?) };
    // A single hex digit repeated, e.g. `#RGB`'s `f` standing for `ff`.
    let expand = |c: u8| -> Result<u8, ColorError> {
        let d = digit(c)?;
        Ok(d * 16 + d)
    };

    let bytes = hex.as_bytes();
    match bytes.len() {
        3 => Ok(Color32::from_rgb(
            expand(bytes[0])?,
            expand(bytes[1])?,
            expand(bytes[2])?,
        )),
        4 => Ok(Color32::from_rgba_unmultiplied(
            expand(bytes[0])?,
            expand(bytes[1])?,
            expand(bytes[2])?,
            expand(bytes[3])?,
        )),
        6 => Ok(Color32::from_rgb(
            byte(bytes[0], bytes[1])?,
            byte(bytes[2], bytes[3])?,
            byte(bytes[4], bytes[5])?,
        )),
        8 => Ok(Color32::from_rgba_unmultiplied(
            byte(bytes[0], bytes[1])?,
            byte(bytes[2], bytes[3])?,
            byte(bytes[4], bytes[5])?,
            byte(bytes[6], bytes[7])?,
        )),
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb() {
        assert_eq!(parse_hex_color("#ff0000"), Ok(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse_hex_color("00ff00"), Ok(Color32::from_rgb(0, 255, 0)));
    }

    #[test]
    fn parses_short_rgb() {
        assert_eq!(parse_hex_color("#f00"), Ok(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse_hex_color("#0f0"), Ok(Color32::from_rgb(0, 255, 0)));
    }

    #[test]
    fn parses_rgba_with_alpha() {
        assert_eq!(
            parse_hex_color("#ff000080"),
            Ok(Color32::from_rgba_unmultiplied(255, 0, 0, 0x80))
        );
    }

    #[test]
    fn parses_short_rgba() {
        assert_eq!(
            parse_hex_color("#f008"),
            Ok(Color32::from_rgba_unmultiplied(255, 0, 0, 0x88))
        );
    }

    #[test]
    fn rejects_invalid() {
        assert!(parse_hex_color("#zzz").is_err());
        assert!(parse_hex_color("#ff00").is_ok()); // 4 digits: RGBA short
        assert!(parse_hex_color("#ff0").is_ok()); // 3 digits: RGB short
        assert!(parse_hex_color("#12345").is_err()); // 5 digits: no valid form
        assert!(parse_hex_color("").is_err());
    }
}

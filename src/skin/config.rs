//! The two text files in a skin: `pledit.txt` and `viscolor.txt`.
//!
//! Both are forgiving formats in practice. Colours arrive with or without a
//! `#`, sometimes with too many digits; the visualiser's list comes with
//! trailing commas, tabs, and comments after each line. Anything that cannot
//! be read keeps its default, as Winamp did.

pub type Rgb = [u8; 3];

/// A window's shape: which of its pixels are the window at all. Whatever
/// a skin's `region.txt` leaves out is see-through, and clicks there go
/// nowhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    /// The visible spans of each row, as `start..end`.
    rows: Vec<Vec<(u32, u32)>>,
}

impl Mask {
    /// The pixels inside any of the polygons, filled the even-odd way at
    /// the pixels' centres.
    pub fn from_polygons(width: u32, height: u32, polygons: &[Vec<(i32, i32)>]) -> Self {
        let mut rows = Vec::with_capacity(height as usize);
        for y in 0..height {
            let mut inside = vec![false; width as usize];
            let centre_y = y as f64 + 0.5;
            for polygon in polygons.iter().filter(|polygon| polygon.len() >= 3) {
                let mut crossings = Vec::new();
                for (index, &(x0, y0)) in polygon.iter().enumerate() {
                    let (x1, y1) = polygon[(index + 1) % polygon.len()];
                    let (x0, y0, x1, y1) =
                        (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1));
                    if y0 == y1 || centre_y < y0.min(y1) || centre_y >= y0.max(y1) {
                        continue;
                    }
                    crossings.push(x0 + (centre_y - y0) * (x1 - x0) / (y1 - y0));
                }
                crossings.sort_by(|a, b| a.total_cmp(b));
                for pair in crossings.as_chunks::<2>().0 {
                    let start = ((pair[0] - 0.5).ceil().max(0.0)) as usize;
                    let end = ((pair[1] - 0.5).ceil().max(0.0) as usize).min(width as usize);
                    for pixel in &mut inside[start.min(end)..end] {
                        *pixel = true;
                    }
                }
            }
            let mut spans = Vec::new();
            let mut start = None;
            for (x, visible) in inside.iter().enumerate() {
                match (start, visible) {
                    (None, true) => start = Some(x as u32),
                    (Some(from), false) => {
                        spans.push((from, x as u32));
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(from) = start {
                spans.push((from, width));
            }
            rows.push(spans);
        }
        Self {
            width,
            height,
            rows,
        }
    }

    pub fn spans(&self, y: u32) -> &[(u32, u32)] {
        self.rows.get(y as usize).map_or(&[], Vec::as_slice)
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        self.spans(y)
            .iter()
            .any(|(start, end)| x >= *start && x < *end)
    }

    /// Whether the shape is the whole rectangle, so masking is pointless.
    pub fn is_everything(&self) -> bool {
        self.rows.iter().all(|spans| spans == &[(0, self.width)])
    }
}

/// The shapes a skin gives its windows, by window. `None` is the whole
/// rectangle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Regions {
    pub normal: Option<Mask>,
    pub shade: Option<Mask>,
    pub equalizer: Option<Mask>,
    pub equalizer_shade: Option<Mask>,
}

/// Reads `region.txt`: sections named for the windows, each with a
/// `NumPoints` list of polygon sizes and a `PointList` of coordinates.
/// Skin tools were loose with it, so a polygon short of points is
/// dropped and a section that covers everything is treated as absent.
pub fn parse_regions(text: &str) -> Regions {
    let mut regions = Regions::default();
    let mut section = String::new();
    let mut num_points: Vec<usize> = Vec::new();
    let mut points: Vec<i32> = Vec::new();
    let mut finish = |section: &str, num_points: &[usize], points: &[i32]| {
        if num_points.is_empty() || points.is_empty() {
            return;
        }
        let (width, height, slot) = match section {
            "normal" => (275, 116, &mut regions.normal),
            "windowshade" => (275, 14, &mut regions.shade),
            "equalizer" => (275, 116, &mut regions.equalizer),
            "equalizerws" => (275, 14, &mut regions.equalizer_shade),
            _ => return,
        };
        let pairs: Vec<(i32, i32)> = points
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| (pair[0], pair[1]))
            .collect();
        let mut polygons = Vec::new();
        let mut at = 0;
        for count in num_points {
            let polygon: Vec<(i32, i32)> = pairs.iter().skip(at).take(*count).copied().collect();
            at += count;
            if polygon.len() == *count && *count >= 3 {
                polygons.push(polygon);
            }
        }
        let mask = Mask::from_polygons(width, height, &polygons);
        if !mask.is_everything() {
            *slot = Some(mask);
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[') {
            finish(&section, &num_points, &points);
            section = name.trim_end_matches(']').trim().to_ascii_lowercase();
            num_points.clear();
            points.clear();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let numbers = value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|field| field.trim().parse::<i32>().ok());
        match key.trim().to_ascii_lowercase().as_str() {
            "numpoints" => num_points.extend(numbers.map(|n| n.max(0) as usize)),
            "pointlist" => points.extend(numbers),
            _ => {}
        }
    }
    finish(&section, &num_points, &points);
    regions
}

/// Colours and font for the playlist editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistStyle {
    pub normal: Rgb,
    pub current: Rgb,
    pub normal_background: Rgb,
    pub selected_background: Rgb,
    pub font: String,
}

impl Default for PlaylistStyle {
    fn default() -> Self {
        Self {
            normal: [0x00, 0xff, 0x00],
            current: [0xff, 0xff, 0xff],
            normal_background: [0x00, 0x00, 0x00],
            selected_background: [0x00, 0x00, 0xff],
            font: "Arial".to_string(),
        }
    }
}

impl PlaylistStyle {
    /// Reads `pledit.txt`: an INI file with a `[Text]` section.
    pub fn parse(text: &str) -> Self {
        let mut style = Self::default();
        let mut in_text = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(section) = line.strip_prefix('[') {
                in_text = section
                    .strip_suffix(']')
                    .unwrap_or(section)
                    .trim()
                    .eq_ignore_ascii_case("text");
                continue;
            }
            if !in_text {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let key = key.trim().to_ascii_lowercase();
            match key.as_str() {
                "normal" => style.normal = hex(value).unwrap_or(style.normal),
                "current" => style.current = hex(value).unwrap_or(style.current),
                "normalbg" => {
                    style.normal_background = hex(value).unwrap_or(style.normal_background)
                }
                "selectedbg" => {
                    style.selected_background = hex(value).unwrap_or(style.selected_background)
                }
                "font" if !value.is_empty() => style.font = value.to_string(),
                _ => {}
            }
        }
        style
    }
}

/// A `#RRGGBB` colour. Winamp accepted the hash missing and extra digits
/// after the six, so both are taken here. Anything but six hex digits at
/// the front is no colour, a multi-byte character included: this reads
/// bytes, since slicing the text two bytes at a time would panic inside
/// one.
pub fn hex(value: &str) -> Option<Rgb> {
    let digits = value.trim().trim_start_matches('#').as_bytes();
    let nibble = |at: usize| digits.get(at).and_then(|byte| (*byte as char).to_digit(16));
    let channel = |at: usize| Some((nibble(at)? << 4 | nibble(at + 1)?) as u8);
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// The visualiser's palette: background, dot grid, 16 spectrum bands from
/// the top down, then five colours for the oscilloscope and one for the
/// analyser's peaks.
pub type VisColors = [Rgb; 24];

/// A palette that suits nothing in particular, for skins without one.
pub const DEFAULT_VIS_COLORS: VisColors = [
    [0, 0, 0],
    [24, 33, 41],
    [239, 49, 16],
    [206, 41, 16],
    [214, 90, 0],
    [214, 102, 0],
    [214, 115, 0],
    [198, 123, 8],
    [222, 165, 24],
    [214, 181, 33],
    [189, 222, 41],
    [148, 222, 33],
    [41, 206, 16],
    [50, 190, 16],
    [57, 181, 16],
    [49, 156, 8],
    [41, 148, 0],
    [24, 132, 8],
    [255, 255, 255],
    [214, 214, 222],
    [181, 189, 189],
    [160, 170, 175],
    [148, 156, 165],
    [150, 150, 150],
];

/// Reads `viscolor.txt`: one `r,g,b` per line, in order. Missing lines keep
/// the default colour at that position.
pub fn parse_vis_colors(text: &str) -> VisColors {
    let mut colors = DEFAULT_VIS_COLORS;
    let parsed = text.lines().filter_map(rgb_line).take(colors.len());
    for (slot, color) in colors.iter_mut().zip(parsed) {
        *slot = color;
    }
    colors
}

/// The leading `r,g,b` of a line, ignoring whatever follows it.
fn rgb_line(line: &str) -> Option<Rgb> {
    let mut channels = line
        .split(',')
        .map(str::trim)
        .map(|field| {
            field
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("")
        })
        .map(|digits| digits.parse::<u32>().ok().map(|value| value.min(255) as u8));
    Some([channels.next()??, channels.next()??, channels.next()??])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typical_pledit_file_is_read() {
        let style = PlaylistStyle::parse(
            "[Text]\r\nNormal=#A0B0C0\r\nCurrent=#FFFFFF\r\nNormalBG=#101010\r\nSelectedBG=#0000FF\r\nFont=Tahoma\r\n",
        );
        assert_eq!(style.normal, [0xa0, 0xb0, 0xc0]);
        assert_eq!(style.normal_background, [0x10, 0x10, 0x10]);
        assert_eq!(style.font, "Tahoma");
    }

    #[test]
    fn comments_missing_hashes_and_long_values_are_tolerated() {
        let style = PlaylistStyle::parse(
            "; Do not change anything!\n[text]\nnormal=00ff00\ncurrent = #FFFFFF00\nnormalbg=#zz0000\n",
        );
        assert_eq!(style.normal, [0x00, 0xff, 0x00]);
        assert_eq!(style.current, [0xff, 0xff, 0xff]);
        assert_eq!(
            style.normal_background,
            PlaylistStyle::default().normal_background
        );
    }

    #[test]
    fn a_colour_with_a_multibyte_character_is_no_colour() {
        // A byte no UTF-8 decoder accepts, such as a Cyrillic А typed for a
        // Latin A in a Windows-1251 file, comes through the lossy decode as
        // a three-byte replacement character. Slicing two bytes into it
        // panicked the skin loader.
        let style = PlaylistStyle::parse(&String::from_utf8_lossy(
            b"[Text]\nNormal=#\xC00B0C0\nCurrent=#FFFFFF\n",
        ));
        assert_eq!(style.normal, PlaylistStyle::default().normal);
        assert_eq!(style.current, [0xff, 0xff, 0xff]);
        assert_eq!(hex("ab\u{fffd}c"), None);
        assert_eq!(hex("#0\u{0410}0B0C"), None);
        assert_eq!(hex("\u{fffd}"), None);
    }

    #[test]
    fn keys_outside_the_text_section_are_ignored() {
        let style = PlaylistStyle::parse("[Other]\nNormal=#123456\n");
        assert_eq!(style, PlaylistStyle::default());
    }

    #[test]
    fn vis_colours_survive_comments_tabs_and_trailing_commas() {
        let colors = parse_vis_colors(
            "38,15,58         // 0\n0,0,0,\n255,255,255\t// 2 = peak\n  7 , 8 , 9 ,// spaced\n",
        );
        assert_eq!(colors[0], [38, 15, 58]);
        assert_eq!(colors[1], [0, 0, 0]);
        assert_eq!(colors[2], [255, 255, 255]);
        assert_eq!(colors[3], [7, 8, 9]);
        assert_eq!(colors[4], DEFAULT_VIS_COLORS[4]);
    }

    #[test]
    fn a_region_cuts_the_corners_and_a_full_one_is_no_region() {
        let text = "; a comment\n[Normal]\nNumPoints=5\nPointList=10,0,275,0,275,116,0,116,0,10\n[WindowShade]\nNumPoints=4\nPointList=0,0,275,0,275,14,0,14\n";
        let regions = parse_regions(text);
        let normal = regions.normal.expect("the cut corner makes a region");
        assert_eq!((normal.width, normal.height), (275, 116));
        assert!(!normal.contains(0, 0));
        assert!(normal.contains(20, 0));
        assert!(normal.contains(0, 20));
        assert!(normal.contains(274, 115));
        // The cut runs from (0,10) to (10,0); at the first row's centre it
        // crosses at 9.5, and the pixel whose centre sits on the edge is in.
        assert_eq!(normal.spans(0), &[(9, 275)]);
        assert!(regions.shade.is_none(), "a whole rectangle is no region");
        assert!(regions.equalizer.is_none());
    }

    #[test]
    fn many_small_polygons_add_up_and_short_ones_are_dropped() {
        let text = "[Normal]\nNumPoints=4,4,2\nPointList=0,0,10,0,10,10,0,10, 20,0 30,0 30,10 20,10, 5,5,6,6\n";
        let normal = parse_regions(text).normal.unwrap();
        assert_eq!(normal.spans(5), &[(0, 10), (20, 30)]);
        assert_eq!(normal.spans(50), &[]);
    }

    #[test]
    fn lines_without_three_numbers_are_skipped_not_misread() {
        let colors = parse_vis_colors("; palette\n\n300,1,2\n");
        assert_eq!(colors[0], [255, 1, 2]);
        assert_eq!(colors[1], DEFAULT_VIS_COLORS[1]);
    }
}

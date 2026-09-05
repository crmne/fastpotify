//! A forgiving reader for the XML-flavoured `.wms` skin definitions.
//!
//! Real skins are close to XML but not of it: attributes repeat, elements
//! close out of order, files come in UTF-16 or Windows-1252, and the
//! declaration is missing. Microsoft's own samples mis-nest tags. So this
//! reader keeps what a skin means and forgives how it is written: the
//! first copy of a repeated attribute wins (as a browser would take it),
//! a close tag for an element further out closes the ones it skips past,
//! an unmatched close tag is ignored, and text content is dropped, since
//! every value a skin carries is an attribute. Entities are decoded, and
//! a skin may not nest deeper than [`MAX_DEPTH`] or carry more than
//! [`MAX_NODES`] elements.

use thiserror::Error;

/// How deeply elements may nest. Real skins stay under ten.
pub const MAX_DEPTH: usize = 64;
/// How many elements a skin may hold at all.
pub const MAX_NODES: usize = 200_000;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("no elements were found")]
    Empty,
    #[error("the skin nests more than {MAX_DEPTH} levels deep")]
    TooDeep,
    #[error("the skin holds more than {MAX_NODES} elements")]
    TooLarge,
}

/// One element: its lower-cased name, its attributes in the order written
/// (keys lower-cased, the first copy of a repeated key kept), and its
/// children in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Node {
    /// The value of an attribute, by its lower-cased name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Reads the skin's bytes into its element tree. The encoding comes from
/// the byte-order mark when there is one, and otherwise from trying UTF-8
/// and falling back to Windows-1252.
pub fn parse(bytes: &[u8]) -> Result<Vec<Node>, ParseError> {
    let text = decode(bytes);
    Parser {
        src: text.as_bytes(),
        pos: 0,
        nodes: 0,
    }
    .document()
}

/// Decodes a skin definition to text. The byte-order mark decides between
/// UTF-16 and the rest; without one, UTF-8 is tried and Windows-1252 (the
/// code page skins were written against) takes what it rejects.
pub fn decode(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return utf16(&bytes[2..], true);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return utf16(&bytes[2..], false);
    }
    let rest = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    match std::str::from_utf8(rest) {
        Ok(text) => text.to_string(),
        Err(_) => rest.iter().map(|&byte| cp1252(byte)).collect(),
    }
}

fn utf16(bytes: &[u8], little_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes(*pair)
            } else {
                u16::from_be_bytes(*pair)
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// One Windows-1252 byte as a character. The 0x80–0x9F range is where it
/// leaves Latin-1; the table's gaps are the code page's own.
fn cp1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[byte as usize - 0x80],
        _ => byte as char,
    }
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    nodes: usize,
}

impl<'a> Parser<'a> {
    /// Reads every top-level element. Stray text and the skin's prologue
    /// are skipped; what is left is the `THEME` and whatever else survived.
    fn document(mut self) -> Result<Vec<Node>, ParseError> {
        let mut stack: Vec<Node> = Vec::new();
        let mut roots: Vec<Node> = Vec::new();
        while let Some(at) = self.find(b'<') {
            self.pos = at + 1;
            match self.rest_prefix() {
                _ if self.starts_with(b"!--") => self.skip_to(b"-->"),
                _ if self.starts_with(b"![CDATA[") => self.skip_to(b"]]>"),
                _ if self.starts_with(b"!") || self.starts_with(b"?") => self.skip_past(b'>'),
                _ if self.starts_with(b"/") => {
                    self.pos += 1;
                    let name = self.read_name();
                    self.skip_past(b'>');
                    self.close(&mut stack, &mut roots, &name);
                }
                _ => self.open(&mut stack, &mut roots)?,
            }
        }
        // Whatever was left open at the end still counts, outermost last.
        while let Some(node) = stack.pop() {
            attach(&mut stack, &mut roots, node);
        }
        if roots.is_empty() {
            return Err(ParseError::Empty);
        }
        Ok(roots)
    }

    /// Reads an opening tag with its attributes and puts it on the stack,
    /// or straight into the tree when it closes itself.
    fn open(&mut self, stack: &mut Vec<Node>, roots: &mut Vec<Node>) -> Result<(), ParseError> {
        let name = self.read_name();
        let mut attrs: Vec<(String, String)> = Vec::new();
        let self_closing = loop {
            self.skip_ws();
            match self.src.get(self.pos) {
                None => break false,
                Some(b'>') => {
                    self.pos += 1;
                    break false;
                }
                Some(b'/') if self.src.get(self.pos + 1) == Some(&b'>') => {
                    self.pos += 2;
                    break true;
                }
                _ => {
                    let Some((key, value)) = self.read_attr() else {
                        // Something unreadable where an attribute belongs;
                        // step over one byte and look again.
                        self.pos += 1;
                        continue;
                    };
                    if !attrs.iter().any(|(kept, _)| *kept == key) {
                        attrs.push((key, value));
                    }
                }
            }
        };
        if name.is_empty() {
            return Ok(());
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(ParseError::TooLarge);
        }
        let node = Node {
            name,
            attrs,
            children: Vec::new(),
        };
        if self_closing {
            attach(stack, roots, node);
        } else {
            if stack.len() >= MAX_DEPTH {
                return Err(ParseError::TooDeep);
            }
            stack.push(node);
        }
        Ok(())
    }

    /// Closes the named element, closing anything inside it that was left
    /// open on the way. A close tag with no match anywhere is dropped.
    fn close(&mut self, stack: &mut Vec<Node>, roots: &mut Vec<Node>, name: &str) {
        let Some(depth) = stack.iter().rposition(|node| node.name == name) else {
            return;
        };
        while stack.len() > depth {
            if let Some(node) = stack.pop() {
                attach(stack, roots, node);
            }
        }
    }

    /// Reads an attribute: a name, then `= value` when the value is there.
    /// Quotes may be single, double, or absent.
    fn read_attr(&mut self) -> Option<(String, String)> {
        let start = self.pos;
        while self
            .src
            .get(self.pos)
            .is_some_and(|byte| !b" \t\r\n=/>".contains(byte))
        {
            self.pos += 1;
        }
        let name = std::str::from_utf8(&self.src[start..self.pos])
            .ok()?
            .to_ascii_lowercase();
        if name.is_empty() {
            return None;
        }
        self.skip_ws();
        if self.src.get(self.pos) != Some(&b'=') {
            return Some((name, String::new()));
        }
        self.pos += 1;
        self.skip_ws();
        let quote = match self.src.get(self.pos) {
            Some(b'"') | Some(b'\'') => self.src[self.pos],
            Some(b'>') | Some(b'/') => return Some((name, String::new())),
            _ => 0,
        };
        let value = if quote == 0 {
            let start = self.pos;
            while self
                .src
                .get(self.pos)
                .is_some_and(|byte| !b" \t\r\n>".contains(byte))
            {
                self.pos += 1;
            }
            self.text(start, self.pos)
        } else {
            self.pos += 1;
            let start = self.pos;
            while self.src.get(self.pos).is_some_and(|&byte| byte != quote) {
                self.pos += 1;
            }
            let value = self.text(start, self.pos.min(self.src.len()));
            self.pos += usize::from(self.src.get(self.pos) == Some(&quote));
            value
        };
        Some((name, value))
    }

    /// The element or attribute name at the cursor, lower-cased.
    fn read_name(&mut self) -> String {
        let start = self.pos;
        while self
            .src
            .get(self.pos)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || b"_:.-".contains(byte))
        {
            self.pos += 1;
        }
        std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    fn rest_prefix(&self) -> &[u8] {
        &self.src[self.pos..(self.pos + 8).min(self.src.len())]
    }

    fn starts_with(&self, prefix: &[u8]) -> bool {
        self.rest_prefix().starts_with(prefix)
    }

    /// The text between two byte offsets, with entities resolved.
    fn text(&self, start: usize, end: usize) -> String {
        let raw = std::str::from_utf8(&self.src[start..end]).unwrap_or_default();
        decode_entities(raw)
    }

    fn skip_ws(&mut self) {
        while self.src.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }

    /// The offset of the next occurrence of `byte`, if any.
    fn find(&mut self, byte: u8) -> Option<usize> {
        let at = self.src[self.pos.min(self.src.len())..]
            .iter()
            .position(|&seen| seen == byte)?
            + self.pos;
        Some(at)
    }

    /// Steps to just after `marker`, or to the end when it never comes.
    fn skip_to(&mut self, marker: &[u8]) {
        let rest = &self.src[self.pos.min(self.src.len())..];
        self.pos = match rest
            .windows(marker.len())
            .position(|window| window == marker)
        {
            Some(at) => self.pos + at + marker.len(),
            None => self.src.len(),
        };
    }

    /// Steps to just after the next `byte`, or to the end.
    fn skip_past(&mut self, byte: u8) {
        if let Some(at) = self.find(byte) {
            self.pos = at + 1;
        } else {
            self.pos = self.src.len();
        }
    }
}

/// Puts a finished node into its parent's children, or among the roots.
fn attach(stack: &mut [Node], roots: &mut Vec<Node>, node: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => roots.push(node),
    }
}

/// Resolves the entities skins use; an unknown one is kept as written.
fn decode_entities(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail.find(';').unwrap_or(0);
        let decoded = if end > 1 {
            let entity = &tail[1..end];
            match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .is_some_and(|hex| {
                        u32::from_str_radix(hex, 16)
                            .ok()
                            .map(char::from_u32)
                            .is_some()
                    }) =>
                {
                    char::from_u32(u32::from_str_radix(&entity[2..], 16).unwrap_or(0))
                }
                _ if entity
                    .strip_prefix('#')
                    .is_some_and(|digits| digits.parse::<u32>().is_ok()) =>
                {
                    char::from_u32(entity[1..].parse().unwrap_or(u32::MAX))
                }
                _ => None,
            }
        } else {
            None
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Node {
        let roots = parse(text.as_bytes()).unwrap();
        assert_eq!(roots.len(), 1, "expected a single root");
        roots.into_iter().next().unwrap()
    }

    #[test]
    fn a_plain_skin_tree_is_read() {
        let root = one(
            "<THEME author=\"Microsoft\"><VIEW width=\"586\" height='335' bare/>\
             <!-- a note --><VIEW/></THEME>",
        );
        assert_eq!(root.name, "theme");
        assert_eq!(root.attr("author"), Some("Microsoft"));
        assert_eq!(root.children.len(), 2);
        let first = &root.children[0];
        assert_eq!(first.name, "view");
        assert_eq!(first.attr("width"), Some("586"));
        assert_eq!(first.attr("height"), Some("335"));
        assert_eq!(first.attr("bare"), Some(""));
        assert!(first.children.is_empty());
    }

    #[test]
    fn multiline_tags_and_odd_whitespace_are_read() {
        let root = one("<theme\n  a = 'one'\n  b\n  ><view\n/>x</theme>");
        assert_eq!(root.attr("a"), Some("one"));
        assert_eq!(root.attr("b"), Some(""));
        assert_eq!(root.children.len(), 1);
    }

    #[test]
    fn the_first_copy_of_a_repeated_attribute_wins() {
        let root = one("<view width=\"10\" width=\"20\"/>");
        assert_eq!(root.attr("width"), Some("10"));
    }

    #[test]
    fn a_close_tag_for_an_outer_element_closes_the_ones_it_skips() {
        // The mis-nesting real skins carry: `video` closed under `subview`.
        let roots = parse(b"<theme><view><subview><video></subview></view></theme>").unwrap();
        let theme = &roots[0];
        let view = &theme.children[0];
        assert_eq!(view.name, "view");
        assert_eq!(view.children.len(), 1);
        let subview = &view.children[0];
        assert_eq!(subview.name, "subview");
        let video = &subview.children[0];
        assert_eq!(video.name, "video");
        assert!(video.children.is_empty());
    }

    #[test]
    fn an_unmatched_close_tag_is_ignored() {
        let root = one("<theme></nothing><view/></theme>");
        assert_eq!(root.children.len(), 1);
    }

    #[test]
    fn text_between_tags_is_dropped_but_entities_in_attributes_are_resolved() {
        let root = one("<view label=\"Tom &amp; Jerry\" num=\"&#65;&#x42;\" \
             half=\"&unknown; & unclosed\"><![CDATA[ignored]]></view>");
        assert_eq!(root.attr("label"), Some("Tom & Jerry"));
        assert_eq!(root.attr("num"), Some("AB"));
        assert_eq!(root.attr("half"), Some("&unknown; & unclosed"));
    }

    #[test]
    fn a_doctype_and_a_processing_instruction_are_skipped() {
        let root = one("<?xml version=\"1.0\"?><!DOCTYPE theme><theme/>");
        assert_eq!(root.name, "theme");
    }

    #[test]
    fn tags_left_open_at_the_end_still_count() {
        let roots = parse(b"<theme><view><subview>").unwrap();
        assert_eq!(roots.len(), 1);
        let subview = &roots[0].children[0].children[0];
        assert_eq!(subview.name, "subview");
    }

    #[test]
    fn utf16_with_a_byte_order_mark_is_decoded() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "<THEME title=\"Revert\"/>".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let root = parse(&bytes).unwrap().pop().unwrap();
        assert_eq!(root.name, "theme");
        assert_eq!(root.attr("title"), Some("Revert"));
    }

    #[test]
    fn windows_1252_text_is_decoded_when_utf8_fails() {
        // "©2000" and a smart quote, as skins wrote them.
        let root = parse(b"<theme copyright=\"\xA9 2000\" quote=\"\x93hi\x94\"/>")
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(root.attr("copyright"), Some("\u{a9} 2000"));
        assert_eq!(root.attr("quote"), Some("\u{201c}hi\u{201d}"));
    }

    #[test]
    fn utf8_wins_when_it_is_valid() {
        let root = parse("<theme note=\"héllo\"/>".as_bytes())
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(root.attr("note"), Some("héllo"));
    }

    #[test]
    fn a_byte_order_mark_with_nothing_behind_it_is_empty_and_so_is_nothing() {
        assert!(matches!(parse(&[0xEF, 0xBB, 0xBF]), Err(ParseError::Empty)));
        assert!(matches!(parse(b" & < <"), Err(ParseError::Empty)));
    }

    #[test]
    fn absurd_nesting_and_element_counts_are_refused() {
        let deep = format!(
            "{}{}",
            "<a>".repeat(MAX_DEPTH + 1),
            "</a>".repeat(MAX_DEPTH + 1)
        );
        assert!(matches!(parse(deep.as_bytes()), Err(ParseError::TooDeep)));
        let many = format!("<a>{}</a>", "<b/>".repeat(MAX_NODES + 1));
        assert!(matches!(parse(many.as_bytes()), Err(ParseError::TooLarge)));
    }
}

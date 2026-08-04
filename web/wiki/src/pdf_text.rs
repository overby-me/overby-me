//! Reading a document back out of a PDF.
//!
//! A PDF is not a document. It is a page-description language: set a font, set a
//! matrix, draw these glyph codes at this point. There are no paragraphs, no
//! headings, no reading order, and often no spaces, because a word gap is a jump
//! in position rather than a character. Everything this module returns has to be
//! RECONSTRUCTED from where the glyphs landed.
//!
//! That would be a poor bet in general. It is a good one here, because of what
//! this wiki actually holds: a random sample of the corpus is overwhelmingly
//! Microsoft Word output, directly or printed through macOS. Word lays out
//! regularly, in one column, with consistent leading inside a paragraph and a
//! clean vertical gap between them, and it embeds its fonts with a `ToUnicode`
//! map. The sample extracted Danish text from 18 of 19 files with no replacement
//! characters at all. The nineteenth was a scan, which nothing short of OCR
//! reaches, and which the caller falls back to the browser's viewer for.
//!
//! So this is deliberately not a general PDF engine. It is tuned to the shape of
//! the documents in front of it, and it says when it does not recognise what it
//! is looking at rather than guessing.

use std::collections::HashMap;

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object};

/// One block of the reconstructed document, in reading order.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// `level` is 1-3, from how much larger the line is than the body text.
    Heading {
        level: u8,
        spans: Vec<Span>,
        align: Align,
    },
    Paragraph {
        spans: Vec<Span>,
        align: Align,
        /// How far the block was set in from the column, in steps of about one
        /// line. Indentation is meaning as much as decoration: it is what marks
        /// a quoted passage, a sub-clause or a nested entry, and a reflow that
        /// flattens everything to the margin throws that away.
        indent: u8,
    },
    /// A line that began with a bullet or a number, with that marker stripped.
    /// `marker` is set when the bullet was a picture rather than a character,
    /// because some documents draw a logo where others write a dot.
    ListItem {
        spans: Vec<Span>,
        marker: Option<String>,
    },
    /// A row of a table of contents: what it points at, and the page it points
    /// to. Kept apart from a paragraph because the two are laid out differently
    /// and reflow differently. On the page the leader dots fill the gap so that
    /// every number lands on one right margin; as text those dots are a literal
    /// string in a proportional font, so the numbers land wherever they land and
    /// the column the document drew is lost.
    IndexEntry {
        spans: Vec<Span>,
        page: String,
        indent: u8,
    },
    /// A picture the page drew, where it drew it.
    Image(Picture),
    /// A place a link in this document points at. Carries nothing to read: it
    /// exists so the link has somewhere to land.
    Anchor(String),
    /// Where one page ended and the next began. Furniture, not content: it
    /// exists so that "see page 12" still means something to someone reading
    /// this instead of the pages.
    PageBreak {
        /// Which page of the file just finished, counting from one.
        ended: usize,
        /// What that page called ITSELF, when it printed a number. Front matter
        /// is usually unnumbered, so the two disagree: in the songbook, the
        /// seventh page of the file is the page its own index calls 3. The
        /// printed one is what a cross-reference means.
        printed: Option<String>,
    },
}

impl Block {
    pub fn spans(&self) -> &[Span] {
        match self {
            Block::Heading { spans, .. }
            | Block::Paragraph { spans, .. }
            | Block::IndexEntry { spans, .. }
            | Block::ListItem { spans, .. } => spans,
            Block::Image(_) | Block::Anchor(_) | Block::PageBreak { .. } => &[],
        }
    }

    /// How this block sat across its column.
    pub fn align(&self) -> Align {
        match self {
            Block::Heading { align, .. } | Block::Paragraph { align, .. } => *align,
            _ => Align::Left,
        }
    }

    /// The block's words, without their colours.
    pub fn text(&self) -> String {
        self.spans().iter().map(|s| s.text.as_str()).collect()
    }
}

/// A picture the page drew, ready for an `<img>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// A `data:` URL. The bytes are already in hand, and a second round trip to
    /// fetch what we just parsed would be one more thing to go wrong.
    pub src: String,
    /// The size it was DRAWN at, in points, which is not the size it was stored
    /// at: a 27-pixel logo placed across half a page is still half a page.
    pub width: f64,
    pub height: f64,
}

/// How a block sat across its column.
///
/// Per paragraph, not per page, which is what lets it survive reflow: a centred
/// title stays centred at any width, where a preserved page layout would not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
}

/// A stretch of one colour within a block.
///
/// Colour is per RUN in a PDF, not per paragraph: a document can turn one word
/// red in the middle of a sentence, and a block that carried a single colour
/// would lose that. `None` is the ordinary ink of the document.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    /// A CSS colour, or `None` for whatever the reading surface uses.
    pub color: Option<String>,
    pub bold: bool,
    pub italic: bool,
    /// Where this stretch takes the reader, if the file made it a link.
    pub link: Option<Link>,
}

/// Where a link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Somewhere else in this same document, named by the anchor put there for
    /// it. A contents list is the case that matters: the file already knows
    /// which song each row points at, and a reflow that drops that leaves the
    /// reader scrolling a hundred pages by hand.
    Place(String),
    /// Out on the web.
    Url(String),
}

impl Span {
    /// Whether two stretches can be one span: same colour, same face, same
    /// destination.
    fn matches(&self, other: &Span) -> bool {
        self.color == other.color
            && self.bold == other.bold
            && self.italic == other.italic
            && self.link == other.link
    }
}

/// What came out of a PDF, and how much of it there was to find.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub blocks: Vec<Block>,
    pub pages: usize,
    /// Pages that yielded no text at all. All of them means a scan, and the
    /// caller should offer the browser's viewer instead of an empty page.
    pub pages_without_text: usize,
}

impl Extracted {
    /// Whether this is worth showing. A scan extracts nothing and must not be
    /// presented as an empty document.
    pub fn has_text(&self) -> bool {
        // Blocks that hold nothing but spaces are not text. A page of rules and
        // whitespace can produce them, and "there is a block" is not the same
        // claim as "there is something to read".
        self.pages_without_text < self.pages
            && self.blocks.iter().any(|b| !b.text().trim().is_empty())
    }
}

// --- Fonts -----------------------------------------------------------------

/// How to turn one font's byte codes into text, and how wide each code is.
#[derive(Default, Clone)]
struct Font {
    /// Code to string, from the font's own `/ToUnicode` CMap. Present on most
    /// of what Word embeds, and the reason Danish survives.
    to_unicode: HashMap<u32, String>,
    /// Two bytes per code rather than one: composite (Type0) fonts, which is
    /// what Word uses for anything but the simplest Latin text.
    two_byte: bool,
    /// Code to width in 1/1000 em, for advancing the pen.
    widths: HashMap<u32, f64>,
    default_width: f64,
    /// No `/ToUnicode`: fall back to treating codes as an 8-bit encoding.
    win_ansi: bool,
    /// Weight and slant, which in a PDF are properties of the FONT rather than
    /// of the text. There is no "make this bold" operator: the writer switches
    /// to a different font resource, and the only way to know is to ask that
    /// resource what it is.
    bold: bool,
    italic: bool,
}

impl Font {
    /// Split a PDF string into (code, text) pairs.
    fn decode(&self, bytes: &[u8]) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        if self.two_byte {
            for pair in bytes.chunks(2) {
                let code = match pair {
                    [hi, lo] => ((*hi as u32) << 8) | *lo as u32,
                    [hi] => (*hi as u32) << 8,
                    _ => continue,
                };
                out.push((code, self.text_for(code)));
            }
        } else {
            for b in bytes {
                out.push((*b as u32, self.text_for(*b as u32)));
            }
        }
        out
    }

    fn text_for(&self, code: u32) -> String {
        if let Some(s) = self.to_unicode.get(&code) {
            return s.clone();
        }
        if self.win_ansi {
            return win_ansi_char(code as u8)
                .map(String::from)
                .unwrap_or_default();
        }
        String::new()
    }

    fn width(&self, code: u32) -> f64 {
        self.widths
            .get(&code)
            .copied()
            .unwrap_or(self.default_width)
    }
}

/// WinAnsi (code page 1252), which is Latin-1 except for 0x80-0x9F.
///
/// Only the printable range matters: this is a fallback for fonts that shipped
/// no `/ToUnicode`, and those are the ones using a standard Latin encoding.
/// Danish is `æ` 0xE6, `ø` 0xF8, `å` 0xE5, which Latin-1 already covers.
fn win_ansi_char(b: u8) -> Option<char> {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}',
        '\u{017D}', '\u{FFFD}', '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
    ];
    match b {
        0x00..=0x1F => None,
        0x80..=0x9F => Some(HIGH[(b - 0x80) as usize]).filter(|c| *c != '\u{FFFD}'),
        _ => Some(b as char),
    }
}

/// Parse a `/ToUnicode` CMap: the `bfchar` and `bfrange` sections that map codes
/// to text. Written by hand rather than with a parser because the grammar in
/// play is two shapes, and both are hex strings and integers.
fn parse_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let text = String::from_utf8_lossy(data);
    let mut rest = text.as_ref();

    while let Some(start) = rest.find("beginbfchar") {
        let after = &rest[start + "beginbfchar".len()..];
        let end = after.find("endbfchar").unwrap_or(after.len());
        let toks: Vec<&str> = hex_tokens(&after[..end]);
        for pair in toks.chunks(2) {
            if let [src, dst] = pair {
                if let (Some(code), Some(s)) = (hex_to_u32(src), hex_to_string(dst)) {
                    map.insert(code, s);
                }
            }
        }
        rest = &after[end..];
    }

    let mut rest = text.as_ref();
    while let Some(start) = rest.find("beginbfrange") {
        let after = &rest[start + "beginbfrange".len()..];
        let end = after.find("endbfrange").unwrap_or(after.len());
        parse_bfrange(&after[..end], &mut map);
        rest = &after[end..];
    }
    map
}

/// `<lo> <hi> <dst>` maps a run of codes to consecutive values; `<lo> <hi> [..]`
/// maps them one by one.
fn parse_bfrange(section: &str, map: &mut HashMap<u32, String>) {
    let mut chars = section.char_indices().peekable();
    let mut pending: Vec<&str> = Vec::new();
    let bytes = section;
    while let Some((i, c)) = chars.next() {
        match c {
            '<' => {
                let start = i + 1;
                let mut end = start;
                for (j, d) in chars.by_ref() {
                    if d == '>' {
                        end = j;
                        break;
                    }
                }
                if end > start {
                    pending.push(&bytes[start..end]);
                }
                if pending.len() == 3 {
                    apply_range(&pending, map);
                    pending.clear();
                }
            }
            '[' => {
                // The list form: one destination per code in the range.
                if pending.len() == 2 {
                    let (Some(lo), Some(hi)) = (hex_to_u32(pending[0]), hex_to_u32(pending[1]))
                    else {
                        pending.clear();
                        continue;
                    };
                    let mut code = lo;
                    let mut item = String::new();
                    let mut inside = false;
                    for (_, d) in chars.by_ref() {
                        match d {
                            '<' => {
                                inside = true;
                                item.clear();
                            }
                            '>' => {
                                inside = false;
                                if code <= hi {
                                    if let Some(s) = hex_to_string(&item) {
                                        map.insert(code, s);
                                    }
                                    code += 1;
                                }
                            }
                            ']' => break,
                            _ if inside => item.push(d),
                            _ => {}
                        }
                    }
                }
                pending.clear();
            }
            _ => {}
        }
    }
}

fn apply_range(parts: &[&str], map: &mut HashMap<u32, String>) {
    let (Some(lo), Some(hi), Some(dst)) = (
        hex_to_u32(parts[0]),
        hex_to_u32(parts[1]),
        hex_to_u32(parts[2]),
    ) else {
        return;
    };
    // A range of more than a page of codes is a malformed map, not a real run.
    if hi < lo || hi - lo > 0xFFFF {
        return;
    }
    for (n, code) in (lo..=hi).enumerate() {
        if let Some(c) = char::from_u32(dst + n as u32) {
            map.insert(code, c.to_string());
        }
    }
}

fn hex_tokens(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(a) = rest.find('<') {
        let after = &rest[a + 1..];
        let Some(b) = after.find('>') else { break };
        out.push(&after[..b]);
        rest = &after[b + 1..];
    }
    out
}

fn hex_to_u32(s: &str) -> Option<u32> {
    let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.is_empty() || clean.len() > 8 {
        return None;
    }
    u32::from_str_radix(&clean, 16).ok()
}

/// A `ToUnicode` destination is UTF-16BE, and may be several code units: a
/// ligature maps one code to "fi".
fn hex_to_string(s: &str) -> Option<String> {
    let clean: Vec<char> = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.is_empty() {
        return None;
    }
    let units: Vec<u16> = clean
        .chunks(4)
        .filter(|c| c.len() == 4)
        .filter_map(|c| u16::from_str_radix(&c.iter().collect::<String>(), 16).ok())
        .collect();
    if units.is_empty() {
        return None;
    }
    Some(String::from_utf16_lossy(&units))
}

/// Build a decoder for one font dictionary.
fn read_font(doc: &Document, dict: &Dictionary) -> Font {
    let mut font = Font {
        default_width: 500.0,
        ..Default::default()
    };

    let subtype = dict
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .unwrap_or(b"");
    font.two_byte = subtype == b"Type0";

    if let Ok(obj) = dict.get(b"ToUnicode") {
        if let Some(bytes) = stream_bytes(doc, obj) {
            font.to_unicode = parse_to_unicode(&bytes);
        }
    }
    font.win_ansi = font.to_unicode.is_empty() && !font.two_byte;

    // Weight and slant, from the two places that state them. The name is the
    // reliable one for Word, which embeds subsets called things like
    // "BCDEEE+Calibri-Bold" or "Arial,BoldItalic"; the descriptor is the
    // fallback and the tie-breaker.
    let base = dict
        .get(b"BaseFont")
        .ok()
        .and_then(|o| o.as_name().ok())
        .map(|n| String::from_utf8_lossy(n).to_ascii_lowercase())
        .unwrap_or_default();
    font.bold = base.contains("bold") || base.contains("black") || base.contains("heavy");
    font.italic = base.contains("italic") || base.contains("oblique");

    let descriptor = descriptor_of(doc, dict);
    if let Some(desc) = descriptor {
        if let Ok(weight) = desc.get(b"FontWeight").and_then(number) {
            // 600 is where the CSS scale calls it bold, and where Word's
            // semibold styles sit.
            font.bold = font.bold || weight >= 600.0;
        }
        if let Ok(flags) = desc.get(b"Flags").and_then(number) {
            let flags = flags as u32;
            // Bit 19 (1-based) is ForceBold, bit 7 is Italic.
            font.bold = font.bold || flags & (1 << 18) != 0;
            font.italic = font.italic || flags & (1 << 6) != 0;
        }
        if let Ok(angle) = desc.get(b"ItalicAngle").and_then(number) {
            font.italic = font.italic || angle.abs() > 4.0;
        }
    }

    // Simple fonts carry /FirstChar + /Widths; composite ones carry /W on the
    // descendant, with /DW as the default.
    if font.two_byte {
        font.default_width = 1000.0;
        if let Some(desc) = descendant(doc, dict) {
            if let Ok(dw) = desc.get(b"DW").and_then(number) {
                font.default_width = dw;
            }
            if let Ok(w) = desc.get(b"W").and_then(|o| resolve(doc, o)) {
                if let Ok(arr) = w.as_array() {
                    read_cid_widths(doc, arr, &mut font.widths);
                }
            }
        }
    } else if let (Ok(first), Ok(widths)) = (
        dict.get(b"FirstChar").and_then(number),
        dict.get(b"Widths").and_then(|o| resolve(doc, o)),
    ) {
        if let Ok(arr) = widths.as_array() {
            for (i, w) in arr.iter().enumerate() {
                if let Ok(w) = number(w) {
                    font.widths.insert(first as u32 + i as u32, w);
                }
            }
        }
    }
    font
}

/// `/W` is `[ code [w w w] code1 code2 w ]`, mixing both forms.
fn read_cid_widths(doc: &Document, arr: &[Object], out: &mut HashMap<u32, f64>) {
    let mut i = 0;
    while i < arr.len() {
        let Ok(first) = number(&arr[i]) else {
            i += 1;
            continue;
        };
        let Some(next) = arr.get(i + 1) else { break };
        match resolve(doc, next).unwrap_or(next) {
            Object::Array(list) => {
                for (n, w) in list.iter().enumerate() {
                    if let Ok(w) = number(w) {
                        out.insert(first as u32 + n as u32, w);
                    }
                }
                i += 2;
            }
            _ => {
                let (Ok(last), Some(w)) =
                    (number(next), arr.get(i + 2).and_then(|o| number(o).ok()))
                else {
                    i += 2;
                    continue;
                };
                if last >= first && last - first < 65_536.0 {
                    for code in first as u32..=last as u32 {
                        out.insert(code, w);
                    }
                }
                i += 3;
            }
        }
    }
}

/// A font's descriptor, which is on the font itself for a simple font and on
/// the descendant for a composite one.
fn descriptor_of<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
    let holder = descendant(doc, dict).unwrap_or(dict);
    let obj = holder.get(b"FontDescriptor").ok()?;
    resolve(doc, obj).ok()?.as_dict().ok()
}

fn descendant<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
    let obj = dict.get(b"DescendantFonts").ok()?;
    let arr = resolve(doc, obj).ok()?.as_array().ok()?;
    let first = arr.first()?;
    resolve(doc, first).ok()?.as_dict().ok()
}

fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Result<&'a Object, lopdf::Error> {
    match obj {
        Object::Reference(id) => doc.get_object(*id),
        other => Ok(other),
    }
}

fn stream_bytes(doc: &Document, obj: &Object) -> Option<Vec<u8>> {
    let resolved = resolve(doc, obj).ok()?;
    let stream = resolved.as_stream().ok()?;
    stream
        .decompressed_content()
        .ok()
        .or_else(|| Some(stream.content.clone()))
}

fn number(obj: &Object) -> Result<f64, lopdf::Error> {
    match obj {
        Object::Integer(i) => Ok(*i as f64),
        Object::Real(r) => Ok(*r as f64),
        _ => Err(lopdf::Error::ObjectType {
            expected: "number",
            found: "other",
        }),
    }
}

/// Wrap raw 8-bit samples as a BMP, which every browser reads.
///
/// BMP rather than PNG because PNG's pixel data is zlib-wrapped and would want a
/// deflate implementation for something the browser is about to undo anyway.
/// A BMP is a header and the rows, bottom-up, in BGR, padded to four bytes.
fn bmp_data_url(width: usize, height: usize, comps: usize, samples: &[u8]) -> Option<String> {
    if width == 0 || height == 0 || !(comps == 1 || comps == 3) {
        return None;
    }
    if samples.len() < width * height * comps {
        return None;
    }
    let row_bytes = width * 3;
    let padding = (4 - row_bytes % 4) % 4;
    let pixels = (row_bytes + padding) * height;
    let mut out = Vec::with_capacity(54 + pixels);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + pixels) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    for _ in 0..6 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    // Bottom-up, and BGR rather than RGB: both are BMP's idea, not ours.
    for y in (0..height).rev() {
        for x in 0..width {
            let i = (y * width + x) * comps;
            let (r, g, b) = match comps {
                1 => (samples[i], samples[i], samples[i]),
                _ => (samples[i], samples[i + 1], samples[i + 2]),
            };
            out.extend_from_slice(&[b, g, r]);
        }
        out.extend(std::iter::repeat_n(0u8, padding));
    }
    Some(format!("data:image/bmp;base64,{}", base64_encode(&out)))
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The filters a stream declares, innermost first.
fn filters_of(dict: &Dictionary) -> Vec<String> {
    let Ok(obj) = dict.get(b"Filter") else {
        return Vec::new();
    };
    match obj {
        Object::Name(n) => vec![String::from_utf8_lossy(n).to_string()],
        Object::Array(items) => items
            .iter()
            .filter_map(|o| o.as_name().ok())
            .map(|n| String::from_utf8_lossy(n).to_string())
            .collect(),
        _ => Vec::new(),
    }
}

/// Turn an image XObject into something an `<img>` can show, or `None` for the
/// encodings this does not read.
///
/// JPEG needs no work at all: the stream IS a JPEG, so it goes straight into a
/// data URL. Anything Flate-compressed is raw samples, which become a BMP.
/// JPEG 2000, CCITT fax and JBIG2 are declined rather than guessed at: they are
/// scanner formats, and a scan has no text for this renderer to sit beside
/// anyway.
fn decode_image(doc: &Document, stream: &lopdf::Stream) -> Option<String> {
    let dict = &stream.dict;
    let filters = filters_of(dict);
    if filters.iter().any(|f| f == "DCTDecode") {
        return Some(format!(
            "data:image/jpeg;base64,{}",
            base64_encode(&stream.content)
        ));
    }
    if filters
        .iter()
        .any(|f| matches!(f.as_str(), "JPXDecode" | "CCITTFaxDecode" | "JBIG2Decode"))
    {
        return None;
    }
    let width = dict.get(b"Width").ok().and_then(|o| number(o).ok())? as usize;
    let height = dict.get(b"Height").ok().and_then(|o| number(o).ok())? as usize;
    let bpc = dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(|o| number(o).ok())
        .unwrap_or(8.0);
    if bpc != 8.0 {
        return None;
    }
    let comps = match dict
        .get(b"ColorSpace")
        .ok()
        .and_then(|o| resolve(doc, o).ok())
    {
        Some(Object::Name(n)) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => 1,
            b"DeviceRGB" | b"CalRGB" | b"RGB" => 3,
            _ => return None,
        },
        // ICCBased carries its component count on the stream it points at.
        Some(Object::Array(items)) => {
            let icc = items.get(1).and_then(|o| resolve(doc, o).ok())?;
            let n = icc
                .as_stream()
                .ok()?
                .dict
                .get(b"N")
                .ok()
                .and_then(|o| number(o).ok())?;
            n as usize
        }
        _ => return None,
    };
    // A stream declares its own size, and believing that is how a decompression
    // bomb gets in: a few kilobytes that expand to gigabytes. The declared
    // dimensions say exactly how many bytes this can legitimately be, so ask for
    // that many and no more, with a little slack for a producer that padded.
    let need = width.checked_mul(height)?.checked_mul(comps)?;
    if need == 0 || need > RAW_SAMPLE_LIMIT {
        return None;
    }
    let samples = stream
        .decompressed_content_with_limit(need.saturating_add(1 << 12))
        .ok()?;
    match shrink_to_fit(width, height, comps, &samples) {
        Some((width, height, samples)) => bmp_data_url(width, height, comps, &samples),
        None => bmp_data_url(width, height, comps, &samples),
    }
}

/// How many bytes of picture one document is worth carrying.
///
/// Every picture is a data URL sitting in the page's own markup, so a file made
/// of full-page images would otherwise hand a phone hundreds of megabytes of
/// base64 to hold at once. Past this the pictures stop and the text carries on;
/// someone who wants the pages as pages has the browser's viewer one tap away.
const PICTURE_BUDGET: usize = 24 << 20;

/// The most raw samples worth decompressing for one picture: more than a
/// 300-dpi A4 page in full colour, and well under what would hurt.
const RAW_SAMPLE_LIMIT: usize = 64 << 20;

/// The longest side a raw picture is scaled down to fit.
///
/// A JPEG arrives compressed and passes through untouched, but raw samples
/// become a BMP, which spends a byte per component and compresses nothing, and
/// then base64, which adds a third again. A 300-dpi A4 cover is 2481 by 3509:
/// 26 MB of samples, 35 MB of data URL, for something a phone shows four
/// hundred points wide. At this size it is 2.8 MB and looks the same.
const RAW_MAX_SIDE: usize = 1000;

/// Scale raw samples down to fit [`RAW_MAX_SIDE`], or `None` if they already do.
///
/// Each output pixel is the average of the source box it covers rather than one
/// pixel picked out of it. Picking aliases, and what these documents put on a
/// full page is a photograph or a scan, where that shows as speckle.
fn shrink_to_fit(
    width: usize,
    height: usize,
    comps: usize,
    samples: &[u8],
) -> Option<(usize, usize, Vec<u8>)> {
    let longest = width.max(height);
    if longest <= RAW_MAX_SIDE || samples.len() < width * height * comps {
        return None;
    }
    let scale = RAW_MAX_SIDE as f64 / longest as f64;
    let to_w = ((width as f64 * scale).round() as usize).clamp(1, width);
    let to_h = ((height as f64 * scale).round() as usize).clamp(1, height);
    let mut out = vec![0u8; to_w * to_h * comps];
    for ty in 0..to_h {
        let y0 = ty * height / to_h;
        let y1 = ((ty + 1) * height / to_h).clamp(y0 + 1, height);
        for tx in 0..to_w {
            let x0 = tx * width / to_w;
            let x1 = ((tx + 1) * width / to_w).clamp(x0 + 1, width);
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            for c in 0..comps {
                let mut sum = 0u32;
                for sy in y0..y1 {
                    let row = sy * width;
                    for sx in x0..x1 {
                        sum += u32::from(samples[(row + sx) * comps + c]);
                    }
                }
                out[(ty * to_w + tx) * comps + c] = (sum / count) as u8;
            }
        }
    }
    Some((to_w, to_h, out))
}

/// The page's image XObjects, by the name its content stream calls them.
///
/// `lopdf` has `get_page_fonts` for exactly this job one resource key over, but
/// nothing for XObjects, and reaching for `get_page_resources` alone is a trap.
/// It hands back the `/Resources` dictionary itself only when the page wrote it
/// inline; a page that points at a shared one, or inherits it from the page
/// tree, gets back object ids to resolve instead, and a caller reading only the
/// first of those two finds no pictures at all. Books built from one resource
/// dictionary take that second path, which is how a songbook lost its cover.
fn page_xobjects(doc: &Document, page_id: lopdf::ObjectId) -> HashMap<Vec<u8>, lopdf::ObjectId> {
    let Ok((inline, inherited)) = doc.get_page_resources(page_id) else {
        return HashMap::new();
    };
    let inherited = inherited
        .into_iter()
        .filter_map(|id| doc.get_dictionary(id).ok());
    let mut named = HashMap::new();
    // Nearest first, and first name wins: what the page itself says beats what
    // it inherited, which is the order the page tree gives them in.
    for resources in inline.into_iter().chain(inherited) {
        let listed = match resources.get(b"XObject") {
            Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).ok(),
            Ok(Object::Dictionary(dict)) => Some(dict),
            _ => None,
        };
        for (name, value) in listed.into_iter().flat_map(Dictionary::iter) {
            if let Object::Reference(id) = value {
                named.entry(name.to_vec()).or_insert(*id);
            }
        }
    }
    named
}

// --- Walking the page ------------------------------------------------------

/// One run of text, where it landed and how big it was.
#[derive(Debug, Clone)]
struct Run {
    x: f64,
    /// Where the pen ENDED, exactly, from the advance widths. Estimating this
    /// from the character count put spaces inside words: Word emits a line as
    /// many small runs, and a guess at where one ends is wrong often enough to
    /// read as "G eneralsekretæren".
    end_x: f64,
    y: f64,
    size: f64,
    text: String,
    /// The non-stroking colour in force, or `None` for ordinary ink.
    color: Option<String>,
    bold: bool,
    italic: bool,
    /// Set when a link annotation covers this run. Attached after the page is
    /// walked: the annotations live beside the content stream, not in it.
    link: Option<Link>,
}

/// A fill colour, as CSS, or `None` when it is the document's ordinary ink.
///
/// Black is not a colour here, it is the default, and the same rule the Word
/// renderer learned applies: a document that states black text and a document
/// that states nothing look identical on paper, and forcing black makes both
/// unreadable on a dark reading surface. So near-black is dropped and the
/// surface decides.
///
/// Near-WHITE is dropped for the opposite reason. White text in a PDF sits on a
/// coloured shape that this renderer does not draw, so keeping it would leave an
/// invisible paragraph. Falling back to ordinary ink at least shows the words.
fn ink(r: f64, g: f64, b: f64) -> Option<String> {
    let (r, g, b) = (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
    // Rec. 601 luma, which is what "how dark is this" means to an eye.
    let luma = 0.299 * r + 0.587 * g + 0.114 * b;
    let spread = r.max(g).max(b) - r.min(g).min(b);
    // Grey enough to be ink rather than a colour, and dark or light enough to be
    // the page's own extremes.
    //
    // The dark end is 0.22 rather than something nearer zero because Office does
    // not write body text as #000000. It writes #201f1e, and Google writes
    // #202124: near-blacks that are the DEFAULT ink of those tools, not a choice
    // anyone made. Found in the corpus, where they were the most common colour
    // by far. Keeping them would pin almost every paragraph to near-black and
    // make the whole document unreadable in the dark theme, which is the same
    // trap the Word renderer fell into.
    //
    // Real colours are safe from this: the spread test lets them through first.
    // Word's heading blue #1f3864 is dark, but its channels are 0.27 apart.
    if spread < 0.08 && !(0.22..=0.85).contains(&luma) {
        return None;
    }
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8
    ))
}

/// CMYK as PDF states it, converted the simple way. Word writes RGB; this is
/// for the documents that came through a print pipeline.
fn cmyk(c: f64, m: f64, y: f64, k: f64) -> Option<String> {
    let f = |v: f64| (1.0 - v.clamp(0.0, 1.0)) * (1.0 - k.clamp(0.0, 1.0));
    ink(f(c), f(m), f(y))
}

/// Multiply two PDF matrices (a b c d e f).
fn mul(m: [f64; 6], n: [f64; 6]) -> [f64; 6] {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Pull the positioned runs off one page.
/// What one page drew: its text, and its pictures with where they landed.
struct Drawn {
    runs: Vec<Run>,
    /// (y, picture), so a picture can be ordered against the lines.
    pictures: Vec<(f64, Picture)>,
    /// The small marks: bullets, rules, and whatever else a page draws too small
    /// to be looked at. Sorted out against the lines afterwards.
    marks: Vec<Mark>,
}

/// A mark too small to be a picture, and where it sits.
struct Mark {
    x: f64,
    right: f64,
    /// Halfway up it, which is what a bullet aligns to a line by.
    middle: f64,
    src: String,
}

fn page_runs(
    doc: &Document,
    page_id: lopdf::ObjectId,
    budget: &mut usize,
    links: &[LinkArea],
) -> Drawn {
    let fonts: HashMap<Vec<u8>, Font> = match doc.get_page_fonts(page_id) {
        Ok(map) => map
            .into_iter()
            .map(|(name, dict)| (name, read_font(doc, dict)))
            .collect(),
        Err(_) => HashMap::new(),
    };
    let Ok(content) = Content::decode(&doc.get_page_content(page_id)) else {
        return Drawn {
            runs: Vec::new(),
            pictures: Vec::new(),
            marks: Vec::new(),
        };
    };
    let xobjects = page_xobjects(doc, page_id);

    let mut runs = Vec::new();
    let mut pictures: Vec<(f64, Picture)> = Vec::new();
    let mut marks: Vec<Mark> = Vec::new();
    let mut decoded: HashMap<lopdf::ObjectId, String> = HashMap::new();
    let mut ctm = IDENTITY;
    // The fill colour rides with the CTM: `q`/`Q` save and restore the whole
    // graphics state, and a heading's colour set inside one would otherwise leak
    // into the body text after it.
    let mut fill: Option<String> = None;
    let mut stack: Vec<([f64; 6], Option<String>)> = Vec::new();
    let mut tm = IDENTITY;
    let mut tlm = IDENTITY;
    let mut font: Option<Font> = None;
    let mut size = 0.0f64;
    let mut leading = 0.0f64;
    let mut char_space = 0.0f64;
    let mut word_space = 0.0f64;
    let mut h_scale = 1.0f64;

    for op in content.operations {
        let nums: Vec<f64> = op.operands.iter().filter_map(|o| number(o).ok()).collect();
        match op.operator.as_str() {
            "q" => stack.push((ctm, fill.clone())),
            "Q" => {
                let (c, f) = stack.pop().unwrap_or((IDENTITY, None));
                ctm = c;
                fill = f;
            }
            // The non-stroking colour, in each of the ways a PDF states it.
            // `sc`/`scn` take their meaning from the current colour space; the
            // operand count tells us which, and a pattern (which has a name
            // operand) is left alone rather than guessed at.
            "g" if nums.len() == 1 => fill = ink(nums[0], nums[0], nums[0]),
            "rg" if nums.len() == 3 => fill = ink(nums[0], nums[1], nums[2]),
            "k" if nums.len() == 4 => fill = cmyk(nums[0], nums[1], nums[2], nums[3]),
            "sc" | "scn" => match nums.len() {
                1 => fill = ink(nums[0], nums[0], nums[0]),
                3 => fill = ink(nums[0], nums[1], nums[2]),
                4 => fill = cmyk(nums[0], nums[1], nums[2], nums[3]),
                _ => {}
            },
            "cm" if nums.len() == 6 => {
                ctm = mul([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]], ctm);
            }
            "BT" => {
                tm = IDENTITY;
                tlm = IDENTITY;
            }
            "Tf" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    font = fonts.get(name.as_slice()).cloned();
                }
                size = nums.last().copied().unwrap_or(size);
            }
            "TL" => leading = nums.first().copied().unwrap_or(leading),
            "Tc" => char_space = nums.first().copied().unwrap_or(char_space),
            "Tw" => word_space = nums.first().copied().unwrap_or(word_space),
            "Tz" => h_scale = nums.first().copied().unwrap_or(100.0) / 100.0,
            "Td" if nums.len() == 2 => {
                tlm = mul([1.0, 0.0, 0.0, 1.0, nums[0], nums[1]], tlm);
                tm = tlm;
            }
            "TD" if nums.len() == 2 => {
                leading = -nums[1];
                tlm = mul([1.0, 0.0, 0.0, 1.0, nums[0], nums[1]], tlm);
                tm = tlm;
            }
            "Tm" if nums.len() == 6 => {
                tlm = [nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]];
                tm = tlm;
            }
            "T*" => {
                tlm = mul([1.0, 0.0, 0.0, 1.0, 0.0, -leading], tlm);
                tm = tlm;
            }
            "Tj" | "'" | "\"" => {
                if op.operator != "Tj" {
                    tlm = mul([1.0, 0.0, 0.0, 1.0, 0.0, -leading], tlm);
                    tm = tlm;
                }
                if let Some(Object::String(bytes, _)) = op.operands.last() {
                    show(
                        bytes,
                        &font,
                        size,
                        h_scale,
                        char_space,
                        word_space,
                        &mut tm,
                        ctm,
                        fill.as_deref(),
                        links,
                        &mut runs,
                    );
                }
            }
            "TJ" => {
                if let Some(Object::Array(items)) = op.operands.first() {
                    for item in items {
                        match item {
                            Object::String(bytes, _) => show(
                                bytes,
                                &font,
                                size,
                                h_scale,
                                char_space,
                                word_space,
                                &mut tm,
                                ctm,
                                fill.as_deref(),
                                links,
                                &mut runs,
                            ),
                            other => {
                                if let Ok(k) = number(other) {
                                    // A kerning number shifts the pen without
                                    // drawing: this is where word gaps live in
                                    // Word's output.
                                    let tx = -k / 1000.0 * size * h_scale;
                                    tm = mul([1.0, 0.0, 0.0, 1.0, tx, 0.0], tm);
                                }
                            }
                        }
                    }
                }
            }
            "Do" => {
                let Some(Object::Name(name)) = op.operands.first() else {
                    continue;
                };
                let Some(id) = xobjects.get(name.as_slice()) else {
                    continue;
                };
                let Ok(stream) = doc.get_object(*id).and_then(|o| o.as_stream()) else {
                    continue;
                };
                if stream
                    .dict
                    .get(b"Subtype")
                    .ok()
                    .and_then(|o| o.as_name().ok())
                    != Some(b"Image")
                {
                    continue;
                }
                if *budget == 0 {
                    continue;
                }
                // A document sets its bullet once and draws it a hundred times,
                // so decode each image only the first time it is met.
                let src = match decoded.get(id) {
                    Some(src) => src.clone(),
                    None => {
                        let Some(src) = decode_image(doc, stream) else {
                            continue;
                        };
                        decoded.insert(*id, src.clone());
                        src
                    }
                };
                // The CTM maps the unit square onto where the image goes, so its
                // width and height fall straight out of the matrix.
                let width = (ctm[0].powi(2) + ctm[1].powi(2)).sqrt();
                let height = (ctm[2].powi(2) + ctm[3].powi(2)).sqrt();
                // Below about a line of text in both directions it is not a
                // picture someone put there to be looked at: it is a mark. Most
                // are furniture and go, but one drawn in the margin beside a line
                // is that line's bullet, and this document's bullet is a logo
                // rather than a dot, so it cannot be swapped for one. Held aside
                // and matched against the lines once those are known.
                if width < 24.0 && height < 24.0 {
                    marks.push(Mark {
                        x: ctm[4],
                        right: ctm[4] + width,
                        middle: ctm[5] + height / 2.0,
                        src,
                    });
                    continue;
                }
                // Past the budget the pictures stop, and stay stopped: a
                // document that hands over one whole page of raw samples has
                // more of them coming, and decoding each one to refuse it is
                // work nobody sees.
                if src.len() > *budget {
                    *budget = 0;
                    continue;
                }
                *budget -= src.len();
                pictures.push((
                    // The matrix places the image's BOTTOM edge; the top is what
                    // orders it against the lines around it.
                    ctm[5] + height,
                    Picture { src, width, height },
                ));
            }
            _ => {}
        }
    }
    Drawn {
        runs,
        pictures,
        marks,
    }
}

#[allow(clippy::too_many_arguments)]
fn show(
    bytes: &[u8],
    font: &Option<Font>,
    size: f64,
    h_scale: f64,
    char_space: f64,
    word_space: f64,
    tm: &mut [f64; 6],
    ctm: [f64; 6],
    color: Option<&str>,
    links: &[LinkArea],
    runs: &mut Vec<Run>,
) {
    let Some(font) = font else { return };
    let at = mul(*tm, ctm);
    let scale = (at[0] * at[0] + at[1] * at[1]).sqrt().max(0.01);
    // A link covers a BOX, and a draw call is under no obligation to stop at its
    // edge: one call can carry a name, an address and a telephone number, of
    // which the file linked only the middle. So the glyphs are walked and the
    // run is cut wherever the link under the pen changes, which is what puts the
    // link on the address rather than on the line it sits in.
    let mut text = String::new();
    let mut advance = 0.0;
    let mut piece_at = 0.0;
    let mut piece_link: Option<Link> = None;
    let mut started = false;
    let cut = |from: f64, to: f64, text: &mut String, link: &Option<Link>, runs: &mut Vec<Run>| {
        let start = mul(mul([1.0, 0.0, 0.0, 1.0, from, 0.0], *tm), ctm);
        let end = mul(mul([1.0, 0.0, 0.0, 1.0, to, 0.0], *tm), ctm);
        if !text.trim().is_empty() {
            runs.push(Run {
                link: link.clone(),
                x: start[4],
                end_x: end[4],
                y: start[5],
                size: size * scale,
                text: std::mem::take(text),
                color: color.map(str::to_string),
                bold: font.bold,
                italic: font.italic,
            });
        }
        text.clear();
    };
    for (code, s) in font.decode(bytes) {
        let w = font.width(code) / 1000.0 * size;
        let extra = if !font.two_byte && code == 32 {
            word_space
        } else {
            0.0
        };
        let step = (w + char_space + extra) * h_scale;
        // Which link holds this glyph, decided at its middle: a glyph half
        // inside a box belongs to whichever side has most of it.
        let mid = mul(
            mul([1.0, 0.0, 0.0, 1.0, advance + step / 2.0, 0.0], *tm),
            ctm,
        );
        let here = links
            .iter()
            .find(|a| a.holds(mid[4], mid[5]))
            .map(|a| a.link.clone());
        if started && here != piece_link {
            cut(piece_at, advance, &mut text, &piece_link, runs);
            piece_at = advance;
        }
        if !started {
            piece_at = advance;
            started = true;
        }
        piece_link = here;
        text.push_str(&s);
        advance += step;
    }
    cut(piece_at, advance, &mut text, &piece_link, runs);
    *tm = mul([1.0, 0.0, 0.0, 1.0, advance, 0.0], *tm);
}

// --- Reconstructing the document -------------------------------------------

/// One reconstructed line: the runs that share a baseline, joined.
#[derive(Debug, Clone)]
struct Line {
    y: f64,
    size: f64,
    /// Where the line's last glyph ended. A line that stops well short of the
    /// column it is set in ended because the writer meant it to, and the line
    /// after it starts something new rather than continuing.
    right: f64,
    /// The joined text, for the heuristics that read it.
    text: String,
    /// The same words, keeping where the colour changed.
    spans: Vec<Span>,
    /// Where the line STARTED, which together with `right` says how it sat
    /// across the column.
    left: f64,
    /// Which page drew it, 1-based.
    page: usize,
    /// The mark drawn in the margin beside this line, which makes it an item of
    /// a list. Kept as the image the document drew rather than swapped for a
    /// dot: here it is the organisation's own logo.
    bullet: Option<String>,
    /// Set when this "line" is a picture rather than words. It takes its place
    /// in the flow by where it was drawn, like everything else.
    picture: Option<Picture>,
}

/// Group runs into lines by baseline, then join each line left to right,
/// inserting a space where the gap between runs is wide enough to be one.
fn lines_from(mut runs: Vec<Run>) -> Vec<Line> {
    if runs.is_empty() {
        return Vec::new();
    }
    // Top to bottom: PDF y grows upward.
    runs.sort_by(|a, b| {
        b.y.partial_cmp(&a.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Run> = Vec::new();
    for run in runs {
        let same_line = current.first().is_some_and(|f: &Run| {
            // Half the font size of tolerance: subscripts and a slightly raised
            // bullet belong to the line they sit on.
            (f.y - run.y).abs() < (f.size.max(run.size) * 0.5).max(1.0)
        });
        if same_line {
            current.push(run);
        } else {
            if let Some(line) = join_line(&current) {
                lines.push(line);
            }
            current = vec![run];
        }
    }
    if let Some(line) = join_line(&current) {
        lines.push(line);
    }
    lines
}

/// How wide a gap between two runs has to be, as a fraction of the em, before it
/// is a word break.
///
/// Chosen by sweeping it against poppler over seventeen real documents and
/// counting the words the two disagree on, rather than by eye: a U with a flat
/// bottom at 0.08 to 0.09, missed spaces above it and words coming apart below.
/// At 0.20 the justified samværspolitik read "Der eraltid nogen at gå til",
/// because justification compresses a word space to 0.187 of the em. The
/// measurement is in docs/pdf-word-gap.md.
const WORD_GAP: f64 = 0.09;

/// How wide a gap between two runs has to be, as a fraction of the em, before it
/// is a word break rather than the ordinary jitter between draw calls.
///
/// Overridable under test so the value can be swept against an oracle rather
/// than argued about.
fn word_gap() -> f64 {
    #[cfg(test)]
    {
        use std::sync::OnceLock;
        static OVERRIDE: OnceLock<Option<f64>> = OnceLock::new();
        if let Some(v) = OVERRIDE.get_or_init(|| {
            std::env::var("PDF_WORD_GAP")
                .ok()
                .and_then(|v| v.parse().ok())
        }) {
            return *v;
        }
    }
    WORD_GAP
}

fn join_line(runs: &[Run]) -> Option<Line> {
    let first = runs.first()?;
    let mut spans: Vec<Span> = Vec::new();
    let mut prev_end = f64::NEG_INFINITY;
    let mut size: f64 = 0.0;
    let mut sorted: Vec<&Run> = runs.iter().collect();
    sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    for run in sorted {
        size = size.max(run.size);
        // Compared against the pen's TRUE end, not a guess from the character
        // count, which is what put spaces inside words. See [`WORD_GAP`].
        let gap = run.x - prev_end;
        let wants_space = prev_end.is_finite()
            && gap > run.size * word_gap()
            && !spans.last().is_some_and(|s| s.text.ends_with(' '));
        // A run continues the one before it when the colour has not changed, so
        // a paragraph in one colour is one span rather than one per draw call.
        let here = Span {
            text: String::new(),
            color: run.color.clone(),
            bold: run.bold,
            italic: run.italic,
            link: run.link.clone(),
        };
        match spans.last_mut() {
            Some(last) if last.matches(&here) => {
                if wants_space {
                    last.text.push(' ');
                }
                last.text.push_str(&run.text);
            }
            _ => {
                let mut text = String::new();
                // The space belongs to the run BEFORE the colour change, so a
                // colour does not start with whitespace in it.
                if wants_space {
                    if let Some(last) = spans.last_mut() {
                        last.text.push(' ');
                    } else {
                        text.push(' ');
                    }
                }
                text.push_str(&run.text);
                spans.push(Span { text, ..here });
            }
        }
        prev_end = run.end_x;
    }
    let spans = mend_email_spans(tidy(spans));
    if spans.is_empty() {
        return None;
    }
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    let right = runs
        .iter()
        .map(|r| r.end_x)
        .fold(f64::NEG_INFINITY, f64::max);
    let left = runs.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
    Some(Line {
        y: first.y,
        size,
        right,
        text,
        spans,
        left,
        // Filled in by the caller, which is the only place that knows.
        page: 0,
        bullet: None,
        picture: None,
    })
}

/// Collapse runs of whitespace and drop what is left empty, without losing the
/// colour boundaries.
fn tidy(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for span in spans {
        let mut text = String::new();
        let mut last_was_space =
            out.last().is_some_and(|s: &Span| s.text.ends_with(' ')) || out.is_empty();
        for c in span.text.chars() {
            if c.is_whitespace() {
                if !last_was_space {
                    text.push(' ');
                }
                last_was_space = true;
            } else {
                text.push(c);
                last_was_space = false;
            }
        }
        if !text.is_empty() {
            out.push(Span { text, ..span });
        }
    }
    // Nothing but spaces is not a line.
    if out.iter().all(|s| s.text.trim().is_empty()) {
        return Vec::new();
    }
    if let Some(last) = out.last_mut() {
        while last.text.ends_with(' ') {
            last.text.pop();
        }
    }
    out
}

/// The size most of the document is set in, which is what a heading is large
/// relative to. The median rather than the mean: a title page would drag a mean.
fn body_size(lines: &[Line]) -> f64 {
    let mut sizes: Vec<f64> = lines.iter().map(|l| l.size).collect();
    if sizes.is_empty() {
        return 12.0;
    }
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sizes[sizes.len() / 2]
}

/// Whether this line opens a new entry rather than continuing a sentence.
///
/// A clock time at the start of a line. Narrow and deliberately so: it exists
/// because the thing this wiki holds most of is a programme, where every entry
/// opens with one, and a long entry that happens to fill the column would
/// otherwise swallow the next. The short-line rule catches the rest; this
/// catches the case it cannot see, where the previous line legitimately reached
/// the margin.
fn starts_new_entry(text: &str) -> bool {
    let t = text.trim_start();
    let mut chars = t.chars();
    let mut digits = 0;
    for c in chars.by_ref() {
        match c {
            '0'..='9' if digits < 2 => digits += 1,
            ':' | '.' if digits >= 1 => break,
            _ => return false,
        }
    }
    if digits == 0 {
        return false;
    }
    // Exactly two digits after the separator, then a boundary: 14:30 is a time,
    // 14.302 is not, and neither is a version number.
    let rest: Vec<char> = chars.collect();
    matches!(rest.as_slice(), ['0'..='9', '0'..='9', after, ..] if !after.is_ascii_digit())
        || matches!(rest.as_slice(), ['0'..='9', '0'..='9'])
}

/// Whether this line is an entry in a table of contents.
///
/// Leader dots, which no ordinary sentence contains: an ellipsis is three dots
/// or one character, and a leader runs the width of the column. Six is well
/// clear of both.
///
/// Needed because a contents page defeats every other rule at once. Every line
/// reaches the column edge, because the dots are what fill it, so nothing looks
/// like it ended early; the lines are single-spaced, so there is no gap; and
/// there is no time to open on. The songbook's index arrived as one paragraph
/// per SECTION, with every song in it run together.
fn is_index_entry(text: &str) -> bool {
    text.as_bytes()
        .windows(6)
        .any(|w| w.iter().all(|b| *b == b'.'))
}

/// Split a table-of-contents row into what it names and the page it names.
///
/// Read from the end, because that is where the certainty is: a number, then the
/// leader that carried the eye to it. Anything else is a sentence that happens to
/// end in a digit, and left alone.
fn index_entry(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim_end();
    let digits_start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i)?;
    let page = &trimmed[digits_start..];
    // A page number, not a year in a title and not a whole line of digits.
    if page.len() > 4 {
        return None;
    }
    let lead = trimmed[..digits_start].trim_end();
    // The leader itself: dots, with whatever spacing the file set them in.
    let dots = lead
        .chars()
        .rev()
        .take_while(|c| matches!(c, '.' | '\u{00B7}' | '\u{2024}' | '\u{2026}' | ' '))
        .filter(|c| *c != ' ')
        .count();
    if dots < 3 {
        return None;
    }
    let title = lead
        .trim_end_matches([' ', '.', '\u{00B7}', '\u{2024}', '\u{2026}'])
        .trim();
    match title.is_empty() {
        true => None,
        false => Some((title, page)),
    }
}

/// How far a block was set in from the column, in steps of roughly one line.
///
/// Quantised rather than measured to the point, because this reflows: a phone's
/// column is not the page's, and carrying an exact 18.2pt inset into it would
/// eat a third of the width. What survives is the DEPTH, which is what the
/// indent meant.
fn indent_steps(left: f64, col_left: f64, size: f64) -> u8 {
    if !left.is_finite() || !col_left.is_finite() || size <= 0.0 {
        return 0;
    }
    let inset = left - col_left;
    // Half a line of slack: a paragraph is not indented because its first glyph
    // is a hair right of the one above it.
    if inset < size * 0.6 {
        return 0;
    }
    ((inset / (size * 1.6)).round() as u8).clamp(1, 4)
}

/// How far right the text column runs.
///
/// Not the maximum, which one stray element pushes past the real margin, but
/// close to it: the line that reaches furthest among the great majority. A line
/// ending well short of this stopped on purpose.
fn column_right(lines: &[Line]) -> f64 {
    let mut rights: Vec<f64> = lines
        .iter()
        .map(|l| l.right)
        .filter(|r| r.is_finite())
        .collect();
    if rights.is_empty() {
        return f64::INFINITY;
    }
    rights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    rights[rights.len() * 9 / 10]
}

/// Where the text column starts. The mirror of [`column_right`]: a low
/// percentile rather than the minimum, so one line hanging into the margin does
/// not move the whole page.
fn column_left(lines: &[Line]) -> f64 {
    let mut lefts: Vec<f64> = lines
        .iter()
        .filter(|l| l.picture.is_none())
        .map(|l| l.left)
        .filter(|v| v.is_finite())
        .collect();
    if lefts.is_empty() {
        return 0.0;
    }
    lefts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    lefts[lefts.len() / 10]
}

/// How a BLOCK sat across its column, from the geometry of all its lines.
///
/// One line cannot tell you this, which is what the first attempt got wrong. An
/// indented paragraph has both edges pulled in and its midpoint near the
/// column's, and so does a centred one. What separates them is that a centred
/// block's lines each start somewhere DIFFERENT, while an indented block's all
/// start at the same place.
///
/// A single line has no such evidence, so it must be short as well as centred:
/// a title is a few words in the middle of the measure, an indented body line
/// runs most of the way across it.
///
/// There is no right-alignment here, and no variant for one. A table cell starts
/// well inside the column and ends near its right edge, which is exactly what a
/// right-aligned line looks like, and until the table itself is recognised the
/// two cannot be told apart. Inferring it turned eleven rows of an activity plan
/// into right-aligned paragraphs. When tables land they can bring it back.
fn alignment_of(lines: &[(f64, f64)], col_left: f64, col_right: f64, size: f64) -> Align {
    if lines.is_empty() || col_right <= col_left {
        return Align::Left;
    }
    let mid = (col_left + col_right) / 2.0;
    let width = col_right - col_left;
    let usable: Vec<&(f64, f64)> = lines
        .iter()
        .filter(|(l, r)| l.is_finite() && r.is_finite() && r > l)
        .collect();
    if usable.is_empty() {
        return Align::Left;
    }
    // Every line has to sit about the middle, or this is not centred text.
    if !usable
        .iter()
        .all(|(l, r)| ((l + r) / 2.0 - mid).abs() < size * 1.5)
    {
        return Align::Left;
    }
    // And every line has to be pulled in from both margins.
    if !usable
        .iter()
        .all(|(l, r)| l - col_left > size && col_right - r > size)
    {
        return Align::Left;
    }
    if usable.len() == 1 {
        let (l, r) = usable[0];
        return match r - l < width * 0.6 {
            true => Align::Center,
            false => Align::Left,
        };
    }
    // Several lines: their left edges must VARY, which is what centring does and
    // indenting does not.
    let min_left = usable.iter().map(|(l, _)| *l).fold(f64::INFINITY, f64::min);
    let max_left = usable
        .iter()
        .map(|(l, _)| *l)
        .fold(f64::NEG_INFINITY, f64::max);
    match max_left - min_left > size {
        true => Align::Center,
        false => Align::Left,
    }
}

/// A leading bullet or number, and the text after it.
fn list_marker(text: &str) -> Option<&str> {
    let t = text.trim_start();
    for bullet in [
        '\u{2022}', '\u{25CF}', '\u{25AA}', '\u{00B7}', '\u{2013}', '\u{2043}', '\u{2014}', '-',
        'o',
    ] {
        if let Some(rest) = t.strip_prefix(bullet) {
            // A bare "o" is only a bullet when something follows it as a word.
            if rest.starts_with(' ') && rest.trim().len() > 1 {
                return Some(rest.trim_start());
            }
        }
    }
    // "1." / "1)" / "a)" at the very start.
    let mut chars = t.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_alphanumeric() {
        return None;
    }
    for (i, c) in chars {
        match c {
            '.' | ')' => {
                let after = &t[i + c.len_utf8()..];
                // A marker is FOLLOWED BY A SPACE. Without that, a time reads as
                // one: "11.30: Udvalgscafé" was arriving as a list item saying
                // "30: Udvalgscafé", which is both wrong and unreadable.
                if !after.starts_with([' ', '\t']) {
                    return None;
                }
                let rest = after.trim_start();
                // A marker is short, and what follows it is not empty.
                if i > 3 || rest.is_empty() {
                    return None;
                }
                // A LETTERED marker is one letter, or a roman numeral. Any other
                // short word before a full stop is an abbreviation, and Danish
                // statutes are written in them: "Stk. 1" is subsection one, not
                // a bullet whose text is "1", which is how it was arriving.
                let head = &t[..i];
                let lettered = head.chars().all(char::is_alphabetic);
                let roman = head
                    .chars()
                    .all(|c| "ivxlcdm".contains(c.to_ascii_lowercase()));
                if lettered && head.chars().count() > 1 && !roman {
                    return None;
                }
                // And what follows a NUMBER starts with a capital. Danish is
                // full of ordinals that look exactly like list markers: "1. maj
                // 2025", "1. udgave", "1. oplag" were all arriving as list
                // items with the number eaten. A month and an ordinal noun are
                // lowercase; the first word of a list item is not.
                let numeric = t[..i].chars().all(|c| c.is_ascii_digit());
                if numeric && !rest.starts_with(char::is_uppercase) {
                    return None;
                }
                return Some(rest);
            }
            c if c.is_alphanumeric() => continue,
            _ => return None,
        }
    }
    None
}

/// Turn lines into blocks: paragraphs broken on the vertical gap, headings on
/// relative size, list items on a leading marker.
fn blocks_from(
    lines: Vec<Line>,
    printed: &HashMap<usize, String>,
    anchors: &HashMap<usize, Vec<String>>,
) -> Vec<Block> {
    let body = body_size(&lines);
    let column = column_right(&lines);
    let col_left = column_left(&lines);
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<Span> = Vec::new();
    let mut para_size = body;
    let mut prev: Option<&Line> = None;

    // A block's alignment is its FIRST line's: a centred title's second line is
    // centred too, and a left paragraph's last line is short without meaning
    // anything by it.
    let mut para_lines: Vec<(f64, f64)> = Vec::new();
    let flush = |para: &mut Vec<Span>,
                 size: f64,
                 geometry: &mut Vec<(f64, f64)>,
                 bullet: &mut Option<String>,
                 blocks: &mut Vec<Block>| {
        let drawn_bullet = bullet.take();
        let align = alignment_of(geometry, col_left, column, size);
        // The block's own left edge is the leftmost its lines reached. Not the
        // first line's: a paragraph whose opening line is indented has not moved
        // as a block, and taking the first would say it had.
        let left = geometry
            .iter()
            .map(|(l, _)| *l)
            .filter(|l| l.is_finite())
            .fold(f64::INFINITY, f64::min);
        let indent = indent_steps(left, col_left, size);
        geometry.clear();
        let spans = tidy(std::mem::take(para));
        if spans.is_empty() {
            return;
        }
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        if let Some((title, page)) = index_entry(&text) {
            let keep = title.chars().count();
            let page = page.to_string();
            blocks.push(Block::IndexEntry {
                spans: keep_prefix(spans, keep),
                page,
                indent,
            });
        } else if let Some(rest) = list_marker(&text) {
            // Drop the marker from the spans, keeping the colours of the words
            // that follow it.
            let dropped = text.chars().count() - rest.chars().count();
            blocks.push(Block::ListItem {
                spans: drop_prefix(spans, dropped),
                marker: drawn_bullet,
            });
        } else if drawn_bullet.is_some() {
            // A bullet the page DREW, so there is no marker in the text to take
            // off: the words are the item, whole.
            blocks.push(Block::ListItem {
                spans,
                marker: drawn_bullet,
            });
        } else if size > body * 1.12 {
            // Calibrated against Word's own defaults rather than picked: on an
            // 11pt body, Title is 28 (2.5x), Heading 1 is 16 (1.45x) and
            // Heading 2 is 13 (1.18x). Heading 3 is 12 (1.09x), which is below
            // the cutoff on purpose, because Word distinguishes it by weight and
            // colour rather than size and nothing here can see those.
            let level = if size > body * 1.7 {
                1
            } else if size > body * 1.3 {
                2
            } else {
                3
            };
            blocks.push(Block::Heading {
                level,
                spans,
                align,
            });
        } else {
            blocks.push(Block::Paragraph {
                spans,
                align,
                indent,
            });
        }
    };

    let lines_ref = lines;
    let mut para_bullet: Option<String> = None;
    let mut page = lines_ref.first().map(|l| l.page).unwrap_or(1);
    for (at, line) in lines_ref.iter().enumerate() {
        // An anchor goes BEFORE what it points at, so whatever was being built
        // ends here: a link should land on the start of the thing it names, not
        // in the middle of the paragraph above it.
        if let Some(here) = anchors.get(&at) {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut blocks,
            );
            prev = None;
            blocks.extend(here.iter().cloned().map(Block::Anchor));
        }
        // A page turning over ends whatever was being built: a paragraph that
        // continues across the break is rare, and running two pages together
        // silently is worse than one break too many.
        if line.page != page && line.page != 0 {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut blocks,
            );
            blocks.push(Block::PageBreak {
                ended: page,
                printed: printed.get(&(page - 1)).cloned(),
            });
            // Every page gets somewhere to land, whether or not a link names it
            // yet: a contents row whose own link is broken is pointed here
            // instead, by the number printed at the end of the row.
            blocks.push(Block::Anchor(format!("pdf-page-{}", line.page)));
            page = line.page;
            prev = None;
        }
        if let Some(picture) = &line.picture {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut blocks,
            );
            blocks.push(Block::Image(picture.clone()));
            prev = None;
            continue;
        }
        let starts_new = match prev {
            None => false,
            Some(p) => {
                let gap = p.y - line.y;
                // A line that stopped well short of the column did not wrap: it
                // ended. This is what tells an agenda from a paragraph, and the
                // vertical gap cannot, because both are single-spaced. Without
                // it every time on a programme ran into the next entry, so a
                // whole day arrived as one wall of text.
                //
                // Two ems of slack, because a wrapped line stops a word short of
                // the margin rather than exactly on it.
                let ended_early = p.right.is_finite() && p.right < column - p.size * 2.0;
                // And the other reasons: spacing, a size change, a list marker,
                // or the page turning over.
                ended_early
                    || starts_new_entry(&line.text)
                    || is_index_entry(&line.text)
                    || gap > p.size * 1.6
                    || (line.size - p.size).abs() > p.size * 0.15
                    || list_marker(&line.text).is_some()
                    // A page break: y jumps back UP the page.
                    || gap < -1.0
            }
        };
        if starts_new {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut blocks,
            );
        }
        if para.is_empty() {
            para_size = line.size;
            para_bullet = line.bullet.clone();
        } else {
            // A line break inside a paragraph is a space, not a join.
            if let Some(last) = para.last_mut() {
                last.text.push(' ');
            }
        }
        para.extend(line.spans.iter().cloned());
        para_lines.push((line.left, line.right));
        prev = Some(line);
    }
    flush(
        &mut para,
        para_size,
        &mut para_lines,
        &mut para_bullet,
        &mut blocks,
    );
    blocks
}

/// Drop the first `n` characters, keeping the colours of what survives.
fn drop_prefix(spans: Vec<Span>, n: usize) -> Vec<Span> {
    let mut left = n;
    let mut out = Vec::new();
    for span in spans {
        if left == 0 {
            out.push(span);
            continue;
        }
        let count = span.text.chars().count();
        if count <= left {
            left -= count;
            continue;
        }
        let text: String = span.text.chars().skip(left).collect();
        left = 0;
        out.push(Span { text, ..span });
    }
    tidy(out)
}

/// Keep the first `n` characters, with the colours of what survives.
fn keep_prefix(spans: Vec<Span>, n: usize) -> Vec<Span> {
    let mut left = n;
    let mut out = Vec::new();
    for span in spans {
        if left == 0 {
            break;
        }
        let count = span.text.chars().count();
        if count <= left {
            left -= count;
            out.push(span);
            continue;
        }
        let text: String = span.text.chars().take(left).collect();
        left = 0;
        out.push(Span { text, ..span });
    }
    tidy(out)
}

/// A link the page drew over part of itself: the box it covers, and where it
/// goes.
struct LinkArea {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    link: Link,
}

impl LinkArea {
    /// Whether a run of text falls under this box.
    ///
    /// The baseline decides it vertically, because that is the one point on a
    /// run that is certainly inside the box a producer drew around the line;
    /// its ascenders and descenders may not be. Horizontally, any overlap at
    /// all: a link ends mid-run only if a producer split it there, and half a
    /// word linked is worse than one word too many.
    /// Whether one point on the page falls inside this box.
    fn holds(&self, x: f64, y: f64) -> bool {
        y >= self.y0 - 1.0 && y <= self.y1 + 1.0 && x > self.x0 && x < self.x1
    }
}

/// Where a `/GoTo` lands: a page, and how far down it.
type Place = (usize, f64);

/// Follow a destination to the page it names and the height on that page.
///
/// A destination is an array whose head is the page and whose tail says where
/// to put it on screen. Only the forms that carry a height are read for one;
/// `/Fit` says "the whole page" and gets the top, which is the honest answer.
fn place_of(
    doc: &Document,
    dest: &Object,
    pages: &HashMap<lopdf::ObjectId, usize>,
) -> Option<Place> {
    let found = resolve(doc, dest).ok()?;
    // A named destination: a name or a string, standing for an array kept in the
    // catalogue. Word writes contents lists this way.
    let dest = match found {
        Object::Name(name) => named_destination(doc, name)?,
        Object::String(bytes, _) => named_destination(doc, bytes)?,
        // Some files wrap the array in a dictionary under /D.
        Object::Dictionary(d) => resolve(doc, d.get(b"D").ok()?).ok()?.clone(),
        other => other.clone(),
    };
    let items = dest.as_array().ok()?;
    let page = match items.first()? {
        Object::Reference(id) => *pages.get(&(id.0, id.1))?,
        // A bare number is a page index, counting from zero.
        other => number(other).ok()? as usize,
    };
    let top = match items.get(1).and_then(|o| o.as_name().ok()) {
        Some(b"XYZ") => items.get(3).and_then(|o| number(o).ok()),
        Some(b"FitH") | Some(b"FitBH") => items.get(2).and_then(|o| number(o).ok()),
        // Fit, FitV, FitR and friends put no height on the page: take the top.
        _ => None,
    };
    Some((page, top.unwrap_or(f64::INFINITY)))
}

/// Look a named destination up in the catalogue, in both places a file may keep
/// one: the modern `/Names /Dests` name tree, and the old `/Dests` dictionary.
fn named_destination(doc: &Document, name: &[u8]) -> Option<Object> {
    let catalog = doc.catalog().ok()?;
    let old = catalog
        .get(b"Dests")
        .ok()
        .and_then(|o| resolve(doc, o).ok())
        .and_then(|o| o.as_dict().ok())
        .and_then(|dict| dict.get(name).ok())
        .and_then(|found| resolve(doc, found).ok())
        .cloned();
    if old.is_some() {
        return old;
    }
    let names = resolve(doc, catalog.get(b"Names").ok()?).ok()?;
    let tree = names.as_dict().ok()?.get(b"Dests").ok()?.clone();
    search_name_tree(doc, &tree, name, 0)
}

/// Walk a name tree for one key. The tree is sorted, but it is also small here,
/// so this reads every leaf rather than bisecting.
fn search_name_tree(doc: &Document, node: &Object, want: &[u8], depth: usize) -> Option<Object> {
    // A malformed file can point a tree at itself.
    if depth > 32 {
        return None;
    }
    let node = resolve(doc, node).ok()?;
    let dict = node.as_dict().ok()?;
    let listed = dict
        .get(b"Names")
        .ok()
        .and_then(|o| resolve(doc, o).ok())
        .and_then(|o| o.as_array().ok().cloned());
    if let Some(pairs) = listed {
        for pair in pairs.chunks(2) {
            let (Some(key), Some(value)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let matches = match key {
                Object::String(bytes, _) => bytes.as_slice() == want,
                Object::Name(bytes) => bytes.as_slice() == want,
                _ => false,
            };
            if matches {
                return resolve(doc, value).ok().cloned();
            }
        }
    }
    let kids = resolve(doc, dict.get(b"Kids").ok()?).ok()?;
    for kid in kids.as_array().ok()? {
        if let Some(found) = search_name_tree(doc, kid, want, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// The links one page draws over itself.
///
/// `places` collects the destinations as they are met and hands each one a
/// name, so two rows pointing at the same song share an anchor.
fn page_links(
    doc: &Document,
    page_id: lopdf::ObjectId,
    pages: &HashMap<lopdf::ObjectId, usize>,
    places: &mut Vec<Place>,
) -> Vec<LinkArea> {
    let Ok(page) = doc.get_dictionary(page_id) else {
        return Vec::new();
    };
    let Some(annots) = page
        .get(b"Annots")
        .ok()
        .and_then(|o| resolve(doc, o).ok())
        .and_then(|o| o.as_array().ok().cloned())
    else {
        return Vec::new();
    };
    let mut areas = Vec::new();
    for annot in annots {
        let Some(annot) = resolve(doc, &annot)
            .ok()
            .and_then(|o| o.as_dict().ok().cloned())
        else {
            continue;
        };
        if annot.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Link".as_slice()) {
            continue;
        }
        // The action comes first: a file may carry both, and /A is the one a
        // viewer follows.
        let action = annot.get(b"A").ok().and_then(|o| resolve(doc, o).ok());
        let kind = action
            .as_ref()
            .and_then(|a| a.as_dict().ok())
            .and_then(|a| a.get(b"S").and_then(Object::as_name).ok())
            .map(<[u8]>::to_vec);
        let link = match kind.as_deref() {
            Some(b"URI") => action
                .as_ref()
                .and_then(|a| a.as_dict().ok())
                .and_then(|a| a.get(b"URI").ok())
                .and_then(|o| resolve(doc, o).ok())
                .and_then(|o| o.as_str().ok().map(<[u8]>::to_vec))
                .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                .and_then(safe_url)
                .map(Link::Url),
            _ => {
                let dest = match kind.as_deref() {
                    Some(b"GoTo") => action
                        .as_ref()
                        .and_then(|a| a.as_dict().ok())
                        .and_then(|a| a.get(b"D").ok())
                        .cloned(),
                    // No action, or one this does not follow: the annotation may
                    // still carry a plain destination.
                    _ => annot.get(b"Dest").ok().cloned(),
                };
                dest.and_then(|d| place_of(doc, &d, pages)).map(|place| {
                    let at = places.iter().position(|p| *p == place).unwrap_or_else(|| {
                        places.push(place);
                        places.len() - 1
                    });
                    Link::Place(format!("pdf-d{at}"))
                })
            }
        };
        let (Some(link), Ok(rect)) = (link, annot.get(b"Rect").and_then(|o| o.as_array())) else {
            continue;
        };
        let corners: Vec<f64> = rect.iter().filter_map(|o| number(o).ok()).collect();
        let [x0, y0, x1, y1] = corners[..] else {
            continue;
        };
        areas.push(LinkArea {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
            link,
        });
    }
    areas
}

/// Put an email link on the address it names, and point it there.
///
/// A stale annotation is worse here than anywhere else in a document, because
/// the reader cannot see where a link goes before following it, and following
/// it writes to a stranger. The samværspolitik is the case in hand: its
/// Trustmember list was retyped and the link boxes were not moved, so one box
/// sits exactly on `msthorup@gmail.com` while naming `magnus@muj.dk`, and
/// another covers two whole lines belonging to two other people.
///
/// The address is written right there in the text, so it decides. Any address
/// in the line becomes its own link to itself, and a mailto the file put on
/// words that are not an address is dropped when the line has one, because a
/// name linked to somebody else's inbox is the harm being undone. Where a line
/// carries no address at all, a mailto on a name is left alone: that one can
/// only be what the file says it is.
fn mend_email_spans(spans: Vec<Span>) -> Vec<Span> {
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    let found = emails_in(&text);
    if found.is_empty() {
        return spans;
    }
    // Character by character, so a span may be cut anywhere an address starts or
    // ends without the offsets having to be tracked through the spans.
    let mut chars: Vec<(char, Span)> = Vec::new();
    for span in &spans {
        for c in span.text.chars() {
            chars.push((
                c,
                Span {
                    text: String::new(),
                    ..span.clone()
                },
            ));
        }
    }
    for (from, to, address) in &found {
        for (_, span) in chars.iter_mut().take(*to).skip(*from) {
            span.link = Some(Link::Url(format!("mailto:{address}")));
        }
    }
    // And off everything else on the line: those are the stale ones.
    for (at, (_, span)) in chars.iter_mut().enumerate() {
        let inside = found.iter().any(|(from, to, _)| at >= *from && at < *to);
        if !inside && matches!(&span.link, Some(Link::Url(u)) if u.starts_with("mailto:")) {
            span.link = None;
        }
    }
    // Back into spans, merging what still matches.
    let mut out: Vec<Span> = Vec::new();
    for (c, attrs) in chars {
        match out.last_mut() {
            Some(last) if last.matches(&attrs) => last.text.push(c),
            _ => out.push(Span {
                text: c.to_string(),
                ..attrs
            }),
        }
    }
    out
}

/// Every email address in a line, as character ranges.
///
/// Found by looking either side of an `@`, which is what makes an address an
/// address. No pattern language: the shapes worth matching are simple, and the
/// ones that are not are not addresses.
fn emails_in(text: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let local = |c: char| c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-');
    let host = |c: char| c.is_alphanumeric() || matches!(c, '.' | '-');
    let mut out: Vec<(usize, usize, String)> = Vec::new();
    for (at, c) in chars.iter().enumerate() {
        if *c != '@' {
            continue;
        }
        let mut from = at;
        while from > 0 && local(chars[from - 1]) {
            from -= 1;
        }
        let mut to = at + 1;
        while to < chars.len() && host(chars[to]) {
            to += 1;
        }
        // A trailing stop is the sentence's, not the address's.
        while to > at + 1 && chars[to - 1] == '.' {
            to -= 1;
        }
        // Something before the @, and a dotted host after it with a real suffix.
        let domain: String = chars[at + 1..to].iter().collect();
        let suffix = domain.rsplit('.').next().unwrap_or_default();
        if from == at || !domain.contains('.') || suffix.len() < 2 {
            continue;
        }
        // Overlapping matches cannot happen, but a run of @s could produce one.
        if out.last().is_some_and(|(_, prev, _)| *prev > from) {
            continue;
        }
        out.push((from, to, chars[from..to].iter().collect()));
    }
    out
}

/// Only the web schemes, and only as an absolute address.
///
/// A PDF can ask a viewer to open anything it likes, and this one is opening it
/// inside a wiki someone trusts. `javascript:` and `data:` are the ones that
/// would run in that page's own origin; a relative address would resolve
/// against the wiki rather than against the document, and point somewhere the
/// file never named.
fn safe_url(url: String) -> Option<String> {
    let head = url.trim_start().to_ascii_lowercase();
    (head.starts_with("http://") || head.starts_with("https://") || head.starts_with("mailto:"))
        .then_some(url)
}

/// Take the running heads and folios off every page, and hand back the number
/// each page printed on itself.
///
/// The signal is POSITION, not words. A folio changes every page, so matching
/// text would never catch it; what does not change is where it sits and that it
/// sits alone. A line at the same height page after page, cut off from the body
/// by a gap far wider than the leading, is the running head or the page number,
/// and it is furniture: on the page it sits in the margin where the eye skips
/// it, but a reflow drops it into the middle of the reading, every page.
///
/// Two guards keep this off real text. The gap, because in an ordinary document
/// the first body line also starts at the same height on every page, and cutting
/// THAT would behead every page. And agreement: the lines either say the same
/// thing every time, which is a running head, or they are numbers, which is a
/// folio. A line that varies and is not a number is prose, and stays.
fn strip_running_furniture(pages: &mut [Vec<Line>]) -> HashMap<usize, String> {
    /// Where a candidate sat, to a couple of points: page geometry repeats
    /// exactly, but not always to the last decimal.
    fn key(y: f64, top: bool) -> (i64, bool) {
        ((y / 2.0).round() as i64, top)
    }
    // What each page offers up: its topmost and bottommost line, if either is
    // stranded away from the rest.
    let mut offered: Vec<Vec<(usize, (i64, bool))>> = Vec::new();
    for lines in pages.iter() {
        let text: Vec<(usize, &Line)> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.picture.is_none() && !l.text.trim().is_empty())
            .collect();
        let mut here = Vec::new();
        // Lines arrive down the page, so the first is the top and the last the
        // bottom. Three lines minimum: with two, "the rest of the page" is one
        // line and any gap looks like a stranding.
        if text.len() >= 3 {
            for (at, top) in [(0usize, true), (text.len() - 1, false)] {
                let (index, line) = text[at];
                let neighbour = match top {
                    true => text[1].1,
                    false => text[text.len() - 2].1,
                };
                let gap = (line.y - neighbour.y).abs();
                if gap > line.size * 2.5 {
                    here.push((index, key(line.y, top)));
                }
            }
        }
        offered.push(here);
    }
    // A place is furniture when most of the document uses it.
    let mut tally: HashMap<(i64, bool), Vec<(usize, usize)>> = HashMap::new();
    for (page_no, here) in offered.iter().enumerate() {
        for (index, k) in here {
            tally.entry(*k).or_default().push((page_no, *index));
        }
    }
    let with_text = pages
        .iter()
        .filter(|ls| ls.iter().any(|l| !l.text.trim().is_empty()))
        .count();
    let mut printed: HashMap<usize, String> = HashMap::new();
    let mut condemned: Vec<(usize, usize)> = Vec::new();
    for (_, seen) in tally {
        if seen.len() < 3 || seen.len() * 2 < with_text {
            continue;
        }
        let texts: Vec<&str> = seen
            .iter()
            .map(|(p, i)| pages[*p][*i].text.trim())
            .collect();
        // A folio is a NUMBER, with at most the decoration a designer puts round
        // one: "3", "- 3 -", "3.". Not a word with a number in it. "Kapitel 1"
        // repeats its position on every page and changes every page exactly as a
        // folio does, and it is a heading.
        let numeric = |t: &str| {
            let digits: String = t.chars().filter(char::is_ascii_digit).collect();
            let wordy = t.chars().any(char::is_alphabetic);
            (!digits.is_empty() && !wordy && t.chars().count() <= 12).then_some(digits)
        };
        let all_numbers = texts.iter().all(|t| numeric(t).is_some());
        let same = texts.windows(2).all(|w| w[0] == w[1]);
        if !all_numbers && !same {
            continue;
        }
        for ((page_no, index), text) in seen.iter().zip(texts.iter()) {
            if let Some(digits) = numeric(text) {
                printed.insert(*page_no, digits);
            }
            condemned.push((*page_no, *index));
        }
    }
    // Back to front, so removing one does not move the next.
    condemned.sort_unstable();
    for (page_no, index) in condemned.into_iter().rev() {
        pages[page_no].remove(index);
    }
    printed
}

/// Read a PDF and hand back what it says.
pub fn extract(bytes: &[u8]) -> Result<Extracted, String> {
    let doc = Document::load_mem(bytes).map_err(|e| format!("not a readable PDF: {e}"))?;
    let pages = doc.get_pages();
    let total = pages.len();
    // Which page each object id is, so a link's destination can name one.
    let by_id: HashMap<lopdf::ObjectId, usize> = pages
        .iter()
        .enumerate()
        .map(|(at, (_, id))| (*id, at))
        .collect();
    let mut places: Vec<Place> = Vec::new();
    let mut per_page: Vec<Vec<Line>> = Vec::new();
    let mut budget = PICTURE_BUDGET;
    for (page_no, (_, page_id)) in pages.into_iter().enumerate() {
        // The links live beside the content stream rather than in it, so they
        // are read first and handed to the walk, which cuts its runs on them.
        let areas = page_links(&doc, page_id, &by_id, &mut places);
        let drawn = page_runs(&doc, page_id, &mut budget, &areas);
        let mut lines = lines_from(drawn.runs);
        mark_the_lines(&mut lines, &drawn.marks);
        for line in &mut lines {
            line.page = page_no + 1;
        }
        // A picture takes its place in the flow by where it was drawn, so it
        // lands between the paragraphs it sat between on the page rather than
        // being swept to the end.
        for (y, picture) in drawn.pictures {
            lines.push(Line {
                y,
                size: picture.height.max(1.0),
                right: f64::NEG_INFINITY,
                left: f64::INFINITY,
                page: page_no + 1,
                text: String::new(),
                spans: Vec::new(),
                bullet: None,
                picture: Some(picture),
            });
        }
        lines.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));
        per_page.push(lines);
    }
    let printed = strip_running_furniture(&mut per_page);
    // Counted after the furniture goes: a page holding nothing but its own page
    // number has nothing on it to read.
    let empty = per_page
        .iter()
        .filter(|ls| {
            ls.iter()
                .all(|l| l.picture.is_some() || l.text.trim().is_empty())
        })
        .count();
    let all_lines: Vec<Line> = per_page.into_iter().flatten().collect();
    let anchors = anchors_for(&places, &all_lines);
    let mut blocks = blocks_from(all_lines, &printed, &anchors);
    mend_contents_links(&mut blocks, &places, &printed, total);
    Ok(Extracted {
        blocks,
        pages: total,
        pages_without_text: empty,
    })
}

/// Point a contents row at the page it names, when the file does not.
///
/// A contents list is the one place where a document says the same thing twice:
/// once in the link, and once in the number printed at the end of the row. The
/// two can be checked against each other, and they need to be, because the link
/// is often wrong. The songbook is the case in hand: all thirty-six links on its
/// index point back at the index itself, which poppler and this reader agree on,
/// so following them faithfully would take the reader nowhere.
///
/// When both agree on a page, the file's own link is kept: it names a height on
/// that page, so it lands on the song rather than on the page's first line.
/// When they disagree, the printed number wins, resolved through the folios to
/// whichever page printed it. A number that no page printed cannot be resolved,
/// and there the file's link stands, right or wrong.
fn mend_contents_links(
    blocks: &mut Vec<Block>,
    places: &[Place],
    printed: &HashMap<usize, String>,
    total: usize,
) -> usize {
    // Which page each anchor sits on, and which page printed each number.
    let anchored: HashMap<String, usize> = places
        .iter()
        .enumerate()
        .map(|(at, (page, _))| (format!("pdf-d{at}"), *page))
        .collect();
    let by_printed: HashMap<&str, usize> = printed
        .iter()
        .map(|(page, number)| (number.as_str(), *page))
        .collect();
    // Where a page's own folio was missed, the numbering still says where the
    // page is: a book's printed numbers run in a straight line, so the distance
    // from a number to the page carrying it is the same all through. Taken only
    // when nearly every folio agrees on it, because a book whose front matter is
    // numbered separately has no single offset and should not be guessed at.
    let offset = {
        let mut tally: HashMap<i64, usize> = HashMap::new();
        for (page, number) in printed {
            if let Ok(n) = number.parse::<i64>() {
                *tally.entry(*page as i64 - n).or_default() += 1;
            }
        }
        tally
            .into_iter()
            .max_by_key(|(_, seen)| *seen)
            .filter(|(_, seen)| seen * 5 >= printed.len() * 4)
            .map(|(offset, _)| offset)
    };
    let mut mended = 0usize;
    // One slot per contents row, in order, so the second pass can line them up
    // with the rows again. `None` where the row keeps whatever link it had.
    let mut wanted: Vec<Option<(String, usize)>> = Vec::new();
    for block in blocks.iter_mut() {
        let Block::IndexEntry { spans, page, .. } = block else {
            continue;
        };
        let found = by_printed.get(page.as_str()).copied().or_else(|| {
            let at = page.parse::<i64>().ok()? + offset?;
            (at >= 0 && (at as usize) < total).then_some(at as usize)
        });
        let Some(names) = found.as_ref() else {
            wanted.push(None);
            continue;
        };
        let agrees = match spans.iter().find_map(|s| s.link.as_ref()) {
            Some(Link::Place(id)) => anchored.get(id) == Some(names),
            // A row that leaves the document is doing something else entirely.
            Some(Link::Url(_)) => true,
            None => false,
        };
        if agrees {
            wanted.push(None);
            continue;
        }
        let link = Some(Link::Place(format!("pdf-page-{}", names + 1)));
        for (at, span) in spans.iter_mut().enumerate() {
            span.link = match at {
                0 => link.clone(),
                _ => None,
            };
        }
        let title = squash(&spans.iter().map(|s| s.text.as_str()).collect::<String>());
        wanted.push(Some((title, names + 1)));
        mended += 1;
    }
    aim_at_the_headings(blocks, &wanted);
    mended
}

/// Letters and digits only, for comparing a contents row against the heading it
/// names. What differs between the two is punctuation and spacing.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Move a mended row's target from the top of its page onto the thing it names.
///
/// A page number is a coarse answer: a song starting halfway down a page leaves
/// the reader at the tail of the song before it. But the row says what it is
/// looking for, so look for it, on the page it said. Where the words turn up,
/// an anchor goes in front of them and the row points there instead.
fn aim_at_the_headings(blocks: &mut Vec<Block>, wanted: &[Option<(String, usize)>]) {
    // Where each row's own words turn up, on the page that row named.
    let mut found: Vec<(usize, usize)> = Vec::new();
    let mut hit = vec![false; wanted.len()];
    let mut page = 1usize;
    for (at, block) in blocks.iter().enumerate() {
        if let Block::PageBreak { ended, .. } = block {
            page = ended + 1;
            continue;
        }
        // A contents row is not the heading it names, however well it matches.
        if matches!(block, Block::IndexEntry { .. }) {
            continue;
        }
        let here = squash(&block.text());
        // Short titles match too much: "RU" opens half the songs in this book.
        if here.len() < 6 {
            continue;
        }
        for (which, want) in wanted.iter().enumerate() {
            let Some((title, on)) = want else { continue };
            if !hit[which] && *on == page && title.len() >= 6 && here.starts_with(title.as_str()) {
                hit[which] = true;
                found.push((at, which));
                break;
            }
        }
    }
    if found.is_empty() {
        return;
    }
    // Point the rows at their new anchors before anything moves.
    let mut which = 0usize;
    for block in blocks.iter_mut() {
        if let Block::IndexEntry { spans, .. } = block {
            if hit[which] {
                if let Some(span) = spans.first_mut() {
                    span.link = Some(Link::Place(format!("pdf-t{which}")));
                }
            }
            which += 1;
        }
    }
    // Then put the anchors in, back to front so the earlier indices still hold.
    found.sort_unstable();
    for (at, which) in found.iter().rev() {
        blocks.insert(*at, Block::Anchor(format!("pdf-t{which}")));
    }
}

/// Give each line the mark drawn in the margin beside it.
///
/// A page draws plenty of small things, and most are furniture: rules, a device
/// in a header, a shadow under a box. What makes one a bullet is where it sits,
/// which is level with a line of text and just to its left, in the margin the
/// text is indented past. Nothing else about it says "bullet" at all: this
/// document's is the organisation's logo, drawn ten points square.
fn mark_the_lines(lines: &mut [Line], marks: &[Mark]) {
    for line in lines.iter_mut() {
        if line.picture.is_some() || !line.left.is_finite() {
            continue;
        }
        let found = marks.iter().find(|m| {
            // Level with the line: a bullet sits about the middle of the
            // lowercase letters, above the baseline it is set on.
            let level = m.middle > line.y - line.size * 0.2 && m.middle < line.y + line.size;
            // And in the margin to its left, within a couple of ems: further
            // out than that and it belongs to the page, not to this line.
            let beside = m.right <= line.left + 1.0 && line.left - m.x < line.size * 3.0;
            level && beside
        });
        line.bullet = found.map(|m| m.src.clone());
    }
}

/// Put each destination on the line it points at, so the links have something
/// to land on.
///
/// A destination is a height on a page, which in a reflow is no longer a place.
/// The nearest thing that survives is the first line at or below it: what the
/// reader would have seen at the top of the view had they followed the link on
/// the page. A destination past the end of its page falls to that page's first
/// line rather than being dropped, so the link still goes roughly right.
fn anchors_for(places: &[Place], lines: &[Line]) -> HashMap<usize, Vec<String>> {
    let mut out: HashMap<usize, Vec<String>> = HashMap::new();
    for (at, (page, top)) in places.iter().enumerate() {
        // Lines run down the page, so the first one at or below the destination
        // is the first that matches.
        let on_page = || lines.iter().position(|l| l.page == page + 1);
        let found = lines
            .iter()
            .position(|l| l.page == page + 1 && l.y <= top + 1.0)
            .or_else(on_page);
        if let Some(index) = found {
            out.entry(index).or_default().push(format!("pdf-d{at}"));
        }
    }
    out
}

/// Every destination this document's links name, for the harness to show.
#[cfg(test)]
fn places_of(bytes: &[u8]) -> Vec<(usize, Place)> {
    let Ok(doc) = Document::load_mem(bytes) else {
        return Vec::new();
    };
    let pages = doc.get_pages();
    let by_id: HashMap<lopdf::ObjectId, usize> = pages
        .iter()
        .enumerate()
        .map(|(at, (_, id))| (*id, at))
        .collect();
    let mut places = Vec::new();
    let mut from = Vec::new();
    for (page_no, (_, page_id)) in pages.iter().enumerate() {
        let before = places.len();
        page_links(&doc, *page_id, &by_id, &mut places);
        for _ in before..places.len() {
            from.push(page_no);
        }
    }
    from.into_iter().zip(places).collect()
}

/// What the furniture rule would take off this document, for the harness to
/// show. Exactly the walk [`extract`] does, stopping at the strip.
#[cfg(test)]
fn furniture_of(bytes: &[u8]) -> (Vec<String>, HashMap<usize, String>) {
    let Ok(doc) = Document::load_mem(bytes) else {
        return (Vec::new(), HashMap::new());
    };
    let mut per_page: Vec<Vec<Line>> = Vec::new();
    let mut budget = PICTURE_BUDGET;
    for (page_no, (_, page_id)) in doc.get_pages().into_iter().enumerate() {
        let drawn = page_runs(&doc, page_id, &mut budget, &[]);
        let mut lines = lines_from(drawn.runs);
        for line in &mut lines {
            line.page = page_no + 1;
        }
        lines.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));
        per_page.push(lines);
    }
    let before: Vec<String> = per_page
        .iter()
        .flatten()
        .map(|l| format!("{}|{}", l.page, l.text))
        .collect();
    let printed = strip_running_furniture(&mut per_page);
    let after: std::collections::HashSet<String> = per_page
        .iter()
        .flatten()
        .map(|l| format!("{}|{}", l.page, l.text))
        .collect();
    let gone = before.into_iter().filter(|k| !after.contains(k)).collect();
    (gone, printed)
}

/// The link boxes one page draws over itself, for the harness to show beside the
/// runs they are supposed to land on.
#[cfg(test)]
fn page_areas(bytes: &[u8], want: usize) -> Vec<(f64, f64, f64, f64, Link)> {
    let Ok(doc) = Document::load_mem(bytes) else {
        return Vec::new();
    };
    let pages = doc.get_pages();
    let by_id: HashMap<lopdf::ObjectId, usize> = pages
        .iter()
        .enumerate()
        .map(|(at, (_, id))| (*id, at))
        .collect();
    let mut places = Vec::new();
    for (page_no, (_, page_id)) in pages.iter().enumerate() {
        if page_no + 1 != want {
            continue;
        }
        return page_links(&doc, *page_id, &by_id, &mut places)
            .into_iter()
            .map(|a| (a.x0, a.y0, a.x1, a.y1, a.link))
            .collect();
    }
    Vec::new()
}

/// Every run one page drew, before they are joined into lines. The lens for
/// word-gap questions: what the joiner sees is where a missing space comes from.
#[cfg(test)]
fn page_runs_raw(bytes: &[u8], want: usize) -> Vec<Run> {
    let Ok(doc) = Document::load_mem(bytes) else {
        return Vec::new();
    };
    let mut budget = PICTURE_BUDGET;
    let pages = doc.get_pages();
    let by_id: HashMap<lopdf::ObjectId, usize> = pages
        .iter()
        .enumerate()
        .map(|(at, (_, id))| (*id, at))
        .collect();
    let mut places = Vec::new();
    for (page_no, (_, page_id)) in pages.iter().enumerate() {
        if page_no + 1 == want {
            let areas = page_links(&doc, *page_id, &by_id, &mut places);
            return page_runs(&doc, *page_id, &mut budget, &areas).runs;
        }
    }
    Vec::new()
}

/// Every line of one page with the geometry it was drawn at. A lens for the
/// harness, so the layout rules can be designed against real numbers.
#[cfg(test)]
fn page_lines(bytes: &[u8], want: usize) -> Vec<Line> {
    let Ok(doc) = Document::load_mem(bytes) else {
        return Vec::new();
    };
    let mut budget = PICTURE_BUDGET;
    for (page_no, (_, page_id)) in doc.get_pages().into_iter().enumerate() {
        if page_no + 1 != want {
            continue;
        }
        let drawn = page_runs(&doc, page_id, &mut budget, &[]);
        let mut lines = lines_from(drawn.runs);
        lines.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));
        return lines;
    }
    Vec::new()
}

#[cfg(test)]
mod harness {
    /// Every PDF in a directory, one line each, and what the furniture rule took
    /// off it. The regression surface for a change that touches every document.
    ///
    ///   PDF_SWEEP=/path/to/corpus cargo test pdf_text::harness -- --nocapture
    #[test]
    fn sweep_a_corpus() {
        let Ok(dir) = std::env::var("PDF_SWEEP") else {
            return;
        };
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("read the corpus")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
            .collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let bytes = std::fs::read(&path).expect("read");
            let (gone, folios) = super::furniture_of(&bytes);
            let numbered = folios.len();
            match super::extract(&bytes) {
                Err(e) => println!("{name:34.34} FAILED {e}"),
                Ok(doc) => {
                    let chars: usize = doc.blocks.iter().map(|b| b.text().len()).sum();
                    let drawn = doc
                        .blocks
                        .iter()
                        .filter(|b| {
                            matches!(
                                b,
                                super::Block::ListItem {
                                    marker: Some(_),
                                    ..
                                }
                            )
                        })
                        .count();
                    let linked = doc
                        .blocks
                        .iter()
                        .flat_map(|b| b.spans())
                        .filter(|s| s.link.is_some())
                        .count();
                    let anchors = doc
                        .blocks
                        .iter()
                        .filter(|b| matches!(b, super::Block::Anchor(_)))
                        .count();
                    let toc = doc
                        .blocks
                        .iter()
                        .filter(|b| matches!(b, super::Block::IndexEntry { .. }))
                        .count();
                    let indented = doc
                        .blocks
                        .iter()
                        .filter(|b| match b {
                            super::Block::Paragraph { indent, .. }
                            | super::Block::IndexEntry { indent, .. } => *indent > 0,
                            _ => false,
                        })
                        .count();
                    println!(
                        "{name:34.34} pages={:<4} blocks={:<5} chars={:<7} toc={toc:<4} bullets={drawn:<4} linked={linked:<4} anchors={anchors:<4} indent={indented:<4} numbered={numbered:<4} stripped={}",
                        doc.pages,
                        doc.blocks.len(),
                        chars,
                        gone.len()
                    );
                    // What it took, so a rule that eats prose is visible rather
                    // than merely plausible.
                    let mut shown: Vec<&String> = gone.iter().take(3).collect();
                    shown.dedup();
                    for g in shown {
                        println!("      - {}", g.chars().take(70).collect::<String>());
                    }
                }
            }
        }
    }

    /// Every link in a document beside what it actually lands on, which is the
    /// only way to see that a contents row points at its own song.
    ///
    ///   PDF_UNDER_TEST=/path/to.pdf PDF_LINKS=1 cargo test pdf_text::harness -- --nocapture
    #[test]
    fn follow_every_link() {
        let (Ok(path), Ok(_)) = (std::env::var("PDF_UNDER_TEST"), std::env::var("PDF_LINKS"))
        else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let (_, folios) = super::furniture_of(&bytes);
        let mut numbers: Vec<&String> = folios.values().collect();
        numbers.sort();
        println!(
            "  printed numbers: {} distinct of {} pages",
            {
                let mut u: Vec<&String> = numbers.clone();
                u.dedup();
                u.len()
            },
            folios.len()
        );
        for want in ["3", "15", "19", "21"] {
            let on: Vec<usize> = folios
                .iter()
                .filter(|(_, v)| v.as_str() == want)
                .map(|(k, _)| k + 1)
                .collect();
            println!("    \"{want}\" printed on file pages {on:?}");
        }
        let places = super::places_of(&bytes);
        println!("  {} destinations, resolved by lopdf:", places.len());
        for (from, (page, y)) in places.iter().take(6) {
            println!(
                "    link on page {} -> page {} at y {y}",
                from + 1,
                page + 1
            );
        }
        let doc = super::extract(&bytes).expect("read the pdf");
        // Where each anchor sits: the first words after it.
        let mut lands: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (at, block) in doc.blocks.iter().enumerate() {
            let super::Block::Anchor(id) = block else {
                continue;
            };
            let after = doc.blocks[at + 1..]
                .iter()
                .find(|b| !b.text().trim().is_empty())
                .map(|b| b.text())
                .unwrap_or_default();
            lands.insert(id.clone(), after.chars().take(46).collect());
        }
        // Every linked stretch of words beside the address it goes to, which is
        // the only way to see that a link points where it looks like it points.
        println!("  linked text -> destination:");
        for block in doc.blocks.iter() {
            for span in block.spans() {
                if let Some(super::Link::Url(url)) = &span.link {
                    println!("    {:?} -> {url}", span.text.trim());
                }
            }
        }
        let mut shown = 0;
        let mut rows = 0;
        for block in doc.blocks.iter() {
            let (title, page, link) = match block {
                super::Block::IndexEntry { spans, page, .. } => (
                    spans.iter().map(|s| s.text.as_str()).collect::<String>(),
                    page.clone(),
                    spans.iter().find_map(|s| s.link.clone()),
                ),
                _ => continue,
            };
            rows += 1;
            let target = match &link {
                Some(super::Link::Place(id)) => lands
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "!! nowhere !!".into()),
                Some(super::Link::Url(u)) => u.clone(),
                None => "!! not linked !!".into(),
            };
            let title: String = title.chars().take(40).collect();
            let squash = |s: &str| {
                s.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
            };
            let exact = squash(&target).starts_with(&squash(&title)) && !title.trim().is_empty();
            if exact {
                shown += 1;
                continue;
            }
            println!("  NOT ON ITS OWN TITLE: {title:<42.42} p.{page:<5} -> {target}");
        }
        println!("  {rows} contents rows, {shown} land on their own heading");
    }

    /// Just the words, for diffing against another reader.
    ///
    ///   PDF_UNDER_TEST=/path/to.pdf PDF_DUMP=1 cargo test pdf_text::harness -- --nocapture
    #[test]
    fn dump_the_words() {
        let (Ok(path), Ok(_)) = (std::env::var("PDF_UNDER_TEST"), std::env::var("PDF_DUMP")) else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let doc = super::extract(&bytes).expect("read the pdf");
        for block in doc.blocks.iter() {
            let text = block.text();
            if !text.trim().is_empty() {
                println!("{text}");
            }
        }
    }

    /// Every run of one page, with the gap to the run before it as a fraction of
    /// the em. Where a missing space comes from.
    ///
    ///   PDF_UNDER_TEST=/path/to.pdf PDF_RUNS=1 cargo test pdf_text::harness -- --nocapture
    #[test]
    fn look_at_the_runs() {
        let (Ok(path), Ok(page)) = (
            std::env::var("PDF_UNDER_TEST"),
            std::env::var("PDF_RUNS").map(|v| v.parse::<usize>().unwrap_or(1)),
        ) else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let want = std::env::var("PDF_NEAR").unwrap_or_default();
        println!("--- page {page} link boxes ---");
        for (x0, y0, x1, y1, link) in super::page_areas(&bytes, page) {
            println!(
                "  x {x0:>7.1}..{x1:<7.1} y {y0:>7.1}..{y1:<7.1} h={:<5.1} {link:?}",
                y1 - y0
            );
        }
        let mut runs = super::page_runs_raw(&bytes, page);
        runs.sort_by(|a, b| {
            b.y.partial_cmp(&a.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        println!("--- page {page} runs (x, end, gap/em) ---");
        let mut prev_end = f64::NEG_INFINITY;
        let mut prev_y = f64::NAN;
        for r in &runs {
            let fresh = (r.y - prev_y).abs() > 0.5;
            let gap = match fresh {
                true => f64::NAN,
                false => (r.x - prev_end) / r.size,
            };
            prev_end = r.end_x;
            prev_y = r.y;
            if want.is_empty() || r.text.contains(&want) {
                println!(
                    "  x={:>7.2} end={:>7.2} y={:>7.1} size={:>4.1} gap={:>6.3}  {:?}",
                    r.x, r.end_x, r.y, r.size, gap, r.text
                );
            }
        }
    }

    /// Every line of one page with where it sat, for designing layout rules.
    ///
    ///   PDF_UNDER_TEST=/path/to.pdf PDF_GEO=5 cargo test pdf_text::harness -- --nocapture
    #[test]
    fn look_at_one_page_geometry() {
        let (Ok(path), Ok(page)) = (
            std::env::var("PDF_UNDER_TEST"),
            std::env::var("PDF_GEO").map(|v| v.parse::<usize>().unwrap_or(1)),
        ) else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        println!("--- page {page} geometry (x, end, y, size) ---");
        for l in super::page_lines(&bytes, page) {
            println!(
                "  {:>6.1} {:>6.1} {:>7.1} {:>4.1}  {}",
                l.left,
                l.right,
                l.y,
                l.size,
                l.text.chars().take(78).collect::<String>()
            );
        }
    }

    /// Run the extractor over a real file and print what it made of it. Not a
    /// test: a lens, for checking the heuristics against the actual corpus.
    ///
    ///   PDF_UNDER_TEST=/path/to.pdf cargo test pdf_text::harness -- --nocapture
    #[test]
    fn look_at_a_real_pdf() {
        let Ok(path) = std::env::var("PDF_UNDER_TEST") else {
            return;
        };
        if std::env::var("PDF_GEO").is_ok() {
            return;
        }
        let bytes = std::fs::read(&path).expect("read");
        match super::extract(&bytes) {
            Err(e) => println!("FAILED {path}: {e}"),
            Ok(doc) => {
                let chars: usize = doc.blocks.iter().map(|b| block_text(b).len()).sum();
                println!(
                    "{path}\n  pages={} empty={} blocks={} chars={}",
                    doc.pages,
                    doc.pages_without_text,
                    doc.blocks.len(),
                    chars
                );
                let coloured: Vec<String> = doc
                    .blocks
                    .iter()
                    .flat_map(|b| b.spans())
                    .filter_map(|s| s.color.clone())
                    .collect();
                let mut seen: Vec<String> = coloured.clone();
                seen.sort();
                seen.dedup();
                println!("  coloured spans={} distinct={:?}", coloured.len(), seen);
                let bold = doc
                    .blocks
                    .iter()
                    .flat_map(|b| b.spans())
                    .filter(|s| s.bold)
                    .count();
                let italic = doc
                    .blocks
                    .iter()
                    .flat_map(|b| b.spans())
                    .filter(|s| s.italic)
                    .count();
                let total = doc.blocks.iter().flat_map(|b| b.spans()).count();
                println!("  spans={total} bold={bold} italic={italic}");
                for sp in doc
                    .blocks
                    .iter()
                    .flat_map(|b| b.spans())
                    .filter(|s| s.bold)
                    .take(6)
                {
                    println!("    bold: {}", sp.text.chars().take(60).collect::<String>());
                }
                let show: usize = std::env::var("PDF_SHOW")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(12);
                let skip: usize = std::env::var("PDF_SKIP")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                for b in doc.blocks.iter().skip(skip).take(show) {
                    let kind = match b {
                        super::Block::Heading { level, .. } => format!("h{level}"),
                        super::Block::ListItem { marker, .. } => match marker {
                            Some(_) => "li*".into(),
                            None => "li".into(),
                        },
                        super::Block::Paragraph { indent, .. } => match indent {
                            0 => "p".into(),
                            n => format!("p>{n}"),
                        },
                        super::Block::IndexEntry { page, indent, .. } => {
                            format!("toc>{indent} .{page}")
                        }
                        super::Block::Anchor(_) => "anch".into(),
                        super::Block::PageBreak { ended, printed } => match printed {
                            Some(p) => format!("--{ended}({p})--"),
                            None => format!("--{ended}--"),
                        },
                        super::Block::Image(p) => {
                            format!(
                                "img{}x{} {} {}B",
                                p.width.round(),
                                p.height.round(),
                                p.src.chars().take(24).collect::<String>(),
                                p.src.len()
                            )
                        }
                    };
                    let t = block_text(b);
                    let width: usize = std::env::var("PDF_WIDTH")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(90);
                    let a = match b.align() {
                        super::Align::Center => " [center]",
                        super::Align::Left => "",
                    };
                    println!(
                        "  {kind:<3}{a} {}",
                        t.chars().take(width).collect::<String>()
                    );
                }
            }
        }
    }

    fn block_text(b: &super::Block) -> String {
        b.text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The map most of Word's fonts ship, in both of its shapes.
    #[test]
    fn a_tounicode_map_is_read_in_both_forms() {
        let cmap = br#"
            /CIDInit /ProcSet findresource begin
            2 beginbfchar
            <0003> <0020>
            <0044> <00E6>
            endbfchar
            2 beginbfrange
            <0010> <0012> <0041>
            <0020> <0022> [<00F8> <00E5> <0041>]
            endbfrange
            endcmap
        "#;
        let map = parse_to_unicode(cmap);
        // bfchar: a code to one character, including a Danish one.
        assert_eq!(map.get(&0x0003).map(String::as_str), Some(" "));
        assert_eq!(map.get(&0x0044).map(String::as_str), Some("æ"));
        // bfrange, consecutive: 0x10..0x12 maps to A, B, C.
        assert_eq!(map.get(&0x0010).map(String::as_str), Some("A"));
        assert_eq!(map.get(&0x0012).map(String::as_str), Some("C"));
        // bfrange, listed: one destination per code.
        assert_eq!(map.get(&0x0020).map(String::as_str), Some("ø"));
        assert_eq!(map.get(&0x0021).map(String::as_str), Some("å"));
    }

    /// A surrogate pair and a ligature both arrive as several UTF-16 units.
    #[test]
    fn a_destination_may_be_more_than_one_unit() {
        assert_eq!(hex_to_string("00660069").as_deref(), Some("fi"));
        assert_eq!(hex_to_string("00E5").as_deref(), Some("å"));
        assert_eq!(hex_to_string(""), None);
    }

    /// The fallback for a font with no map. Danish sits in the Latin-1 range,
    /// which is why this covers the documents that need it.
    #[test]
    fn win_ansi_covers_danish_and_the_windows_block() {
        assert_eq!(win_ansi_char(0xE6), Some('æ'));
        assert_eq!(win_ansi_char(0xF8), Some('ø'));
        assert_eq!(win_ansi_char(0xE5), Some('å'));
        // 0x92 is a right single quote in WinAnsi, NOT the Latin-1 control.
        assert_eq!(win_ansi_char(0x92), Some('\u{2019}'));
        // Control codes are not text.
        assert_eq!(win_ansi_char(0x07), None);
    }

    /// A malformed range must not try to allocate the world.
    #[test]
    fn an_absurd_range_is_declined() {
        let mut map = HashMap::new();
        apply_range(&["0000", "FFFFFF", "0041"], &mut map);
        assert!(map.is_empty());
        // And a backwards one.
        apply_range(&["0100", "0010", "0041"], &mut map);
        assert!(map.is_empty());
    }

    #[test]
    fn a_bullet_or_a_number_starts_a_list_item() {
        assert_eq!(list_marker("\u{2022} Første punkt"), Some("Første punkt"));
        assert_eq!(list_marker("1. Første punkt"), Some("Første punkt"));
        assert_eq!(list_marker("a) Andet punkt"), Some("Andet punkt"));
        assert_eq!(list_marker("o Tredje punkt"), Some("Tredje punkt"));
    }

    /// Ordinary prose must not be mistaken for a list, or half a document
    /// becomes bullets. A sentence ending in a full stop is the trap.
    #[test]
    fn prose_is_not_a_list() {
        assert_eq!(list_marker("Dette er en sætning. Og en til."), None);
        assert_eq!(
            list_marker("Forsamlingen besluttede at udsætte sagen"),
            None
        );
        // A long run before the dot is a sentence, not a marker.
        assert_eq!(list_marker("Landsmoedet. Noget"), None);
        assert_eq!(list_marker(""), None);
        // A marker with nothing after it is not a list item either.
        assert_eq!(list_marker("1."), None);
    }

    /// A line that reached the column's right edge, so it WRAPPED. The
    /// short-line rule is exercised by `full` below.
    fn line(y: f64, size: f64, text: &str) -> Line {
        Line {
            y,
            size,
            right: 400.0,
            left: 0.0,
            page: 1,
            text: text.into(),
            spans: vec![Span {
                link: None,
                text: text.into(),
                color: None,
                bold: false,
                italic: false,
            }],
            bullet: None,
            picture: None,
        }
    }

    fn texts(blocks: &[Block]) -> Vec<String> {
        blocks.iter().map(|b| b.text()).collect()
    }

    /// Lines close together are one paragraph; a wider gap starts another. This
    /// is the rule the whole reconstruction rests on.
    #[test]
    fn the_vertical_gap_decides_where_a_paragraph_ends() {
        let blocks = blocks_from(
            vec![
                line(700.0, 11.0, "Første linje i afsnittet"),
                line(686.0, 11.0, "og anden linje."),
                // A gap of 30 against a size of 11: a new paragraph.
                line(656.0, 11.0, "Et nyt afsnit."),
            ],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(
            texts(&blocks),
            vec!["Første linje i afsnittet og anden linje.", "Et nyt afsnit.",]
        );
    }

    /// A larger line is a heading, and how much larger decides the level.
    #[test]
    fn size_relative_to_the_body_makes_a_heading() {
        let blocks = blocks_from(
            vec![
                // Word's own sizes: Title 28, Heading 1 16, body 11.
                line(700.0, 28.0, "Titel"),
                line(660.0, 16.0, "Overskrift"),
                line(630.0, 11.0, "Brødtekst her."),
                line(616.0, 11.0, "Mere brødtekst."),
                line(600.0, 11.0, "Endnu mere."),
            ],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(
            texts(&blocks),
            vec![
                "Titel",
                "Overskrift",
                "Brødtekst her. Mere brødtekst. Endnu mere.",
            ]
        );
        assert!(matches!(blocks[0], Block::Heading { level: 1, .. }));
        assert!(matches!(blocks[1], Block::Heading { level: 2, .. }));
    }

    /// A page break sends y back UP the page, and must not be read as the
    /// paragraph continuing.
    #[test]
    fn a_page_break_ends_the_paragraph() {
        let blocks = blocks_from(
            vec![
                line(60.0, 11.0, "Sidste linje på siden."),
                line(700.0, 11.0, "Første linje på næste side."),
            ],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(blocks.len(), 2, "a page break starts a new block");
    }

    /// A scan: pages, but nothing on them.
    #[test]
    fn a_document_with_no_text_says_so() {
        let scan = Extracted {
            blocks: vec![],
            pages: 7,
            pages_without_text: 7,
        };
        assert!(!scan.has_text());
        let real = Extracted {
            blocks: vec![Block::Paragraph {
                indent: 0,
                spans: vec![Span {
                    link: None,
                    text: "noget".into(),
                    color: None,
                    bold: false,
                    italic: false,
                }],
                align: Align::Left,
            }],
            pages: 2,
            pages_without_text: 0,
        };
        assert!(real.has_text());
    }

    /// Black is not a colour, it is the default ink. Keeping it would force
    /// black text onto a dark reading surface, which is the same mistake the
    /// Word renderer had to unlearn.
    #[test]
    fn ordinary_ink_is_left_to_the_reading_surface() {
        assert_eq!(ink(0.0, 0.0, 0.0), None);
        assert_eq!(ink(0.05, 0.05, 0.05), None);
        // Office's own body-text near-black, #201f1e, and Google's #202124.
        // These were the most common colours in the corpus and are the default
        // ink of those tools, not a decision.
        assert_eq!(ink(0.125, 0.122, 0.118), None);
        assert_eq!(ink(0.125, 0.129, 0.141), None);
        // And white, which would otherwise be an invisible paragraph: the shape
        // it was written on is not drawn here.
        assert_eq!(ink(1.0, 1.0, 1.0), None);
        // A mid grey IS a choice, and is kept.
        assert_eq!(ink(0.5, 0.5, 0.5).as_deref(), Some("#808080"));
    }

    /// A stated colour survives, in each of the ways a PDF can state it.
    #[test]
    fn a_stated_colour_is_kept() {
        assert_eq!(ink(1.0, 0.0, 0.0).as_deref(), Some("#ff0000"));
        // Word's default heading blue, near enough. Dark, but its channels are
        // far enough apart that the near-black rule cannot swallow it.
        assert_eq!(ink(0.17, 0.33, 0.59).as_deref(), Some("#2b5496"));
        // And the darkest of Word's heading blues, #1f3864, which sits below the
        // luma cutoff and survives on spread alone.
        assert_eq!(ink(0.122, 0.22, 0.392).as_deref(), Some("#1f3864"));
        // CMYK cyan.
        assert_eq!(cmyk(1.0, 0.0, 0.0, 0.0).as_deref(), Some("#00ffff"));
        // CMYK black is still ordinary ink.
        assert_eq!(cmyk(0.0, 0.0, 0.0, 1.0), None);
    }

    /// A span with no styling but its colour, for the fixtures below.
    fn span(text: &str, color: Option<&str>) -> Span {
        Span {
            link: None,
            text: text.into(),
            color: color.map(str::to_string),
            bold: false,
            italic: false,
        }
    }

    fn run(x: f64, end_x: f64, text: &str, color: Option<&str>, bold: bool) -> Run {
        Run {
            link: None,
            x,
            end_x,
            y: 700.0,
            size: 11.0,
            text: text.into(),
            color: color.map(str::to_string),
            bold,
            italic: false,
        }
    }

    /// A colour change inside a line becomes a span boundary, so one red word in
    /// a black sentence keeps its colour and the rest keeps none.
    #[test]
    fn a_colour_change_splits_the_line_and_nothing_else() {
        let lines = lines_from(vec![
            // Gaps of 4pt at 11pt type: a real word space, comfortably over
            // the 0.20em the joiner asks for.
            run(0.0, 30.0, "Vi stemte", None, false),
            run(34.0, 46.0, "nej", Some("#ff0000"), false),
            run(50.0, 74.0, "til forslaget", None, false),
        ]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Vi stemte nej til forslaget");
        // The gap-space lands on the span BEFORE the change, so a colour never
        // begins with whitespace. Where it sits is invisible either way, a space
        // having no glyph to take the colour; what matters is that the coloured
        // WORD is its own span and the text around it carries no colour at all.
        assert_eq!(
            lines[0].spans,
            vec![
                span("Vi stemte ", None),
                span("nej ", Some("#ff0000")),
                span("til forslaget", None),
            ]
        );
    }

    /// A face change splits a span the same way a colour does, so a bold word
    /// inside a sentence stays bold and nothing around it does. There is no
    /// "make this bold" operator in a PDF: the writer switches font, and the
    /// weight has to be read off the font resource.
    #[test]
    fn a_bold_word_is_its_own_span() {
        let lines = lines_from(vec![
            run(0.0, 30.0, "Forslaget blev", None, false),
            run(34.0, 52.0, "vedtaget", None, true),
            run(56.0, 70.0, "enstemmigt", None, false),
        ]);
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans.len(), 3, "one span per face");
        assert!(!spans[0].bold);
        assert!(spans[1].bold, "the emphasised word keeps its weight");
        assert!(!spans[2].bold);
        assert_eq!(lines[0].text, "Forslaget blev vedtaget enstemmigt");
    }

    /// Stripping a list marker must not take the colour of the words after it.
    #[test]
    fn dropping_a_marker_keeps_the_colours_that_follow() {
        let spans = vec![span("1. ", None), span("Rødt punkt", Some("#ff0000"))];
        assert_eq!(
            drop_prefix(spans, 3),
            vec![span("Rødt punkt", Some("#ff0000"))]
        );
    }

    /// A line that stopped well short of the column ENDED. Without this every
    /// entry on a programme ran into the next and a whole day arrived as one
    /// wall of text, which is what the agenda for LM 2026 looked like.
    #[test]
    fn a_line_that_stops_short_ends_the_block() {
        let short = |y: f64, right: f64, text: &str| Line {
            y,
            size: 11.0,
            right,
            left: 0.0,
            page: 1,
            text: text.into(),
            spans: vec![Span {
                link: None,
                text: text.into(),
                color: None,
                bold: false,
                italic: false,
            }],
            bullet: None,
            picture: None,
        };
        // Column runs to 400. The first line stops at 180, far short of it.
        let blocks = blocks_from(
            vec![
                short(700.0, 180.0, "16:30 Ankomst"),
                short(686.0, 400.0, "17:50 Åbning af landsmødet"),
                short(672.0, 398.0, "og en fortsættelse af samme punkt"),
            ],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(
            texts(&blocks),
            vec![
                "16:30 Ankomst",
                "17:50 Åbning af landsmødet og en fortsættelse af samme punkt",
            ],
            "a short line ends its block; a full-width one wraps into the next"
        );
    }

    /// The rule that catches what the short-line one cannot: an entry whose text
    /// happens to fill the column, followed by the next entry.
    #[test]
    fn a_clock_time_opens_a_new_entry() {
        assert!(starts_new_entry("16:30 Ankomst"));
        assert!(starts_new_entry("08.00 Morgenmad"));
        assert!(starts_new_entry(" 9:15 Morgenmad"));
        // Not a time: a date, a decimal, a version, ordinary prose.
        assert!(!starts_new_entry("Fredag d. 14. august:"));
        assert!(!starts_new_entry("14.302 kroner"));
        assert!(!starts_new_entry("2026 var et år"));
        assert!(!starts_new_entry("Deltagere: Kristine"));
        assert!(!starts_new_entry(""));
    }

    /// Word writes a plain hyphen for its bullets as often as a real one, and
    /// the agenda's sub-points arrived as paragraphs until this.
    #[test]
    fn a_hyphen_is_a_bullet_too() {
        assert_eq!(
            list_marker("- Valg af dirigenter"),
            Some("Valg af dirigenter")
        );
        assert_eq!(list_marker("\u{2014} Et punkt"), Some("Et punkt"));
        // But not a hyphenated word or a minus sign.
        assert_eq!(list_marker("-5 grader"), None);
        assert_eq!(list_marker("noget-andet"), None);
    }

    /// A time is not a numbered list marker, however much "11." looks like one.
    #[test]
    fn a_time_is_not_a_list_marker() {
        assert_eq!(list_marker("11.30: Udvalgscafé"), None);
        assert_eq!(list_marker("12.20 Valg af revisionsselskab"), None);
        // The real thing still works.
        assert_eq!(list_marker("11. Et punkt"), Some("Et punkt"));
    }

    /// A contents page defeats every other rule at once: leader dots fill each
    /// line to the column edge, the lines are single-spaced, and there is no
    /// time to open on. The songbook's index arrived as one paragraph per
    /// section with every song run together inside it.
    #[test]
    fn leader_dots_mark_an_index_entry() {
        assert!(is_index_entry("Kampsange...............................3"));
        assert!(is_index_entry("Ode til Rohde......  62"));
        // An ellipsis is not a leader, in either spelling.
        assert!(!is_index_entry("Hør nu kampens toner..."));
        assert!(!is_index_entry("Hør nu kampens toner…"));
        assert!(!is_index_entry("En ganske almindelig sætning."));
    }

    /// Danish is full of ordinals that look exactly like list markers. These
    /// were arriving as list items with the number eaten, so the songbook's
    /// colophon read "maj 2025" and "udgave".
    #[test]
    fn an_ordinal_is_not_a_list_marker() {
        assert_eq!(list_marker("1. maj 2025"), None);
        assert_eq!(list_marker("1. udgave"), None);
        assert_eq!(list_marker("2. oplag"), None);
        // A real numbered item still is one: its text starts with a capital.
        assert_eq!(list_marker("1. Første punkt"), Some("Første punkt"));
        // And a bullet needs no such test, having no other meaning.
        assert_eq!(
            list_marker("- valg af dirigenter"),
            Some("valg af dirigenter")
        );
    }

    /// The raw-sample path. JPEG needs no encoder, being a JPEG already; this is
    /// what everything else in the corpus needs, and a wrong stride or row order
    /// shows up as a sheared or upside-down picture rather than an error.
    #[test]
    fn raw_samples_become_a_readable_bmp() {
        use base64::Engine;
        // Two by two: red, green / blue, white.
        let samples = [
            255, 0, 0, 0, 255, 0, // top row
            0, 0, 255, 255, 255, 255, // bottom row
        ];
        let url = bmp_data_url(2, 2, 3, &samples).expect("encodes");
        let b64 = url
            .strip_prefix("data:image/bmp;base64,")
            .expect("data url");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("valid base64");

        assert_eq!(&bytes[0..2], b"BM", "the magic every reader looks for");
        assert_eq!(u32::from_le_bytes(bytes[10..14].try_into().unwrap()), 54);
        assert_eq!(i32::from_le_bytes(bytes[18..22].try_into().unwrap()), 2);
        assert_eq!(i32::from_le_bytes(bytes[22..26].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(bytes[28..30].try_into().unwrap()), 24);

        // Rows run bottom-up and pixels are BGR, so the FIRST row of pixel data
        // is the image's LAST row: blue then white.
        let row = &bytes[54..54 + 6];
        assert_eq!(row, &[255, 0, 0, 255, 255, 255], "blue, then white");
        // Each row is padded to a multiple of four bytes: 6 becomes 8.
        let second = &bytes[62..62 + 6];
        assert_eq!(second, &[0, 0, 255, 0, 255, 0], "red, then green");
    }

    /// Grey expands to grey rather than being read as a third of a colour.
    #[test]
    fn a_grey_image_is_not_misread() {
        use base64::Engine;
        let url = bmp_data_url(2, 1, 1, &[0, 255]).expect("encodes");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/bmp;base64,").unwrap())
            .unwrap();
        assert_eq!(
            &bytes[54..60],
            &[0, 0, 0, 255, 255, 255],
            "black, then white"
        );
    }

    /// A truncated or impossible image is declined rather than panicking on a
    /// slice: these bytes come from a file anyone can upload.
    #[test]
    fn a_broken_image_is_declined() {
        assert_eq!(bmp_data_url(0, 4, 3, &[0; 12]), None);
        assert_eq!(bmp_data_url(4, 4, 3, &[0; 3]), None, "not enough samples");
        assert_eq!(
            bmp_data_url(2, 2, 2, &[0; 8]),
            None,
            "two components is not a colour space we read"
        );
    }

    /// A centred title stays centred at any width, which is the part of page
    /// composition that survives reflow.
    #[test]
    fn a_short_line_in_the_middle_is_centred() {
        // Column runs 72..472, so its middle is 272 and its width is 400.
        let (l, r) = (72.0, 472.0);
        assert_eq!(alignment_of(&[(200.0, 344.0)], l, r, 11.0), Align::Center);
        // A full-width line is left, however you look at it.
        assert_eq!(alignment_of(&[(72.0, 470.0)], l, r, 11.0), Align::Left);
        // The last line of a left paragraph: starts at the margin, ends early.
        assert_eq!(alignment_of(&[(72.0, 200.0)], l, r, 11.0), Align::Left);
        assert_eq!(alignment_of(&[], l, r, 11.0), Align::Left);
    }

    /// The distinction one line cannot make. An indented block and a centred
    /// block both sit about the middle with both edges pulled in; what tells
    /// them apart is that centred lines each START somewhere different.
    ///
    /// Got this wrong first time round, and a policy document full of indented
    /// paragraphs came out centred.
    #[test]
    fn an_indented_block_is_not_a_centred_one() {
        let (l, r) = (72.0, 472.0);
        // Every line starts at 122 and ends raggedly: indented, not centred.
        let indented = [(122.0, 430.0), (122.0, 419.0), (122.0, 400.0)];
        assert_eq!(alignment_of(&indented, l, r, 11.0), Align::Left);
        // Lines that each begin somewhere else, about the same middle: centred.
        let centred = [(150.0, 394.0), (180.0, 364.0), (120.0, 424.0)];
        assert_eq!(alignment_of(&centred, l, r, 11.0), Align::Center);
    }

    /// A single line that runs most of the way across is an indented line, not a
    /// title, whatever its midpoint says.
    #[test]
    fn a_long_lone_line_is_not_a_title() {
        let (l, r) = (72.0, 472.0);
        assert_eq!(alignment_of(&[(100.0, 444.0)], l, r, 11.0), Align::Left);
    }

    /// The page turning over ends whatever was being built, and leaves a mark
    /// carrying the number of the page that just finished.
    #[test]
    fn a_page_ending_leaves_a_mark() {
        let on_page = |page: usize, y: f64, text: &str| Line {
            y,
            size: 11.0,
            right: 400.0,
            left: 0.0,
            page,
            text: text.into(),
            spans: vec![span(text, None)],
            bullet: None,
            picture: None,
        };
        let blocks = blocks_from(
            vec![
                on_page(1, 700.0, "Sidste linje på side et"),
                on_page(2, 700.0, "Første linje på side to"),
            ],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(blocks.len(), 4, "text, break, the new page's anchor, text");
        assert_eq!(
            blocks[1],
            Block::PageBreak {
                ended: 1,
                printed: None
            },
            "the page that just ended"
        );
        assert_eq!(
            blocks[2],
            Block::Anchor("pdf-page-2".into()),
            "somewhere for a link to page two to land"
        );
        assert_eq!(
            texts(&blocks),
            vec!["Sidste linje på side et", "", "", "Første linje på side to"]
        );
    }

    /// An address is found by looking either side of the @.
    #[test]
    fn an_address_is_found_in_a_line_of_prose() {
        let line = "Marie Strunge Thorup, msthorup@gmail.com, 23 60 66 61.";
        let found = emails_in(line);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].2, "msthorup@gmail.com",
            "and not the comma after it"
        );
        // A sentence's full stop is the sentence's.
        assert_eq!(
            emails_in("Skriv til anja@example.dk.")[0].2,
            "anja@example.dk"
        );
        // Two on a line, both found.
        assert_eq!(emails_in("a@b.dk og c@d.org").len(), 2);
        // And what is not an address.
        assert!(emails_in("klokken 12@13").is_empty());
        assert!(emails_in("@handle").is_empty());
        assert!(emails_in("a@b").is_empty(), "a host needs a dot");
    }

    /// The harm being undone: a link box left behind when the text under it was
    /// retyped, so it names one person and sits on another.
    #[test]
    fn a_stale_email_link_is_pointed_at_the_address_it_sits_on() {
        let stale = Some(Link::Url("mailto:magnus@muj.dk".into()));
        let spans = vec![Span {
            text: "Marie Strunge Thorup, msthorup@gmail.com, 23 60 66 61".into(),
            color: None,
            bold: false,
            italic: false,
            link: stale,
        }];
        let mended = mend_email_spans(spans);
        let linked: Vec<(&str, &Link)> = mended
            .iter()
            .filter_map(|s| s.link.as_ref().map(|l| (s.text.as_str(), l)))
            .collect();
        assert_eq!(
            linked,
            vec![(
                "msthorup@gmail.com",
                &Link::Url("mailto:msthorup@gmail.com".into())
            )],
            "the address links to itself, and the name is not a link at all"
        );
    }

    /// But a name the file linked to an address, with no address written beside
    /// it, can only be what the file says it is.
    #[test]
    fn a_name_linked_to_an_address_is_left_alone() {
        let link = Some(Link::Url("mailto:formand@example.dk".into()));
        let spans = vec![Span {
            text: "skriv til formanden".into(),
            color: None,
            bold: false,
            italic: false,
            link: link.clone(),
        }];
        assert_eq!(mend_email_spans(spans.clone()), spans);
    }

    /// A statute is written in abbreviations, and they are not bullets.
    #[test]
    fn an_abbreviation_is_not_a_list_marker() {
        // The vedtægter, where every subsection opens this way. It was arriving
        // as a bullet whose entire text was the number.
        assert_eq!(
            list_marker("Stk. 1 Foreningens navn er Radikal Ungdom"),
            None
        );
        assert_eq!(list_marker("Nr. 4 er vedtaget"), None);
        assert_eq!(list_marker("Bl.a. dette"), None);
        // What a lettered marker actually looks like, and still does.
        assert_eq!(list_marker("a) Første forslag"), Some("Første forslag"));
        assert_eq!(list_marker("iv) Fjerde forslag"), Some("Fjerde forslag"));
        assert_eq!(list_marker("1. Første punkt"), Some("Første punkt"));
    }

    /// A contents row is read from its end: the page number, then the leader
    /// that carried the eye to it.
    #[test]
    fn a_contents_row_splits_into_what_it_names_and_where() {
        assert_eq!(
            index_entry("Kampsange....................3"),
            Some(("Kampsange", "3"))
        );
        // The file may leave a space before the leader, or none before the
        // number, and both are the same row.
        assert_eq!(
            index_entry("Radikal Ungdoms holdning til rigsfællesskabet .........19"),
            Some(("Radikal Ungdoms holdning til rigsfællesskabet", "19"))
        );
        assert_eq!(
            index_entry("Internationale................................9"),
            Some(("Internationale", "9"))
        );
    }

    /// And a sentence that merely ends in a digit is left alone.
    #[test]
    fn prose_is_not_a_contents_row() {
        assert_eq!(index_entry("Vi mødes i 2025"), None);
        assert_eq!(
            index_entry("Der var engang… 7"),
            None,
            "one ellipsis is not a leader"
        );
        assert_eq!(index_entry("1. maj 2025"), None);
        assert_eq!(index_entry("Kampsange...."), None, "no page, no row");
        assert_eq!(index_entry("....7"), None, "no title, no row");
    }

    /// The inset is kept as depth rather than as points: the page's column is
    /// not the column this will be read in.
    #[test]
    fn an_inset_becomes_a_depth() {
        // The songbook's index: sections at the margin, songs one step in.
        assert_eq!(indent_steps(56.7, 56.7, 11.0), 0);
        assert_eq!(indent_steps(74.7, 56.7, 11.0), 1);
        // A policy's numbered clauses, two and four levels down.
        assert_eq!(indent_steps(96.0, 56.7, 11.0), 2);
        assert_eq!(indent_steps(132.2, 56.7, 11.0), 4);
        // A hair of drift is not an indent, and nothing goes deeper than four.
        assert_eq!(indent_steps(59.0, 56.7, 11.0), 0);
        assert_eq!(indent_steps(400.0, 56.7, 11.0), 4);
    }

    /// The folio is furniture: on the page it sits in the margin where the eye
    /// skips it, and in a reflow it lands in the middle of the reading.
    #[test]
    fn a_page_number_printed_every_page_is_taken_off_it() {
        let body = |page: usize, y: f64, text: &str| Line {
            y,
            size: 11.0,
            right: 400.0,
            left: 56.0,
            page,
            text: text.into(),
            spans: vec![span(text, None)],
            bullet: None,
            picture: None,
        };
        let folio = |page: usize, text: &str| Line {
            y: 37.7,
            size: 12.0,
            right: 538.0,
            left: 532.0,
            page,
            text: text.into(),
            spans: vec![span(text, None)],
            bullet: None,
            picture: None,
        };
        let mut pages: Vec<Vec<Line>> = (1..=4)
            .map(|p| {
                vec![
                    body(p, 700.0, "Første linje"),
                    body(p, 680.0, "Anden linje"),
                    body(p, 660.0, "Tredje linje"),
                    folio(p, &format!("{}", p + 2)),
                ]
            })
            .collect();
        let printed = strip_running_furniture(&mut pages);

        assert!(
            pages.iter().all(|ls| ls.len() == 3),
            "the folio goes, the body stays"
        );
        assert_eq!(printed.get(&0).map(String::as_str), Some("3"));
        assert_eq!(printed.get(&3).map(String::as_str), Some("6"));
    }

    /// And the guard that matters: in an ordinary document the first line of
    /// every page also sits at the same height, and cutting THAT would behead
    /// every page.
    #[test]
    fn text_that_merely_repeats_its_position_is_left_alone() {
        let line = |page: usize, y: f64, text: &str| Line {
            y,
            size: 11.0,
            right: 400.0,
            left: 56.0,
            page,
            text: text.into(),
            spans: vec![span(text, None)],
            bullet: None,
            picture: None,
        };
        let mut pages: Vec<Vec<Line>> = (1..=4)
            .map(|p| {
                vec![
                    // Stranded at the top the way a heading is, and different
                    // every page, because it is the heading.
                    line(p, 700.0, &format!("Kapitel {p}")),
                    line(p, 600.0, "Brødtekst her"),
                    line(p, 580.0, "og mere brødtekst"),
                ]
            })
            .collect();
        let printed = strip_running_furniture(&mut pages);
        assert!(printed.is_empty());
        assert!(
            pages.iter().all(|ls| ls.len() == 3),
            "prose that varies is not furniture"
        );
    }

    /// A page may write its resources out or point at them, and a book made of
    /// one shared dictionary takes the second road. Reading only the first lost
    /// every picture in such a file, which is how a songbook lost its cover.
    #[test]
    fn a_picture_survives_a_shared_resource_dictionary() {
        use lopdf::{dictionary, Stream};

        let mut doc = Document::with_version("1.5");
        let image = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 40,
                "Height" => 40,
                "BitsPerComponent" => 8,
                "ColorSpace" => "DeviceGray",
            },
            vec![128u8; 40 * 40],
        ));
        let listed = doc.add_object(dictionary! { "Im1" => image });
        // The whole point: a reference where a dictionary would also be legal.
        let resources = doc.add_object(dictionary! { "XObject" => listed });
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"q 40 0 0 40 10 700 cm /Im1 Do Q".to_vec(),
        ));
        let pages_id = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Resources" => resources,
            "Contents" => content,
            "MediaBox" => vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(595),
                Object::Integer(842),
            ],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page)],
                "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("writes back out");

        let out = extract(&bytes).expect("reads back in");
        let Some(Block::Image(picture)) = out.blocks.first() else {
            panic!("expected the picture, got {:?}", out.blocks);
        };
        assert_eq!((picture.width, picture.height), (40.0, 40.0));
        assert!(picture.src.starts_with("data:image/bmp;base64,"));
    }

    /// A cover scanned at 300 dpi is 26 MB of samples and 35 MB of data URL, for
    /// something a phone shows four hundred points wide.
    #[test]
    fn a_full_page_picture_is_scaled_to_something_a_phone_can_hold() {
        let (w, h) = (2481usize, 3509usize);
        let (to_w, to_h, small) =
            shrink_to_fit(w, h, 1, &vec![200u8; w * h]).expect("far too big to keep");
        assert_eq!((to_w, to_h), (707, RAW_MAX_SIDE), "the aspect is kept");
        assert_eq!(small.len(), to_w * to_h);
        assert!(small.iter().all(|&v| v == 200), "an even field is even");
        assert!(
            shrink_to_fit(80, 60, 1, &vec![0u8; 80 * 60]).is_none(),
            "what already fits is left alone"
        );
    }

    /// Averaged, not sampled: picking one pixel per box turns the halftone in a
    /// scan into speckle.
    #[test]
    fn shrinking_averages_what_it_covers() {
        let stripes: Vec<u8> = (0..2000)
            .map(|i| if i % 2 == 0 { 0 } else { 255 })
            .collect();
        let (to_w, to_h, small) = shrink_to_fit(2000, 1, 1, &stripes).expect("too wide to keep");
        assert_eq!((to_w, to_h), (1000, 1));
        assert!(small.iter().all(|&v| v == 127), "black and white pair off");
    }
}

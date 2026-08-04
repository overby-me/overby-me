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
    },
    Paragraph(Vec<Span>),
    /// A line that began with a bullet or a number, with that marker stripped.
    ListItem(Vec<Span>),
}

impl Block {
    pub fn spans(&self) -> &[Span] {
        match self {
            Block::Heading { spans, .. } | Block::Paragraph(spans) | Block::ListItem(spans) => {
                spans
            }
        }
    }

    /// The block's words, without their colours.
    pub fn text(&self) -> String {
        self.spans().iter().map(|s| s.text.as_str()).collect()
    }
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
}

impl Span {
    /// Whether two stretches can be one span: same colour, same face.
    fn matches(&self, other: &Span) -> bool {
        self.color == other.color && self.bold == other.bold && self.italic == other.italic
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
fn page_runs(doc: &Document, page_id: lopdf::ObjectId) -> Vec<Run> {
    let fonts: HashMap<Vec<u8>, Font> = match doc.get_page_fonts(page_id) {
        Ok(map) => map
            .into_iter()
            .map(|(name, dict)| (name, read_font(doc, dict)))
            .collect(),
        Err(_) => HashMap::new(),
    };
    let Ok(content) = Content::decode(&doc.get_page_content(page_id)) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
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
            _ => {}
        }
    }
    runs
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
    runs: &mut Vec<Run>,
) {
    let Some(font) = font else { return };
    let at = mul(*tm, ctm);
    let scale = (at[0] * at[0] + at[1] * at[1]).sqrt().max(0.01);
    let mut text = String::new();
    let mut advance = 0.0;
    for (code, s) in font.decode(bytes) {
        text.push_str(&s);
        let w = font.width(code) / 1000.0 * size;
        let extra = if !font.two_byte && code == 32 {
            word_space
        } else {
            0.0
        };
        advance += (w + char_space + extra) * h_scale;
    }
    let after = mul(mul([1.0, 0.0, 0.0, 1.0, advance, 0.0], *tm), ctm);
    if !text.trim().is_empty() {
        runs.push(Run {
            x: at[4],
            end_x: after[4],
            y: at[5],
            size: size * scale,
            text,
            color: color.map(str::to_string),
            bold: font.bold,
            italic: font.italic,
        });
    }
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

fn join_line(runs: &[Run]) -> Option<Line> {
    let first = runs.first()?;
    let mut spans: Vec<Span> = Vec::new();
    let mut prev_end = f64::NEG_INFINITY;
    let mut size: f64 = 0.0;
    let mut sorted: Vec<&Run> = runs.iter().collect();
    sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    for run in sorted {
        size = size.max(run.size);
        // A fifth of the em. Measured over the corpus: at 0.25 six words across
        // eight documents ran together, at 0.20 none did, and 0.16 and 0.12
        // measure identically, so this is the start of a plateau rather than a
        // knife edge. Compared against the pen's TRUE end, not a guess from the
        // character count, which is what put spaces inside words.
        let gap = run.x - prev_end;
        let wants_space = prev_end.is_finite()
            && gap > run.size * 0.20
            && !spans.last().is_some_and(|s| s.text.ends_with(' '));
        // A run continues the one before it when the colour has not changed, so
        // a paragraph in one colour is one span rather than one per draw call.
        let here = Span {
            text: String::new(),
            color: run.color.clone(),
            bold: run.bold,
            italic: run.italic,
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
    let spans = tidy(spans);
    if spans.is_empty() {
        return None;
    }
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    let right = runs
        .iter()
        .map(|r| r.end_x)
        .fold(f64::NEG_INFINITY, f64::max);
    Some(Line {
        y: first.y,
        size,
        right,
        text,
        spans,
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
fn blocks_from(lines: Vec<Line>) -> Vec<Block> {
    let body = body_size(&lines);
    let column = column_right(&lines);
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<Span> = Vec::new();
    let mut para_size = body;
    let mut prev: Option<&Line> = None;

    let flush = |para: &mut Vec<Span>, size: f64, blocks: &mut Vec<Block>| {
        let spans = tidy(std::mem::take(para));
        if spans.is_empty() {
            return;
        }
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        if let Some(rest) = list_marker(&text) {
            // Drop the marker from the spans, keeping the colours of the words
            // that follow it.
            let dropped = text.chars().count() - rest.chars().count();
            blocks.push(Block::ListItem(drop_prefix(spans, dropped)));
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
            blocks.push(Block::Heading { level, spans });
        } else {
            blocks.push(Block::Paragraph(spans));
        }
    };

    let lines_ref = lines;
    for line in &lines_ref {
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
            flush(&mut para, para_size, &mut blocks);
        }
        if para.is_empty() {
            para_size = line.size;
        } else {
            // A line break inside a paragraph is a space, not a join.
            if let Some(last) = para.last_mut() {
                last.text.push(' ');
            }
        }
        para.extend(line.spans.iter().cloned());
        prev = Some(line);
    }
    flush(&mut para, para_size, &mut blocks);
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

/// Read a PDF and hand back what it says.
pub fn extract(bytes: &[u8]) -> Result<Extracted, String> {
    let doc = Document::load_mem(bytes).map_err(|e| format!("not a readable PDF: {e}"))?;
    let pages = doc.get_pages();
    let total = pages.len();
    let mut all_lines = Vec::new();
    let mut empty = 0usize;
    for (_, page_id) in pages {
        let runs = page_runs(&doc, page_id);
        let lines = lines_from(runs);
        if lines.is_empty() {
            empty += 1;
        }
        all_lines.extend(lines);
    }
    Ok(Extracted {
        blocks: blocks_from(all_lines),
        pages: total,
        pages_without_text: empty,
    })
}

#[cfg(test)]
mod harness {
    /// Run the extractor over a real file and print what it made of it. Not a
    /// test: a lens, for checking the heuristics against the actual corpus.
    ///
    ///   PDF_UNDER_TEST=/path/to.pdf cargo test pdf_text::harness -- --nocapture
    #[test]
    fn look_at_a_real_pdf() {
        let Ok(path) = std::env::var("PDF_UNDER_TEST") else {
            return;
        };
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
                        super::Block::ListItem(_) => "li".into(),
                        super::Block::Paragraph(_) => "p".into(),
                    };
                    let t = block_text(b);
                    let width: usize = std::env::var("PDF_WIDTH")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(90);
                    println!("  {kind:<3} {}", t.chars().take(width).collect::<String>());
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
            text: text.into(),
            spans: vec![Span {
                text: text.into(),
                color: None,
                bold: false,
                italic: false,
            }],
        }
    }

    fn texts(blocks: &[Block]) -> Vec<String> {
        blocks.iter().map(|b| b.text()).collect()
    }

    /// Lines close together are one paragraph; a wider gap starts another. This
    /// is the rule the whole reconstruction rests on.
    #[test]
    fn the_vertical_gap_decides_where_a_paragraph_ends() {
        let blocks = blocks_from(vec![
            line(700.0, 11.0, "Første linje i afsnittet"),
            line(686.0, 11.0, "og anden linje."),
            // A gap of 30 against a size of 11: a new paragraph.
            line(656.0, 11.0, "Et nyt afsnit."),
        ]);
        assert_eq!(
            texts(&blocks),
            vec!["Første linje i afsnittet og anden linje.", "Et nyt afsnit.",]
        );
    }

    /// A larger line is a heading, and how much larger decides the level.
    #[test]
    fn size_relative_to_the_body_makes_a_heading() {
        let blocks = blocks_from(vec![
            // Word's own sizes: Title 28, Heading 1 16, body 11.
            line(700.0, 28.0, "Titel"),
            line(660.0, 16.0, "Overskrift"),
            line(630.0, 11.0, "Brødtekst her."),
            line(616.0, 11.0, "Mere brødtekst."),
            line(600.0, 11.0, "Endnu mere."),
        ]);
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
        let blocks = blocks_from(vec![
            line(60.0, 11.0, "Sidste linje på siden."),
            line(700.0, 11.0, "Første linje på næste side."),
        ]);
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
            blocks: vec![Block::Paragraph(vec![Span {
                text: "noget".into(),
                color: None,
                bold: false,
                italic: false,
            }])],
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
            text: text.into(),
            color: color.map(str::to_string),
            bold: false,
            italic: false,
        }
    }

    fn run(x: f64, end_x: f64, text: &str, color: Option<&str>, bold: bool) -> Run {
        Run {
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
            text: text.into(),
            spans: vec![Span {
                text: text.into(),
                color: None,
                bold: false,
                italic: false,
            }],
        };
        // Column runs to 400. The first line stops at 180, far short of it.
        let blocks = blocks_from(vec![
            short(700.0, 180.0, "16:30 Ankomst"),
            short(686.0, 400.0, "17:50 Åbning af landsmødet"),
            short(672.0, 398.0, "og en fortsættelse af samme punkt"),
        ]);
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
}

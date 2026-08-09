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
        /// How far in the item was set, in the same steps a paragraph uses. A
        /// list under an introducing line is normally set deeper than it, and
        /// without this the bullets came out at the margin, to the LEFT of the
        /// words introducing them.
        indent: u8,
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
    /// Rows whose words stood in the same columns down the page.
    ///
    /// A page draws a table exactly as it draws a sentence -- glyphs at
    /// positions -- so what makes these a table is that line after line the
    /// groups start at the same x. Read as sentences they are nonsense of a
    /// particular kind: the board page reads as three names, then three roles,
    /// and a statement reads as a label followed by every year's figure in a
    /// row, with nothing to say which is which.
    Table { rows: Vec<Vec<Vec<Span>>> },
    /// A flat line the page drew across itself: the rule over a total, the one
    /// under a table's headings, the one that separates two sections.
    ///
    /// A reflowed page cannot keep a rule where it was drawn -- it was drawn at
    /// an x, and the text under it is a different width now -- but WHICH lines
    /// it separated survives, and that is what the rule was for. `width` is how
    /// much of the page's width it spanned, so a full separator and the short
    /// stroke over one column's total do not come out the same.
    ///
    /// `thickness` is in points, as the page drew it. A statement says "this is
    /// a total" by drawing a heavier line, and a hairline between two rows of a
    /// contents list means something quieter; one weight for both throws that
    /// away.
    Rule { width: f64, thickness: f64 },
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
        /// And what the page BEGINNING here calls itself. The mark sits at the
        /// top of it, so this is the page a reader arriving at the mark is on.
        /// Keeping only `printed` meant everything that jumped to a mark landed
        /// one page late, and stepping forward moved by the tail of the page you
        /// were already on rather than by a page.
        starts: Option<String>,
    },
}

impl Block {
    pub fn spans(&self) -> &[Span] {
        match self {
            Block::Heading { spans, .. }
            | Block::Paragraph { spans, .. }
            | Block::IndexEntry { spans, .. }
            | Block::ListItem { spans, .. } => spans,
            Block::Image(_)
            | Block::Anchor(_)
            | Block::PageBreak { .. }
            | Block::Rule { .. }
            | Block::Table { .. } => &[],
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
        if let Block::Table { rows } = self {
            // Cells apart, rows apart: the words of a table are still words, and
            // a reader searching the page for one of them must find it.
            return rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.iter().map(|s| s.text.as_str()).collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        self.spans().iter().map(|s| s.text.as_str()).collect()
    }
}

/// A picture the page drew, ready for an `<img>`, or a shape it drew, ready for
/// an `<svg>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// Where its left edge sat on the page, in points. Meaningless to a reflow,
    /// which puts a picture in the column like everything else, and everything
    /// to a page laid out as it was drawn.
    pub left: f64,
    /// SVG path data, when the page DREW this instead of placing an image. A
    /// signature is the case that matters: it arrives as a thousand little line
    /// segments and no image at all, so there is nothing for an `<img>` to show.
    /// Drawn rather than rasterised so it inherits the reading colour and stays
    /// legible on a dark surface, which a black bitmap would not.
    pub path: Option<String>,
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
    Right,
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
    pub underline: bool,
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
            && self.underline == other.underline
            && self.link == other.link
    }
}

/// Which of the faces this app ships a PDF's font is closest to.
///
/// The substitutes are metric-compatible on purpose (see `main.rs`): Liberation
/// Sans has Helvetica's and Arial's advance widths, Liberation Serif has Times',
/// Carlito has Calibri's. That is what makes a fixed layout possible without
/// measuring anything in the browser -- a run placed at its x with its size is
/// the width the page drew it, because the letters are the same widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Family {
    #[default]
    Sans,
    Serif,
    /// Calibri, which Carlito matches. Word's default face for two decades, so
    /// most of what this wiki carries is set in it.
    Calibri,
    /// Cambria, which Caladea matches. Word's default SERIF over the same years,
    /// and not Times: substituting Times for it made every letter of the
    /// songbook's imprint page a little too wide, so each ran into the next.
    Cambria,
    Mono,
}

/// One thing a page drew, where it drew it. Points, y from the TOP.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub what: What,
}

/// What the thing is.
#[derive(Debug, Clone, PartialEq)]
pub enum What {
    Text {
        text: String,
        size: f64,
        color: Option<String>,
        bold: bool,
        italic: bool,
        family: Family,
        link: Option<Link>,
    },
    Image(Picture),
    /// A flat line: `height` is its thickness.
    Rule,
}

/// One page as it was laid out, for the reader who wants the document rather
/// than the reading.
#[derive(Debug, Clone, PartialEq)]
pub struct PageLayout {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Placed>,
}

/// What came out of a PDF, and how much of it there was to find.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub blocks: Vec<Block>,
    /// Every page as it was drawn, in order. Empty when nothing could be read.
    pub layout: Vec<PageLayout>,
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
    /// No `/ToUnicode`: the 8-bit table this font's codes are to be read
    /// through, as the font itself declares. `None` when a ToUnicode map
    /// answered instead, or when codes are two bytes.
    base: Option<Base>,
    /// The font's own `/Differences`: codes it re-points at named glyphs,
    /// overriding the table above. A subset font commonly ships nothing else.
    differences: HashMap<u32, char>,
    /// Weight and slant, which in a PDF are properties of the FONT rather than
    /// of the text. There is no "make this bold" operator: the writer switches
    /// to a different font resource, and the only way to know is to ask that
    /// resource what it is.
    bold: bool,
    italic: bool,
    /// Which shipped face stands in for it. Only the shape of the letters
    /// differs; the widths are the same, which is what makes a fixed layout
    /// possible without measuring anything.
    family: Family,
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
        let text = self.mapped(code);
        // Every route out of a font can land in the private use area, so the
        // reading happens once, here, rather than at each of them.
        match text.chars().any(is_private_use) {
            true => text.chars().filter_map(out_of_private_use).collect(),
            false => text,
        }
    }

    fn mapped(&self, code: u32) -> String {
        if let Some(s) = self.to_unicode.get(&code) {
            return s.clone();
        }
        if let Some(c) = self.differences.get(&code) {
            return ligature_text(*c);
        }
        match self.base {
            Some(base) => base
                .char_for(code as u8)
                .map(ligature_text)
                .unwrap_or_default(),
            None => String::new(),
        }
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

/// Mac OS Roman's upper half, which `/MacRomanEncoding` names.
///
/// Not an exotic case: anything laid out on a Mac and printed to PDF declares
/// it, and such a font often ships no `/ToUnicode` because the encoding IS the
/// answer. Read as WinAnsi instead, a Danish document comes out as `Œrsrapport`
/// for `årsrapport` and `¿konomi` for `økonomi` -- every å, ø and æ wrong, which
/// is most of the words that matter, on the reports this wiki carries.
const MAC_ROMAN_HIGH: [char; 128] = [
    'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', 'ê', 'ë', 'í',
    'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', '†', '°', '¢', '£', '§', '•',
    '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏',
    'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…',
    '\u{00A0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄',
    '€', '‹', '›', 'ﬁ', 'ﬂ', '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì',
    'Ó', 'Ô', '\u{FFFD}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛',
    'ˇ',
];

/// The 8-bit table a simple font's codes are read through when it ships no
/// `/ToUnicode`. The font says which in its `/Encoding`; the default for a
/// non-symbolic font is close enough to WinAnsi for the Latin range.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
enum Base {
    #[default]
    WinAnsi,
    MacRoman,
}

impl Base {
    fn char_for(self, b: u8) -> Option<char> {
        match self {
            Base::WinAnsi => win_ansi_char(b),
            Base::MacRoman => match b {
                0x00..=0x1F => None,
                0x20..=0x7F => Some(b as char),
                _ => Some(MAC_ROMAN_HIGH[(b - 0x80) as usize]).filter(|c| *c != '\u{FFFD}'),
            },
        }
    }
}

/// A ligature as the letters it stands for.
///
/// The glyph is one character in the font and two letters to a reader, and to
/// the browser's find. A page that says `ﬂere` answers no search for `flere`,
/// so the pair goes in as a pair. (Poppler drops these entirely, which is worse
/// than either.)
fn ligature_text(c: char) -> String {
    match c {
        'ﬁ' => "fi".to_string(),
        'ﬂ' => "fl".to_string(),
        'ﬀ' => "ff".to_string(),
        'ﬃ' => "ffi".to_string(),
        'ﬄ' => "ffl".to_string(),
        other => other.to_string(),
    }
}

/// A glyph name from a `/Differences` array as the character it names.
///
/// The Adobe glyph list is thousands of entries; this is the part a European
/// document uses, plus the two mechanical forms (`uniXXXX`, and a single letter
/// naming itself) that cover most of the rest.
/// The private use area: a codepoint that means whatever the font that drew it
/// says it means, and nothing at all to any other font.
fn is_private_use(c: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&c)
}

/// What a symbol font's private glyph was drawing.
///
/// Word writes a bulleted list's marker as a character of Symbol or Wingdings
/// and maps it into the private use area: the auditor's report in the 2024
/// annual report carries U+F0B7, which is Symbol's own 0xB7, a bullet. No font a
/// reader has says anything about U+F0B7, so a browser drew the missing-glyph
/// box for every item in that list, and the bullet was not recognised as a
/// marker either -- the items came out as paragraphs beginning with a box.
///
/// The F000 block is the symbol font's byte with F000 added, so the byte is what
/// has to be read. The named ones are the markers Word actually writes; the rest
/// fall back to the byte read as Latin-1, which is right for the letters and
/// digits a symbol-encoded subset carries.
fn out_of_private_use(c: char) -> Option<char> {
    let code = u32::from(c);
    if !(0xF000..=0xF0FF).contains(&code) {
        // Some other font's private glyph, whose meaning is not knowable from
        // the file. Dropped rather than kept: a box in the middle of a sentence
        // says nothing a reader can use, and the sentence reads without it.
        return (!is_private_use(c)).then_some(c);
    }
    let byte = (code - 0xF000) as u8;
    Some(match byte {
        // Symbol: the bullet Word's first list level uses.
        0xB7 => '•',
        // Wingdings: the square and circle of its second and third levels.
        0xA7 => '▪',
        0x6C => '●',
        0x6E => '■',
        0xFC => '✓',
        0xFD => '✗',
        // Anything else: the byte itself, which is right for the letters and
        // digits a symbol-encoded subset carries, and for the Latin-1 block.
        b if b.is_ascii_graphic() || b == b' ' => b as char,
        b if b >= 0xA0 => char::from(b),
        _ => return None,
    })
}

fn glyph_char(name: &str) -> Option<char> {
    if let Some(hex) = name.strip_prefix("uni") {
        if hex.len() >= 4 {
            if let Ok(cp) = u32::from_str_radix(&hex[..4], 16) {
                return char::from_u32(cp);
            }
        }
    }
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(c);
    }
    Some(match name {
        "aring" => 'å',
        "Aring" => 'Å',
        "oslash" => 'ø',
        "Oslash" => 'Ø',
        "ae" => 'æ',
        "AE" => 'Æ',
        "adieresis" => 'ä',
        "Adieresis" => 'Ä',
        "odieresis" => 'ö',
        "Odieresis" => 'Ö',
        "udieresis" => 'ü',
        "Udieresis" => 'Ü',
        "eacute" => 'é',
        "Eacute" => 'É',
        "egrave" => 'è',
        "agrave" => 'à',
        "acute" => '´',
        "ccedilla" => 'ç',
        "Ccedilla" => 'Ç',
        "ntilde" => 'ñ',
        "atilde" => 'ã',
        "otilde" => 'õ',
        "aacute" => 'á',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        "ocircumflex" => 'ô',
        "ecircumflex" => 'ê',
        "acircumflex" => 'â',
        "icircumflex" => 'î',
        "ucircumflex" => 'û',
        "fi" => 'ﬁ',
        "fl" => 'ﬂ',
        "ff" => 'ﬀ',
        "ffi" => 'ﬃ',
        "ffl" => 'ﬄ',
        "quoteright" => '’',
        "quoteleft" => '‘',
        "quotedblleft" => '“',
        "quotedblright" => '”',
        "quotedblbase" => '„',
        "quotesinglbase" => '‚',
        "endash" => '–',
        "emdash" => '—',
        "bullet" => '•',
        "periodcentered" => '·',
        "ellipsis" => '…',
        "dagger" => '†',
        "daggerdbl" => '‡',
        "section" => '§',
        "paragraph" => '¶',
        "germandbls" => 'ß',
        "sterling" => '£',
        "yen" => '¥',
        "currency" => '¤',
        "Euro" => '€',
        "euro" => '€',
        "trademark" => '™',
        "copyright" => '©',
        "registered" => '®',
        "degree" => '°',
        "plusminus" => '±',
        "guillemotleft" => '«',
        "guillemotright" => '»',
        "questiondown" => '¿',
        "exclamdown" => '¡',
        "space" => ' ',
        "hyphen" => '-',
        "period" => '.',
        "comma" => ',',
        "colon" => ':',
        "semicolon" => ';',
        "slash" => '/',
        "parenleft" => '(',
        "parenright" => ')',
        "percent" => '%',
        "ampersand" => '&',
        "zero" => '0',
        "one" => '1',
        "two" => '2',
        "three" => '3',
        "four" => '4',
        "five" => '5',
        "six" => '6',
        "seven" => '7',
        "eight" => '8',
        "nine" => '9',
        _ => return None,
    })
}

/// The base table a `/Encoding` name asks for.
fn base_named(name: &[u8]) -> Base {
    match name {
        b"MacRomanEncoding" => Base::MacRoman,
        _ => Base::WinAnsi,
    }
}

/// A `/Differences` array: numbers set the next code, names claim one each.
fn read_differences(items: &[lopdf::Object]) -> HashMap<u32, char> {
    let mut out = HashMap::new();
    let mut code: u32 = 0;
    for item in items {
        match item {
            lopdf::Object::Integer(n) => code = (*n).max(0) as u32,
            lopdf::Object::Real(n) => code = (*n).max(0.0) as u32,
            lopdf::Object::Name(name) => {
                if let Some(c) = glyph_char(&String::from_utf8_lossy(name)) {
                    out.insert(code, c);
                }
                code += 1;
            }
            _ => {}
        }
    }
    out
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

/// Which shipped face stands in for a font, from its name.
///
/// Only three, because only three are needed: the substitutes are chosen for
/// having the same advance widths as what they replace, and a name this does not
/// know is likelier to be a sans than anything else -- that is what an unnamed
/// subset of a corporate typeface almost always is.
fn family_of(base: &str) -> Family {
    const SERIF: &[&str] = &[
        "times", "serif", "georgia", "garamond", "book", "roman", "minion", "palatino", "century",
    ];
    const MONO: &[&str] = &["courier", "mono", "consol", "menlo"];
    if MONO.iter().any(|n| base.contains(n)) {
        return Family::Mono;
    }
    // Named before the general rules, because the match is exact: these two have
    // stand-ins with their own metrics rather than a family resemblance.
    if base.contains("cambria") || base.contains("caladea") {
        return Family::Cambria;
    }
    if base.contains("calibri") || base.contains("carlito") {
        return Family::Calibri;
    }
    // "Sans" wins over "serif" inside it: "SansSerif" is a sans.
    if base.contains("sans") {
        return Family::Sans;
    }
    match SERIF.iter().any(|n| base.contains(n)) {
        true => Family::Serif,
        false => Family::Sans,
    }
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
    // What the codes MEAN, when the font did not spell it out in a ToUnicode
    // map. `/Encoding` is either a name or a dictionary naming a base and then
    // re-pointing individual codes; both are common, and a subset font that
    // ships neither is read as WinAnsi, which is the sensible default for the
    // Latin range and what this did for every font until now.
    if font.to_unicode.is_empty() && !font.two_byte {
        let mut base = Base::WinAnsi;
        if let Ok(obj) = dict.get(b"Encoding") {
            if let Ok(name) = obj.as_name() {
                base = base_named(name);
            } else if let Ok(enc) = obj.as_dict().or_else(|_| {
                obj.as_reference()
                    .and_then(|r| doc.get_object(r))
                    .and_then(|o| o.as_dict())
            }) {
                if let Ok(name) = enc.get(b"BaseEncoding").and_then(|o| o.as_name()) {
                    base = base_named(name);
                }
                if let Ok(diffs) = enc.get(b"Differences").and_then(|o| o.as_array()) {
                    font.differences = read_differences(diffs);
                }
            }
        }
        font.base = Some(base);
    }

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
    font.family = family_of(&base);

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
    /// Which of the shipped faces this run's font is closest to.
    family: Family,
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
    /// Set when a rule is drawn under this run. A PDF has no underline as such;
    /// it has a line under some words.
    underline: bool,
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
    /// The flat lines the page drew. The ones that turned out to be underlines
    /// are marked as such on their runs; what is left separated something.
    rules: Vec<Rule>,
}

/// A piece of a path, before anyone has said whether it will be painted.
enum Shape {
    Box { x: f64, y: f64, w: f64, h: f64 },
    Line { from: (f64, f64), to: (f64, f64) },
}

/// A flat line drawn on the page. An underline is one of these under some words.
struct Rule {
    x0: f64,
    x1: f64,
    y: f64,
    thickness: f64,
}

/// Whether a text matrix has turned the baseline off horizontal.
///
/// Not a slant: `slanted` asks whether the letters lean along their own
/// baseline, which is a fake italic. This asks where that baseline points. A
/// signing stamp up the page edge, a watermark across the corner and a rotated
/// table label are all set at an angle to the page, and none of them belongs in
/// the sentence a reader is halfway through.
///
/// Generous about what counts as horizontal (30 degrees), because the thing this
/// separates is upright text from text at a right angle, and nothing sets a
/// paragraph at twenty degrees by accident.
fn turned(m: [f64; 6]) -> bool {
    let along = (m[0] * m[0] + m[1] * m[1]).sqrt();
    if along <= 0.0 {
        return false;
    }
    // The baseline's rise against its run: tan(30 degrees) is about 0.577.
    (m[1] / along).abs() > 0.5
}

/// Whether a text matrix slants the letters it draws.
///
/// A file with no italic face to hand fakes one by shearing the matrix, and
/// nothing in the font then says italic at all. The songbook does exactly this:
/// its only faces are upright Times, Arial, Calibri and Cambria, and Marianne
/// Jelved's "Æresmedlem af Radikal Ungdom" is set with `Tm 50 0 16.99 50`,
/// nineteen degrees of slant.
///
/// Measured against the baseline the matrix itself sets rather than against the
/// page, so text turned on its side is not read as italic for being sideways.
fn slanted(m: [f64; 6]) -> bool {
    let along_baseline = (m[0] * m[0] + m[1] * m[1]).sqrt();
    if along_baseline <= 0.0 {
        return false;
    }
    let (ux, uy) = (m[0] / along_baseline, m[1] / along_baseline);
    // How far the letters' up-axis leans along the baseline, against how tall it
    // stands away from it.
    let leans = m[2] * ux + m[3] * uy;
    let stands = (m[3] * ux - m[2] * uy).abs();
    // A hair over five degrees. Real italics slant twelve to twenty; nothing
    // upright slants at all.
    stands > 0.0 && (leans / stands).abs() > 0.1
}

/// Where the current transform puts a point.
fn put(ctm: [f64; 6], x: f64, y: f64) -> (f64, f64) {
    (
        ctm[0] * x + ctm[2] * y + ctm[4],
        ctm[1] * x + ctm[3] * y + ctm[5],
    )
}

impl Shape {
    /// The corners this shape reaches, on the page.
    fn corners(&self, ctm: [f64; 6]) -> [(f64, f64); 2] {
        match self {
            Shape::Box { x, y, w, h } => [put(ctm, *x, *y), put(ctm, x + w, y + h)],
            Shape::Line { from, to } => [put(ctm, from.0, from.1), put(ctm, to.0, to.1)],
        }
    }

    /// This shape as a rule when the path is STROKED: a flat segment drawn with
    /// the pen. A stroked box is a border round something and not an underline
    /// under anything, so it draws none.
    fn stroked(&self, ctm: [f64; 6], pen: f64) -> Vec<Rule> {
        let scale = (ctm[2].powi(2) + ctm[3].powi(2)).sqrt();
        // The pen as it is, not floored. A floor of 0.2 was applied here and it
        // was the reason no two lines looked different: this file draws with a
        // pen of 1.2 in a space a tenth of the page, so a single stroke is
        // 0.12pt and was read as 0.2 -- and a stack of sixteen of them, which is
        // how it draws a heavy bar, was read as 0.2 as well. What separates a
        // hairline from a bar is the true value; the floor is the renderer's
        // business, and it applies one.
        let thickness = (pen * scale).max(0.01);
        let [(x0, y0), (x1, y1)] = self.corners(ctm);
        match self {
            Shape::Line { .. } => {
                // Flat, within a hair: a diagonal is a drawing, not a rule.
                match (y1 - y0).abs() > 0.6 {
                    true => Vec::new(),
                    false => vec![Rule {
                        x0: x0.min(x1),
                        x1: x0.max(x1),
                        y: (y0 + y1) / 2.0,
                        thickness,
                    }],
                }
            }
            // A STROKED box is a frame, and its horizontal edges are rules like
            // any other: the line over a table's totals is commonly drawn as one.
            // Skipping them left a page with only the hairlines it happened to
            // draw as segments, so every line in the report came out at the
            // thinnest weight there is and the heavy ones were missing entirely.
            //
            // The uprights are dropped, as they always were: a column's edge
            // means nothing once the text it bounded has reflowed.
            Shape::Box { .. } => {
                let (top, bottom) = (y0.max(y1), y0.min(y1));
                let (left, right) = (x0.min(x1), x0.max(x1));
                // A BAR drawn as a stroked rectangle: both its edges are the
                // line. A tall one is a frame, and its edges are the frame's --
                // a box around a verse is not two rules across the page.
                match right - left < 4.0 || top - bottom > 2.5 {
                    true => Vec::new(),
                    false => vec![
                        Rule {
                            x0: left,
                            x1: right,
                            y: top,
                            thickness,
                        },
                        Rule {
                            x0: left,
                            x1: right,
                            y: bottom,
                            thickness,
                        },
                    ],
                }
            }
        }
    }
}

/// How many segments a path needs before it is a drawing rather than furniture.
///
/// A rule is one segment, a box is four, a table's borders a couple of dozen.
/// The signature in the forælder letter is one thousand one hundred and
/// sixty-eight: whoever made it traced the handwriting in straight lines.
const DRAWING_SEGMENTS: usize = 40;

/// One stroke of a drawing, on the page: where it runs, and whether it closes
/// into a box rather than running from one point to the other.
struct Stroke {
    boxed: bool,
    from: (f64, f64),
    to: (f64, f64),
}

/// A drawing the page made, in the page's own coordinates.
struct Art {
    strokes: Vec<Stroke>,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Art {
    /// Whether these two are one drawing. A signature is written in two words
    /// with a gap between them, painted as two paths, and the page shows one
    /// signature: on the same band, near enough across, they belong together.
    fn joins(&self, other: &Art) -> bool {
        let overlap = self.y1.min(other.y1) - self.y0.max(other.y0);
        let height = (self.y1 - self.y0).min(other.y1 - other.y0);
        let apart = other.x0.max(self.x0) - self.x1.min(other.x1);
        overlap > height * 0.4 && apart < 40.0
    }

    fn absorb(&mut self, other: Art) {
        self.strokes.extend(other.strokes);
        self.x0 = self.x0.min(other.x0);
        self.y0 = self.y0.min(other.y0);
        self.x1 = self.x1.max(other.x1);
        self.y1 = self.y1.max(other.y1);
    }

    /// As SVG path data, with the page's coordinates flipped into the screen's
    /// and sized in points, so it lands at the size it was drawn.
    fn into_picture(self) -> Option<Picture> {
        let (width, height) = (self.x1 - self.x0, self.y1 - self.y0);
        let left = self.x0;
        if !width.is_finite() || width <= 1.0 || height <= 1.0 {
            return None;
        }
        let put = |(x, y): (f64, f64)| (x - self.x0, self.y1 - y);
        let mut d = String::new();
        let mut pen: Option<(f64, f64)> = None;
        for Stroke { boxed, from, to } in self.strokes {
            let (ax, ay) = put(from);
            let (bx, by) = put(to);
            if boxed {
                d.push_str(&format!("M{ax:.1} {ay:.1}H{bx:.1}V{by:.1}H{ax:.1}Z"));
                pen = None;
                continue;
            }
            // A stroke that starts where the last one ended continues it, which
            // is what makes a thousand segments into a few pen strokes.
            if pen != Some(from) {
                d.push_str(&format!("M{ax:.1} {ay:.1}"));
            }
            d.push_str(&format!("L{bx:.1} {by:.1}"));
            pen = Some(to);
        }
        Some(Picture {
            path: Some(d),
            src: String::new(),
            left,
            width,
            height,
        })
    }
}

/// A path as a drawing, when there is enough of it to be one.
fn drawing_of(shapes: &[Shape], ctm: [f64; 6]) -> Option<Art> {
    if shapes.len() < DRAWING_SEGMENTS {
        return None;
    }
    let (mut x0, mut x1) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut strokes = Vec::with_capacity(shapes.len());
    for shape in shapes {
        let [a, b] = shape.corners(ctm);
        for (x, y) in [a, b] {
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        strokes.push(Stroke {
            boxed: matches!(shape, Shape::Box { .. }),
            from: a,
            to: b,
        });
    }
    Some(Art {
        strokes,
        x0,
        y0,
        x1,
        y1,
    })
}

/// Put the drawings that belong together back together, then turn each into a
/// picture placed by its top.
fn gather_drawings(arts: Vec<Art>) -> Vec<(f64, Picture)> {
    let mut merged: Vec<Art> = Vec::new();
    for art in arts {
        match merged.iter_mut().find(|kept| kept.joins(&art)) {
            Some(kept) => kept.absorb(art),
            None => merged.push(art),
        }
    }
    merged
        .into_iter()
        .filter_map(|art| {
            let top = art.y1;
            art.into_picture().map(|picture| (top, picture))
        })
        .collect()
}

/// A FILLED path as a rule, if the whole path is thin enough to be one.
///
/// The path entire, not its edges. Taking each edge of a filled box for a line
/// is what turned the white box the songbook paints behind every line of lyrics
/// into an underline under it, twenty-nine times a page: the box's top and
/// bottom edges each looked like a hairline, and one of them always sat just
/// under some words.
fn filled_rule(shapes: &[Shape], ctm: [f64; 6]) -> Vec<Rule> {
    // EACH shape, not the path they share. A page is free to lay out every rule
    // of a table in one path and fill it once, and the annual report does: the
    // bounding box of that path is the height of the table, which is not a rule
    // by any measure, so every bar in the statement was discarded and the only
    // lines left were the hairlines drawn one at a time. The weights that mark a
    // total -- 1.2pt and 2.4pt in this file -- never reached the reader at all.
    // How tall the whole path is decides which question to ask. A path no taller
    // than a heavy stroke IS one bar, however many pieces drew it. A taller one
    // is either a frame around something -- four segments enclosing a verse, in
    // the songbook -- or a stack of separate bars, and only the second kind has
    // rules in it: a frame's edges are not lines the reader is meant to see once
    // the text has reflowed out of it.
    let corners: Vec<(f64, f64)> = shapes.iter().flat_map(|s| s.corners(ctm)).collect();
    let (lo, hi) = corners
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, y)| {
            (lo.min(*y), hi.max(*y))
        });
    let framed = hi - lo > 2.5;
    shapes
        .iter()
        .filter_map(|shape| {
            // Inside a tall path, only a filled bar counts; a segment there is
            // one side of a frame.
            if framed && !matches!(shape, Shape::Box { .. }) {
                return None;
            }
            let [(ax, ay), (bx, by)] = shape.corners(ctm);
            let (x0, x1) = (ax.min(bx), ax.max(bx));
            let (y0, y1) = (ay.min(by), ay.max(by));
            // A rule is a bar: wide, and no taller than a heavy stroke. Anything
            // taller is a box with something in it.
            match !x0.is_finite() || y1 - y0 > 2.5 || x1 - x0 < 4.0 {
                true => None,
                false => Some(Rule {
                    x0,
                    x1,
                    y: (y0 + y1) / 2.0,
                    thickness: (y1 - y0).max(0.2),
                }),
            }
        })
        .collect()
}

/// Whether a colour is too pale to be a mark on white paper.
///
/// A white rule is not an underline: it is a box drawn behind something, or a
/// knockout, and the page shows nothing there at all.
fn too_pale(r: f64, g: f64, b: f64) -> bool {
    0.2126 * r + 0.7152 * g + 0.0722 * b > 0.75
}

/// Mark the runs that have a rule drawn under them.
///
/// Under, and only just: a rule within half a line below the baseline, thin
/// enough to be a pen stroke rather than a box, and covering most of the run's
/// width. A table's rules and a page's separators live further from any baseline
/// or run the width of a column, and are left where they are.
fn underline_runs(runs: &mut [Run], rules: &[Rule]) {
    for run in runs.iter_mut() {
        let width = run.end_x - run.x;
        // A run of spaces sits over the rule between two underlined words as
        // often as not, and a stroke under nothing is just a stray dash.
        if width <= 0.0 || run.size <= 0.0 || run.text.trim().is_empty() {
            continue;
        }
        run.underline = rules.iter().any(|rule| {
            let under = rule.y < run.y - run.size * 0.02 && rule.y > run.y - run.size * 0.45;
            let thin = rule.thickness < run.size * 0.2;
            let covered = rule.x1.min(run.end_x) - rule.x0.max(run.x);
            under && thin && covered > width * 0.6
        });
    }
}

/// The rules that separated something, as (y, share of the widest line's width).
///
/// Not every flat line a page draws belongs in the reading. An underline is
/// already carried by the words above it. A hairline the width of a character
/// is a tick in a form. Two strokes a fraction apart are one rule drawn twice,
/// which is what a table does when every cell draws its own edge.
///
/// What is left is the rule over a total, the one under a table's headings, and
/// the one that divides two sections -- which is how a financial statement says
/// which numbers belong together, and the whole of what it had to say before
/// this kept any of them.
fn separators(rules: &[Rule], lines: &[Line], page_width: f64) -> Vec<(f64, f64, f64)> {
    // How far the stack reaches, not where its middle is: top, bottom, left,
    // right, and the heaviest single stroke in it.
    let mut out: Vec<(f64, f64, f64, f64, f64)> = Vec::new();
    for rule in rules {
        let width = rule.x1 - rule.x0;
        // A tenth of the column: shorter is a tick, a bullet's stroke, or the
        // box around a single word.
        if width < page_width * 0.1 {
            continue;
        }
        let underlines = lines.iter().any(|line| {
            let over = rule.y < line.y && rule.y > line.y - line.size * 0.45;
            over && rule.x0 < line.right && rule.x1 > line.left
        });
        if underlines {
            continue;
        }
        // One rule, however many strokes drew it -- and as heavy and as wide as
        // the STACK, not as the heaviest or widest stroke in it. This is how the
        // annual report draws a weight: every stroke in the file is 0.12pt, and
        // a heavier line is more of them a tenth of a point apart. The bar over
        // the masthead is sixteen of them and measures 1.92pt; the rules across
        // the income statement are four and measure 0.48pt, drawn column by
        // column so one rule arrives as eight strokes. Keeping the RUNNING
        // MIDPOINT was what hid this: the middle of the stack slid down as the
        // stack grew, so the span was always measured from the middle rather
        // than from the top and every stack, sixteen strokes or four, converged
        // on a third of a point. That is why no two lines in the document looked
        // any different.
        match out
            .iter_mut()
            .find(|(top, bottom, ..)| rule.y <= *top + 2.0 && rule.y >= *bottom - 2.0)
        {
            Some((top, bottom, left, right, stroke)) => {
                *top = top.max(rule.y);
                *bottom = bottom.min(rule.y);
                *left = left.min(rule.x0);
                *right = right.max(rule.x1);
                *stroke = stroke.max(rule.thickness);
            }
            None => out.push((rule.y, rule.y, rule.x0, rule.x1, rule.thickness)),
        }
    }
    out.into_iter()
        .map(|(top, bottom, left, right, stroke)| {
            (
                (top + bottom) / 2.0,
                ((right - left) / page_width).min(1.0),
                top - bottom + stroke,
            )
        })
        .collect()
}

/// Teach every line where the page's columns are, and re-split it there.
///
/// A gap alone is not enough on its own line. "Anton Munkholm Petersen" nearly
/// fills its column, so only fourteen points separate it from the name in the
/// next one -- less than the width of two of its own spaces -- and the row came
/// out as one cell while the row of ROLES under it, set smaller, came out as
/// three. One row of a table splitting differently from the next is the same as
/// no table at all.
///
/// What is steady is not the gaps but the column starts: on that page every
/// second column begins at x=259 and every third at x=408, line after line. So
/// the starts that RECUR are taken as the page's columns, and every line is cut
/// at them -- including the lines whose own gaps were too small to see.
fn align_to_column_stops(lines: &mut [Line], per_line: &[Vec<Run>]) {
    // Where cells began, from the lines whose gaps were wide enough to be sure.
    let mut seen: Vec<(f64, usize)> = Vec::new();
    for line in lines.iter().filter(|l| l.cells.len() > 1) {
        for cell in line.cells.iter().skip(1) {
            match seen.iter_mut().find(|(x, _)| (*x - cell.left).abs() < 4.0) {
                Some((_, n)) => *n += 1,
                None => seen.push((cell.left, 1)),
            }
        }
    }
    // Twice is a column; once is a sentence with a gap in it.
    let mut stops: Vec<f64> = seen
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(x, _)| x)
        .collect();
    if stops.is_empty() {
        return;
    }
    stops.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    for (line, runs) in lines.iter_mut().zip(per_line.iter()) {
        if line.cells.is_empty() {
            continue;
        }
        line.cells = cells_at_stops(runs, &stops, line.size);
    }
}

/// Cut a line's runs at the page's column stops.
///
/// Exact, because a run knows its own x: a word belongs to the last column that
/// starts at or before it. Nothing is guessed from the text, which is what made
/// "1.400.000" come apart into "1." and "400.000".
fn cells_at_stops(runs: &[Run], stops: &[f64], size: f64) -> Vec<Cell> {
    let mut groups: Vec<Vec<&Run>> = Vec::new();
    let mut sorted: Vec<&Run> = runs.iter().collect();
    sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let mut current_stop: Option<f64> = None;
    for run in sorted {
        // Which column this word is in: the last stop it reaches. A word left of
        // every stop is in the first column, which no stop names.
        let mine = stops
            .iter()
            .copied()
            .filter(|x| run.x + 1.0 >= *x)
            .fold(None, |_, x| Some(x));
        if groups.is_empty() || mine != current_stop {
            groups.push(Vec::new());
            current_stop = mine;
        }
        if let Some(g) = groups.last_mut() {
            g.push(run);
        }
    }
    groups
        .into_iter()
        .filter_map(|g| {
            let owned: Vec<Run> = g.into_iter().cloned().collect();
            let cell = cells_of(&owned, size);
            // One group is one cell: the gaps inside it were not columns.
            let left = owned.first()?.x;
            let right = owned.iter().map(|r| r.end_x).fold(f64::MIN, f64::max);
            let spans: Vec<Span> = cell.into_iter().flat_map(|c| c.spans).collect();
            match spans.iter().any(|s| !s.text.trim().is_empty()) {
                true => Some(Cell {
                    left,
                    right,
                    spans: tidy(spans),
                }),
                false => None,
            }
        })
        .collect()
}

/// One phrase of a page, with where it starts and how wide the page drew it.
struct Word {
    x: f64,
    y: f64,
    width: f64,
    size: f64,
    text: String,
    color: Option<String>,
    bold: bool,
    italic: bool,
    family: Family,
    link: Option<Link>,
}

/// Gather a page's runs into PHRASES: everything on one baseline that no gap
/// wider than a column separates, with the spaces put back in.
///
/// Not words. A word placed on its own has only its position to say where the
/// next one starts, and the stand-in face draws it a few per cent wider than the
/// file's own advances -- so the space between two words is eaten and the page
/// reads "RadikalUngdomsSangbog2025". A phrase carries its spaces as text and
/// the browser lays them out, which is what a face with the right metrics does
/// well.
///
/// Cut at the COLUMN gap rather than the word gap, so what the page set apart
/// stays apart: a label and its figure, a name and its role. Inside a phrase the
/// spacing is the font's; between phrases it is the page's.
fn phrases_of(runs: &[Run]) -> Vec<Word> {
    let mut sorted: Vec<&Run> = runs.iter().filter(|r| !r.text.trim().is_empty()).collect();
    sorted.sort_by(|a, b| {
        b.y.partial_cmp(&a.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut out: Vec<Word> = Vec::new();
    let mut last_end = f64::NEG_INFINITY;
    for run in sorted {
        let gap = run.x - last_end;
        let joins = out.last().is_some_and(|w| {
            (w.y - run.y).abs() < (w.size.max(run.size) * 0.3).max(0.5)
                && w.bold == run.bold
                && w.italic == run.italic
                && w.family == run.family
                && w.color == run.color
                && w.link == run.link
                && gap <= run.size * COLUMN_GAP
                // Not backwards -- but kerning IS backwards, by a little. The
                // songbook tucks the "o" of "Tobias" under the arm of its "T",
                // which a strict rule read as a new phrase and split the name in
                // two. A third of a size is more than any pair kerns and less
                // than any column steps back.
                && run.x >= last_end - run.size * 0.3
        });
        match joins {
            true => {
                if let Some(word) = out.last_mut() {
                    // The space the page left rather than drew.
                    if gap > run.size * word_gap() && !word.text.ends_with(' ') {
                        word.text.push(' ');
                    }
                    word.text.push_str(&run.text);
                    word.width = run.end_x - word.x;
                    word.size = word.size.max(run.size);
                }
            }
            false => out.push(Word {
                x: run.x,
                y: run.y,
                width: (run.end_x - run.x).max(0.0),
                size: run.size,
                text: run.text.clone(),
                color: run.color.clone(),
                bold: run.bold,
                italic: run.italic,
                family: run.family,
                link: run.link.clone(),
            }),
        }
        last_end = run.end_x;
    }
    out
}

/// A page's size in points, from its own box or the one it inherits.
fn page_size(doc: &Document, page_id: lopdf::ObjectId) -> (f64, f64) {
    // A4 when the file does not say, which is what every document here is.
    const A4: (f64, f64) = (595.276, 841.89);
    let Ok(page) = doc.get_dictionary(page_id) else {
        return A4;
    };
    // CropBox is what a reader is shown when it differs from the media.
    let box_of = |name: &[u8]| {
        page.get(name)
            .ok()
            .and_then(|o| resolve(doc, o).ok())
            .and_then(|o| o.as_array().ok().cloned())
            .or_else(|| {
                doc.get_dictionary(page_id)
                    .ok()
                    .and_then(|p| p.get(b"Parent").ok().cloned())
                    .and_then(|parent| parent.as_reference().ok())
                    .and_then(|id| doc.get_dictionary(id).ok())
                    .and_then(|p| p.get(name).ok())
                    .and_then(|o| resolve(doc, o).ok())
                    .and_then(|o| o.as_array().ok().cloned())
            })
    };
    let rect = box_of(b"CropBox").or_else(|| box_of(b"MediaBox"));
    let Some(rect) = rect else { return A4 };
    let nums: Vec<f64> = rect.iter().filter_map(|o| number(o).ok()).collect();
    match nums.as_slice() {
        [x0, y0, x1, y1] => {
            let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
            match w > 1.0 && h > 1.0 {
                true => (w, h),
                false => A4,
            }
        }
        _ => A4,
    }
}

/// One page as it was drawn: everything with its place on it.
///
/// The y axis is turned over here, once: a PDF measures up from the foot of the
/// page and a screen measures down from the top, and every consumer of this
/// would otherwise have to remember which it was holding.
fn page_layout(doc: &Document, page_id: lopdf::ObjectId, drawn: &Drawn) -> PageLayout {
    let (width, height) = page_size(doc, page_id);
    let mut items: Vec<Placed> = Vec::new();
    // WORDS, not glyphs. A file is free to place every letter itself, and the
    // songbook does: each glyph at its own x, spaced tighter than the font's own
    // advances. Placed one by one in a stand-in face, each letter is drawn at
    // ITS width and reaches into the next, and a name comes out a jumble.
    //
    // A word placed at its start and drawn by the browser cannot do that: the
    // letters inside it are spaced by the face, which has the widths the
    // original had. Any error is a fraction of a point over a word rather than a
    // collision at every letter -- and there are twenty-five words on a page
    // where there were a hundred and fifty glyphs, which the DOM notices too.
    for word in phrases_of(&drawn.runs) {
        items.push(Placed {
            x: word.x,
            // The baseline, from the top. What sits ON it is drawn above it.
            y: height - word.y,
            width: word.width,
            height: word.size,
            what: What::Text {
                // A tab drawn inside a word is a space: the page positions its
                // words itself, so the character is only there to be seen, and
                // `pre` would draw it eight columns wide.
                text: word.text.replace(['\t', '\n', '\r'], " "),
                size: word.size,
                color: word.color,
                bold: word.bold,
                italic: word.italic,
                family: word.family,
                link: word.link,
            },
        });
    }
    for (top, picture) in &drawn.pictures {
        items.push(Placed {
            x: picture.left,
            y: height - top,
            width: picture.width,
            height: picture.height,
            what: What::Image(picture.clone()),
        });
    }
    // One line, however many strokes drew it. A bar is commonly laid down as a
    // stack of hairlines a tenth of a point apart -- the report's header rule is
    // eight of them -- and drawing each is eight elements for one line, at the
    // weight of the thinnest rather than of the bar.
    let mut drawn_rules: Vec<(f64, f64, f64, f64)> = Vec::new();
    for rule in &drawn.rules {
        let (x0, x1) = (rule.x0.min(rule.x1), rule.x0.max(rule.x1));
        let same = drawn_rules
            .iter_mut()
            .find(|(y, a, b, _)| (*y - rule.y).abs() < 1.0 && x0 <= *b + 1.0 && x1 >= *a - 1.0);
        match same {
            Some((y, a, b, thick)) => {
                *a = a.min(x0);
                *b = b.max(x1);
                // As thick as the stack reaches, for the reason above.
                let (top, bottom) = (y.max(rule.y), y.min(rule.y));
                *thick = thick.max(rule.thickness).max(top - bottom + rule.thickness);
                *y = (top + bottom) / 2.0;
            }
            None => drawn_rules.push((rule.y, x0, x1, rule.thickness)),
        }
    }
    for (y, x0, x1, thickness) in drawn_rules {
        items.push(Placed {
            x: x0,
            y: height - y,
            width: (x1 - x0).max(0.0),
            height: thickness.max(0.4),
            what: What::Rule,
        });
    }
    PageLayout {
        width,
        height,
        items,
    }
}

/// The stretches of lines that stood in columns, as [start, end) line indices.
///
/// A table is not one row: one line with a gap in it is a heading with a page
/// number after it, a sentence with a tab, a name and a date. What makes it a
/// table is that the NEXT line stands in the same columns, and the one after
/// that. Two rows is the least that can say so.
///
/// Rows must also be near each other: the same columns half a page apart are
/// two different things that happen to share a margin.
fn table_ranges(lines: &[Line]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let here = &lines[i];
        if here.cells.len() < 2 || here.rule.is_some() || here.picture.is_some() {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        let mut last_row = i;
        while end < lines.len() {
            let next = &lines[end];
            let close = (lines[end - 1].y - next.y).abs() < here.size * 3.0;
            if next.page != here.page || !close {
                break;
            }
            if rows_align(&lines[last_row].cells, &next.cells, here.size) {
                last_row = end;
                end += 1;
                continue;
            }
            // A cell whose words did not fit on one line. "Ansvarlig for
            // internationalt samarbejde" wraps, and the tail sits alone under
            // its own column -- fewer cells than the row, every one of them
            // standing in a column the row already has.
            if continues_row(&lines[last_row].cells, &next.cells, here.size) {
                end += 1;
                continue;
            }
            break;
        }
        // Two rows or it is not a table.
        if end - i >= 2 {
            out.push((i, end));
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// Whether a line is the tail of the row above rather than a row of its own.
///
/// Fewer cells than the row, and each one standing in a column the row already
/// has. A row of its own would fill the columns; a wrapped cell fills one.
fn continues_row(row: &[Cell], tail: &[Cell], size: f64) -> bool {
    if tail.is_empty() || tail.len() >= row.len() {
        return false;
    }
    let slack = (size * 1.5).max(6.0);
    tail.iter()
        .all(|t| row.iter().any(|c| (c.left - t.left).abs() <= slack))
}

/// The rows of a table, with each wrapped tail folded back into its column.
fn table_rows(lines: &[Line], size: f64) -> Vec<Vec<Vec<Span>>> {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    for line in lines {
        let continuation = rows
            .last()
            .is_some_and(|row| continues_row(row, &line.cells, size));
        if !continuation {
            rows.push(line.cells.clone());
            continue;
        }
        let slack = (size * 1.5).max(6.0);
        if let Some(row) = rows.last_mut() {
            for tail in &line.cells {
                // Into the column it sits in, with a space where the line broke.
                if let Some(cell) = row.iter_mut().find(|c| (c.left - tail.left).abs() <= slack) {
                    if let Some(last) = cell.spans.last_mut() {
                        last.text.push(' ');
                    }
                    cell.spans.extend(tail.spans.iter().cloned());
                    cell.spans = tidy(std::mem::take(&mut cell.spans));
                }
            }
        }
    }
    rows.into_iter()
        .map(|row| row.into_iter().map(|c| c.spans).collect())
        .collect()
}

/// Whether two rows stand in the same columns.
///
/// The same number of groups, each starting within a size of the one above it.
/// Generous, because a column's contents are not all the same width and a
/// right-aligned figure starts wherever its digits begin -- but the LEFT edges
/// of a column's cells are set by the column, and they line up or they do not.
fn rows_align(a: &[Cell], b: &[Cell], size: f64) -> bool {
    if a.len() != b.len() || a.len() < 2 {
        return false;
    }
    let slack = (size * 1.5).max(6.0);
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| (x.left - y.left).abs() <= slack)
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
            rules: Vec::new(),
        };
    };
    let xobjects = page_xobjects(doc, page_id);

    let mut runs = Vec::new();
    let mut pictures: Vec<(f64, Picture)> = Vec::new();
    let mut shapes: Vec<Shape> = Vec::new();
    let mut arts: Vec<Art> = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    let mut pen = 1.0f64;
    let mut pen_at = (0.0f64, 0.0f64);
    let mut pale_fill = false;
    let mut pale_stroke = false;
    let mut marks: Vec<Mark> = Vec::new();
    let mut decoded: HashMap<lopdf::ObjectId, String> = HashMap::new();
    let mut ctm = IDENTITY;
    // The fill colour rides with the CTM: `q`/`Q` save and restore the whole
    // graphics state, and a heading's colour set inside one would otherwise leak
    // into the body text after it.
    let mut fill: Option<String> = None;
    let mut stack: Vec<([f64; 6], Option<String>, f64)> = Vec::new();
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
            // The pen rides with the rest of the graphics state. Leaving it out
            // meant a `0 w` set inside one block -- "as thin as this device can
            // draw" -- stayed in force after the block closed, and every rule
            // drawn afterwards came out at the floor. The annual report's rules
            // measure 0.48pt and 0.72pt in the ink; all 141 of them were
            // arriving as 0.2, which is why no two lines looked different.
            "q" => stack.push((ctm, fill.clone(), pen)),
            "Q" => {
                let (c, f, w) = stack.pop().unwrap_or((IDENTITY, None, 1.0));
                ctm = c;
                fill = f;
                pen = w;
            }
            // A PDF has no underline: it has a line drawn under some words. The
            // path is gathered here and turned into rules when it is painted,
            // because until then it may still be discarded.
            "w" if nums.len() == 1 => pen = nums[0],
            "re" if nums.len() == 4 => {
                let (x, y) = (nums[0], nums[1]);
                shapes.push(Shape::Box {
                    x,
                    y,
                    w: nums[2],
                    h: nums[3],
                });
            }
            "m" if nums.len() == 2 => pen_at = (nums[0], nums[1]),
            "l" if nums.len() == 2 => {
                shapes.push(Shape::Line {
                    from: pen_at,
                    to: (nums[0], nums[1]),
                });
                pen_at = (nums[0], nums[1]);
            }
            // Painted. A fill is an AREA and counts only if the whole path is
            // thin; a stroke is a line and counts per segment. Either way, only
            // if it is dark enough to be seen at all.
            "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "S" | "s" => {
                let filling = !matches!(op.operator.as_str(), "S" | "s");
                let stroking = !matches!(op.operator.as_str(), "f" | "F" | "f*");
                // Enough of a path is a drawing, whichever way it is painted.
                let pale = match filling {
                    true => pale_fill,
                    false => pale_stroke,
                };
                if !pale {
                    if let Some(art) = drawing_of(&shapes, ctm) {
                        arts.push(art);
                        shapes.clear();
                        continue;
                    }
                }
                if filling && !pale_fill {
                    rules.extend(filled_rule(&shapes, ctm));
                }
                if stroking && !pale_stroke {
                    rules.extend(shapes.iter().flat_map(|s| s.stroked(ctm, pen)));
                }
                shapes.clear();
            }
            // Discarded, or kept only as a clip: either way it draws nothing.
            "n" | "W" | "W*" => shapes.clear(),
            // The non-stroking colour, in each of the ways a PDF states it.
            // `sc`/`scn` take their meaning from the current colour space; the
            // operand count tells us which, and a pattern (which has a name
            // operand) is left alone rather than guessed at.
            "g" if nums.len() == 1 => {
                fill = ink(nums[0], nums[0], nums[0]);
                pale_fill = too_pale(nums[0], nums[0], nums[0]);
            }
            "rg" if nums.len() == 3 => {
                fill = ink(nums[0], nums[1], nums[2]);
                pale_fill = too_pale(nums[0], nums[1], nums[2]);
            }
            "k" if nums.len() == 4 => {
                fill = cmyk(nums[0], nums[1], nums[2], nums[3]);
                pale_fill = nums[0] + nums[1] + nums[2] + nums[3] < 0.25;
            }
            // The stroking colour is tracked only for how pale it is: text is
            // filled, not stroked, so nothing else here needs it.
            "G" if nums.len() == 1 => pale_stroke = too_pale(nums[0], nums[0], nums[0]),
            "RG" if nums.len() == 3 => pale_stroke = too_pale(nums[0], nums[1], nums[2]),
            "K" if nums.len() == 4 => {
                pale_stroke = nums[0] + nums[1] + nums[2] + nums[3] < 0.25;
            }
            "SC" | "SCN" => match nums.len() {
                1 => pale_stroke = too_pale(nums[0], nums[0], nums[0]),
                3 => pale_stroke = too_pale(nums[0], nums[1], nums[2]),
                4 => pale_stroke = nums[0] + nums[1] + nums[2] + nums[3] < 0.25,
                _ => {}
            },
            "sc" | "scn" => match nums.len() {
                1 => {
                    fill = ink(nums[0], nums[0], nums[0]);
                    pale_fill = too_pale(nums[0], nums[0], nums[0]);
                }
                3 => {
                    fill = ink(nums[0], nums[1], nums[2]);
                    pale_fill = too_pale(nums[0], nums[1], nums[2]);
                }
                4 => {
                    fill = cmyk(nums[0], nums[1], nums[2], nums[3]);
                    pale_fill = nums[0] + nums[1] + nums[2] + nums[3] < 0.25;
                }
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
                    Picture {
                        path: None,
                        src,
                        left: ctm[4],
                        width,
                        height,
                    },
                ));
            }
            _ => {}
        }
    }
    underline_runs(&mut runs, &rules);
    pictures.extend(gather_drawings(arts));
    Drawn {
        runs,
        pictures,
        marks,
        rules,
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
    // Sideways text is not part of the reading, and reading it as though it were
    // puts it in the middle of a sentence. The annual report carries a signing
    // service's document key up the edge of all 27 pages, and it landed between
    // the paragraphs on every one of them: "Penneo dokumentnøgle: UMG2H-..."
    // where the next line of the balance sheet should be. A stamp, a watermark
    // and a rotated page label are all this shape.
    if turned(at) {
        // The pen still has to move, or everything after it lands in the wrong
        // place: a matrix that draws nothing is not a matrix that drew nothing.
        let width = bytes.len() as f64 * size * h_scale;
        tm[4] += width * at[0].signum().max(0.0);
        return;
    }
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
                family: font.family,
                bold: font.bold,
                // A file may say italic in the font, or draw it with a shear.
                italic: font.italic || slanted(at),
                underline: false,
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

/// One group of words on a line, and where it sat.
#[derive(Debug, Clone)]
struct Cell {
    left: f64,
    right: f64,
    spans: Vec<Span>,
}

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
    /// Set when it is a flat line the page drew: how much of the page's width it
    /// spanned, and how heavy it was. Same idea: it belongs where it was drawn.
    rule: Option<(f64, f64)>,
    /// The gap before the line's last word, in multiples of its type size.
    ///
    /// What tells a contents row from a sentence when there are no leader dots
    /// to go by: "Ledelsespåtegning" and its "4" are a third of the page apart,
    /// and nothing in the text says so.
    tail_gap: f64,
    /// Where this line's words sat, in groups, when gaps too wide to be spaces
    /// separated them: (left, right, the words). One group is an ordinary line.
    /// Several is a row of something -- a name over each role, a label and two
    /// years of figures -- and a reader who is handed it as one run of words
    /// cannot tell which figure belongs to which year.
    cells: Vec<Cell>,
    /// How many groups the line's OWN gaps made, before the page's columns were
    /// applied to it. A contents row has two, whatever the page does elsewhere:
    /// the page numbers are right-aligned, so a range like "5 - 10" starts left
    /// of where a single digit does and the column stop falls inside it.
    natural_cells: usize,
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
    // Each line's runs, kept beside it: the page's columns are worked out from
    // all the lines together and then applied to the RUNS, which know where each
    // word sat. Applying them to joined text instead meant guessing where a word
    // fell from how far along the string it was, and a guess inside "1.400.000"
    // cuts a number in half.
    let mut per_line: Vec<Vec<Run>> = Vec::new();
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
                per_line.push(std::mem::take(&mut current));
            }
            current = vec![run];
        }
    }
    if let Some(line) = join_line(&current) {
        lines.push(line);
        per_line.push(current);
    }
    align_to_column_stops(&mut lines, &per_line);
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

/// How wide a gap starts a new COLUMN rather than a new word, in type sizes.
///
/// Wider than a word space by a long way and narrower than the gaps a table
/// leaves: the annual report's figures stand five sizes clear of their labels,
/// and the board's three columns twice that. A justified line can stretch a
/// space to about one, so two is the first width that cannot be one.
const COLUMN_GAP: f64 = 2.0;

/// Split a line's runs where the gaps are too wide to be spaces.
///
/// A page draws a row of a table exactly as it draws a sentence: glyphs at
/// positions, with nothing to say that this group and that one are different
/// cells. The gap is the whole of the evidence.
fn cells_of(runs: &[Run], size: f64) -> Vec<Cell> {
    if size <= 0.0 {
        return Vec::new();
    }
    let mut sorted: Vec<&Run> = runs.iter().collect();
    sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let mut cells: Vec<Cell> = Vec::new();
    let mut prev_end = f64::NEG_INFINITY;
    for run in sorted {
        let starts_cell = !prev_end.is_finite() || (run.x - prev_end) > size * COLUMN_GAP;
        if starts_cell {
            cells.push(Cell {
                left: run.x,
                right: run.end_x,
                spans: Vec::new(),
            });
        }
        if let Some(cell) = cells.last_mut() {
            cell.right = cell.right.max(run.end_x);
            let wants_space = !starts_cell
                && !cell.spans.is_empty()
                && (run.x - prev_end) > run.size * word_gap();
            let here = Span {
                text: String::new(),
                color: run.color.clone(),
                bold: run.bold,
                italic: run.italic,
                underline: run.underline,
                link: run.link.clone(),
            };
            match cell.spans.last_mut() {
                Some(last) if last.matches(&here) => {
                    if wants_space {
                        last.text.push(' ');
                    }
                    last.text.push_str(&run.text);
                }
                _ => {
                    let mut text = String::new();
                    if wants_space {
                        match cell.spans.last_mut() {
                            Some(last) => last.text.push(' '),
                            None => text.push(' '),
                        }
                    }
                    text.push_str(&run.text);
                    cell.spans.push(Span { text, ..here });
                }
            }
        }
        prev_end = run.end_x;
    }
    for cell in &mut cells {
        cell.spans = tidy(std::mem::take(&mut cell.spans));
    }
    cells.retain(|c| c.spans.iter().any(|s| !s.text.trim().is_empty()));
    cells
}

fn join_line(runs: &[Run]) -> Option<Line> {
    let first = runs.first()?;
    let mut spans: Vec<Span> = Vec::new();
    let mut prev_end = f64::NEG_INFINITY;
    let mut tail_gap = 0.0_f64;
    let mut size: f64 = 0.0;
    let mut sorted: Vec<&Run> = runs.iter().collect();
    sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    for run in sorted {
        size = size.max(run.size);
        // Compared against the pen's TRUE end, not a guess from the character
        // count, which is what put spaces inside words. See [`WORD_GAP`].
        let gap = run.x - prev_end;
        if prev_end.is_finite() && run.size > 0.0 {
            tail_gap = gap / run.size;
        }
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
            underline: run.underline,
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
    let cells = cells_of(runs, size);
    let spans = mend_written_links(tidy(spans));
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
        rule: None,
        tail_gap,
        natural_cells: cells.len(),
        cells,
        left,
        // Filled in by the caller, which is the only place that knows.
        page: 0,
        bullet: None,
        picture: None,
    })
}

/// Collapse runs of whitespace and drop what is left empty, without losing the
/// colour boundaries.
///
/// A line ending survives the collapse where a space does not. It is the one
/// piece of whitespace here that carries meaning: it says the page put the next
/// words on their own line, which is what a verse is made of.
fn tidy(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for span in spans {
        let mut text = String::new();
        for c in span.text.chars() {
            // What was last written, here or at the end of the span before.
            let last = text
                .chars()
                .last()
                .or_else(|| out.last().and_then(|s: &Span| s.text.chars().last()));
            match (c.is_whitespace(), last) {
                // Leading whitespace, and whitespace after a line ending, go.
                (true, None) | (true, Some('\n')) => {}
                // A line ending replaces a space already written: the ending is
                // meaning and the space beside it is not.
                (true, Some(' ')) if c == '\n' => match text.pop() {
                    Some(_) => text.push('\n'),
                    None => {
                        if let Some(prev) = out.last_mut() {
                            prev.text.pop();
                            prev.text.push('\n');
                        }
                    }
                },
                (true, Some(' ')) => {}
                (true, _) => text.push(if c == '\n' { '\n' } else { ' ' }),
                (false, _) => text.push(c),
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

/// How wide a gap says "contents row" rather than "sentence", in type sizes.
///
/// A word space is about a quarter of the size and the widest justified space
/// rarely reaches one. Two and a half is a gap nothing sets by accident: on the
/// annual report's contents page the entries and their numbers are a third of
/// the page apart.
const CONTENTS_GAP: f64 = 2.5;

/// Split a table-of-contents row into what it names and the page it names.
///
/// Read from the end, because that is where the certainty is: a number, then
/// whatever carried the eye to it. Anything else is a sentence that happens to
/// end in a digit, and left alone.
///
/// Two kinds of carrier. Leader dots, which say so in the text. And plain
/// space, which says it only in the geometry -- the annual report's contents
/// page sets "Ledelsespåtegning" and its "4" a third of the page apart with
/// nothing between them, and read as text that is a sentence ending in a digit.
/// `gap` is how far apart the last two words were, in type sizes, which is the
/// only place that distinction survives.
fn index_entry(text: &str, gap: f64, two_columns: bool) -> Option<(&str, &str)> {
    let trimmed = text.trim_end();
    // A page, or a range of them: "13", "5 - 10", "17 - 25".
    let tail_start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit() || matches!(c, ' ' | '-' | '\u{2013}'))
        .last()
        .map(|(i, _)| i)?;
    let page = trimmed[tail_start..].trim();
    if page.is_empty() || !page.ends_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    // A page number, not a year in a title and not a whole line of digits.
    if page.chars().filter(char::is_ascii_digit).count() > 5 || page.len() > 9 {
        return None;
    }
    let lead = trimmed[..tail_start].trim_end();
    let dots = lead
        .chars()
        .rev()
        .take_while(|c| matches!(c, '.' | '\u{00B7}' | '\u{2024}' | '\u{2026}' | ' '))
        .filter(|c| *c != ' ')
        .count();
    // Dots say so by themselves: they ARE the row, and they bridge the gap they
    // fill, so such a row is one group of words and asking it for two columns
    // refuses every dotted contents page there is -- which is what happened to
    // the songbook's, links and all.
    //
    // A gap says so only with the columns to back it: a statement's row also
    // ends in a figure after a wide gap, and "I alt -1.260.170 -1.016.876
    // 197.801" is not an entry pointing at page 801.
    let by_dots = dots >= 3;
    let by_gap = gap >= CONTENTS_GAP && two_columns;
    if !by_dots && !by_gap {
        return None;
    }
    // Only strip a leader that is actually there. "Foreningsoplysninger m.v."
    // ends in a full stop that belongs to the abbreviation, and a row carried by
    // a gap has no leader to remove -- taking the dots off both alike turned it
    // into "Foreningsoplysninger m.v".
    let title = match by_dots {
        true => lead
            .trim_end_matches([' ', '.', '\u{00B7}', '\u{2024}', '\u{2026}'])
            .trim(),
        false => lead.trim(),
    };
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
    // Near the top of the range, not a tenth down it. A tenth holds while most
    // of a document is prose that fills its column, and fails on a book that is
    // not: the songbook is a hundred pages of verse, none of whose lines come
    // near the margin, so a tenth down put the column at 392 when the margin is
    // at 539. Its longer lines then measured as running PAST the column, which
    // reads as "this line reached the margin and wrapped", and Ode an die Freude
    // arrived as one long line.
    //
    // A fiftieth still keeps a stray out: something drawn in the margin is one
    // line or two, and this wants a fiftieth of them to agree.
    let near_the_end = rights.len().saturating_sub(1) * 49 / 50;
    rights[near_the_end]
}

/// Where the text column starts: the leftmost place that enough lines begin at.
///
/// Not a low percentile, which is what this was and which fails whenever most of
/// a document is indented. The alkoholpolitik sets four lines at its margin and
/// forty in a list one step in from it, so a tenth percentile lands on the LIST,
/// calls the margin a negative indent and flattens every depth in the file to
/// nothing: the bullets came out past the title. Nor the minimum, which one line
/// hanging into the margin would move.
///
/// So: the leftmost edge that a real share of the lines share. A stray element
/// further out is one or two lines and does not reach the share; an indented
/// body is not the leftmost.
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
    // A twentieth of the lines, and never fewer than three: enough that a mark
    // or a hanging initial cannot pass for a margin.
    let enough = ((lefts.len() as f64 * 0.05).ceil() as usize).max(3);
    let mut at = 0;
    while at < lefts.len() {
        let edge = lefts[at];
        // Within a point of each other is the same edge: a line's left is the
        // pen's, and the pen does not land twice in exactly the same place.
        let mut to = at;
        while to < lefts.len() && lefts[to] - edge <= 1.0 {
            to += 1;
        }
        if to - at >= enough {
            return edge;
        }
        at = to;
    }
    // Nothing is shared by that many: fall back to where this came in.
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
/// Right-alignment waited for the tables. A table cell starts well inside the
/// column and ends near its right edge, which is exactly what a right-aligned
/// line looks like, and while the rows still arrived as paragraphs the two could
/// not be told apart: inferring it turned eleven rows of an activity plan into
/// right-aligned paragraphs. Now a row is a row, and what is left standing flush
/// against the right margin is there because the document put it there -- the
/// annual report sets every statement's title that way. It is read the same way
/// centring is, and refused for the same reason: several lines must each START
/// somewhere different, or this is an indented block that happens to reach the
/// margin, and one line must be short enough that reaching the margin means
/// something.
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
    // Flush against the right margin, and clear of the left one: the two edges
    // together, since text that merely reaches the right margin is ordinary
    // text that filled its line.
    let varies = |edges: &dyn Fn(&(f64, f64)) -> f64| {
        let lo = usable
            .iter()
            .map(|p| edges(p))
            .fold(f64::INFINITY, f64::min);
        let hi = usable
            .iter()
            .map(|p| edges(p))
            .fold(f64::NEG_INFINITY, f64::max);
        hi - lo > size
    };
    let flush_right = usable
        .iter()
        .all(|(l, r)| (col_right - r).abs() < size * 0.5 && l - col_left > size);
    if flush_right {
        let (l, r) = usable[0];
        let convincing = match usable.len() {
            // One line: short enough that ending at the margin is a choice
            // rather than the width of the words.
            1 => r - l < width * 0.6,
            // Several: their left edges must vary, or this is an indented block
            // whose lines happen to reach the margin.
            _ => varies(&|(l, _)| *l),
        };
        if convincing {
            return Align::Right;
        }
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

/// Whether a line break falls INSIDE a web address, so the two sides join with
/// nothing between them.
///
/// A wrapped line is one piece of text broken for the margin's sake, and the
/// break is usually between words, where a space belongs. Not always: the
/// samværspolitik prints its alkoholpolitik link across three lines, breaking it
/// after "https://" and again after a hyphen, and a space at either place is a
/// broken address rather than a line ending. Only for a WRAP: a line that ended
/// on purpose ended, and the next line starts something, even after an address.
fn continues_a_url(before: &str, after: &str) -> bool {
    let tail = before.split_whitespace().next_back().unwrap_or_default();
    let head = after.split_whitespace().next().unwrap_or_default();
    if tail.is_empty() || head.is_empty() {
        return false;
    }
    let started = tail.to_ascii_lowercase();
    let started = started.starts_with("http://")
        || started.starts_with("https://")
        || started.starts_with("www.")
        || started.contains("://");
    // An address in progress breaks where it runs out of room, not at a word,
    // so what follows carries straight on from it.
    started && !head.starts_with(|c: char| c.is_ascii_punctuation() && c != '/' && c != '-')
}

/// What one line is to the line before it.
enum Flow {
    /// It ran out of room and carried on: the two are one line of prose.
    Wrapped,
    /// It ended where it meant to, and the next starts underneath: a verse, an
    /// address, an agenda. One block still, but the break is kept.
    Line,
    /// A blank line, a change of size, a marker, a new page: a new block.
    Apart,
}

/// How much room the first word of this line would have wanted on the line
/// before it, including the space in front of it.
///
/// Measured from the line's own ink: its width divided by its characters is
/// what a character costs in the face it is set in, which is closer than any
/// table of averages and needs no font metrics at this stage.
fn next_word_width(line: &Line, fallback_size: f64) -> f64 {
    let letters = line.text.chars().count();
    let first = line.text.split_whitespace().next().unwrap_or_default();
    if first.is_empty() {
        return 0.0;
    }
    let per_letter = match (letters, line.right - line.left) {
        (0, _) => fallback_size * 0.5,
        (n, width) if width.is_finite() && width > 0.0 => width / n as f64,
        _ => fallback_size * 0.5,
    };
    // The word, and the space that would have gone before it.
    (first.chars().count() as f64 + 1.0) * per_letter
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
    // The gap before the last word of the line being built, carried the way the
    // drawn bullet is: it is a property of the LINE and the decision it feeds is
    // made about the block. With how many columns it stood in, which is what
    // tells a contents row from a row of figures: both end in a number after a
    // wide gap, and only one of them has just the two.
    let mut para_gap = 0.0_f64;
    let mut para_cells = 0usize;
    let flush = |para: &mut Vec<Span>,
                 size: f64,
                 geometry: &mut Vec<(f64, f64)>,
                 bullet: &mut Option<String>,
                 gap: &mut f64,
                 cells: &mut usize,
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
        // Again at the block, not only at the line: an address broken across
        // lines is only whole once they are joined, and the file linked the
        // pieces it drew rather than the address it meant. Here it becomes one
        // link over the whole of it, "https://" included.
        let spans = mend_written_links(tidy(std::mem::take(para)));
        if spans.is_empty() {
            return;
        }
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        let tail = std::mem::take(gap);
        // Two columns, or it is a row of a table that happens to end in a
        // figure: "I alt -1.260.170 -1.016.876 197.801" was arriving as a
        // contents entry pointing at page 801.
        let two_columns = std::mem::replace(cells, 0) == 2;
        if let Some((title, page)) = index_entry(&text, tail, two_columns) {
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
                indent,
            });
        } else if drawn_bullet.is_some() {
            // A bullet the page DREW, so there is no marker in the text to take
            // off: the words are the item, whole.
            blocks.push(Block::ListItem {
                spans,
                marker: drawn_bullet,
                indent,
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
    // Which stretches of lines stood in columns. Worked out before the reading,
    // because the question is about a run of lines together and the loop below
    // decides one at a time.
    let tables = table_ranges(&lines_ref);
    let mut skip_to = 0usize;
    for (at, line) in lines_ref.iter().enumerate() {
        if at < skip_to {
            continue;
        }
        if let Some((_, end)) = tables.iter().find(|(start, _)| *start == at) {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut para_gap,
                &mut para_cells,
                &mut blocks,
            );
            blocks.push(Block::Table {
                rows: table_rows(&lines_ref[at..*end], line.size),
            });
            skip_to = *end;
            prev = None;
            continue;
        }
        // An anchor goes BEFORE what it points at, so whatever was being built
        // ends here: a link should land on the start of the thing it names, not
        // in the middle of the paragraph above it.
        if let Some(here) = anchors.get(&at) {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut para_gap,
                &mut para_cells,
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
                &mut para_gap,
                &mut para_cells,
                &mut blocks,
            );
            blocks.push(Block::PageBreak {
                ended: page,
                printed: printed.get(&(page - 1)).cloned(),
                starts: printed.get(&page).cloned(),
            });
            page = line.page;
            prev = None;
        }
        if let Some(picture) = &line.picture {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut para_gap,
                &mut para_cells,
                &mut blocks,
            );
            blocks.push(Block::Image(picture.clone()));
            prev = None;
            continue;
        }
        // A rule ends whatever was being built, which is the point of it: the
        // paragraph above and the one below were separated on the page.
        if let Some((width, thickness)) = line.rule {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut para_gap,
                &mut para_cells,
                &mut blocks,
            );
            blocks.push(Block::Rule { width, thickness });
            prev = None;
            continue;
        }
        let flow = match prev {
            None => Flow::Wrapped,
            Some(p) => {
                let gap = p.y - line.y;
                // A line that stopped short of the column may have ended on
                // purpose, which is what tells an agenda from a paragraph, and
                // the vertical gap cannot because both are single-spaced.
                // Without it every time on a programme ran into the next entry
                // and a whole day arrived as one wall of text.
                //
                // But short of WHAT? Two ems of slack was the first answer and it
                // is wrong for prose: a line wraps where the next word stops
                // fitting, and Danish has some long ones. "Landsmøde skal der
                // dog være tre..." stops sixty points short of the margin
                // because "internationale" follows it, and the alkoholpolitik's
                // first rule was arriving as three paragraphs.
                //
                // So ask whether the next line's first word would have FITTED. If
                // it would have, the line ended because someone ended it; if it
                // would not have, the line simply ran out of room.
                let ended_early = p.right.is_finite() && {
                    let room = column - p.right;
                    room > p.size * 2.0 && room > next_word_width(line, p.size)
                };
                // A line that ended on purpose is a LINE, not a paragraph. A
                // verse is a stack of them, single-spaced, and turning each into
                // a paragraph put a paragraph's air between every line of
                // Lokalforeningssangen. What separates one block from the next is
                // the blank line between them, which is a wider gap; a size
                // change; a marker; or the page turning over.
                let apart = gap > p.size * 1.6
                    || (line.size - p.size).abs() > p.size * 0.15
                    || is_index_entry(&line.text)
                    // Or the same row without leader dots, where the page
                    // number is a third of the page away and only the geometry
                    // says so.
                    || index_entry(&line.text, line.tail_gap, line.natural_cells == 2).is_some()
                    || list_marker(&line.text).is_some()
                    // A bullet the page DREW starts an item as surely as one
                    // written in the text does, and without this a whole list
                    // collapsed into a single item.
                    || line.bullet.is_some()
                    // A page break: y jumps back UP the page.
                    || gap < -1.0;
                match (apart, ended_early || starts_new_entry(&line.text)) {
                    (true, _) => Flow::Apart,
                    (false, true) => Flow::Line,
                    (false, false) => Flow::Wrapped,
                }
            }
        };
        if matches!(flow, Flow::Apart) {
            flush(
                &mut para,
                para_size,
                &mut para_lines,
                &mut para_bullet,
                &mut para_gap,
                &mut para_cells,
                &mut blocks,
            );
        }
        if para.is_empty() {
            para_size = line.size;
            para_bullet = line.bullet.clone();
        } else {
            // A wrapped line joins with a space; a line that ended on purpose
            // keeps its ending, and the reading surface honours the newline.
            // Unless the wrap fell inside a web address, which joins with
            // nothing: a space there is a broken address.
            let so_far: String = para.iter().map(|s| s.text.as_str()).collect();
            let joiner = match flow {
                Flow::Line => Some('\n'),
                _ if continues_a_url(&so_far, &line.text) => None,
                _ => Some(' '),
            };
            if let (Some(c), Some(last)) = (joiner, para.last_mut()) {
                last.text.push(c);
            }
        }
        para.extend(line.spans.iter().cloned());
        para_lines.push((line.left, line.right));
        // The last line's gap is the block's: a contents row is one line, and a
        // paragraph that happens to end in a number did not leave a hole before
        // it. Taking the last rather than the widest keeps a table row from
        // being read as a contents entry for having columns.
        para_gap = line.tail_gap;
        para_cells = line.natural_cells;
        prev = Some(line);
    }
    flush(
        &mut para,
        para_size,
        &mut para_lines,
        &mut para_bullet,
        &mut para_gap,
        &mut para_cells,
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
fn mend_written_links(spans: Vec<Span>) -> Vec<Span> {
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    let addresses = emails_in(&text);
    let mut found = addresses.clone();
    found.extend(urls_in(&text));
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
    for (from, to, href) in &found {
        for (_, span) in chars.iter_mut().take(*to).skip(*from) {
            span.link = Some(Link::Url(href.clone()));
        }
    }
    // And a stale mailto comes off everything else on the line. Only mailto: a
    // link on words that are not an address is ordinary and usually right, but
    // an address written beside one says the file's is out of date.
    if !addresses.is_empty() {
        for (at, (_, span)) in chars.iter_mut().enumerate() {
            let inside = found.iter().any(|(from, to, _)| at >= *from && at < *to);
            if !inside && matches!(&span.link, Some(Link::Url(u)) if u.starts_with("mailto:")) {
                span.link = None;
            }
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
        let address: String = chars[from..to].iter().collect();
        out.push((from, to, format!("mailto:{address}")));
    }
    out
}

/// Every web address WRITTEN OUT in a line, as character ranges.
///
/// A file often prints its links rather than making them: "kan læses her:" and
/// then the address on the next line, with no annotation anywhere. The reader
/// sees something that looks like a link and it does nothing, so what is
/// written is made to work.
fn urls_in(text: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let opens = |at: usize, want: &str| {
        want.chars().enumerate().all(|(k, c)| {
            chars
                .get(at + k)
                .is_some_and(|g| g.eq_ignore_ascii_case(&c))
        })
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let prefix = ["https://", "http://", "www."]
            .into_iter()
            .find(|p| opens(at, p));
        let Some(prefix) = prefix else {
            at += 1;
            continue;
        };
        // At a word boundary: not the tail of something longer.
        if at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '.') {
            at += 1;
            continue;
        }
        let mut to = at;
        while to < chars.len() && !chars[to].is_whitespace() {
            to += 1;
        }
        // The sentence's punctuation is the sentence's.
        while to > at
            && matches!(
                chars[to - 1],
                '.' | ',' | ';' | ':' | ')' | ']' | '"' | '\'' | '»'
            )
        {
            to -= 1;
        }
        let written: String = chars[at..to].iter().collect();
        if written.chars().count() > prefix.chars().count() {
            let href = match prefix {
                "www." => format!("https://{written}"),
                _ => written,
            };
            out.push((at, to, href));
        }
        at = to.max(at + 1);
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
    let mut layout: Vec<PageLayout> = Vec::new();
    let mut budget = PICTURE_BUDGET;
    for (page_no, (_, page_id)) in pages.into_iter().enumerate() {
        // The links live beside the content stream rather than in it, so they
        // are read first and handed to the walk, which cuts its runs on them.
        let areas = page_links(&doc, page_id, &by_id, &mut places);
        let drawn = page_runs(&doc, page_id, &mut budget, &areas);
        // The page as it was drawn, before the reading takes it apart. Built
        // here because this is the last place the positions exist: `lines_from`
        // turns the runs into lines and the lines into a reading order, which is
        // a different thing from a page and cannot be turned back into one.
        layout.push(page_layout(&doc, page_id, &drawn));
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
                rule: None,
                tail_gap: 0.0,
                cells: Vec::new(),
                natural_cells: 0,
            });
        }
        // And so does a line the page drew. A rule that turned out to be an
        // underline is already carried by its run and must not be drawn twice;
        // what is left separated one thing from another, which is worth keeping.
        let page_width = lines
            .iter()
            .filter(|l| l.right.is_finite() && l.left.is_finite())
            .map(|l| l.right - l.left)
            .fold(0.0_f64, f64::max)
            .max(1.0);
        for (y, width, thickness) in separators(&drawn.rules, &lines, page_width) {
            lines.push(Line {
                y,
                size: 1.0,
                right: f64::NEG_INFINITY,
                left: f64::INFINITY,
                page: page_no + 1,
                text: String::new(),
                spans: Vec::new(),
                bullet: None,
                picture: None,
                rule: Some((width, thickness)),
                tail_gap: 0.0,
                cells: Vec::new(),
                natural_cells: 0,
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
    drop_unused_anchors(&mut blocks);
    Ok(Extracted {
        blocks,
        layout,
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

/// Take out the anchors nothing points at any more.
///
/// A destination the file named gets an anchor whether or not the destination
/// was any good, and the songbook's are all bad: its every contents row points
/// into the contents page itself, so the anchors land in the middle of the list
/// and the rows were repointed elsewhere. What is left is forty-five landing
/// places for nobody, sitting BETWEEN the rows, and each one broke the list in
/// two so that the gaps down the page came out uneven.
fn drop_unused_anchors(blocks: &mut Vec<Block>) {
    let wanted: std::collections::HashSet<&str> = blocks
        .iter()
        .flat_map(|b| b.spans())
        .filter_map(|s| match &s.link {
            Some(Link::Place(id)) => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let wanted: std::collections::HashSet<String> = wanted.into_iter().map(String::from).collect();
    blocks.retain(|b| match b {
        Block::Anchor(id) => wanted.contains(id),
        _ => true,
    });
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

/// The operators one page runs, for when a rule turns up that the page does not
/// show.
#[cfg(test)]
fn page_ops(bytes: &[u8], want: usize) -> Vec<String> {
    let Ok(doc) = Document::load_mem(bytes) else {
        return Vec::new();
    };
    for (page_no, (_, page_id)) in doc.get_pages().into_iter().enumerate() {
        if page_no + 1 != want {
            continue;
        }
        let raw = doc.get_page_content(page_id);
        let Ok(content) = lopdf::content::Content::decode(&raw) else {
            return Vec::new();
        };
        return content
            .operations
            .iter()
            .map(|op| {
                let args: Vec<String> = op
                    .operands
                    .iter()
                    .map(|o| format!("{o:?}").chars().take(16).collect())
                    .collect();
                format!("{} {}", op.operator, args.join(" "))
            })
            .collect();
    }
    Vec::new()
}

/// The flat lines one page drew, for the harness.
#[cfg(test)]
fn page_rules(bytes: &[u8], want: usize) -> Vec<(f64, f64, f64, f64)> {
    let Ok(doc) = Document::load_mem(bytes) else {
        return Vec::new();
    };
    let mut budget = PICTURE_BUDGET;
    for (page_no, (_, page_id)) in doc.get_pages().into_iter().enumerate() {
        if page_no + 1 == want {
            return page_runs(&doc, page_id, &mut budget, &[])
                .rules
                .into_iter()
                .map(|r| (r.x0, r.x1, r.y, r.thickness))
                .collect();
        }
    }
    Vec::new()
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

/// Where the whole document's column runs to, for the harness.
#[cfg(test)]
fn document_column(bytes: &[u8]) -> f64 {
    match extract(bytes) {
        Ok(_) => {}
        Err(_) => return f64::INFINITY,
    }
    let Ok(doc) = Document::load_mem(bytes) else {
        return f64::INFINITY;
    };
    let mut budget = PICTURE_BUDGET;
    let mut all = Vec::new();
    for (page_no, (_, page_id)) in doc.get_pages().into_iter().enumerate() {
        let drawn = page_runs(&doc, page_id, &mut budget, &[]);
        let mut lines = lines_from(drawn.runs);
        for line in &mut lines {
            line.page = page_no + 1;
        }
        all.extend(lines);
    }
    column_right(&all)
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
                    let drawings = doc
                        .blocks
                        .iter()
                        .filter(|b| matches!(b, super::Block::Image(p) if p.path.is_some()))
                        .count();
                    let italics = doc
                        .blocks
                        .iter()
                        .flat_map(|b| b.spans())
                        .filter(|s| s.italic)
                        .count();
                    let underlined = doc
                        .blocks
                        .iter()
                        .flat_map(|b| b.spans())
                        .filter(|s| s.underline)
                        .count();
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
                        "{name:34.34} pages={:<4} blocks={:<5} chars={:<7} toc={toc:<4} draw={drawings:<3} bullets={drawn:<4} ul={underlined:<4} it={italics:<4} linked={linked:<4} anchors={anchors:<4} indent={indented:<4} numbered={numbered:<4} stripped={}",
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

    /// Every drawing a document made, as a page that can be opened, so what the
    /// renderer produces can be looked at rather than reasoned about.
    ///
    ///   PDF_UNDER_TEST=/path/to.pdf PDF_SVG=1 cargo test pdf_text::harness -- --nocapture > out.html
    #[test]
    fn draw_the_drawings() {
        let (Ok(path), Ok(_)) = (std::env::var("PDF_UNDER_TEST"), std::env::var("PDF_SVG")) else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let doc = super::extract(&bytes).expect("read the pdf");
        println!("<!doctype html><meta charset=utf-8><body style='background:#fff;color:#111'>");
        for block in doc.blocks.iter() {
            let super::Block::Image(picture) = block else {
                continue;
            };
            let Some(d) = &picture.path else { continue };
            println!(
                "<div style='border:1px dashed #ccc;margin:8px;display:inline-block'>\
                 <svg viewBox='0 0 {} {}' width='{}' height='{}'>\
                 <path d='{d}' fill='none' stroke='currentColor' stroke-width='1'/></svg></div>",
                picture.width, picture.height, picture.width, picture.height
            );
        }
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
        if std::env::var("PDF_OPS").is_ok() {
            for line in super::page_ops(&bytes, page).iter().take(60) {
                println!("  op: {line}");
            }
        }
        if std::env::var("PDF_OPS").is_ok() {
            for line in super::page_ops(&bytes, page).iter() {
                println!("  op: {line}");
            }
        }
        println!("--- page {page} rules drawn ---");
        for r in super::page_rules(&bytes, page).iter().take(14) {
            println!(
                "  x {:>7.1}..{:<7.1} y {:>7.1}  thick {:>5.2}",
                r.0, r.1, r.2, r.3
            );
        }
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

    /// What the block builder decides about each line of a page, and why.
    ///
    ///   PDF_UNDER_TEST=/path/to.pdf PDF_FLOW=17 cargo test pdf_text::harness -- --nocapture
    #[test]
    fn why_each_line_flows_as_it_does() {
        let (Ok(path), Ok(page)) = (
            std::env::var("PDF_UNDER_TEST"),
            std::env::var("PDF_FLOW").map(|v| v.parse::<usize>().unwrap_or(1)),
        ) else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read");
        let column = super::document_column(&bytes);
        println!("--- column runs to {column:.1} ---");
        let mut prev: Option<super::Line> = None;
        for line in super::page_lines(&bytes, page) {
            if let Some(p) = &prev {
                let room = column - p.right;
                let wanted = super::next_word_width(&line, p.size);
                println!(
                    "  room {:>6.1}  next word wants {:>5.1}  ends={:<6} {}",
                    room,
                    wanted,
                    room > p.size * 2.0 && room > wanted,
                    line.text.chars().take(46).collect::<String>()
                );
            }
            prev = Some(line);
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
                for sp in doc
                    .blocks
                    .iter()
                    .flat_map(|b| b.spans())
                    .filter(|s| s.italic)
                    .take(5)
                {
                    println!(
                        "    italic: {}",
                        sp.text.chars().take(60).collect::<String>()
                    );
                }
                for sp in doc
                    .blocks
                    .iter()
                    .flat_map(|b| b.spans())
                    .filter(|s| s.underline)
                    .take(6)
                {
                    println!(
                        "    underlined: {}",
                        sp.text.chars().take(60).collect::<String>()
                    );
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
                        super::Block::ListItem { marker, indent, .. } => match marker {
                            Some(_) => format!("li*>{indent}"),
                            None => format!("li>{indent}"),
                        },
                        super::Block::Rule { width, .. } => format!("hr {:.0}%", width * 100.0),
                        super::Block::Table { rows } => {
                            format!("table {}x{}", rows.len(), rows.first().map_or(0, Vec::len))
                        }
                        super::Block::Paragraph { indent, .. } => match indent {
                            0 => "p".into(),
                            n => format!("p>{n}"),
                        },
                        super::Block::IndexEntry { page, indent, .. } => {
                            format!("toc>{indent} .{page}")
                        }
                        super::Block::Anchor(_) => "anch".into(),
                        super::Block::PageBreak { ended, printed, .. } => match printed {
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
                        super::Align::Right => " [right]",
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
    /// A font is matched to the stand-in with ITS metrics, not a lookalike.
    ///
    /// The songbook's imprint page is set in Cambria and drawn a letter at a
    /// time, each glyph placed at its own x. Substituting Times for Cambria made
    /// every letter a fraction too wide, so each ran into the next and a name
    /// read as a jumble. Caladea has Cambria's widths and Carlito has Calibri's,
    /// which is why this app already ships them.
    #[test]
    fn a_font_is_matched_by_its_metrics() {
        use super::Family;
        assert_eq!(super::family_of("aaaaac+cambriamath"), Family::Cambria);
        assert_eq!(super::family_of("aaaaae+calibri-bold"), Family::Calibri);
        assert_eq!(super::family_of("aaaaak+timesnewromanpsmt"), Family::Serif);
        assert_eq!(super::family_of("arialmt"), Family::Sans);
        assert_eq!(super::family_of("helvetica-bold"), Family::Sans);
        assert_eq!(super::family_of("couriernew"), Family::Mono);
        // A name nobody knows is likelier a sans than anything else.
        assert_eq!(super::family_of("bcdeee+corporatefacename"), Family::Sans);
        // And "sans" inside a name wins over "serif" inside it.
        assert_eq!(super::family_of("dejavusansserif"), Family::Sans);
    }

    /// A dotted contents row needs nothing but its dots.
    ///
    /// The regression this exists to stop: asking every contents row for two
    /// columns of words refused the dotted ones, because the dots bridge the very
    /// gap that would have made two -- so the songbook's index stopped being an
    /// index, and its links to each song went with it.
    #[test]
    fn dots_carry_a_row_on_their_own() {
        // One group of words, no gap to speak of: the dots are the whole signal.
        assert_eq!(
            super::index_entry("Kampsange...............................3", 0.2, false),
            Some(("Kampsange", "3"))
        );
        assert_eq!(
            super::index_entry("Ode til Rohde......  62", 0.4, false),
            Some(("Ode til Rohde", "62"))
        );
        // A gap without the columns is a row of figures, not an entry.
        assert_eq!(
            super::index_entry("I alt -1.260.170 -1.016.876 197.801", 5.0, false),
            None
        );
    }

    /// A contents row whose leader is empty space, not dots.
    ///
    /// The annual report's contents page sets each entry against a page number
    /// at the right margin with nothing in between, so as text it is a sentence
    /// that ends in a digit -- which is exactly what the dot rule refuses to
    /// touch. The gap is the only place the distinction survives.
    #[test]
    fn a_contents_row_can_be_carried_by_the_gap_alone() {
        let wide = super::CONTENTS_GAP + 1.0;
        assert_eq!(
            super::index_entry("Ledelsespåtegning 4", wide, true),
            Some(("Ledelsespåtegning", "4"))
        );
        // A range of pages is a page too.
        assert_eq!(
            super::index_entry("Den uafhængige revisors erklæring 5 - 10", wide, true),
            Some(("Den uafhængige revisors erklæring", "5 - 10"))
        );
        // The abbreviation keeps its full stop: there is no leader to strip.
        assert_eq!(
            super::index_entry("Foreningsoplysninger m.v. 3", wide, true),
            Some(("Foreningsoplysninger m.v.", "3"))
        );
        // Without the gap it is a sentence that happens to end in a number.
        assert_eq!(super::index_entry("Vi var 12", 0.5, true), None);
        // And a leader still works with no gap to speak of.
        assert_eq!(
            super::index_entry("Kampsange...........3", 0.0, false),
            Some(("Kampsange", "3"))
        );
    }

    /// A rule that separated something is content; one under a word is not.
    ///
    /// The 2024 annual report draws a half-width rule over every subtotal --
    /// "Indtægter i alt", "Resultat af primær drift" -- and those rules are how
    /// the statement says which numbers add up to which. They were being read
    /// only for whether they underlined a word, and dropped when they did not.
    #[test]
    fn a_rule_that_separates_survives_and_an_underline_does_not() {
        let line = |y: f64, left: f64, right: f64| super::Line {
            y,
            size: 10.0,
            right,
            left,
            text: "x".into(),
            spans: Vec::new(),
            page: 1,
            bullet: None,
            picture: None,
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
        };
        let lines = vec![line(100.0, 50.0, 300.0)];
        let rule = |x0: f64, x1: f64, y: f64| super::Rule {
            x0,
            x1,
            y,
            thickness: 0.5,
        };

        // Under the words on the line above: that is an underline, carried by
        // the run itself.
        let under = super::separators(&[rule(50.0, 300.0, 97.0)], &lines, 500.0);
        assert!(
            under.is_empty(),
            "an underline is not a separator: {under:?}"
        );

        // Clear of any line: a separator, at the share of the width it spanned.
        let between = super::separators(&[rule(50.0, 300.0, 60.0)], &lines, 500.0);
        assert_eq!(between.len(), 1);
        assert!(
            (between[0].1 - 0.5).abs() < 0.01,
            "half the width: {between:?}"
        );

        // Too short to be anything but a tick.
        assert!(super::separators(&[rule(50.0, 70.0, 60.0)], &lines, 500.0).is_empty());

        // One rule, however many strokes drew it: a table's cells each draw
        // their own edge along the same line.
        let doubled = vec![rule(50.0, 300.0, 60.0), rule(300.0, 450.0, 60.4)];
        let joined = super::separators(&doubled, &lines, 500.0);
        assert_eq!(joined.len(), 1);
        // And as wide as the two of them together, not as wide as the first.
        assert!(
            (joined[0].1 - 0.8).abs() < 0.01,
            "the whole span: {joined:?}"
        );
    }

    /// A stack of hairlines is as heavy as the stack.
    ///
    /// Every stroke in the 2024 annual report is 0.12pt; weight is drawn by
    /// repeating one a tenth of a point lower. Sixteen of them make the bar over
    /// the masthead and four make the rules on the income statement, a fourfold
    /// difference that arrived as no difference at all while the collapse
    /// measured each stroke against the running MIDDLE of the stack instead of
    /// its top.
    #[test]
    fn a_stack_of_hairlines_weighs_what_the_stack_spans() {
        let lines: Vec<super::Line> = Vec::new();
        let stack = |n: usize, top: f64| -> Vec<super::Rule> {
            (0..n)
                .map(|i| super::Rule {
                    x0: 113.0,
                    x1: 552.0,
                    y: top - i as f64 * 0.12,
                    thickness: 0.12,
                })
                .collect()
        };

        let masthead = super::separators(&stack(16, 822.12), &lines, 440.0);
        assert_eq!(masthead.len(), 1);
        assert!(
            (masthead[0].2 - 1.92).abs() < 0.01,
            "sixteen strokes span 1.92pt: {masthead:?}"
        );

        let ordinary = super::separators(&stack(4, 784.20), &lines, 440.0);
        assert!(
            (ordinary[0].2 - 0.48).abs() < 0.01,
            "four strokes span 0.48pt: {ordinary:?}"
        );
        assert!(
            masthead[0].2 / ordinary[0].2 > 3.5,
            "and the masthead is four times the weight of an ordinary rule"
        );
    }

    /// Text at a right angle to the page is not part of the sentence.
    ///
    /// The 2024 annual report carries a signing service's document key up the
    /// edge of every page. Read as ordinary text it landed between the
    /// paragraphs: "Penneo dokumentnøgle: UMG2H-..." where the next line of the
    /// balance sheet belongs, on all 27 pages. It is separate from a slant,
    /// which is a fake italic and must still read as text.
    #[test]
    fn text_turned_on_its_side_is_not_in_the_sentence() {
        assert!(!super::turned([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), "upright");
        assert!(
            super::turned([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
            "up the page"
        );
        assert!(
            super::turned([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]),
            "down the page"
        );
        // The songbook's faked italic: sheared, not turned, and still readable.
        assert!(!super::turned([50.0, 0.0, 16.99, 50.0, 0.0, 0.0]));
        assert!(super::slanted([50.0, 0.0, 16.99, 50.0, 0.0, 0.0]));
        // A few degrees off level is a scan or a rounding, not a stamp.
        assert!(!super::turned([1.0, 0.05, 0.0, 1.0, 0.0, 0.0]));
    }

    /// A font's codes mean what the FONT says they mean.
    ///
    /// From "Kritisk revision rapport LM25" in production: two of its three
    /// fonts declare MacRomanEncoding and ship no ToUnicode, so every code above
    /// 0x7F was being read through WinAnsi. The four that matter in Danish, and
    /// what they came out as: å→Œ, ø→¿, æ→¾, and the fl ligature→ß. That is most
    /// of the words in a Danish sentence.
    #[test]
    fn a_mac_roman_font_reads_as_mac_roman() {
        let mac = super::Base::MacRoman;
        assert_eq!(mac.char_for(0x8C), Some('å'));
        assert_eq!(mac.char_for(0xBF), Some('ø'));
        assert_eq!(mac.char_for(0xBE), Some('æ'));
        assert_eq!(mac.char_for(0xAF), Some('Ø'));
        assert_eq!(mac.char_for(0xAE), Some('Æ'));
        assert_eq!(mac.char_for(0x81), Some('Å'));
        // ASCII is ASCII in both, which is why this went unnoticed for so long.
        assert_eq!(mac.char_for(b'a'), Some('a'));
        // And the same codes through the table that was being used instead.
        let win = super::Base::WinAnsi;
        assert_eq!(win.char_for(0x8C), Some('Œ'));
        assert_eq!(win.char_for(0xBF), Some('¿'));
        assert_eq!(
            win.char_for(0xE5),
            Some('å'),
            "WinAnsi has its own å elsewhere"
        );
    }

    /// A ligature is the letters it stands for, or nobody can search for them.
    #[test]
    fn a_ligature_comes_out_as_its_letters() {
        assert_eq!(super::ligature_text('ﬂ'), "fl");
        assert_eq!(super::ligature_text('ﬁ'), "fi");
        assert_eq!(super::ligature_text('ﬄ'), "ffl");
        assert_eq!(super::ligature_text('a'), "a");
        // The MacRoman code that made "flere" read as "ßere".
        assert_eq!(
            super::Base::MacRoman
                .char_for(0xDF)
                .map(super::ligature_text),
            Some("fl".to_string())
        );
    }

    /// `/Differences` re-points individual codes at named glyphs, and a number
    /// in the array moves the cursor.
    #[test]
    fn a_differences_array_names_its_glyphs() {
        use lopdf::Object;
        let items = vec![
            Object::Integer(200),
            Object::Name(b"aring".to_vec()),
            Object::Name(b"oslash".to_vec()),
            Object::Integer(65),
            Object::Name(b"fl".to_vec()),
            Object::Name(b"uni00E6".to_vec()),
        ];
        let map = super::read_differences(&items);
        assert_eq!(map.get(&200), Some(&'å'));
        assert_eq!(map.get(&201), Some(&'ø'), "the code advances by one");
        assert_eq!(map.get(&65), Some(&'ﬂ'), "a number resets it");
        assert_eq!(
            map.get(&66),
            Some(&'æ'),
            "and uniXXXX is read as its code point"
        );
        assert_eq!(super::glyph_char("notaglyphname"), None);
    }

    /// Scratch: dump what the extractor makes of a real file.
    /// `PDF=/path/to.pdf cargo test dump_a_pdf -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_a_pdf() {
        let path = std::env::var("PDF").expect("PDF=<path>");
        let bytes = std::fs::read(&path).expect("readable");
        // RAWRULES=<page> dumps that page's rules as extracted, before any of
        // them are collapsed: the stacks are visible here and nowhere else.
        if let Ok(want) = std::env::var("RAWRULES") {
            let want: usize = want.parse().unwrap_or(1);
            let doc = lopdf::Document::load_mem(&bytes).expect("loads");
            for (page_no, (_, page_id)) in doc.get_pages().into_iter().enumerate() {
                if page_no != want.saturating_sub(1) {
                    continue;
                }
                let mut budget = 0usize;
                let drawn = super::page_runs(&doc, page_id, &mut budget, &[]);
                let mut rules: Vec<&super::Rule> = drawn.rules.iter().collect();
                rules.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));
                for r in rules {
                    println!(
                        "  y={:8.2} x={:7.1}..{:7.1} thick={:.3}",
                        r.y, r.x0, r.x1, r.thickness
                    );
                }
                return;
            }
            return;
        }
        // ITEMS=<page> dumps that page's placed items: what the fixed layout
        // draws, in the order it draws them.
        if let Ok(want) = std::env::var("ITEMS") {
            let want: usize = want.parse().unwrap_or(1);
            let out = super::extract(&bytes).expect("extracts");
            if let Some(page) = out.layout.get(want.saturating_sub(1)) {
                println!("page {want}: {:.0} x {:.0}", page.width, page.height);
                for item in &page.items {
                    if let super::What::Rule = &item.what {
                        println!(
                            "  RULE x={:7.1} y={:7.1} w={:6.1} thick={:.2}",
                            item.x, item.y, item.width, item.height
                        );
                    }
                    if let super::What::Text {
                        text, size, family, ..
                    } = &item.what
                    {
                        println!(
                            "  x={:7.1} y={:7.1} w={:6.1} size={:4.1} {family:?} {text:?}",
                            item.x, item.y, item.width, size
                        );
                    }
                }
            }
            return;
        }
        // LINES=<page> dumps that page's lines with their columns, which is what
        // the table pass reads.
        if let Ok(want) = std::env::var("LINES") {
            let want: usize = want.parse().unwrap_or(1);
            let doc = lopdf::Document::load_mem(&bytes).expect("loads");
            for (n, (page_no, page_id)) in doc.get_pages().into_iter().enumerate() {
                if page_no as usize != want {
                    continue;
                }
                let _ = n;
                let mut budget = 0usize;
                let drawn = super::page_runs(&doc, page_id, &mut budget, &[]);
                for line in super::lines_from(drawn.runs) {
                    let cells: Vec<String> = line
                        .cells
                        .iter()
                        .map(|c| {
                            format!(
                                "[{:.0}..{:.0}]{}",
                                c.left,
                                c.right,
                                c.spans.iter().map(|s| s.text.as_str()).collect::<String>()
                            )
                        })
                        .collect();
                    println!(
                        "y={:.0} size={:.1} cells={} {}",
                        line.y,
                        line.size,
                        line.cells.len(),
                        cells.join(" ")
                    );
                }
                return;
            }
            return;
        }
        let out = super::extract(&bytes).expect("extracts");
        let text = |spans: &[super::Span]| spans.iter().map(|s| s.text.clone()).collect::<String>();
        println!(
            "PAGES {} (without text: {})",
            out.pages, out.pages_without_text
        );
        let first: usize = std::env::var("FROM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let count: usize = std::env::var("COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(40);
        for (i, b) in out.blocks.iter().enumerate().skip(first).take(count) {
            match b {
                super::Block::Heading { level, spans, .. } => {
                    println!("{i:>4} H{level}  {}", text(spans))
                }
                super::Block::Paragraph { spans, indent, .. } => {
                    println!("{i:>4} P{indent}   {}", text(spans))
                }
                super::Block::ListItem { spans, indent, .. } => {
                    println!("{i:>4} LI{indent}  {}", text(spans))
                }
                super::Block::IndexEntry { spans, page, .. } => {
                    println!("{i:>4} TOC  {} .... {page}", text(spans))
                }
                super::Block::Image(_) => println!("{i:>4} IMG"),
                super::Block::Rule { width, thickness } => {
                    println!("{i:>4} ---- {:.0}% {thickness:.2}pt", width * 100.0)
                }
                super::Block::Table { rows } => {
                    println!(
                        "{i:>4} TABLE {}x{}",
                        rows.len(),
                        rows.first().map_or(0, Vec::len)
                    );
                    for row in rows {
                        let cells: Vec<String> = row
                            .iter()
                            .map(|c| c.iter().map(|s| s.text.as_str()).collect())
                            .collect();
                        println!("        | {}", cells.join(" | "));
                    }
                }
                other => println!(
                    "{i:>4} {}",
                    format!("{other:?}").chars().take(60).collect::<String>()
                ),
            }
        }
    }

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
                underline: false,
                link: None,
                text: text.into(),
                color: None,
                bold: false,
                italic: false,
            }],
            bullet: None,
            picture: None,
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
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
            layout: Vec::new(),
            pages: 7,
            pages_without_text: 7,
        };
        assert!(!scan.has_text());
        let real = Extracted {
            layout: Vec::new(),
            blocks: vec![Block::Paragraph {
                indent: 0,
                spans: vec![Span {
                    underline: false,
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
            underline: false,
            link: None,
            text: text.into(),
            color: color.map(str::to_string),
            bold: false,
            italic: false,
        }
    }

    fn run(x: f64, end_x: f64, text: &str, color: Option<&str>, bold: bool) -> Run {
        Run {
            family: super::Family::Sans,
            underline: false,
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

    /// A line that stopped well short of the column ENDED, and the next starts
    /// underneath it rather than running on. Without this every entry on a
    /// programme ran into the next and a whole day arrived as one wall of text,
    /// which is what the agenda for LM 2026 looked like. It stays one block: a
    /// paragraph between every line is what put a verse's lines a paragraph
    /// apart.
    #[test]
    fn a_line_that_stops_short_ends_the_line() {
        let short = |y: f64, right: f64, text: &str| Line {
            y,
            size: 11.0,
            right,
            left: 0.0,
            page: 1,
            text: text.into(),
            spans: vec![Span {
                underline: false,
                link: None,
                text: text.into(),
                color: None,
                bold: false,
                italic: false,
            }],
            bullet: None,
            picture: None,
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
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
            vec!["16:30 Ankomst\n17:50 Åbning af landsmødet og en fortsættelse af samme punkt"],
            "the short line ends; the full-width one wraps into the next; one block"
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

    /// A bullet a symbol font drew is a bullet, not a box.
    ///
    /// Word writes the marker of a bulleted list as a character of Symbol or
    /// Wingdings mapped into the private use area. The auditor's report in the
    /// 2024 annual report carries U+F0B7 at the head of five items, and no font
    /// a reader has knows what that is.
    #[test]
    fn a_symbol_fonts_bullet_reads_as_a_bullet() {
        assert_eq!(out_of_private_use('\u{F0B7}'), Some('•'));
        assert_eq!(out_of_private_use('\u{F0A7}'), Some('▪'));
        // A letter in a symbol-encoded subset is that letter.
        assert_eq!(out_of_private_use('\u{F041}'), Some('A'));
        // Ordinary text passes through untouched.
        assert_eq!(out_of_private_use('å'), Some('å'));
        // A private glyph from some other font means nothing here, and a
        // sentence reads better without a box in it.
        assert_eq!(out_of_private_use('\u{E123}'), None);

        // And once it is a bullet, the list rule takes it as the marker.
        assert_eq!(
            list_marker("• Identificerer og vurderer vi risikoen"),
            Some("Identificerer og vurderer vi risikoen")
        );
    }

    /// A title set against the right margin stays there.
    ///
    /// The annual report sets every statement's title that way: page 13's
    /// "Resultatopgørelse" runs 429..553 in a column that runs 89..553.
    #[test]
    fn a_title_against_the_right_margin_is_right_aligned() {
        let (l, r) = (89.0, 552.7);
        assert_eq!(
            alignment_of(&[(429.0, 552.6)], l, r, 14.0),
            Align::Right,
            "flush right, clear of the left margin, and short"
        );
        // Prose that filled its line reaches the margin too, and starts at the
        // other one: ordinary text.
        assert_eq!(alignment_of(&[(89.0, 552.6)], l, r, 11.0), Align::Left);
        // A line that ends short of the margin is not against it.
        assert_eq!(alignment_of(&[(429.0, 500.0)], l, r, 11.0), Align::Left);
        // An indented block whose lines all reach the margin is indented: every
        // line starts in the same place, which right-aligned lines do not.
        let indented = [(122.0, 552.6), (122.0, 552.4), (122.0, 552.5)];
        assert_eq!(alignment_of(&indented, l, r, 11.0), Align::Left);
        // Several lines ending at the margin from different starts: right.
        let ranged = [(300.0, 552.6), (380.0, 552.4), (250.0, 552.5)];
        assert_eq!(alignment_of(&ranged, l, r, 11.0), Align::Right);
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
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
        };
        let blocks = blocks_from(
            vec![
                on_page(1, 700.0, "Sidste linje på side et"),
                on_page(2, 700.0, "Første linje på side to"),
            ],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(blocks.len(), 3, "text, break, text");
        assert_eq!(
            blocks[1],
            Block::PageBreak {
                ended: 1,
                printed: None,
                starts: None,
            },
            "the page that just ended"
        );
        assert_eq!(
            texts(&blocks),
            vec!["Sidste linje på side et", "", "Første linje på side to"],
            "the mark itself is where page two begins; it needs no anchor beside it"
        );
    }

    /// An address broken for the margin's sake is still one address.
    #[test]
    fn a_wrapped_address_joins_without_a_space() {
        // The samværspolitik breaks its alkoholpolitik link twice.
        assert!(continues_a_url(
            "Du kan læse Radikal Ungdoms alkoholpolitik her: https://",
            "www.radikalungdom.dk/wp-content/uploads/2022/03/Alkoholpolitik-i-Radikal-"
        ));
        assert!(continues_a_url(
            "www.radikalungdom.dk/wp-content/uploads/2022/03/Alkoholpolitik-i-Radikal-",
            "Ungdom_MARTS2022.pdf"
        ));
        // Ordinary prose keeps its space.
        assert!(!continues_a_url(
            "Radikal Ungdom har både en samværs- og",
            "alkoholpolitik. De sætter rammerne"
        ));
        // And a sentence that merely follows an address is not more of it.
        assert!(!continues_a_url(
            "Se https://example.dk",
            ", som beskriver det"
        ));
    }

    /// A file often prints its links instead of making them.
    #[test]
    fn a_written_address_becomes_a_link() {
        let found = urls_in(
            "Samværspolitikken kan læses her: https://acrobat.adobe.com/id/urn:aaid:sc:EU:53f0eb63",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].2,
            "https://acrobat.adobe.com/id/urn:aaid:sc:EU:53f0eb63"
        );
        // A bare host gets the scheme it needs to be followable.
        assert_eq!(
            urls_in("se www.radikalungdom.dk for mere")[0].2,
            "https://www.radikalungdom.dk"
        );
        // The sentence's punctuation stays with the sentence.
        assert_eq!(
            urls_in("Se https://example.dk/side.")[0].2,
            "https://example.dk/side"
        );
        // And what is not an address.
        assert!(urls_in("noget http:// andet").is_empty(), "a scheme alone");
        assert!(urls_in("wwwwww.dk").is_empty());
    }

    /// A fill is an area. Reading its edges as lines is what put an underline
    /// under every line of the songbook's lyrics.
    #[test]
    fn a_filled_box_is_not_an_underline() {
        // The songbook's white box behind a line, as a polygon: 18.7pt tall.
        let box_path = vec![
            Shape::Line {
                from: (55.7, 672.9),
                to: (540.5, 672.9),
            },
            Shape::Line {
                from: (540.5, 672.9),
                to: (540.5, 654.2),
            },
            Shape::Line {
                from: (540.5, 654.2),
                to: (55.7, 654.2),
            },
        ];
        assert!(
            filled_rule(&box_path, IDENTITY).is_empty(),
            "a box the height of a line is a box"
        );
        // A hairline the same width IS a rule.
        let hairline = vec![Shape::Box {
            x: 55.7,
            y: 654.2,
            w: 484.8,
            h: 0.8,
        }];
        let found = filled_rule(&hairline, IDENTITY);
        let rule = found.first().expect("a hairline is a rule");
        assert!((rule.thickness - 0.8).abs() < 0.01);
        assert!((rule.y - 654.6).abs() < 0.01);

        // And a path holding a whole table's worth of bars gives one rule per
        // bar, at the weight each was drawn: taking the path as a whole makes a
        // box the height of the table and throws all of them away.
        let many = vec![
            Shape::Box {
                x: 55.0,
                y: 700.0,
                w: 480.0,
                h: 1.2,
            },
            Shape::Box {
                x: 55.0,
                y: 600.0,
                w: 480.0,
                h: 2.4,
            },
            Shape::Box {
                x: 55.0,
                y: 500.0,
                w: 480.0,
                h: 1.2,
            },
        ];
        let bars = filled_rule(&many, IDENTITY);
        assert_eq!(bars.len(), 3, "one rule per bar");
        let weights: Vec<f64> = bars
            .iter()
            .map(|r| (r.thickness * 10.0).round() / 10.0)
            .collect();
        assert_eq!(weights, vec![1.2, 2.4, 1.2]);
    }

    /// And white ink marks nothing at all.
    #[test]
    fn white_is_not_ink() {
        assert!(too_pale(1.0, 1.0, 1.0), "the songbook's boxes");
        assert!(!too_pale(0.0, 0.0, 0.0));
        assert!(!too_pale(0.18, 0.26, 0.47), "a dark blue still marks");
    }

    /// An address is found by looking either side of the @.
    #[test]
    fn an_address_is_found_in_a_line_of_prose() {
        let line = "Marie Strunge Thorup, msthorup@gmail.com, 23 60 66 61.";
        let found = emails_in(line);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].2, "mailto:msthorup@gmail.com",
            "the address, and not the comma after it"
        );
        // A sentence's full stop is the sentence's.
        assert_eq!(
            emails_in("Skriv til anja@example.dk.")[0].2,
            "mailto:anja@example.dk"
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
            underline: false,
            text: "Marie Strunge Thorup, msthorup@gmail.com, 23 60 66 61".into(),
            color: None,
            bold: false,
            italic: false,
            link: stale,
        }];
        let mended = mend_written_links(spans);
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
            underline: false,
            text: "skriv til formanden".into(),
            color: None,
            bold: false,
            italic: false,
            link: link.clone(),
        }];
        assert_eq!(mend_written_links(spans.clone()), spans);
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
            index_entry("Kampsange....................3", 0.0, false),
            Some(("Kampsange", "3"))
        );
        // The file may leave a space before the leader, or none before the
        // number, and both are the same row.
        assert_eq!(
            index_entry(
                "Radikal Ungdoms holdning til rigsfællesskabet .........19",
                0.0,
                false
            ),
            Some(("Radikal Ungdoms holdning til rigsfællesskabet", "19"))
        );
        assert_eq!(
            index_entry(
                "Internationale................................9",
                0.0,
                false
            ),
            Some(("Internationale", "9"))
        );
    }

    /// And a sentence that merely ends in a digit is left alone.
    #[test]
    fn prose_is_not_a_contents_row() {
        assert_eq!(index_entry("Vi mødes i 2025", 0.0, false), None);
        assert_eq!(
            index_entry("Der var engang… 7", 0.0, false),
            None,
            "one ellipsis is not a leader"
        );
        assert_eq!(index_entry("1. maj 2025", 0.0, false), None);
        assert_eq!(
            index_entry("Kampsange....", 0.0, false),
            None,
            "no page, no row"
        );
        assert_eq!(index_entry("....7", 0.0, false), None, "no title, no row");
    }

    /// The margin is near the end of the range, not a tenth down it: a book of
    /// verse has no line anywhere near its margin.
    #[test]
    fn the_margin_is_where_the_longest_lines_reach() {
        let ends = |right: f64| Line {
            y: 700.0,
            size: 14.0,
            right,
            left: 56.7,
            page: 1,
            text: "noget".into(),
            spans: vec![span("noget", None)],
            bullet: None,
            picture: None,
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
        };
        // The songbook: a hundred short verse lines, and a contents list whose
        // rows all run to the margin. A tenth down the range gave 392 and the
        // margin is at 538, so its longer lines measured as running PAST the
        // column and Ode an die Freude arrived as one line.
        let mut lines: Vec<Line> = (0..100).map(|i| ends(150.0 + i as f64)).collect();
        lines.extend((0..10).map(|_| ends(538.6)));
        assert!(
            (column_right(&lines) - 538.6).abs() < 0.01,
            "the margin the contents rows reach"
        );
        // But one line drawn out past it is still a stray.
        let mut stray = vec![ends(700.0)];
        stray.extend((0..99).map(|_| ends(400.0)));
        assert!(
            (column_right(&stray) - 400.0).abs() < 0.01,
            "one line is not a margin"
        );
    }

    /// A verse is lines, not paragraphs. Turning each into a paragraph put a
    /// paragraph's air between every line of Lokalforeningssangen.
    #[test]
    fn a_verse_is_one_block_of_lines() {
        let verse = |y: f64, right: f64, text: &str| Line {
            y,
            size: 14.0,
            right,
            left: 56.7,
            page: 1,
            text: text.into(),
            spans: vec![span(text, None)],
            bullet: None,
            picture: None,
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
        };
        // Single-spaced at 18.8, as the songbook sets them, then a blank line
        // before the next stanza. The prose line at the top is what sets the
        // column: without something reaching the margin, nothing can be short of
        // it.
        let blocks = blocks_from(
            vec![
                verse(
                    700.0,
                    540.0,
                    "En linje der løber helt ud til margenen og fylder den",
                ),
                verse(
                    681.2,
                    536.0,
                    "og endnu en, for margenen er hvor flere linjer siger den er",
                ),
                verse(413.1, 168.9, "Vi er unge radikale"),
                verse(394.3, 187.3, "og vi står i samlet flok"),
                verse(375.5, 200.0, "vi vil kæmpe for det gode"),
                verse(337.9, 150.0, "ÆRU:"),
                verse(319.1, 210.0, "Nu’ det ÆRU, der synger,"),
            ],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(
            texts(&blocks),
            vec![
                "En linje der løber helt ud til margenen og fylder den og endnu en, for margenen er hvor flere linjer siger den er",
                "Vi er unge radikale\nog vi står i samlet flok\nvi vil kæmpe for det gode",
                "ÆRU:\nNu’ det ÆRU, der synger,",
            ],
            "one block per stanza, its lines kept"
        );
    }

    /// An anchor nobody points at is furniture in the middle of a list, and it
    /// pushed the rows around it apart.
    #[test]
    fn an_anchor_nobody_points_at_is_taken_out() {
        let row = |text: &str, to: Option<&str>| Block::IndexEntry {
            spans: vec![Span {
                text: text.into(),
                color: None,
                bold: false,
                italic: false,
                underline: false,
                link: to.map(|id| Link::Place(id.into())),
            }],
            page: "3".into(),
            indent: 0,
        };
        let mut blocks = vec![
            row("Kampsange", Some("pdf-t0")),
            Block::Anchor("pdf-d7".into()),
            row("Lokalforeningssangen", Some("pdf-t1")),
            Block::Anchor("pdf-t0".into()),
        ];
        drop_unused_anchors(&mut blocks);
        assert_eq!(blocks.len(), 3, "the stale one goes, the used one stays");
        assert_eq!(blocks[2], Block::Anchor("pdf-t0".into()));
    }

    /// A file with no italic face fakes one by shearing the matrix, and then
    /// nothing in the font says italic at all.
    #[test]
    fn a_sheared_matrix_is_an_italic() {
        // The songbook's own: "Æresmedlem af Radikal Ungdom", nineteen degrees.
        assert!(slanted([50.0, 0.0, 16.992, 50.0, 186.0, -2144.0]));
        // Upright, at any size.
        assert!(!slanted([50.0, 0.0, 0.0, 50.0, 0.0, 0.0]));
        assert!(!slanted([11.0, 0.0, 0.0, 11.0, 56.0, 700.0]));
        // Turned on its side is not italic for being sideways: a watermark down
        // the margin is upright text that has been rotated.
        assert!(!slanted([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]));
        // And rotated AND sheared still reads as sheared.
        assert!(slanted([0.0, 1.0, -1.0, 0.34, 0.0, 0.0]));
    }

    /// A line stops short for two different reasons, and only one of them ends
    /// the paragraph.
    #[test]
    fn a_line_that_ran_out_of_room_did_not_end() {
        let line = |text: &str, left: f64, right: f64| Line {
            y: 700.0,
            size: 11.0,
            right,
            left,
            page: 1,
            text: text.into(),
            spans: vec![span(text, None)],
            bullet: None,
            picture: None,
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
        };
        // The alkoholpolitik: the line stops sixty points short because
        // "internationale" is what comes next, and it does not fit.
        let wrapped = line(
            "internationale studieture. Når man er ædruvagt, skal man holde sig ædru",
            110.7,
            533.1,
        );
        assert!(
            next_word_width(&wrapped, 11.0) > 60.0,
            "the word wanted more room than was left"
        );
        // A programme: the next entry opens with a time, which would have fitted
        // on the line before it several times over.
        let entry = line("10.30 Oplæg ved gæstetaler", 110.7, 300.0);
        assert!(
            next_word_width(&entry, 11.0) < 60.0,
            "a short word would have fitted, so the line before it ended on purpose"
        );
    }

    /// A signature is written in two words and painted as two paths, and the
    /// page shows one signature.
    #[test]
    fn two_halves_of_a_signature_are_one_drawing() {
        let art = |x0: f64, x1: f64| Art {
            strokes: Vec::new(),
            x0,
            y0: 100.0,
            x1,
            y1: 170.0,
        };
        // "Malika" and "Rosenskjold", side by side with a word's gap.
        assert!(art(56.0, 145.0).joins(&art(160.0, 313.0)));
        // Something on the other side of the page is not part of it.
        assert!(!art(56.0, 145.0).joins(&art(400.0, 500.0)));
        // Nor is something on another line.
        let below = Art {
            strokes: Vec::new(),
            x0: 160.0,
            y0: 10.0,
            x1: 313.0,
            y1: 80.0,
        };
        assert!(!art(56.0, 145.0).joins(&below));
    }

    /// The margin is where the FEWEST lines may sit: a document whose body is
    /// one step in still has its margin where its title is.
    #[test]
    fn the_margin_is_not_wherever_most_lines_start() {
        let at = |left: f64| Line {
            y: 700.0,
            size: 11.0,
            right: 500.0,
            left,
            page: 1,
            text: "noget".into(),
            spans: vec![span("noget", None)],
            bullet: None,
            picture: None,
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
        };
        // The alkoholpolitik: a title and three lines at the margin, and forty
        // in a list one step in. A tenth percentile lands on the LIST, which
        // flattened every depth in the file to nothing and put the bullets out
        // past the title.
        let mut lines: Vec<Line> = (0..4).map(|_| at(56.6)).collect();
        lines.extend((0..40).map(|_| at(110.7)));
        assert!(
            (column_left(&lines) - 56.6).abs() < 0.01,
            "the margin, not the body"
        );
        // And one line hanging out into the margin does not move it.
        let mut stray = vec![at(12.0)];
        stray.extend((0..20).map(|_| at(56.6)));
        assert!(
            (column_left(&stray) - 56.6).abs() < 0.01,
            "one line is not a margin"
        );
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
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
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
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
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
            rule: None,
            tail_gap: 0.0,
            cells: Vec::new(),
            natural_cells: 0,
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

//! A Word document, rendered by this app rather than by somebody else's server.
//!
//! The third option beside Microsoft's and Google's viewers. Those two work by
//! sending the document to a third party and embedding what comes back; this one
//! parses the file in the browser and renders it as ordinary elements.
//!
//! It buys three things the embedded viewers cannot: the document reaches
//! nobody, the text is selectable and findable with the browser's own search,
//! and it reflows on a phone instead of being a fixed page in a scrolling box.
//!
//! It costs fidelity. This is not a pagination engine: there are no page breaks,
//! no margins, no line-breaking to Word's rules. A heading is an `<h2>`, a
//! paragraph is a `<p>`, a table is a `<table>`. For minutes and agendas — which
//! is what this wiki holds — that reads better than the real thing. For a
//! carefully laid-out document it will not, and the other two options are still
//! there.
//!
//! Parsing is `docx-parser` (MIT, Yuki Yokotani, github.com/yukiyokotani/
//! office-open-xml-viewer), which hands back the document model as JSON. Only
//! the parts of that model this renders are deserialised; the model carries far
//! more (typography acquisition, font slots, tab stops) than a DOM rendering can
//! use.

use dioxus::prelude::*;
use serde::Deserialize;
use wasm_bindgen::JsCast;

/// One block in the document body.
///
/// The parser tags these with `type`, and the two that matter are paragraphs and
/// tables. Anything else is skipped rather than guessed at — a block this does
/// not understand is better absent than rendered wrong.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type")]
pub enum Block {
    #[serde(rename = "paragraph")]
    Paragraph(Paragraph),
    #[serde(rename = "table")]
    Table(Table),
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    #[serde(default)]
    pub runs: Vec<Run>,
    /// `Heading1`, `Title`, … when the document uses styles.
    #[serde(default)]
    pub style_id: Option<String>,
    /// 0-8 for headings 1-9 in OOXML; 9 (or absent) is body text.
    #[serde(default)]
    pub outline_level: Option<i64>,
    #[serde(default)]
    pub alignment: Option<String>,
    /// Boxed because it is by far the largest thing a paragraph carries, and a
    /// paragraph is one variant of [`Block`]: unboxed it made that enum as big
    /// as its biggest member for every block in a document, tables included.
    #[serde(default)]
    pub numbering: Option<Box<Numbering>>,
    #[serde(default)]
    pub indent_left: Option<f64>,
    /// The FIRST line's extra indent, separate from `indent_left` and usually
    /// negative: a hanging indent. See [`paragraph_style`].
    #[serde(default)]
    pub indent_first: Option<f64>,
    /// Space above and below the paragraph, in points, as the document asks for
    /// it. Often zero: a document that separates its paragraphs with blank ones
    /// wants no space at all, and adding some doubles every gap it has.
    #[serde(default)]
    pub space_before: Option<f64>,
    #[serde(default)]
    pub space_after: Option<f64>,
    #[serde(default)]
    pub line_spacing: Option<LineSpacing>,
    /// The size this paragraph's text ends up at, in points, resolved by the
    /// parser through the style chain AND the document defaults. Read for the
    /// pagination, which needs the size Word would actually set: a document can
    /// state a size on almost no run at all -- one here states one on five runs
    /// of a hundred and eighty, and those five are its headings -- so taking
    /// the commonest size the RUNS state measures the whole document at its
    /// heading size.
    #[serde(default)]
    pub default_font_size: Option<f64>,
    /// The typeface Word resolves for this paragraph, through the style chain
    /// and the theme. Read for the measuring, which has to lay the text out in
    /// the face the document sets: one face for every document was tried, and a
    /// Times New Roman document measured in Calibri's metrics came out a tenth
    /// too tall, which is a whole page.
    #[serde(default)]
    pub default_font_family: Option<String>,
    /// Word suppresses the space between adjacent paragraphs of the same style
    /// when this is set, which is how a bulleted list closes up. Resolved
    /// through the style chain. Without it a list measures 8pt per item taller
    /// than Word lays it out.
    #[serde(default)]
    pub contextual_spacing: bool,
    /// Word keeps this paragraph on the same page as the one after it, which
    /// every heading style does. A heading is exactly what a page tends to
    /// break after, and Word moves it down with the text it introduces rather
    /// than leaving it stranded at the foot of a page.
    #[serde(default)]
    pub keep_next: bool,
    /// How big this heading is against the document's own body text, as a
    /// multiplier. Not from the document: computed by [`scale_headings`] after
    /// parsing, because it takes the whole document to know what "body text"
    /// means here.
    #[serde(skip)]
    pub heading_scale: Option<f64>,
}

/// A paragraph's line spacing.
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LineSpacing {
    /// A multiplier when `rule` is `auto`; a measurement in points otherwise.
    #[serde(default)]
    pub value: f64,
    /// `auto`, `exact` or `atLeast`.
    #[serde(default)]
    pub rule: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Numbering {
    /// `bullet` or a number format (`decimal`, `lowerLetter`, …).
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub level: Option<i64>,

    /// A list whose bullet is a picture rather than a character. Word calls it
    /// `numPicBullet`; the browser's own disc is not it.
    #[serde(default)]
    pub pic_bullet_image_path: Option<String>,
    #[serde(default)]
    pub pic_bullet_mime_type: Option<String>,
    #[serde(default)]
    pub pic_bullet_width_pt: Option<f64>,
    #[serde(default)]
    pub pic_bullet_height_pt: Option<f64>,
    /// Filled in by [`attach_images`], like [`Run::src`].
    #[serde(skip)]
    pub src: Option<std::rc::Rc<str>>,
}

impl Numbering {
    /// The picture this list uses for its bullet, and its format.
    pub fn picture(&self) -> Option<(&str, &str)> {
        let path = self
            .pic_bullet_image_path
            .as_deref()
            .filter(|p| !p.is_empty())?;
        Some((path, self.pic_bullet_mime_type.as_deref().unwrap_or("")))
    }

    /// The SHAPE of that bullet. Not its size.
    ///
    /// The numbering definition carries a size, and it is a page-layout number
    /// from whatever template the document came out of: the one in this wiki
    /// asks for an 18pt bullet beside 11pt text, twice the size of the very
    /// same image used as a bullet elsewhere in the same document. Drawing that
    /// literally is what Word does and what a paginating renderer should do.
    /// This renderer is not one, and says so: it favours a document that reads
    /// over a document that is reproduced.
    ///
    /// So the stylesheet sets the height, in `em`, and a bullet tracks the text
    /// it leads and the reader's own font size. All that is needed from the
    /// document is the aspect ratio, so the picture is not squashed.
    pub fn bullet_style(&self) -> String {
        match (
            self.pic_bullet_width_pt.filter(|w| *w > 0.0),
            self.pic_bullet_height_pt.filter(|h| *h > 0.0),
        ) {
            (Some(w), Some(h)) => format!("aspect-ratio:{w:.2}/{h:.2};"),
            _ => String::new(),
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// What the parser tagged this run as. Only `break` matters here: OOXML
    /// writes a line break as a run of its OWN (`<w:br/>`), carrying no text, so
    /// without reading this it arrived as an empty run and vanished. That is
    /// what dropped the newline after a bold lead-in line.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    /// Kept as a loose value because the shape varies: this parser emits a
    /// bool, OOXML itself carries a style name (`single`, `dotted`, `none`), and
    /// some producers write an object. [`is_underlined`] is what asks.
    #[serde(default)]
    pub underline: Option<serde_json::Value>,
    #[serde(default)]
    pub strikethrough: bool,
    /// `RRGGBB`, or `auto` — which means "whatever the reader's theme says", so
    /// it must NOT be turned into a colour.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub vert_align: Option<String>,
    #[serde(default)]
    pub hyperlink: Option<String>,
    /// Points, as the document resolved them through its styles.
    #[serde(default)]
    pub font_size: Option<f64>,

    // --- pictures ---
    /// Zip path of the raster the run draws, `word/media/image1.png`. A run
    /// either has this or has text; the parser writes one node per picture.
    #[serde(default)]
    pub image_path: Option<String>,
    /// The vector original, when Word kept one beside the raster fallback.
    /// Preferred: it scales to the reader's screen instead of blurring.
    #[serde(default)]
    pub svg_image_path: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    /// The size Word gives the picture, in points. Points are a CSS unit too,
    /// and the same one, so these carry across untouched.
    #[serde(default)]
    pub width_pt: Option<f64>,
    #[serde(default)]
    pub height_pt: Option<f64>,
    #[serde(default)]
    pub rotation: Option<f64>,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,

    /// The picture itself, as a `data:` url, filled in by [`attach_images`]
    /// after parsing. Not from the document: the model carries a path into the
    /// zip, and the bytes have to be fetched out of it separately.
    ///
    /// `Rc` because one picture is often drawn many times — a bullet glyph, a
    /// logo in a header — and a megabyte of base64 should be stored once
    /// however many runs point at it.
    #[serde(skip)]
    pub src: Option<std::rc::Rc<str>>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    #[serde(default)]
    pub rows: Vec<Row>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    #[serde(default)]
    pub cells: Vec<Cell>,
    #[serde(default)]
    pub is_header: bool,
    /// `w:cantSplit`: the document forbids Word to break this row across two
    /// pages. Word splits a row by default, and where it splits one the page
    /// below it is full rather than ending early.
    #[serde(default)]
    pub cant_split: bool,
    /// `w:trHeight`, in points: a height the document asks this row to have,
    /// which is usually the trace of someone dragging its boundary in Word. It
    /// is a MINIMUM here whatever the rule says -- a row is never made shorter
    /// than its text, and clipping the text is not something a reader that
    /// reflows should do.
    #[serde(default)]
    pub row_height: Option<f64>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    /// A cell holds blocks, not text: a table cell can contain paragraphs and
    /// further tables, so rendering recurses here.
    #[serde(default)]
    pub content: Vec<Block>,
    #[serde(default = "one")]
    pub col_span: u32,
    /// `continue` on a cell that is covered by a vertical merge from above; such
    /// a cell is not drawn at all, or the row grows an extra column.
    #[serde(default)]
    pub v_merge: Option<String>,
    /// The cell's shading, as `RRGGBB`. A table's header row is usually the only
    /// thing telling a reader it is a header, so this is not decoration.
    #[serde(default)]
    pub background: Option<String>,
    /// The width Word prefers for this column, in points. NOT used to lay the
    /// measuring copy out: forcing the columns to these widths was tried and
    /// measured, and it made every document worse -- one table's columns come
    /// to 670px against a 642px text column, so pinning them squeezes every row
    /// taller and an eight-page document measured ten. Kept because the field
    /// is what the document says; whatever uses it will have to reconcile that
    /// overflow the way Word does.
    #[serde(default)]
    pub width_pt: Option<f64>,
}

/// A cell's shading, as CSS.
///
/// `auto` means "whatever the reader's theme says", exactly as it does on a
/// run's colour, so it must NOT become a colour — a document that says `auto`
/// on a dark theme wants the dark background, not a white one painted over it.
pub fn cell_style(cell: &Cell) -> String {
    cell_colour(cell)
}

/// What a cell is painted, if the document paints it.
fn cell_colour(cell: &Cell) -> String {
    match cell.background.as_deref() {
        Some(hex) if is_real_colour(hex) => {
            let hex = hex.trim().trim_start_matches('#');
            // Ink to match. A document that paints a header dark navy expects
            // light text on it, and the theme's own on-surface colour is not
            // that: the background is the DOCUMENT'S, not the theme's, so what
            // reads against it has to be worked out from it. A run carrying its
            // own colour still wins — this only sets what the cell inherits.
            format!("background:#{hex};color:{};", readable_ink(hex))
        }
        _ => String::new(),
    }
}

/// Black or white, whichever can be read on `hex`.
///
/// Relative luminance, as WCAG defines it: the eye is far more sensitive to
/// green than to blue, so a flat average of the channels calls a saturated blue
/// light when it is not.
pub fn readable_ink(hex: &str) -> &'static str {
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
    let linear = |c: f64| match c <= 0.03928 {
        true => c / 12.92,
        false => ((c + 0.055) / 1.055).powf(2.4),
    };
    if hex.len() < 6 {
        return "#000";
    }
    let l = 0.2126 * linear(channel(0)) + 0.7152 * linear(channel(2)) + 0.0722 * linear(channel(4));
    // 0.179 is where white and black give the same contrast ratio against a
    // background, so it is the crossover.
    match l > 0.179 {
        true => "#000",
        false => "#fff",
    }
}

/// Whether a colour from the document is an actual colour.
pub fn is_real_colour(hex: &str) -> bool {
    let hex = hex.trim().trim_start_matches('#');
    !hex.eq_ignore_ascii_case("auto")
        && matches!(hex.len(), 6 | 8)
        && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn one() -> u32 {
    1
}

/// Pictures the document embeds, keyed by their path inside the package.
pub type Images = std::collections::HashMap<String, std::rc::Rc<str>>;

/// Formats a browser will draw.
///
/// Word embeds whatever it was given, and two of the things it is given
/// regularly — EMF and WMF, Windows' own vector formats — no browser has ever
/// displayed. Those are left for [`render_gaps`](super::render_gaps) to report
/// rather than turned into a broken image icon. Every picture in this wiki's
/// documents is a PNG or a JPEG, but that is a fact about today's documents.
pub fn is_drawable(mime: &str) -> bool {
    matches!(
        mime.trim().to_ascii_lowercase().as_str(),
        "image/png"
            | "image/jpeg"
            | "image/jpg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "image/bmp"
            | "image/svg+xml"
    )
}

/// Which picture a run draws, and in what format: the vector original when Word
/// kept one, otherwise the raster.
///
/// The SVG sibling has no `mimeType` of its own in the model — the field
/// describes the raster fallback — so it is named here.
pub fn picture_of(run: &Run) -> Option<(&str, &str)> {
    if let Some(svg) = run.svg_image_path.as_deref().filter(|p| !p.is_empty()) {
        return Some((svg, "image/svg+xml"));
    }
    let path = run.image_path.as_deref().filter(|p| !p.is_empty())?;
    Some((path, run.mime_type.as_deref().unwrap_or("")))
}

/// Read every picture the document draws out of the package, once each.
///
/// Deduplicated on the way in: a bullet glyph is one file referenced from every
/// list item, and a document here draws the same two 1 KB JPEGs fifteen times.
/// Undrawable formats are skipped rather than embedded as bytes no browser can
/// decode; so is anything the package turns out not to contain.
/// The metafile pictures a document draws, for the backend to render.
pub fn collect_metafiles(blocks: &[Block]) -> Vec<(String, String)> {
    let mut wanted: Vec<(String, String)> = Vec::new();
    let mut want = |picture: Option<(&str, &str)>| {
        if let Some((path, mime)) = picture {
            if is_metafile(mime) && !wanted.iter().any(|(p, _)| p == path) {
                wanted.push((path.to_string(), mime.to_string()));
            }
        }
    };
    for_each_paragraph(blocks, &mut |p| {
        want(p.numbering.as_ref().and_then(|n| n.picture()));
        p.runs.iter().for_each(|run| want(picture_of(run)));
    });
    wanted
}

pub fn collect_images(blocks: &[Block], package: &[u8]) -> Images {
    let mut wanted: Vec<(String, String)> = Vec::new();
    let mut want = |picture: Option<(&str, &str)>| {
        if let Some((path, mime)) = picture {
            if is_drawable(mime) && !wanted.iter().any(|(p, _)| p == path) {
                wanted.push((path.to_string(), mime.to_string()));
            }
        }
    };
    for_each_paragraph(blocks, &mut |p| {
        // A list's bullet is a picture too, and it is named on the paragraph
        // rather than in its runs.
        want(p.numbering.as_ref().and_then(|n| n.picture()));
        p.runs.iter().for_each(|run| want(picture_of(run)));
    });
    read_images(wanted, package)
}

/// Read a list of already-deduplicated pictures out of a package.
///
/// Shared with the slide renderer, which finds its pictures differently but
/// stores them the same way — a zip path and a mime, out of an OOXML package.
pub fn read_images(wanted: Vec<(String, String)>, package: &[u8]) -> Images {
    if wanted.is_empty() {
        return Images::new();
    }
    // One archive for all of them: opening the zip per picture would re-scan the
    // central directory each time.
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(package)) else {
        return Images::new();
    };
    let mut out = Images::new();
    for (path, mime) in wanted {
        use std::io::Read;
        let mut bytes = Vec::new();
        let read = zip
            .by_name(&path)
            .map(|mut entry| entry.read_to_end(&mut bytes))
            .is_ok();
        if read && !bytes.is_empty() {
            out.insert(path, data_url(&mime, &bytes).into());
        }
    }
    out
}

/// Windows' own vector formats, which Word and PowerPoint keep pasted figures
/// in and which no browser draws.
///
/// Not [`is_drawable`], because nothing here can draw them: they go to the
/// backend, which renders them to PNG (see [`render_metafiles`]).
pub fn is_metafile(mime: &str) -> bool {
    matches!(
        mime.trim().to_ascii_lowercase().as_str(),
        "image/x-emf" | "image/emf" | "image/x-wmf" | "image/wmf" | "application/x-msmetafile"
    )
}

/// Every metafile picture a document draws, rendered to PNG by the backend.
///
/// One request each, and a failure is simply left out: the viewer already draws
/// a labelled placeholder in the shape's box for a picture it has no pixels
/// for, which is the right answer for a figure this cannot render either.
pub async fn render_metafiles(
    wanted: Vec<(String, String)>,
    package: &[u8],
    token: Option<&str>,
) -> Images {
    let mut out = Images::new();
    if wanted.is_empty() {
        return out;
    }
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(package)) else {
        return out;
    };
    for (path, _mime) in wanted {
        use std::io::Read;
        let mut bytes = Vec::new();
        let read = zip
            .by_name(&path)
            .map(|mut entry| entry.read_to_end(&mut bytes))
            .is_ok();
        if !read || bytes.is_empty() {
            continue;
        }
        match crate::backend_api::render_metafile(&bytes, token.unwrap_or_default()).await {
            Ok((drawn, mime)) => {
                out.insert(path, data_url(&mime, &drawn).into());
            }
            Err(e) => log::info!("metafile {path} not rendered: {e}"),
        }
    }
    out
}

/// A `data:` url for one picture.
fn data_url(mime: &str, bytes: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The size of a paragraph's text: whichever size most of its characters are
/// set in. A paragraph is usually all one size, and where it is not, the run
/// carrying the words is the one that matters, not a stray space.
fn dominant_size(p: &Paragraph) -> Option<f64> {
    let mut weights: Vec<(f64, usize)> = Vec::new();
    for run in &p.runs {
        let Some(pt) = run.font_size.filter(|s| *s > 0.0) else {
            continue;
        };
        let chars = run.text.trim().chars().count();
        if chars == 0 {
            continue;
        }
        match weights.iter_mut().find(|(s, _)| (*s - pt).abs() < 0.01) {
            Some((_, n)) => *n += chars,
            None => weights.push((pt, chars)),
        }
    }
    weights.into_iter().max_by_key(|(_, n)| *n).map(|(s, _)| s)
}

/// Work out how prominent each heading is, against the document's own body text.
///
/// A Word heading keeps its level and its size separately, and the two do not
/// track each other. The document this was written for has a 20pt `Titel` and
/// an 11pt `Overskrift1` — both of which are outline level 0, so both become an
/// `<h1>`, and rendering them at the `<h1>` size of the app's type scale makes
/// them identical and loses the hierarchy the author wrote. The 11pt one also
/// arrives three times the size it has in the document.
///
/// So a heading is sized RELATIVE to the document's body text rather than
/// absolutely: `20/11` is a big heading and `11/11` is a heading that is bold
/// and no larger, which is exactly what each of those is in Word. Relative
/// rather than absolute so the reader's own font size still sets the scale — an
/// 8pt document should not arrive at 8pt on a phone.
///
/// Body text is the size most of the document's characters are set in, which is
/// what "body text" means. Headings are excluded from that count, or a document
/// of mostly headings would measure itself against its own headings.
///
/// Clamped below at 1: a heading is never SMALLER than the text it introduces,
/// whatever the document says, because the tag has already promised a reader
/// that it is a heading.
pub fn scale_headings(blocks: &mut [Block]) {
    let mut sizes: Vec<(f64, usize)> = Vec::new();
    for_each_paragraph(blocks, &mut |p| {
        if heading_level(p.style_id.as_deref(), p.outline_level).is_some() {
            return;
        }
        let Some(pt) = dominant_size(p) else { return };
        let chars = p.runs.iter().map(|r| r.text.trim().chars().count()).sum();
        match sizes.iter_mut().find(|(s, _)| (*s - pt).abs() < 0.01) {
            Some((_, n)) => *n += chars,
            None => sizes.push((pt, chars)),
        }
    });
    let Some(body) = sizes
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(s, _)| s)
        .filter(|s| *s > 0.0)
    else {
        return;
    };

    for_each_paragraph_mut(blocks, &mut |p| {
        if heading_level(p.style_id.as_deref(), p.outline_level).is_none() {
            return;
        }
        p.heading_scale = dominant_size(p).map(|pt| (pt / body).clamp(1.0, 2.5));
    });
}

/// Give every picture run the bytes it draws.
///
/// Separate from parsing because the two come from different places: the model
/// carries a path into the package, and the package is the bytes that were
/// parsed. A run whose picture is missing or undrawable keeps `src: None`, which
/// renders as nothing at all rather than as a broken image.
pub fn attach_images(blocks: &mut [Block], images: &Images) {
    for_each_paragraph_mut(blocks, &mut |p| {
        if let Some(n) = p.numbering.as_mut() {
            if let Some(path) = n.pic_bullet_image_path.clone() {
                n.src = images.get(&path).cloned();
            }
        }
        for run in &mut p.runs {
            if let Some((path, _)) = picture_of(run) {
                run.src = images.get(path).cloned();
            }
        }
    });
}

/// Every paragraph in the tree, table cells included.
fn for_each_paragraph(blocks: &[Block], f: &mut impl FnMut(&Paragraph)) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => f(p),
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        for_each_paragraph(&cell.content, f);
                    }
                }
            }
            Block::Unknown => {}
        }
    }
}

fn for_each_paragraph_mut(blocks: &mut [Block], f: &mut impl FnMut(&mut Paragraph)) {
    for block in blocks {
        match block {
            Block::Paragraph(p) => f(p),
            Block::Table(t) => {
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        for_each_paragraph_mut(&mut cell.content, f);
                    }
                }
            }
            Block::Unknown => {}
        }
    }
}

/// How to draw one picture.
///
/// Word's size is in points, which is a CSS unit and the same one, so it
/// carries across exactly. Two things are added:
///
/// * `max-width: 100%` and a matching `aspect-ratio`, so a picture wider than a
///   phone shrinks instead of pushing the document sideways — and shrinks in
///   proportion, which setting both a width and a height in points would not.
/// * the rotation and mirroring Word recorded, as one transform.
pub fn image_style(run: &Run) -> String {
    let mut css = String::from("max-width:100%;");
    let (w, h) = (
        run.width_pt.filter(|w| *w > 0.0),
        run.height_pt.filter(|h| *h > 0.0),
    );
    if let Some(w) = w {
        css.push_str(&format!("width:{w:.2}pt;"));
    }
    match (w, h) {
        // The ratio does the work of the height: it survives the shrink.
        (Some(w), Some(h)) => css.push_str(&format!("aspect-ratio:{w:.2}/{h:.2};height:auto;")),
        (None, Some(h)) => css.push_str(&format!("height:{h:.2}pt;")),
        _ => {}
    }

    let mut transform = String::new();
    if let Some(deg) = run.rotation.filter(|d| d.abs() > 0.01) {
        transform.push_str(&format!("rotate({deg:.2}deg) "));
    }
    // Two mirrors are a rotation, and scale(-1,-1) says so; no special case.
    if run.flip_h || run.flip_v {
        let x = if run.flip_h { -1 } else { 1 };
        let y = if run.flip_v { -1 } else { 1 };
        transform.push_str(&format!("scale({x},{y})"));
    }
    if !transform.trim().is_empty() {
        css.push_str(&format!("transform:{};", transform.trim()));
    }
    css
}

/// Which heading a paragraph is, if any.
///
/// Two sources and they disagree often enough to matter: `outlineLevel` is the
/// OOXML numbering (0 = Heading 1), but a document that never sets it still has
/// `styleId` = `Heading2`. Style wins when it parses, because a document that
/// names its styles means them; outline level is the fallback.
///
/// 9 is OOXML's "body text" outline level, and is deliberately not a heading.
///
/// Style names are LOCALISED. Word writes them in the language of the copy that
/// made the document, so a Danish document has `Titel` and `Overskrift1` where
/// an English one has `Title` and `Heading1` — and this wiki is Danish. Matching
/// only the English names left the title of a document rendering as an ordinary
/// paragraph, which is how this was found. Outline level catches the numbered
/// ones in any language; the plain title has no outline level and needs its
/// name read.
pub fn heading_level(style_id: Option<&str>, outline_level: Option<i64>) -> Option<u8> {
    if let Some(style) = style_id {
        let s = style.to_ascii_lowercase().replace([' ', '-', '_'], "");
        // Trailing digits are the level; what is left is the name.
        let digits = s.len() - s.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let (name, level) = s.split_at(s.len() - digits);
        let level = level.parse::<u8>().ok();

        match (KNOWN_HEADING_NAMES.contains(&name), level) {
            // `Heading2`, `Overskrift2`, `Titre 2` …
            (true, Some(n)) if (1..=9).contains(&n) => return Some(n.min(6)),
            // `Title`, `Titel`, `Titre` — a document's one top heading.
            (true, None) => return Some(1),
            _ => {}
        }
    }
    match outline_level {
        Some(l) if (0..=8).contains(&l) => Some(((l + 1) as u8).min(6)),
        _ => None,
    }
}

/// What Word calls a heading, in the languages a document in this wiki plausibly
/// came from. Lowercased with spaces and dashes removed, and with any trailing
/// level number already stripped.
///
/// Not exhaustive and not meant to be: a heading with a level also carries an
/// outline level, which needs no translating and is the fallback. This list is
/// what rescues the LEVELLESS title, and Danish is the one that matters here.
const KNOWN_HEADING_NAMES: &[&str] = &[
    "heading",
    "title", // English
    "overskrift",
    "titel",      // Danish, Norwegian, German, Dutch, Swedish
    "rubrik",     // Swedish
    "berschrift", // German, once the umlaut is not an ascii letter
    "titre",      // French, which uses one word for both
    "titulo",
    "encabezado", // Spanish, Portuguese
    "titolo",     // Italian
    "otsikko",    // Finnish
    "naglowek",   // Polish
];

/// Whether a run is actually underlined.
///
/// The field was once tested with `.is_some()`, which underlined every run in
/// every document: this parser writes `false` for a run WITHOUT an underline,
/// and `Some(false)` is very much some. A loose type plus a truthiness check is
/// a bug waiting to be reported, and it was.
///
/// Handles the shapes an underline is written in: a bool, a style name where
/// `none` means none, or an object carrying that name under `val`.
pub fn is_underlined(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(on)) => *on,
        Some(serde_json::Value::String(style)) => !matches!(style.as_str(), "" | "none"),
        Some(serde_json::Value::Object(map)) => match map.get("val") {
            Some(serde_json::Value::String(style)) => !matches!(style.as_str(), "" | "none"),
            Some(serde_json::Value::Bool(on)) => *on,
            // An object with no `val` at all still says "there is an underline
            // here" more than it says there is not.
            None => !map.is_empty(),
            _ => false,
        },
        _ => false,
    }
}

/// The inline CSS for a run, or an empty string when it needs none.
///
/// Bold and italic are elements rather than styles (see `RunSpan`), so this
/// carries only what has no element of its own.
pub fn run_style(run: &Run) -> String {
    let mut css = String::new();
    // `auto` is not a colour: it means the consumer decides, and forcing it to
    // black would make a document unreadable in the dark theme.
    //
    // Nor, in practice, is literal black. Word writes 000000 for ordinary body
    // text as readily as it writes auto — this handlingsplan states it on 15 of
    // its list paragraphs — and a reader in the dark theme got black text on a
    // dark surface. Dropping it lets the text inherit the surface it is
    // actually on, which is the theme's ink, or a shaded table cell's own
    // readable ink where there is one.
    //
    // The cost is a document that deliberately set black against something
    // light this renderer does not paint. It could not have shown that in the
    // dark theme anyway, and every other colour is still honoured exactly.
    if let Some(c) = run.color.as_deref() {
        let default_ink = c == "auto" || c == "000000";
        if !default_ink && c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
            css.push_str(&format!("color:#{c};"));
        }
    }
    match run.vert_align.as_deref() {
        Some("superscript") => css.push_str("vertical-align:super;font-size:0.8em;"),
        Some("subscript") => css.push_str("vertical-align:sub;font-size:0.8em;"),
        _ => {}
    }
    // Decoration rather than elements, and both at once when both are set. The
    // element chain this replaced could only pick one, so a bold underlined run
    // silently lost its underline.
    match (is_underlined(run.underline.as_ref()), run.strikethrough) {
        (true, true) => css.push_str("text-decoration:underline line-through;"),
        (true, false) => css.push_str("text-decoration:underline;"),
        (false, true) => css.push_str("text-decoration:line-through;"),
        (false, false) => {}
    }
    css
}

/// The inline CSS for a paragraph: alignment and indent, which are the two that
/// change how a document READS rather than merely how it looks.
pub fn paragraph_style(p: &Paragraph) -> String {
    let mut css = String::new();
    // `left` is emitted, not skipped. It was skipped as "the default", but a
    // document does not render in a vacuum: the file viewer around it centres
    // its contents (for a centred image, and for the no-preview state), so a
    // left-aligned paragraph that says nothing INHERITS centre. Reported from a
    // left-aligned document that rendered centred.
    match p.alignment.as_deref() {
        Some("center") => css.push_str("text-align:center;"),
        Some("right") => css.push_str("text-align:right;"),
        Some("both") | Some("justify") => css.push_str("text-align:justify;"),
        Some("left") | Some("start") => css.push_str("text-align:left;"),
        _ => {}
    }
    // Points to rem against a 16px root, so an indent scales with the reader's
    // font size instead of being pinned to Word's. Both indents go through the
    // same scale, or the hanging indent below would not line up.
    let left = p.indent_left.unwrap_or(0.0).max(0.0);
    // A first-line indent, which Word writes as a SEPARATE number and which is
    // usually NEGATIVE: that is a hanging indent, and it is how a bulleted
    // paragraph is written without being a list. `indentLeft` is then where the
    // WRAPPED lines go, and the first line is pulled back to sit under the
    // bullet. Rendering only `indentLeft` puts such a paragraph 18pt to the
    // right of its neighbours — reported from a document whose bullets were all
    // at one level and rendered at two.
    //
    // CSS says this in one property. Clamped so a first line can never escape
    // to the left of the document and be clipped.
    let first = p.indent_first.unwrap_or(0.0).max(-left);
    if left > 0.0 {
        css.push_str(&format!("margin-left:{:.2}rem;", left / 16.0));
    }
    if first.abs() > 0.01 {
        css.push_str(&format!("text-indent:{:.2}rem;", first / 16.0));
    }

    // Space above and below, when the document says. It usually does — all 44
    // paragraphs of the document this was reported from — and what it usually
    // says here is ZERO, because it separates its paragraphs with blank ones
    // instead. The stylesheet's own comfortable margin is then a SECOND gap on
    // top of the blank line, and every space in the document is twice what it
    // should be. A document that says nothing keeps the stylesheet's margin.
    //
    // Headings are left out of this on purpose. A heading is not rendered as
    // itself: it becomes an `<h1>`-`<h6>` in the app's own type scale, and its
    // rhythm belongs to that scale rather than to points from a Word template.
    // The document this came from asks for zero space around its Heading 1,
    // which in Word sits under a blank paragraph and here would sit flush
    // against the text above it.
    let is_heading = heading_level(p.style_id.as_deref(), p.outline_level).is_some();
    // A heading takes its prominence from the document, not from the tag: see
    // [`scale_headings`]. `em`, so it is measured against the reader's own text
    // size rather than against Word's page.
    if is_heading {
        if let Some(scale) = p.heading_scale.filter(|s| *s > 0.0) {
            css.push_str(&format!("font-size:{scale:.3}em;"));
        }
    }
    if !is_heading {
        if let Some(pt) = p.space_before.filter(|v| *v >= 0.0) {
            css.push_str(&format!("margin-top:{:.2}rem;", pt / 16.0));
        }
        if let Some(pt) = p.space_after.filter(|v| *v >= 0.0) {
            css.push_str(&format!("margin-bottom:{:.2}rem;", pt / 16.0));
        }
    }
    // Line spacing, but only the multiplier form. `exact` and `atLeast` are
    // measurements for a fixed page; honouring them in a reflowing column would
    // clip a line that wraps differently than Word intended.
    if let Some(ls) = p.line_spacing.as_ref() {
        if ls.rule == "auto" && ls.value > 0.0 {
            css.push_str(&format!("line-height:{:.2};", ls.value));
        }
    }
    css.push_str(&measured_style(p));
    css
}

/// What Word resolved for this paragraph, for the off-screen copy that works out
/// where the pages end: the spacing, the size, the face and the indent.
///
/// All of it in custom properties, read only inside `.docx-measure`, so it
/// changes nothing on screen. That is what lets a list item carry its own
/// measurements without moving on the page: a list is indented for READING here,
/// half as deep as Word indents it, and the reader's depth is the right one to
/// read at and the wrong one to paginate by.
pub fn measured_style(p: &Paragraph) -> String {
    let mut css = String::new();
    // What THIS paragraph leaves under itself and sets its lines at, as Word
    // resolved it.
    //
    // Per paragraph, not per document: a document-wide figure was tried and the
    // cells outvoted the body. One file's hundred and thirty cell paragraphs
    // leave nothing under themselves and its hundred and eight body paragraphs
    // leave eight points, so "what most paragraphs do" was nothing at all, and
    // the whole document measured a page short of what Word makes of it.
    css.push_str(&format!(
        "--p-before:{:.2}pt;--p-after:{:.2}pt;",
        p.space_before.filter(|v| *v >= 0.0).unwrap_or(0.0),
        p.space_after.filter(|v| *v >= 0.0).unwrap_or(0.0)
    ));
    // And the size Word resolved for this paragraph. Headings especially: this
    // renderer sizes them by how prominent they are against the body, which is
    // right for reading and wrong for measuring, and a document whose table
    // cells are full of headings was measured a quarter short because of it.
    if let Some(pt) = p.default_font_size.filter(|v| *v > 0.0) {
        css.push_str(&format!("--p-size:{pt:.2}pt;"));
    }
    // And the face, per paragraph. Not one face for the document: a document
    // whose list style is Cambria and whose body is Calibri is most of the
    // corpus, and measuring all of it in whichever face is commonest gets the
    // other one wrong. The line box comes with it -- what Word calls single
    // spacing is a property of the face, and the two shipped faces have theirs
    // measured at runtime.
    if let Some(named) = p
        .default_font_family
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
    {
        css.push_str(&format!("--p-face:{};", measuring_face(Some(named))));
        css.push_str(&format!("--p-single:{};", Measured::of(Some(named)).single()));
    }
    let line = match p.line_spacing.as_ref() {
        Some(ls) if ls.rule == "auto" && ls.value > 0.0 => ls.value,
        // Stating none is stating single.
        _ => 1.0,
    };
    css.push_str(&format!("--p-line:{line:.3};"));
    // Where Word's own margin puts this paragraph, in points, which is not what
    // the visible margin above says: that one is in rem so an indent scales with
    // the reader's text, and at a 16px root Word's 36pt indent reads as 36px
    // where the page gives it 48. Measured at the reader's depth, a list wrapped
    // less than Word wraps it and every page held two items too many.
    let left = p.indent_left.unwrap_or(0.0).max(0.0);
    if left > 0.0 {
        css.push_str(&format!("--p-indent:{left:.2}pt;"));
    }
    // The first line's own indent, for a paragraph. NOT for a list item: there
    // the hanging indent is where the bullet goes, and the text of every line,
    // first included, begins at the indent proper.
    let first = p.indent_first.unwrap_or(0.0).max(-left);
    if first.abs() > 0.01 && p.numbering.is_none() {
        css.push_str(&format!("--p-first:{first:.2}pt;"));
    }
    css
}

/// Whether a paragraph is a list item, and whether the list is ordered.
///
/// `bullet` is the only unordered format in OOXML; every other format is a
/// counter of some kind, so anything that is not a bullet is an ordered list.
pub fn list_kind(p: &Paragraph) -> Option<bool> {
    let n = p.numbering.as_ref()?;
    let ordered = !matches!(n.format.as_deref(), Some("bullet") | None);
    Some(ordered)
}

/// Whether a cell is only there to be covered by a merge from the row above.
pub fn is_merged_away(cell: &Cell) -> bool {
    matches!(cell.v_merge.as_deref(), Some("continue"))
}

/// A whole document: its blocks, in order.
///
/// Consecutive list paragraphs are gathered into one `<ul>`/`<ol>` rather than
/// each becoming its own single-item list, which is what a naive block-by-block
/// walk produces and what makes such a rendering look like a stack of bullets
/// with gaps between them.
#[component]
pub fn DocxBody(
    blocks: Vec<Block>,
    /// Where the pages end, as `(group index, item index, the page that begins
    /// there)`. The item index is [`BEFORE_GROUP`] for a break between groups
    /// and the item's own index for one inside a list.
    /// Worked out by [`PagedDocx`], which is the only thing that fills this in;
    /// a table cell renders through here too and has no pages of its own.
    #[props(default)]
    marks: Vec<(usize, usize, usize, f64)>,
) -> Element {
    // Group first, render second: the grouping is a property of the SEQUENCE,
    // and rsx has no way to look ahead mid-iteration.
    let mut groups: Vec<Group> = Vec::new();
    for block in blocks {
        match &block {
            Block::Paragraph(p) => match list_kind(p) {
                Some(ordered) => match groups.last_mut() {
                    Some(Group::List { ordered: o, items }) if *o == ordered => {
                        items.push(p.clone())
                    }
                    _ => groups.push(Group::List {
                        ordered,
                        items: vec![p.clone()],
                    }),
                },
                None => groups.push(Group::Single(block)),
            },
            _ => groups.push(Group::Single(block)),
        }
    }

    // A mark can sit before a group, or BETWEEN two items of a list. Word breaks
    // a page wherever the text runs out, which in these documents is usually
    // in the middle of a bulleted list -- sixty of one document's sixty-eight
    // paragraphs are list items -- so marking only between groups snapped every
    // break back to where the list started, eight paragraphs early.
    let split_at = |group: usize, item: usize| {
        marks
            .iter()
            .find(|(g, i, _, _)| *g == group && *i == item)
            .map(|(_, _, begins, spare)| (*begins, *spare))
    };

    rsx! {
        div { class: "docx",
            for (i , group) in groups.into_iter().enumerate() {
                // Where a page ends, when someone has worked out where that is
                // (see `PagedDocx`). Empty for a cell's contents and for a
                // document nobody paginated, which is most of them.
                if let Some((begins, spare)) = split_at(i, BEFORE_GROUP) {
                    PageMark { key: "p{i}", begins, spare }
                }
                match group {
                    Group::Single(block) => {
                        let rows_marked: Vec<(usize, usize, f64)> = marks
                            .iter()
                            .filter(|(g, item, _, _)| *g == i && *item != BEFORE_GROUP)
                            .map(|(_, item, begins, spare)| (*item, *begins, *spare))
                            .collect();
                        rsx! {
                            DocxBlock { key: "b{i}", block, rows_marked }
                        }
                    }
                    Group::List { ordered, items } => {
                        // A list drawn with picture bullets places itself from
                        // the document's own indents, so the list must not add
                        // its own padding on top and push it out of line with
                        // the paragraphs around it.
                        let class = match items.iter().any(|it| {
                            it.numbering
                                .as_ref()
                                .is_some_and(|n| n.picture().is_some())
                        }) {
                            true => "docx-list docx-list-pic",
                            false => "docx-list",
                        };
                        // The list, cut into runs wherever a page ends inside
                        // it. Each run is its own list with the mark between,
                        // which is what HTML has room for: a page break is not
                        // a list item.
                        let mut runs: Vec<(Option<(usize, f64)>, Vec<Paragraph>)> =
                            vec![(None, Vec::new())];
                        for (j, item) in items.into_iter().enumerate() {
                            match split_at(i, j) {
                                Some(mark) if j > 0 => runs.push((Some(mark), vec![item])),
                                _ => runs.last_mut().expect("seeded above").1.push(item),
                            }
                        }
                        // A page beginning at the list's FIRST item belongs
                        // before the whole list, since splitting there would
                        // leave an empty one. It used to be dropped instead:
                        // the count knew about the page and nothing on screen
                        // marked it, so the control offered a page it could not
                        // scroll to.
                        let ahead = split_at(i, 0);
                        rsx! {
                            if let Some((begins, spare)) = ahead {
                                PageMark { key: "lp{i}-first", begins, spare }
                            }
                            for (r , (mark , run)) in runs.into_iter().enumerate() {
                                if let Some((begins, spare)) = mark {
                                    PageMark { key: "lp{i}-{r}", begins, spare }
                                }
                                if ordered {
                                    ol { key: "l{i}-{r}", class,
                                        for (j , item) in run.into_iter().enumerate() {
                                            ListItem { key: "i{j}", item }
                                        }
                                    }
                                } else {
                                    ul { key: "l{i}-{r}", class,
                                        for (j , item) in run.into_iter().enumerate() {
                                            ListItem { key: "i{j}", item }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The item index that means "before the whole group" rather than inside it.
pub const BEFORE_GROUP: usize = usize::MAX;

/// The hairline that says a page ended here, the same one the PDF reader draws.
#[component]
fn PageMark(begins: usize, #[props(default)] spare: f64) -> Element {
    rsx! {
        div {
            class: "pdf-page-break",
            role: "separator",
            // The paper left under the text of the page that ends here, drawn as
            // the empty space it is on the page itself.
            style: "--page-spare:{spare:.0}px;",
            "data-page": "{begins}",
            // An ending, like the PDF's: a jump aims past it.
            "data-page-ends": "true",
            span { "{begins - 1}" }
        }
    }
}

/// A run of blocks that render together.
enum Group {
    Single(Block),
    List {
        ordered: bool,
        items: Vec<Paragraph>,
    },
}

/// One cell of a table.
///
/// Its own component so the shading can be read off the cell before its content
/// moves into the body, which rsx has no room to do inline.
#[component]
fn TableCell(cell: Cell, header: bool, #[props(default)] share: Option<f64>) -> Element {
    // The width Word states, for the measuring copy, and the share of the table
    // it is, for anything that would rather fit the text column. Custom
    // properties: the visible table sizes its columns to their contents, which
    // is what keeps a table readable on a phone.
    let mut shade = String::new();
    if let Some(pct) = share {
        shade.push_str(&format!("--c-share:{pct:.3}%;"));
    }
    if let Some(pt) = cell.width_pt.filter(|w| *w > 0.0) {
        shade.push_str(&format!("--c-width:{pt:.2}pt;"));
    }
    shade.push_str(&cell_style(&cell));
    let span = cell.col_span;
    match header {
        true => rsx! {
            th { colspan: "{span}", style: "{shade}", DocxBody { blocks: cell.content } }
        },
        false => rsx! {
            td { colspan: "{span}", style: "{shade}", DocxBody { blocks: cell.content } }
        },
    }
}

/// One item in a list.
///
/// Word lists can use a picture as their bullet — `numPicBullet` — and a
/// browser's own disc is not it. Where the document supplies one, the marker is
/// turned off and the picture is drawn in its place, at the size the numbering
/// definition gives it.
#[component]
fn ListItem(item: Paragraph) -> Element {
    let bullet = item
        .numbering
        .as_ref()
        .filter(|n| n.picture().is_some())
        .and_then(|n| n.src.clone().map(|src| (src, n.bullet_style())));

    match bullet {
        Some((src, style)) => {
            // Built exactly like the picture-bulleted PARAGRAPHS around it: the
            // bullet inline at the head of the text, and the paragraph's own
            // hanging indent placing it. That is what makes one level of
            // bullets look like one level whether the document wrote them as a
            // list or not — and the list's own padding is dropped, since the
            // document has already said where this belongs.
            let indent = paragraph_style(&item);
            rsx! {
                li { class: "docx-li-pic", style: "{indent}",
                    img { class: "docx-bullet", src: "{src}", style: "{style}", alt: "" }
                    {runs_of(&item)}
                }
            }
        }
        None => {
            // `w:ilvl` is the outline level of the item. Every level used to
            // render identically, so a document's second level read as a
            // continuation of its first. Stepped and re-marked here rather than
            // by nesting real lists: the parser hands these over as one flat
            // run of paragraphs, and inferring a tree from the levels would
            // invent structure the document did not state.
            let level = item
                .numbering
                .as_ref()
                .and_then(|n| n.level)
                .unwrap_or(0)
                .clamp(0, 8);
            let class = match level {
                0 => "docx-li",
                1 => "docx-li docx-li-2",
                2 => "docx-li docx-li-3",
                _ => "docx-li docx-li-4",
            };
            // Word closes a list up: where a paragraph sets contextualSpacing,
            // the space under it goes IF the next paragraph shares its style
            // and sets it too. That "if" is why reading the flag off the
            // document and dropping every item's spacing was wrong -- it took
            // six hundred points out of a sixty-item list. The pair is what
            // matters, so each item says whether it is closed up and the
            // stylesheet asks about its neighbour.
            let class = match item.contextual_spacing {
                true => format!("{class} docx-li-tight"),
                false => class.to_string(),
            };
            // Its own resolved typography, for the measuring copy. A list item
            // used to carry none, so a page was worked out with the document's
            // defaults where Word uses the item's face, size, line spacing and
            // indent -- a Cambria list measured in Calibri, at a line box 8%
            // too tall, half as deep as the page indents it.
            let measured = measured_style(&item);
            rsx! {
                li { class: "{class}", style: "{measured}", {runs_of(&item)} }
            }
        }
    }
}

/// One block: a heading, a paragraph, or a table.
#[component]
fn DocxBlock(
    block: Block,
    /// Page ends that fall INSIDE this block, as `(row index, the page that
    /// begins there)`. Only a table has anywhere inside it to put one: Word
    /// carries a table's rows onto the next page, and a document that is mostly
    /// table has almost nowhere else a page can end.
    #[props(default)]
    rows_marked: Vec<(usize, usize, f64)>,
) -> Element {
    match block {
        Block::Paragraph(p) => {
            let style = paragraph_style(&p);
            let inner = runs_of(&p);
            // A dynamic tag name is not expressible in rsx, so the six headings
            // are written out. HTML has six; deeper levels were clamped when the
            // level was worked out.
            match heading_level(p.style_id.as_deref(), p.outline_level) {
                Some(1) => {
                    rsx! { h1 { class: "docx-h", style: "{style}", "data-keep-next": "{p.keep_next}", {inner} } }
                }
                Some(2) => {
                    rsx! { h2 { class: "docx-h", style: "{style}", "data-keep-next": "{p.keep_next}", {inner} } }
                }
                Some(3) => {
                    rsx! { h3 { class: "docx-h", style: "{style}", "data-keep-next": "{p.keep_next}", {inner} } }
                }
                Some(4) => {
                    rsx! { h4 { class: "docx-h", style: "{style}", "data-keep-next": "{p.keep_next}", {inner} } }
                }
                Some(5) => {
                    rsx! { h5 { class: "docx-h", style: "{style}", "data-keep-next": "{p.keep_next}", {inner} } }
                }
                Some(_) => {
                    rsx! { h6 { class: "docx-h", style: "{style}", "data-keep-next": "{p.keep_next}", {inner} } }
                }
                // An empty paragraph is a deliberate blank line in Word, so it
                // keeps its element rather than being dropped.
                None if p.keep_next => rsx! {
                    p { class: "docx-p", style: "{style}", "data-keep-next": "true", {inner} }
                },
                None => rsx! { p { class: "docx-p", style: "{style}", {inner} } },
            }
        }
        Block::Table(t) => {
            // How wide the mark's own row has to be to span the table.
            let across = t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(1);
            // Word's column widths, and the SUM of them, for the off-screen copy
            // to wrap the text where Word wraps it.
            //
            // Left to itself the browser fits columns to their content, which
            // is right on screen and nothing like Word: it gave this table
            // 159/81/402 where Word gives 290/75/305.
            //
            // The sum is the part that took two goes to get right. This
            // document's three columns come to 670px against a 642px text
            // column, because **Word lets a table run into the margin**; a table
            // told only what its columns are is still capped at the box around
            // it, and the browser scales them down to fit — 290/75/305 became
            // 275/76/290, five per cent narrower, and five per cent narrower
            // wraps a seven-line cell into eight. That is one line per row, and
            // over one document's tables it came to the better part of a page,
            // which moved a break a row late. So the table is told its own width
            // as well, and allowed to overflow exactly as it does in Word.
            //
            // Shares alongside, for the reading copy, which would rather fit
            // the text column than reproduce the page.
            let widths: Vec<f64> = t
                .rows
                .iter()
                .max_by_key(|r| r.cells.len())
                .map(|r| r.cells.iter().map(|c| c.width_pt.unwrap_or(0.0)).collect())
                .unwrap_or_default();
            let stated: f64 = widths.iter().sum();
            let shares: Vec<Option<f64>> = match stated > 0.0 {
                true => widths
                    .iter()
                    .map(|w| (*w > 0.0).then(|| w / stated * 100.0))
                    .collect(),
                false => Vec::new(),
            };
            // Only when EVERY column states one: a table with some widths and
            // some not has no total, and a partial sum would be a narrower table
            // rather than a wider one.
            //
            // Always set, `auto` included, because a custom property INHERITS: a
            // nested table that states no widths of its own would otherwise be
            // laid out at the width of the table it sits inside.
            let table_width = match !widths.is_empty() && widths.iter().all(|w| *w > 0.0) {
                true => format!("--t-width:{stated:.2}pt;"),
                false => "--t-width:auto;".to_string(),
            };
            rsx! {
            // Its own scroll container: a wide table must not widen the page and
            // force the whole document sideways on a phone.
            div { class: "docx-table-wrap",
                table { class: "docx-table", style: "{table_width}",
                    for (r , row) in t.rows.into_iter().enumerate() {
                        if let Some((_, begins, spare)) = rows_marked.iter().find(|(at, _, _)| *at == r) {
                            tr { key: "pr{r}", class: "docx-page-row",
                                td { colspan: "{across}",
                                    div {
                                        class: "pdf-page-break",
                                        role: "separator",
                                        style: "--page-spare:{spare:.0}px;",
                                        "data-page": "{begins}",
                                        "data-page-ends": "true",
                                        span { "{begins - 1}" }
                                    }
                                }
                            }
                        }
                        tr {
                            key: "r{r}",
                            "data-cant-split": "{row.cant_split}",
                            // The height the document asks for, for the copy
                            // that measures. Not the visible table: on a phone
                            // a row dragged tall in Word is empty space.
                            style: match row.row_height.filter(|h| *h > 0.0) {
                                Some(pt) => format!("--r-height:{pt:.2}pt;"),
                                None => String::new(),
                            },
                            for (c , cell) in row.cells.into_iter().enumerate() {
                                // A cell covered by a merge from above is not
                                // drawn; drawing it would push the row wider.
                                if !is_merged_away(&cell) {
                                    TableCell {
                                        key: "c{c}",
                                        cell,
                                        header: row.is_header,
                                        share: shares.get(c).copied().flatten(),
                                    }
                                }
                            }
                        }
                    }
                }
            }
            }
        }
        Block::Unknown => rsx! {},
    }
}

/// Whether a paragraph is really several bulleted items written as one.
///
/// A document in this wiki draws its bullets as little pictures rather than
/// using a Word list, and then puts two of those items in a single paragraph:
/// picture, text, picture, text. Word breaks the line between them only because
/// its page has a fixed width and the first item happens to fill it. This
/// renderer reflows, so on a wide screen the two items run together into one
/// sentence — which is wrong at any width, since the wrap point would otherwise
/// land mid-sentence.
///
/// The pattern is what identifies it, not the position: two or more pictures in
/// one paragraph, each followed by text. A single picture in a sentence is a
/// picture in a sentence, and a row of pictures with nothing between them is a
/// row of pictures; neither is touched. Checked in document order because the
/// FIRST picture keeps its place — it is the paragraph's own bullet.
fn picture_bulleted(p: &Paragraph) -> bool {
    let mut bullets = 0;
    for (i, run) in p.runs.iter().enumerate() {
        if run.src.is_none() {
            continue;
        }
        let followed_by_text = p.runs[i + 1..]
            .iter()
            .find(|r| r.src.is_some() || !r.text.trim().is_empty())
            .is_some_and(|r| r.src.is_none());
        if !followed_by_text {
            return false;
        }
        bullets += 1;
    }
    bullets >= 2
}

/// A paragraph's runs, as inline elements.
///
/// Bold and italic become `<strong>`/`<em>` rather than CSS: they are meaning,
/// not decoration, and a screen reader announces them.
fn runs_of(p: &Paragraph) -> Element {
    let runs = p.runs.clone();
    let split = picture_bulleted(p);
    let mut seen_picture = false;
    // A paragraph that ENDS with a line break ends with an empty line, and Word
    // draws it: the break moves to a new line and the paragraph mark sits on it.
    // HTML does not -- a trailing `<br>` closes nothing and takes no room -- so
    // the line has to be held open by something. A zero-width space is the same
    // trick an empty paragraph uses, for the same reason.
    //
    // Ten paragraphs of one document end this way, and the ten missing lines
    // came to a fifth of a page: enough to hold a heading Word puts on the page
    // after.
    let ends_with_a_break = runs
        .last()
        .is_some_and(|r| r.kind.as_deref() == Some("break"));
    rsx! {
        for (i , run) in runs.into_iter().enumerate() {
            if split && run.src.is_some() && std::mem::replace(&mut seen_picture, true) {
                // Not the first: a new item, so a new line. Wrapped so the
                // iteration keeps a single root and its key.
                span { key: "r{i}",
                    br {}
                    RunSpan { run }
                }
            } else {
                RunSpan { key: "r{i}", run }
            }
        }
        if ends_with_a_break {
            "\u{200b}"
        }
    }
}

#[component]
fn RunSpan(run: Run) -> Element {
    // A break is the whole run. Page and column breaks become the same line
    // break: a reading column has no pages to break, and losing the line as
    // well would run two of the author's lines together.
    if run.kind.as_deref() == Some("break") {
        return rsx! { br {} };
    }
    // A picture is a whole run, never text with a picture in it.
    if let Some(src) = run.src.clone() {
        let style = image_style(&run);
        return rsx! {
            img {
                class: "docx-img",
                src: "{src}",
                style: "{style}",
                // No alt text to give: OOXML can carry a description, and this
                // parser does not surface one. An empty alt at least keeps a
                // screen reader from reading out "image1.png".
                alt: "",
                loading: "lazy",
                decoding: "async",
            }
        };
    }

    let style = run_style(&run);
    let text = run.text.clone();
    // Word writes a hyperlink as an ordinary run carrying a target, so the
    // anchor is wrapped around whatever the run turns out to be.
    let inner = rsx! {
        // Bold and italic are elements because they are MEANING and a screen
        // reader announces them; underline and strikethrough are decoration and
        // live in the style, so every combination survives.
        if run.bold && run.italic {
            strong { em { style: "{style}", "{text}" } }
        } else if run.bold {
            strong { style: "{style}", "{text}" }
        } else if run.italic {
            em { style: "{style}", "{text}" }
        } else {
            span { style: "{style}", "{text}" }
        }
    };
    match run.hyperlink.as_deref().filter(|h| !h.is_empty()) {
        Some(href) => rsx! {
            a {
                href: "{href}",
                target: "_blank",
                rel: "noopener noreferrer",
                {inner}
            }
        },
        None => inner,
    }
}

#[cfg(test)]
mod tests {

    /// A paragraph's style without the custom properties that exist only for
    /// the off-screen measuring copy. Those say nothing about how a paragraph
    /// looks on screen, which is what these tests are about.
    fn visible(style: &str) -> String {
        style
            .split_inclusive(';')
            .filter(|d| !d.trim_start().starts_with("--p-"))
            .collect()
    }
    use super::*;

    /// Word writes literal black for ordinary body text, so honouring it put
    /// black on the dark theme's dark surface. It means the same as `auto`.
    #[test]
    fn a_documents_default_ink_is_left_to_the_reader() {
        for stated in ["auto", "000000"] {
            let run = Run {
                color: Some(stated.to_string()),
                ..Run::default()
            };
            assert!(
                !run_style(&run).contains("color:"),
                "{stated} should inherit the surface"
            );
        }
    }

    /// Every other colour a document states is still its own.
    #[test]
    fn a_stated_colour_is_still_honoured() {
        let run = Run {
            color: Some("2F5496".into()),
            ..Run::default()
        };
        assert!(
            run_style(&run).contains("color:#2F5496;"),
            "{:?}",
            run_style(&run)
        );

        // Including one that is merely dark, which is a choice rather than a
        // default.
        let dark = Run {
            color: Some("212121".into()),
            ..Run::default()
        };
        assert!(run_style(&dark).contains("color:#212121;"));
    }

    /// The parser tags a line break as a run of its own. Pinned because the
    /// field is `type`, which needs a serde rename, and getting it wrong is
    /// silent: the run deserialises with no text and simply disappears.
    #[test]
    fn a_break_run_is_recognised_by_its_tag() {
        let run: Run = serde_json::from_value(serde_json::json!({
            "type": "break",
            "breakType": "line"
        }))
        .expect("break run");
        assert_eq!(run.kind.as_deref(), Some("break"));
        assert!(run.text.is_empty());

        // An ordinary run must not be mistaken for one.
        let plain: Run = serde_json::from_value(serde_json::json!({
            "type": "text",
            "text": "hello"
        }))
        .expect("text run");
        assert_ne!(plain.kind.as_deref(), Some("break"));
    }

    /// A second-level bullet has to keep its level, or it renders as a
    /// continuation of the first.
    #[test]
    fn a_nested_item_keeps_its_outline_level() {
        let n: Numbering = serde_json::from_value(serde_json::json!({
            "format": "bullet",
            "level": 1
        }))
        .expect("numbering");
        assert_eq!(n.level, Some(1));
    }

    use super::*;

    #[test]
    fn a_named_style_beats_the_outline_level() {
        // Word writes both, and the style is the one the author chose.
        assert_eq!(heading_level(Some("Heading2"), Some(0)), Some(2));
        assert_eq!(heading_level(Some("heading 3"), None), Some(3));
        assert_eq!(heading_level(Some("Title"), None), Some(1));
    }

    #[test]
    fn the_outline_level_is_the_fallback() {
        assert_eq!(heading_level(None, Some(0)), Some(1), "0 is Heading 1");
        assert_eq!(heading_level(None, Some(2)), Some(3));
        // Deeper than the six HTML has: clamped rather than dropped, so a level
        // 8 heading is still a heading.
        assert_eq!(heading_level(None, Some(8)), Some(6));
    }

    #[test]
    fn body_text_is_not_a_heading() {
        // 9 is OOXML's body-text outline level, and most paragraphs have none.
        assert_eq!(heading_level(None, Some(9)), None);
        assert_eq!(heading_level(None, None), None);
        assert_eq!(heading_level(Some("BodyText"), None), None);
        assert_eq!(heading_level(Some("Heading0"), None), None);
        assert_eq!(heading_level(Some("Heading12"), None), None);
    }

    /// `auto` means "the consumer decides". Turning it into a colour would pin
    /// the text to black and make the document unreadable in the dark theme.
    #[test]
    fn an_automatic_colour_is_left_alone() {
        let auto = Run {
            color: Some("auto".into()),
            ..Default::default()
        };
        assert_eq!(run_style(&auto), "");
        let red = Run {
            color: Some("FF0000".into()),
            ..Default::default()
        };
        assert_eq!(run_style(&red), "color:#FF0000;");
        // Anything that is not a hex triple is not a colour.
        for junk in ["", "red", "12345", "GGGGGG", "#FF0000"] {
            let r = Run {
                color: Some(junk.into()),
                ..Default::default()
            };
            assert_eq!(run_style(&r), "", "{junk:?}");
        }
    }

    /// The bug this exists to prevent: the parser writes `false` for a run with
    /// NO underline, and the old check was `.is_some()`, so every run in every
    /// document came out underlined. Reported from a real Word document that
    /// contains no underline at all — 312 of its 315 runs carry `false`.
    #[test]
    fn a_run_that_is_not_underlined_is_not_underlined() {
        use serde_json::json;
        assert!(!is_underlined(Some(&json!(false))), "the reported case");
        assert!(!is_underlined(None));
        assert!(!is_underlined(Some(&json!(null))));
        assert!(is_underlined(Some(&json!(true))));

        // And through the styling, which is what actually reaches the page.
        let plain = Run {
            underline: Some(json!(false)),
            ..Default::default()
        };
        assert!(
            !run_style(&plain).contains("underline"),
            "{}",
            run_style(&plain)
        );
        let underlined = Run {
            underline: Some(json!(true)),
            ..Default::default()
        };
        assert!(run_style(&underlined).contains("text-decoration:underline;"));
    }

    /// OOXML itself writes an underline as a style NAME, and `none` is one of
    /// them. A producer that writes the name rather than a bool must not turn
    /// every run into an underlined one the same way.
    #[test]
    fn an_underline_style_of_none_is_not_an_underline() {
        use serde_json::json;
        for off in [
            json!("none"),
            json!(""),
            json!({"val": "none"}),
            json!({"val": false}),
        ] {
            assert!(!is_underlined(Some(&off)), "{off}");
        }
        for on in [
            json!("single"),
            json!("dotted"),
            json!({"val": "wave"}),
            json!({"val": true}),
        ] {
            assert!(is_underlined(Some(&on)), "{on}");
        }
    }

    /// Underline and strikethrough are decoration and combine; the element
    /// chain this replaced could only pick one, so a bold underlined run lost
    /// its underline entirely.
    #[test]
    fn decoration_survives_bold_and_combines() {
        use serde_json::json;
        let both = Run {
            underline: Some(json!(true)),
            strikethrough: true,
            bold: true,
            ..Default::default()
        };
        assert_eq!(run_style(&both), "text-decoration:underline line-through;");
        let struck = Run {
            strikethrough: true,
            ..Default::default()
        };
        assert_eq!(run_style(&struck), "text-decoration:line-through;");
    }

    /// The shapes here are copied from what the parser produced for a real
    /// document in the wiki: fifteen picture runs, two distinct files, each
    /// 9pt square and used as the bullet leading a list item.
    fn bullet_run(path: &str) -> Run {
        Run {
            image_path: Some(path.into()),
            mime_type: Some("image/jpeg".into()),
            width_pt: Some(9.0),
            height_pt: Some(8.99984251968504),
            ..Default::default()
        }
    }

    /// The exact json the parser produced for a picture run in
    /// `Strategi 2030.docx`, pasted whole. The field names are the contract
    /// between the parser and this renderer, and a rename on either side would
    /// otherwise fail silently: every field would deserialise to `None` and the
    /// pictures would simply not appear.
    fn picture(path: &str) -> Run {
        Run {
            src: Some(std::rc::Rc::from("data:image/jpeg;base64,AAAA")),
            image_path: Some(path.into()),
            ..Default::default()
        }
    }
    fn words(text: &str) -> Run {
        Run {
            text: text.into(),
            ..Default::default()
        }
    }

    /// Reported from `Strategi 2030.docx`: two bulleted items written as one
    /// paragraph, with no break between them anywhere in the file. Word wraps
    /// because its page is a fixed width; this renderer reflows, so on a wide
    /// screen the two items ran together into one sentence.
    #[test]
    fn two_picture_bullets_in_one_paragraph_are_two_items() {
        let two = Paragraph {
            runs: vec![
                picture("word/media/image1.jpeg"),
                words(" "),
                words("Understøtte og gribe medlemmernes idéer."),
                picture("word/media/image2.jpeg"),
                words(" "),
                words("Undersøge muligheden for bedre faciliteter."),
            ],
            ..Default::default()
        };
        assert!(picture_bulleted(&two));
    }

    #[test]
    fn an_ordinary_picture_is_left_where_it_is() {
        // One bullet leading one item: nothing to split.
        let one = Paragraph {
            runs: vec![picture("word/media/image1.jpeg"), words(" Udarbejde.")],
            ..Default::default()
        };
        assert!(!picture_bulleted(&one), "a single bullet is not a list");

        // A picture inside a sentence, which must not start a new line.
        let inline = Paragraph {
            runs: vec![
                words("As shown in "),
                picture("word/media/figure.png"),
                words(" the numbers rose, see "),
                picture("word/media/figure2.png"),
            ],
            ..Default::default()
        };
        assert!(
            !picture_bulleted(&inline),
            "the last picture ends the paragraph, so this is not the pattern"
        );

        // Pictures in a row with nothing between them are a row of pictures.
        let strip = Paragraph {
            runs: vec![
                picture("word/media/a.png"),
                picture("word/media/b.png"),
                picture("word/media/c.png"),
                words(" Three photos."),
            ],
            ..Default::default()
        };
        assert!(!picture_bulleted(&strip));

        // And a paragraph with no pictures at all is never touched.
        assert!(!picture_bulleted(&Paragraph {
            runs: vec![words("Plain text.")],
            ..Default::default()
        }));
    }

    /// Reported: the bullet before "Arbejde mod at blive en grønnere forening"
    /// was a browser disc. It is a real Word list, and its numbering names a
    /// picture — these are the values the parser gave for it.
    #[test]
    fn a_list_can_have_a_picture_for_its_bullet() {
        let n = Numbering {
            format: Some("bullet".into()),
            level: Some(0),
            pic_bullet_image_path: Some("word/media/image1.jpeg".into()),
            pic_bullet_mime_type: Some("image/jpeg".into()),
            pic_bullet_width_pt: Some(18.0),
            pic_bullet_height_pt: Some(18.75),
            src: None,
        };
        assert_eq!(n.picture(), Some(("word/media/image1.jpeg", "image/jpeg")));
        // The SHAPE only. The size is the stylesheet's, in em, because an 18pt
        // bullet beside 11pt text is a page-layout number and this renderer
        // favours reading over reproduction.
        assert_eq!(n.bullet_style(), "aspect-ratio:18.00/18.75;");

        // An ordinary character bullet has no picture and no style.
        let plain = Numbering {
            format: Some("bullet".into()),
            level: Some(0),
            ..Default::default()
        };
        assert_eq!(plain.picture(), None);
        assert_eq!(plain.bullet_style(), "");
    }

    /// The bullet picture is named on the paragraph, not in its runs, so the
    /// collector has to look there too or the bullet never gets its bytes.
    #[test]
    fn a_bullet_picture_is_collected_and_attached() {
        let mut blocks = vec![Block::Paragraph(Paragraph {
            numbering: Some(Box::new(Numbering {
                format: Some("bullet".into()),
                pic_bullet_image_path: Some("word/media/image1.jpeg".into()),
                pic_bullet_mime_type: Some("image/jpeg".into()),
                ..Default::default()
            })),
            runs: vec![words("Arbejde mod at blive en grønnere forening.")],
            ..Default::default()
        })];

        let mut images = Images::new();
        images.insert(
            "word/media/image1.jpeg".into(),
            std::rc::Rc::from("data:image/jpeg;base64,BBBB"),
        );
        attach_images(&mut blocks, &images);
        let Block::Paragraph(p) = &blocks[0] else {
            panic!()
        };
        assert_eq!(
            p.numbering.as_ref().unwrap().src.as_deref(),
            Some("data:image/jpeg;base64,BBBB")
        );
    }

    /// The numbering shape as the parser actually wrote it, fields and all.
    #[test]
    fn a_real_picture_bullet_deserialises() {
        let json = r#"{
            "fontFamily": "Symbol",
            "format": "bullet",
            "indentLeft": 32.15,
            "jc": "left",
            "level": 0,
            "numId": 1,
            "picBulletHeightPt": 18.75,
            "picBulletImagePath": "word/media/image1.jpeg",
            "picBulletMimeType": "image/jpeg",
            "picBulletWidthPt": 18.0,
            "suff": "tab",
            "tab": 18.0,
            "text": ""
        }"#;
        let n: Numbering = serde_json::from_str(json).expect("the parser's own output");
        assert_eq!(n.format.as_deref(), Some("bullet"));
        assert_eq!(n.picture(), Some(("word/media/image1.jpeg", "image/jpeg")));
        assert_eq!(n.pic_bullet_width_pt, Some(18.0));
    }

    #[test]
    fn a_real_picture_run_deserialises() {
        let json = r#"{
            "allowOverlap": true,
            "anchor": false,
            "anchorXFromMargin": false,
            "anchorXPt": 0.0,
            "anchorYFromPara": false,
            "anchorYPt": 0.0,
            "colorReplaceFrom": null,
            "heightPt": 8.99984251968504,
            "imagePath": "word/media/image2.jpeg",
            "mimeType": "image/jpeg",
            "type": "image",
            "widthPt": 9.0
        }"#;
        let run: Run = serde_json::from_str(json).expect("the parser's own output");
        assert_eq!(run.image_path.as_deref(), Some("word/media/image2.jpeg"));
        assert_eq!(run.mime_type.as_deref(), Some("image/jpeg"));
        assert_eq!(run.width_pt, Some(9.0));
        assert_eq!(
            picture_of(&run),
            Some(("word/media/image2.jpeg", "image/jpeg"))
        );
        assert!(run.src.is_none(), "the bytes are attached separately");
        // 9pt is 12px: a bullet, and it must not be rounded away to nothing.
        assert!(image_style(&run).contains("width:9.00pt;"));
        // Unknown fields do not derail the run, and no text is not a problem.
        assert_eq!(run.text, "");
    }

    /// And the paragraph around it, so the picture is reached the way the
    /// renderer reaches it: a run inside a body block.
    #[test]
    fn a_real_bulleted_paragraph_carries_its_picture() {
        let json = r#"{
            "type": "paragraph",
            "alignment": "left",
            "runs": [
                {"type":"image","imagePath":"word/media/image1.jpeg","mimeType":"image/jpeg",
                 "widthPt":9.0,"heightPt":9.0},
                {"type":"text","text":" Udarbejde et nyt principprogram. "}
            ]
        }"#;
        let block: Block = serde_json::from_str(json).unwrap();
        let Block::Paragraph(p) = &block else {
            panic!("expected a paragraph")
        };
        assert_eq!(p.runs.len(), 2);
        assert!(picture_of(&p.runs[0]).is_some(), "the bullet");
        assert!(picture_of(&p.runs[1]).is_none(), "the text after it");
        assert_eq!(visible(&paragraph_style(p)), "text-align:left;");
    }

    /// Reported: table cells lost their colour. The parser gives it as a hex on
    /// the cell; `auto` means the theme's, exactly as it does on a run.
    #[test]
    fn a_table_cell_keeps_the_colour_the_document_gave_it() {
        let shaded = Cell {
            background: Some("D9E2F3".into()),
            ..Default::default()
        };
        let css = cell_style(&shaded);
        assert!(css.contains("background:#D9E2F3;"), "{css}");
        // Pale blue: black reads on it.
        assert!(css.contains("color:#000;"), "{css}");

        // A dark header wants light text, and the theme's ink is not it.
        let dark = Cell {
            background: Some("1F3864".into()),
            ..Default::default()
        };
        assert!(
            cell_style(&dark).contains("color:#fff;"),
            "{}",
            cell_style(&dark)
        );

        // `auto` is "whatever the reader's theme says" and must not be painted.
        for skip in [Some("auto".to_string()), Some("".to_string()), None] {
            let cell = Cell {
                background: skip.clone(),
                ..Default::default()
            };
            assert_eq!(cell_style(&cell), "", "{skip:?}");
        }
    }

    /// Green is most of what the eye sees, so a flat average calls a saturated
    /// blue light when it is not. These are the colours Word's own table styles
    /// use.
    #[test]
    fn ink_is_chosen_by_luminance_not_by_average() {
        assert_eq!(readable_ink("FFFFFF"), "#000");
        assert_eq!(readable_ink("000000"), "#fff");
        assert_eq!(readable_ink("1F3864"), "#fff", "dark navy");
        assert_eq!(readable_ink("D9E2F3"), "#000", "pale blue");
        assert_eq!(
            readable_ink("0000FF"),
            "#fff",
            "pure blue is DARK to the eye"
        );
        assert_eq!(readable_ink("FFFF00"), "#000", "pure yellow is light");
    }

    #[test]
    fn a_picture_is_sized_as_the_document_says() {
        let css = image_style(&bullet_run("word/media/image2.jpeg"));
        assert!(css.contains("width:9.00pt;"), "{css}");
        // The height rides in the ratio, so shrinking on a narrow screen keeps
        // the shape instead of squashing it.
        assert!(css.contains("aspect-ratio:9.00/9.00;"), "{css}");
        assert!(css.contains("height:auto;"), "{css}");
        assert!(css.contains("max-width:100%;"), "{css}");
        assert!(!css.contains("transform"), "nothing to rotate: {css}");
    }

    #[test]
    fn rotation_and_mirroring_survive() {
        let mut run = bullet_run("word/media/image1.png");
        run.rotation = Some(90.0);
        run.flip_h = true;
        let css = image_style(&run);
        assert!(
            css.contains("transform:rotate(90.00deg) scale(-1,1);"),
            "{css}"
        );

        // A rotation Word wrote as zero is not a transform.
        let mut flat = bullet_run("word/media/image1.png");
        flat.rotation = Some(0.0);
        assert!(!image_style(&flat).contains("transform"));
    }

    #[test]
    fn the_vector_original_is_preferred_over_the_raster_fallback() {
        let mut run = bullet_run("word/media/image1.png");
        run.mime_type = Some("image/png".into());
        run.svg_image_path = Some("word/media/image1.svg".into());
        assert_eq!(
            picture_of(&run),
            Some(("word/media/image1.svg", "image/svg+xml")),
            "the svg scales, the png blurs"
        );
    }

    #[test]
    fn only_formats_a_browser_can_decode_are_drawn() {
        assert!(is_drawable("image/png"));
        assert!(is_drawable("IMAGE/JPEG"));
        assert!(is_drawable("image/svg+xml"));
        // Word embeds these from Windows, and no browser has ever shown one.
        assert!(!is_drawable("image/x-emf"));
        assert!(!is_drawable("image/x-wmf"));
        assert!(!is_drawable("image/tiff"));
        assert!(!is_drawable(""));
    }

    /// One file, many runs: the bytes are stored once and shared, not copied
    /// per reference. A megabyte logo drawn on twenty pages is a megabyte.
    #[test]
    fn a_repeated_picture_is_stored_once() {
        let mut blocks = vec![
            Block::Paragraph(Paragraph {
                runs: vec![bullet_run("word/media/image1.jpeg"), Run::default()],
                ..Default::default()
            }),
            Block::Table(Table {
                rows: vec![Row {
                    cells: vec![Cell {
                        content: vec![Block::Paragraph(Paragraph {
                            runs: vec![bullet_run("word/media/image1.jpeg")],
                            ..Default::default()
                        })],
                        col_span: 1,
                        v_merge: None,
                        background: None,
                        width_pt: None,
                    }],
                    is_header: false,
                    cant_split: false,
                    row_height: None,
                }],
            }),
        ];

        let mut images = Images::new();
        images.insert(
            "word/media/image1.jpeg".into(),
            std::rc::Rc::from("data:image/jpeg;base64,AAAA"),
        );
        attach_images(&mut blocks, &images);

        let Block::Paragraph(p) = &blocks[0] else {
            panic!()
        };
        let Block::Table(t) = &blocks[1] else {
            panic!()
        };
        let Block::Paragraph(nested) = &t.rows[0].cells[0].content[0] else {
            panic!()
        };
        // Reached inside a table cell, which is where the walk has to recurse.
        assert_eq!(
            nested.runs[0].src.as_deref(),
            Some("data:image/jpeg;base64,AAAA")
        );
        assert!(
            std::rc::Rc::ptr_eq(
                p.runs[0].src.as_ref().unwrap(),
                nested.runs[0].src.as_ref().unwrap()
            ),
            "both references share one buffer"
        );
        // A run that draws no picture is untouched.
        assert!(p.runs[1].src.is_none());
    }

    /// A picture the package does not contain, or one in a format that cannot
    /// be drawn, leaves `src` empty and renders as nothing. Better than a
    /// broken-image icon, and the gap notice says what is absent.
    #[test]
    fn a_missing_or_undrawable_picture_is_left_empty() {
        let mut blocks = vec![Block::Paragraph(Paragraph {
            runs: vec![bullet_run("word/media/gone.jpeg")],
            ..Default::default()
        })];
        attach_images(&mut blocks, &Images::new());
        let Block::Paragraph(p) = &blocks[0] else {
            panic!()
        };
        assert!(p.runs[0].src.is_none());
    }

    /// Reading pictures out of a real zip, built here so the test owns its input.
    #[test]
    fn pictures_are_read_out_of_the_package() {
        use std::io::Write;
        // A one-pixel PNG, so the bytes are a real image and not a placeholder.
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89,
        ];
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            zip.start_file("word/media/image1.png", opts).unwrap();
            zip.write_all(PNG).unwrap();
            zip.start_file("word/media/logo.emf", opts).unwrap();
            zip.write_all(b"not a browser format").unwrap();
            zip.finish().unwrap();
        }
        let package = buf.into_inner();

        let mut png = bullet_run("word/media/image1.png");
        png.mime_type = Some("image/png".into());
        let mut emf = bullet_run("word/media/logo.emf");
        emf.mime_type = Some("image/x-emf".into());
        let blocks = vec![Block::Paragraph(Paragraph {
            // The same png twice: it must be read and encoded once.
            runs: vec![png.clone(), png, emf],
            ..Default::default()
        })];

        let images = collect_images(&blocks, &package);
        assert_eq!(images.len(), 1, "the emf is not embedded: {images:?}");
        let url = images.get("word/media/image1.png").unwrap();
        assert!(
            url.starts_with("data:image/png;base64,iVBORw0KGgo"),
            "{url}"
        );
    }

    /// Reported: six bullets that sit at one level in the document rendered at
    /// two. Four of them carry a hanging indent — `indentLeft` 32.75 with
    /// `indentFirst` -18.05 — and the other two are written flat at 14.75. The
    /// first line of every one of them starts in the same place, and rendering
    /// only `indentLeft` moved four of them 18pt right. These are the values
    /// the parser gave for `Strategi 2030.docx`.
    /// Reported: too much space between the bullets. The document sets
    /// `spaceAfter` to zero on all 44 of its paragraphs and separates them with
    /// blank ones instead, so the stylesheet's own margin was a second gap on
    /// top of every blank line. These are its real values.
    /// Word localises its style names, and this wiki is Danish. The title of
    /// the document this was found in is styled `Titel`, which matched nothing
    /// and rendered as an ordinary paragraph.
    #[test]
    fn a_heading_is_recognised_in_the_language_it_was_written_in() {
        assert_eq!(heading_level(Some("Titel"), None), Some(1), "Danish title");
        assert_eq!(heading_level(Some("Overskrift1"), None), Some(1));
        assert_eq!(heading_level(Some("Overskrift3"), None), Some(3));
        assert_eq!(
            heading_level(Some("Titre"), None),
            Some(1),
            "French, no level"
        );
        assert_eq!(
            heading_level(Some("Titre 2"), None),
            Some(2),
            "French, level"
        );
        assert_eq!(heading_level(Some("Rubrik 1"), None), Some(1), "Swedish");

        // Body text is not a heading in any language, and falls through to the
        // outline level, which is where a document without named styles says so.
        assert_eq!(heading_level(Some("Brdtekst"), None), None);
        assert_eq!(heading_level(Some("Normal"), None), None);
        assert_eq!(heading_level(Some("Brdtekst"), Some(1)), Some(2));
    }

    /// Reported: headings did not look like the document's headings. Both
    /// `Titel` (20pt) and `Overskrift1` (11pt) are outline level 0, so both
    /// become an `<h1>`, and rendering both at the app's `<h1>` size made them
    /// identical — losing the hierarchy — and made the 11pt one three times the
    /// size it has in the document. The numbers here are that document's.
    #[test]
    fn a_heading_is_sized_against_the_documents_own_body_text() {
        let text = |pt: f64, bold: bool, words: &str| Run {
            text: words.into(),
            bold,
            font_size: Some(pt),
            ..Default::default()
        };
        let mut blocks = vec![
            Block::Paragraph(Paragraph {
                style_id: Some("Titel".into()),
                runs: vec![text(20.0, true, "Strategi 2030")],
                ..Default::default()
            }),
            Block::Paragraph(Paragraph {
                style_id: Some("Overskrift1".into()),
                outline_level: Some(0),
                runs: vec![text(11.0, true, "Radikal Ungdom skal være nytænkende.")],
                ..Default::default()
            }),
            Block::Paragraph(Paragraph {
                style_id: Some("Brdtekst".into()),
                runs: vec![text(
                    11.0,
                    false,
                    "Radikal Ungdom skal fortsætte med at udvikle sig.",
                )],
                ..Default::default()
            }),
        ];
        scale_headings(&mut blocks);

        let scale = |b: &Block| match b {
            Block::Paragraph(p) => p.heading_scale,
            _ => None,
        };
        assert_eq!(
            scale(&blocks[0]),
            Some(20.0 / 11.0),
            "the title, 20pt on 11pt"
        );
        assert_eq!(
            scale(&blocks[1]),
            Some(1.0),
            "bold, and no bigger, as in Word"
        );
        assert_eq!(scale(&blocks[2]), None, "body text is not a heading");

        // And it reaches the style, in em, so the reader's own size still rules.
        let Block::Paragraph(title) = &blocks[0] else {
            panic!()
        };
        assert!(paragraph_style(title).contains("font-size:1.818em;"));
        let Block::Paragraph(body) = &blocks[2] else {
            panic!()
        };
        assert!(!paragraph_style(body).contains("font-size"));
    }

    #[test]
    fn a_heading_is_never_smaller_than_its_body_and_never_absurd() {
        let text = |pt: f64, words: &str| Run {
            text: words.into(),
            font_size: Some(pt),
            ..Default::default()
        };
        let mut blocks = vec![
            Block::Paragraph(Paragraph {
                style_id: Some("Heading1".into()),
                runs: vec![text(8.0, "A small heading")],
                ..Default::default()
            }),
            Block::Paragraph(Paragraph {
                style_id: Some("Heading2".into()),
                runs: vec![text(96.0, "A poster")],
                ..Default::default()
            }),
            Block::Paragraph(Paragraph {
                runs: vec![text(
                    12.0,
                    "Body text, which is most of the characters here.",
                )],
                ..Default::default()
            }),
        ];
        scale_headings(&mut blocks);
        let scale = |b: &Block| match b {
            Block::Paragraph(p) => p.heading_scale,
            _ => None,
        };
        // The tag has promised a reader this is a heading.
        assert_eq!(
            scale(&blocks[0]),
            Some(1.0),
            "smaller than body, clamped up"
        );
        assert_eq!(
            scale(&blocks[1]),
            Some(2.5),
            "eight times body, clamped down"
        );
    }

    /// A document that never states a size gets the app's own type scale, which
    /// is the sensible thing to fall back to and what OpenDocument gets.
    #[test]
    fn a_document_without_sizes_keeps_the_apps_scale() {
        let mut blocks = vec![
            Block::Paragraph(Paragraph {
                style_id: Some("Heading1".into()),
                runs: vec![Run {
                    text: "Dagsorden".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            Block::Paragraph(Paragraph {
                runs: vec![Run {
                    text: "Velkomst".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        ];
        scale_headings(&mut blocks);
        let Block::Paragraph(h) = &blocks[0] else {
            panic!()
        };
        assert_eq!(h.heading_scale, None);
        assert!(!paragraph_style(h).contains("font-size"));
    }

    /// The run shape carrying a size, as the parser writes it.
    #[test]
    fn a_real_sized_run_deserialises() {
        let json = r#"{
            "type": "text",
            "bold": true,
            "color": "0e4660",
            "fontFamily": "Gill Sans MT",
            "fontSize": 20.0,
            "text": "Strategi 2030"
        }"#;
        let run: Run = serde_json::from_str(json).expect("the parser's own output");
        assert_eq!(run.font_size, Some(20.0));
        assert!(run.bold);
        assert!(run_style(&run).contains("color:#0e4660;"));
    }

    #[test]
    fn the_document_decides_the_space_between_its_paragraphs() {
        let tight = Paragraph {
            space_before: Some(0.05),
            space_after: Some(0.0),
            ..Default::default()
        };
        let css = paragraph_style(&tight);
        assert!(css.contains("margin-top:0.00rem;"), "{css}");
        assert!(css.contains("margin-bottom:0.00rem;"), "{css}");

        let roomy = Paragraph {
            space_before: Some(16.35),
            space_after: Some(8.0),
            ..Default::default()
        };
        let css = paragraph_style(&roomy);
        assert!(css.contains("margin-top:1.02rem;"), "{css}");
        assert!(css.contains("margin-bottom:0.50rem;"), "{css}");

        // A paragraph that says nothing keeps the stylesheet's own margin.
        assert!(!paragraph_style(&Paragraph::default()).contains("margin-top"));
        assert!(!paragraph_style(&Paragraph::default()).contains("margin-bottom"));
    }

    /// A heading is rendered in the app's type scale, so its rhythm is the
    /// scale's. This document's Heading 1 asks for zero space on both sides.
    #[test]
    fn a_heading_keeps_the_apps_rhythm_not_words_points() {
        let heading = Paragraph {
            style_id: Some("Overskrift1".into()),
            outline_level: Some(0),
            space_before: Some(0.0),
            space_after: Some(0.0),
            ..Default::default()
        };
        let css = paragraph_style(&heading);
        assert!(!css.contains("margin-top"), "{css}");
        assert!(!css.contains("margin-bottom"), "{css}");
    }

    #[test]
    fn line_spacing_carries_over_only_as_a_multiplier() {
        let one_and_a_half = Paragraph {
            line_spacing: Some(LineSpacing {
                value: 1.5,
                rule: "auto".into(),
            }),
            ..Default::default()
        };
        assert!(paragraph_style(&one_and_a_half).contains("line-height:1.50;"));

        // `exact` is a measurement for a fixed page. A line that wraps
        // differently here would be clipped by it.
        let exact = Paragraph {
            line_spacing: Some(LineSpacing {
                value: 12.0,
                rule: "exact".into(),
            }),
            ..Default::default()
        };
        assert!(!paragraph_style(&exact).contains("line-height"));
    }

    /// Paragraph spacing as the parser writes it, from the reported document.
    #[test]
    fn real_paragraph_spacing_deserialises() {
        let json = r#"{
            "type": "paragraph",
            "spaceBefore": 0.05,
            "spaceAfter": 0.0,
            "lineSpacing": {"explicit": true, "rule": "auto", "value": 1.5},
            "styleId": "Brdtekst",
            "runs": []
        }"#;
        let block: Block = serde_json::from_str(json).expect("the parser's own output");
        let Block::Paragraph(p) = &block else {
            panic!()
        };
        assert_eq!(p.space_after, Some(0.0));
        assert_eq!(p.space_before, Some(0.05));
        assert_eq!(p.line_spacing.as_ref().unwrap().rule, "auto");
        assert_eq!(p.line_spacing.as_ref().unwrap().value, 1.5);
    }

    #[test]
    fn a_page_ends_between_a_paragraphs_lines_but_leaves_two_on_each_side() {
        // Six lines of twenty pixels, on a page that ends at 95: four fit.
        let lines: Vec<f64> = (0..6).map(|i| i as f64 * 20.0).collect();
        assert_eq!(splits_between_lines(&lines, 0.0, 95.0), Some(80.0));
        // One line left over is a widow, and Word does not leave one: the
        // whole paragraph goes rather than its last line travelling alone.
        assert_eq!(splits_between_lines(&lines, 0.0, 115.0), None);
        // One line fitting is an orphan, and the same answer.
        assert_eq!(splits_between_lines(&lines, 0.0, 35.0), None);
        // A short paragraph can never satisfy two on each side.
        let three: Vec<f64> = (0..3).map(|i| i as f64 * 20.0).collect();
        assert_eq!(splits_between_lines(&three, 0.0, 45.0), None);
        // Nothing to split where the lines could not be read (a table row).
        assert_eq!(splits_between_lines(&[], 0.0, 500.0), None);
        // And it splits the same way part-way down a document.
        assert_eq!(
            splits_between_lines(&(0..6).map(|i| 500.0 + i as f64 * 20.0).collect::<Vec<_>>(), 500.0, 95.0),
            Some(580.0)
        );
    }

    #[test]
    fn each_face_is_measured_in_the_one_cut_to_its_widths() {
        // Metric-compatible substitutes, one per family of widths. Cambria is
        // NOT Times: it is the wider face, and measuring one in the other put
        // five list items too many on a page.
        assert_eq!(Measured::of(Some("Cambria")).family(), "Caladea");
        assert_eq!(Measured::of(Some("cambria")).family(), "Caladea");
        assert_eq!(Measured::of(Some("Times New Roman")).family(), "Liberation Serif");
        assert_eq!(Measured::of(Some("Georgia")).family(), "Liberation Serif");
        assert_eq!(Measured::of(Some("Calibri")).family(), "Carlito");
        // Arial is not Calibri: it is about a tenth wider, which is a line
        // every few paragraphs and a table row every few rows.
        assert_eq!(Measured::of(Some("Arial")).family(), "Liberation Sans");
        assert_eq!(Measured::of(Some("helvetica")).family(), "Liberation Sans");
        assert_ne!(
            Measured::of(Some("Arial")).single(),
            Measured::of(Some("Calibri")).single()
        );
        // Word's own default for a face it does not know.
        assert_eq!(Measured::of(Some("Papyrus")).family(), "Carlito");
        assert_eq!(Measured::of(None).family(), "Carlito");
        // Each reads its line box from its own property, since what SINGLE
        // spacing means belongs to the face.
        assert_eq!(Measured::of(Some("Cambria")).single(), "var(--single-cambria)");
        assert_ne!(
            Measured::of(Some("Cambria")).single(),
            Measured::of(Some("Times")).single()
        );
        // The substitute comes FIRST in the stack, so the measurement does not
        // depend on what the reader happens to have installed.
        assert!(measuring_face(Some("Cambria")).starts_with("'Caladea', 'Cambria'"));
    }

    #[test]
    fn a_hanging_indent_leaves_the_first_line_where_its_neighbours_are() {
        let hanging = Paragraph {
            indent_left: Some(32.75),
            indent_first: Some(-18.05),
            ..Default::default()
        };
        let flat = Paragraph {
            indent_left: Some(14.75),
            ..Default::default()
        };
        let css = paragraph_style(&hanging);
        assert!(css.contains("margin-left:2.05rem;"), "{css}");
        assert!(css.contains("text-indent:-1.13rem;"), "{css}");

        // The point of the whole fix: both first lines land together.
        let start_of =
            |p: &Paragraph| (p.indent_left.unwrap_or(0.0) + p.indent_first.unwrap_or(0.0)) / 16.0;
        assert!(
            (start_of(&hanging) - start_of(&flat)).abs() < 0.01,
            "{} vs {}",
            start_of(&hanging),
            start_of(&flat)
        );
        assert_eq!(visible(&paragraph_style(&flat)), "margin-left:0.92rem;");
    }

    #[test]
    fn a_first_line_indent_can_also_be_positive_and_never_escapes_left() {
        // Ordinary prose: the first line pushed in, the rest flush.
        let prose = Paragraph {
            indent_first: Some(16.0),
            ..Default::default()
        };
        assert_eq!(visible(&paragraph_style(&prose)), "text-indent:1.00rem;");

        // A hanging indent deeper than the margin would put the first line
        // outside the document, where it would be clipped.
        let overhang = Paragraph {
            indent_left: Some(8.0),
            indent_first: Some(-40.0),
            ..Default::default()
        };
        let css = paragraph_style(&overhang);
        assert!(css.contains("margin-left:0.50rem;"), "{css}");
        assert!(css.contains("text-indent:-0.50rem;"), "clamped: {css}");
    }

    /// The paragraph shape as the parser writes it, indents included.
    #[test]
    fn a_real_hanging_paragraph_deserialises() {
        let json = r#"{
            "type": "paragraph",
            "alignment": "left",
            "indentFirst": -18.05,
            "indentLeft": 32.75,
            "indentRight": 4.0,
            "runs": [{"type":"text","text":"Have mindst 15 sunde lokalforeninger."}]
        }"#;
        let block: Block = serde_json::from_str(json).expect("the parser's own output");
        let Block::Paragraph(p) = &block else {
            panic!("expected a paragraph")
        };
        assert_eq!(p.indent_left, Some(32.75));
        assert_eq!(p.indent_first, Some(-18.05));
        assert!(paragraph_style(p).contains("text-indent:-1.13rem;"));
    }

    #[test]
    fn alignment_and_indent_carry_over() {
        let centred = Paragraph {
            alignment: Some("center".into()),
            ..Default::default()
        };
        assert_eq!(visible(&paragraph_style(&centred)), "text-align:center;");
        let right = Paragraph {
            alignment: Some("right".into()),
            ..Default::default()
        };
        assert_eq!(visible(&paragraph_style(&right)), "text-align:right;");
        let indented = Paragraph {
            indent_left: Some(32.0),
            ..Default::default()
        };
        assert_eq!(visible(&paragraph_style(&indented)), "margin-left:2.00rem;");
    }

    /// Reported: a left-aligned document rendered centred. Left used to emit
    /// nothing, on the theory that it was the default — but the file viewer
    /// around the document centres its contents, so saying nothing meant
    /// inheriting centre. An explicit alignment is now always explicit.
    #[test]
    fn left_alignment_is_stated_rather_than_assumed() {
        for value in ["left", "start"] {
            let p = Paragraph {
                alignment: Some(value.into()),
                ..Default::default()
            };
            assert_eq!(visible(&paragraph_style(&p)), "text-align:left;", "{value}");
        }
        // A paragraph that says nothing still says nothing ON SCREEN: the
        // container's `text-align: start` is what saves it, and inline noise on
        // every paragraph of every document is not worth the bytes. It does
        // carry what it resolves to for the measuring copy, which is inert
        // anywhere else.
        assert_eq!(visible(&paragraph_style(&Paragraph::default())), "");
    }

    #[test]
    fn a_bullet_is_unordered_and_everything_else_counts() {
        let bullet = Paragraph {
            numbering: Some(Box::new(Numbering {
                format: Some("bullet".into()),
                level: Some(0),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(list_kind(&bullet), Some(false));
        let decimal = Paragraph {
            numbering: Some(Box::new(Numbering {
                format: Some("decimal".into()),
                level: Some(0),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(list_kind(&decimal), Some(true));
        // No numbering at all is not a list.
        assert_eq!(list_kind(&Paragraph::default()), None);
    }

    /// The real wire shape, as the parser emits it: camelCase keys, a `type`
    /// tag, and far more fields than this deserialises.
    #[test]
    fn the_parsers_json_deserialises() {
        let json = r#"[
            {"type":"paragraph","styleId":"Heading1","outlineLevel":0,"alignment":"left",
             "runs":[{"text":"Dagsorden","bold":true,"italic":false,"color":"auto",
                      "__typographyAcquisition":{"caps":false}}],
             "indentLeft":0.0,"numbering":null,"spaceAfter":120.0},
            {"type":"table","rows":[{"isHeader":true,"cells":[
                {"colSpan":2,"vMerge":null,"content":[
                    {"type":"paragraph","runs":[{"text":"Punkt"}]}]},
                {"colSpan":1,"vMerge":"continue","content":[]}]}]},
            {"type":"sectionBreak"}
        ]"#;
        let blocks: Vec<Block> = serde_json::from_str(json).expect("the model must parse");
        assert_eq!(blocks.len(), 3);

        let Block::Paragraph(p) = &blocks[0] else {
            panic!("expected a paragraph")
        };
        assert_eq!(
            heading_level(p.style_id.as_deref(), p.outline_level),
            Some(1)
        );
        assert_eq!(p.runs[0].text, "Dagsorden");
        assert!(p.runs[0].bold);

        let Block::Table(t) = &blocks[1] else {
            panic!("expected a table")
        };
        assert!(t.rows[0].is_header);
        assert_eq!(t.rows[0].cells[0].col_span, 2);
        assert!(!is_merged_away(&t.rows[0].cells[0]));
        assert!(is_merged_away(&t.rows[0].cells[1]), "covered by a merge");
        // A cell holds blocks, so rendering recurses.
        assert_eq!(t.rows[0].cells[0].content.len(), 1);

        // An unknown block kind must not fail the whole document.
        assert_eq!(blocks[2], Block::Unknown);
    }
}

/// What a Word file says its pages are, in CSS pixels.
///
/// A `.docx` has no pagination in it. Word computes it when it lays the
/// document out, and writes the answer back only as a hint that is absent from
/// anything another editor saved -- of six real documents from this wiki, not
/// one had a single authored page break and only two carried Word's hint. So
/// where the pages fall has to be worked out, and the file does say enough to
/// work it out: how wide the text column is and how tall.
#[derive(Clone, Debug, PartialEq)]
pub struct PageGeometry {
    /// The text column: page width less both margins.
    pub column: f64,
    /// The text height: page height less the top and bottom margins.
    pub height: f64,
    /// What the document sets its body text in, so the measuring is done in
    /// the document's typography rather than the reader's.
    pub font: String,
    /// The body text's size in points, its line spacing as a multiplier, and
    /// the space it leaves under a paragraph in points -- all as the DOCUMENT
    /// sets them.
    ///
    /// Load-bearing. Without these the off-screen copy is laid out in the
    /// READER's typography, which this renderer deliberately makes relative to
    /// the reader rather than pinned to Word's, and it comes out taller: a
    /// three-page document measured as four, with every mark after the first
    /// shifted a page.
    pub size: f64,
    pub line: f64,
    pub after: f64,
    /// What a LIST item leaves under itself, which is nothing at all where the
    /// document closes its lists up.
    pub list_after: f64,
}

impl PageGeometry {
    /// Points to CSS pixels: 72 points to the inch, 96 pixels to the inch.
    fn px(points: f64) -> f64 {
        points * 96.0 / 72.0
    }

    /// What size the document sets its body text, how far apart its lines sit
    /// and how much room it leaves under a paragraph: whatever most of its
    /// paragraphs say, which is what a document's own defaults look like from
    /// the outside. Headings are left out of the vote; there are few of them
    /// and they are not what fills a page.
    /// What most of a document's paragraphs are set in, which is what to
    /// measure it in. A document that mixes faces is measured in its commonest;
    /// the alternative is a face per paragraph, and no document in this wiki
    /// needs that yet.
    fn face(blocks: &[Block]) -> Option<String> {
        let mut seen: Vec<(String, usize)> = Vec::new();
        for_each_paragraph(blocks, &mut |p| {
            let Some(name) = p
                .default_font_family
                .as_deref()
                .map(str::trim)
                .filter(|f| !f.is_empty())
            else {
                return;
            };
            match seen.iter_mut().find(|(f, _)| f == name) {
                Some((_, n)) => *n += 1,
                None => seen.push((name.to_string(), 1)),
            }
        });
        seen.into_iter().max_by_key(|(_, n)| *n).map(|(f, _)| f)
    }

    fn typography(blocks: &[Block]) -> (f64, f64, f64, f64) {
        let mut sizes: Vec<f64> = Vec::new();
        let mut lines: Vec<f64> = Vec::new();
        let mut afters: Vec<f64> = Vec::new();
        let mut closed_up = 0usize;
        let mut items = 0usize;
        // Every paragraph, INCLUDING the ones in table cells. Walking only the
        // top-level blocks took a six-table document's typography from the four
        // paragraphs that were not in a table, and measured all hundred and
        // thirty cell paragraphs in type the document does not use there.
        for_each_paragraph(blocks, &mut |p| {
            if p.outline_level.is_some_and(|l| l < 9) {
                return;
            }
            // The RESOLVED size, not the commonest one the runs happen to
            // state: see the field's own note.
            if let Some(pt) = p.default_font_size.or_else(|| dominant_size(p)) {
                sizes.push(pt);
            }
            // No line spacing stated IS a statement: it means single. Counting
            // only the paragraphs that state one takes the document's spacing
            // from whichever few paragraphs disagreed with it.
            match p.line_spacing.as_ref() {
                // `auto` states a multiplier in 240ths; the other rules state
                // points, which cannot be a multiplier without knowing the size.
                Some(spacing) if spacing.rule == "auto" && spacing.value > 0.0 => {
                    lines.push(spacing.value)
                }
                Some(_) => {}
                None => lines.push(1.0),
            }
            if let Some(pt) = p.space_after.filter(|a| *a >= 0.0) {
                afters.push(pt);
            }
            if p.numbering.is_some() {
                items += 1;
                if p.contextual_spacing {
                    closed_up += 1;
                }
            }
        });
        // Word's contextualSpacing closes a list up, and it is tempting to
        // read it off the paragraphs and drop the space under every item. That
        // was measured and it is wrong: the rule is PAIRWISE -- the space goes
        // only between adjacent paragraphs that share a style and both set it
        // -- and applying it wholesale took 8pt from each of one document's
        // sixty items, some six hundred points, and put its first page break
        // thirty paragraphs past where Word puts it. Until it is applied pair
        // by pair, a list leaves what every other paragraph leaves.
        let _ = (closed_up, items);
        let list_after = commonest(&afters).unwrap_or(8.0);
        // Word's own defaults, for a document that states none of this.
        //
        // The line multiplier is scaled on the way out. Word's `auto` rule
        // means "this many times SINGLE spacing", and single spacing is the
        // font's own line box -- around 1.22 times its size for the faces these
        // documents are set in. CSS `line-height: 1.08` means 1.08 times the
        // font SIZE, which is a fifth short of what Word lays out.
        (
            commonest(&sizes).unwrap_or(11.0),
            commonest(&lines).unwrap_or(1.0),
            commonest(&afters).unwrap_or(8.0),
            list_after,
        )
    }

    /// The geometry out of the parser's `section`, when it is usable. A page
    /// with no size, or margins wider than the paper, says nothing to measure
    /// against.
    pub fn read(section: &serde_json::Value, font: Option<&str>, blocks: &[Block]) -> Option<Self> {
        let at = |key: &str| section.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let column = Self::px(at("pageWidth") - at("marginLeft") - at("marginRight"));
        let height = Self::px(at("pageHeight") - at("marginTop") - at("marginBottom"));
        // A quarter of an A4 column, and a page that holds more than a line or
        // two: below either, something is wrong with what was read.
        let (size, line, after, list_after) = Self::typography(blocks);
        (column > 100.0 && height > 100.0).then(|| PageGeometry {
            column,
            height,
            size,
            line,
            after,
            list_after,
            // The document's own body face first, then the metric-compatible
            // substitutes a reader is likely to actually have. The measuring is
            // only as good as this: a face that is not installed is replaced by
            // one with different metrics, the lines wrap elsewhere, and the
            // pages drift.
            // The document's own face, through a substitute with the same
            // metrics that this app SHIPS -- so the measuring does not depend
            // on what the reader happens to have installed. Calibri and Times
            // New Roman are what these documents are set in, and Carlito and
            // Liberation Serif have their widths and line boxes exactly.
            font: measuring_face(Self::face(blocks).as_deref().or(font)),
        })
    }
}

/// Where each page of a Word document ends, worked out by laying it out at the
/// size the document says its pages are.
///
/// The document is rendered twice: once off-screen at its own text-column width
/// and typography, which is what gets measured, and once visibly in the
/// reader's own column with a mark wherever a page ended. The off-screen copy
/// is dropped as soon as it has been read.
///
/// What this is NOT is Word's own pagination. It is this browser's, at Word's
/// geometry, and the two agree only as far as the fonts do: a document set in a
/// face the reader does not have is laid out in a substitute, its lines wrap in
/// different places, and the difference accumulates down the document. Where
/// the file states no usable page size, nothing is marked at all.
#[component]
pub fn PagedDocx(
    /// Which document this is. Not drawn: it is what tells the reader that the
    /// document has CHANGED. Opening another file puts new props into the same
    /// component, and a signal outlives that, so without this the control kept
    /// the last document's page count and its marks stayed at those indices in
    /// the new text -- a three-page attachment followed by a nine-page one
    /// still read "3", with page breaks at paragraphs that had nothing to do
    /// with them.
    document: String,
    blocks: Vec<Block>,
    page: PageGeometry,
) -> Element {
    let mut marks = use_signal(Vec::<(usize, usize, usize, f64)>::new);
    let mut pages = use_signal(|| 0usize);
    // How much of the last page its text leaves empty, so the reading surface
    // can show that page as the mostly-blank sheet it is.
    let mut last_slack = use_signal(|| 0.0f64);
    // Whether the off-screen copy is still needed. It is a whole second render
    // of the document, so it goes as soon as it has been measured.
    let mut measuring = use_signal(|| true);

    let height = page.height;
    let asked_for = page.size;
    let face = page.font.clone();
    let (size, line, after, list_after) = (page.size, page.line, page.after, page.list_after);
    use_effect(use_reactive!(|(document,)| {
        let _ = &document;
        // A different document, so nothing worked out about the last one holds.
        // Cleared before the measuring starts rather than when it finishes: a
        // count from another file is worse on screen than no count at all.
        marks.set(Vec::new());
        pages.set(0);
        last_slack.set(0.0);
        measuring.set(true);
        // Not before the face this is measured in has arrived. A font is
        // fetched when something first renders in it, so on the first Word
        // document opened it is still on its way while this runs -- and
        // measuring in whatever the browser fell back to is measuring the wrong
        // document. `load` resolves immediately once it is there, so this costs
        // one turn of the event loop afterwards.
        let face = face.clone();
        spawn(async move {
            // After the render that puts the off-screen copy back: `measuring`
            // was just set, and what is in the DOM at this instant is still the
            // document that was open before.
            next_frame().await;
            // Every face this app measures in, because a document may use more
            // than one: its body in one and its lists in another.
            for measured in [
                Measured::Sans,
                Measured::Serif,
                Measured::Cambria,
                Measured::Arial,
            ] {
                wait_for_the_face(asked_for, measured.family()).await;
            }
            wait_for_the_face(asked_for, &face).await;
            // What SINGLE spacing means in each. Measured, not assumed:
            // Calibri's line box is about 1.22 times its size, Cambria's 1.17
            // and Times New Roman's 1.15, and a document states its line
            // spacing as a multiple of whichever its text is set in.
            set_single_spacing();
            // And until the page has been laid out IN those faces. A font's
            // load resolving is not the same as the text having been laid out
            // again in it: `fonts.ready` is the promise that says the loads are
            // done and the layout with them, and without waiting for it the
            // same document measured 7032px of ink on one visit and 7128 on the
            // next -- a page more, from one run to the next, on nothing but
            // timing.
            wait_for_the_layout().await;
            // Measured twice, a frame apart, and only believed when the two
            // agree. Everything above is what SHOULD settle a layout; this is
            // what proves it did.
            let mut settled = measure_pages(height);
            for _ in 0..4 {
                let Some((_, _, ink, _)) = settled else { break };
                next_frame().await;
                let again = measure_pages(height);
                match again {
                    Some((_, _, second, _)) if (second - ink).abs() < 0.5 => break,
                    _ => settled = again,
                }
            }
            let Some((found, count, ink, spare)) = settled else {
                return;
            };
            // What it made of the document, in the console. Pagination that
            // disagrees with Word is reported as "it says N pages", and without
            // this there is no way to tell a measurement that ran long from one
            // that never ran at all.
            log::info!(
                "word pagination: {count} pages, {} marks, pages {height:.0}px, \
                 {ink:.0}px of ink ({:.2} pages' worth), set in {size}pt/{line:.3} \
                 with {after}pt under a paragraph and {list_after}pt under a list item",
                found.len(),
                ink / height
            );
            marks.set(found);
            pages.set(count);
            last_slack.set(spare);
            measuring.set(false);
        });
    }));

    rsx! {
        if measuring() {
            div {
                id: "docx-measure",
                class: "docx-measure",
                aria_hidden: "true",
                style: "width:{page.column}px;font-family:{page.font};font-size:{page.size}pt;--docx-page-line:{page.line};--docx-page-after:{page.after}pt;--docx-page-list-after:{page.list_after}pt;",
                DocxBody { blocks: blocks.clone() }
            }
        }
        DocxBody { blocks, marks: marks() }
        if pages() > 1 {
            // The last page's own number, which no mark inside the document can
            // carry: every other one is drawn where a page ends and the next
            // begins.
            super::pager::LastPageMark { page: pages().to_string(), spare: last_slack() }
            super::pager::PageControl { first: "1".to_string(), last: pages().to_string() }
        }
    }
}

/// Read the off-screen copy: where each page ends, and how many pages there are.
///
/// The flow is walked at the finest granularity a mark can be placed at: the
/// blocks, and the ITEMS of a list. Word breaks a page wherever the text runs
/// out, which in these documents is usually inside a bulleted list -- sixty of
/// one document's sixty-eight paragraphs are list items -- and marking only
/// between blocks snapped every break back to where the list began, eight
/// paragraphs early against Word's own answer.
///
/// Two things a naive reading still gets wrong, both reported from real
/// documents. A Word file usually ends with a few empty paragraphs, and they
/// have height: counted, they spill past a boundary and the reader is told
/// about a last page with nothing on it, so the measuring stops at the last
/// thing with anything in it. And a single element taller than a page swallows
/// more than one boundary -- a table, whose rows cannot be marked between yet
/// -- so the page count is the number of places a reader can actually be TAKEN
/// to, plus the first. Counting the swallowed pages instead left a control that
/// named a page and then would not go there.
fn measure_pages(height: f64) -> Option<(Vec<(usize, usize, usize, f64)>, usize, f64, f64)> {
    if height <= 0.0 {
        return None;
    }
    let document = web_sys::window()?.document()?;
    let root = document.query_selector("#docx-measure > .docx").ok()??;
    let top_of = root.get_bounding_client_rect().top();
    let groups = document
        .query_selector_all("#docx-measure > .docx > *")
        .ok()?;
    if groups.length() == 0 {
        return None;
    }

    // Every place a mark could go, in order, with where it sits.
    struct Spot {
        group: usize,
        item: usize,
        top: f64,
        bottom: f64,
        /// Where its TEXT ends -- the bottom without the space under it. Word
        /// cuts that space at the foot of a page: what has to fit is the last
        /// line, not the gap after it. A page of thirty-two uniform paragraphs
        /// held one too few without this, every one of the eight points under
        /// the last of them counting against the page.
        ends: f64,
        empty: bool,
        /// Word keeps this with whatever follows it -- every heading does -- so
        /// a page cannot end between the two.
        keeps_next: bool,
        /// Whether Word carries the REST of this over the page rather than
        /// moving it whole. A table row does, unless the document says it may
        /// not.
        splits: bool,
        /// Where this element's own LINES sit, when it is text. Word breaks a
        /// paragraph between its lines rather than moving all of it down, and
        /// measuring it whole leaves the foot of every page empty by up to a
        /// paragraph -- which is a page in eight on a document of prose.
        lines: Vec<f64>,
    }
    let mut flow: Vec<Spot> = Vec::new();
    for at in 0..groups.length() {
        let Some(group) = groups
            .item(at)
            .and_then(|c| c.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        // A list is walked by item and a table by row: Word ends a page
        // wherever the text runs out, and in these documents that is nearly
        // always inside one or the other. Anything else is measured whole.
        let inside = match group.tag_name().as_str() {
            "UL" | "OL" => group.query_selector_all(":scope > li").ok(),
            _ => group
                .query_selector_all(":scope table > tbody > tr, :scope table > tr")
                .ok(),
        };
        match (true, inside) {
            (true, Some(items)) if items.length() > 0 => {
                for j in 0..items.length() {
                    let Some(item) = items
                        .item(j)
                        .and_then(|c| c.dyn_into::<web_sys::Element>().ok())
                    else {
                        continue;
                    };
                    let rect = item.get_bounding_client_rect();
                    flow.push(Spot {
                        group: at as usize,
                        item: j as usize,
                        top: rect.top() - top_of,
                        bottom: rect.bottom() - top_of,
                        ends: rect.bottom() - top_of - space_under(&item),
                        empty: is_blank(&item),
                        keeps_next: keeps_next(&item),
                        splits: item.tag_name() == "TR"
                            && item.get_attribute("data-cant-split").as_deref() != Some("true"),
                        lines: line_tops(&item, top_of),
                    });
                }
            }
            _ => {
                let rect = group.get_bounding_client_rect();
                flow.push(Spot {
                    group: at as usize,
                    item: BEFORE_GROUP,
                    top: rect.top() - top_of,
                    bottom: rect.bottom() - top_of,
                    ends: rect.bottom() - top_of - space_under(&group),
                    empty: is_blank(&group),
                    keeps_next: keeps_next(&group),
                    splits: false,
                    lines: line_tops(&group, top_of),
                });
            }
        }
    }

    // Where the content actually stops. Trailing empty paragraphs are a blank
    // page nobody wrote.
    let last = flow.iter().rposition(|spot| !spot.empty)?;

    // Filled a page at a time, not cut out of one long ribbon.
    //
    // The difference is what Word does at the bottom of a page: it moves a
    // paragraph WHOLE rather than leaving a line of it stranded, and a table
    // row the same. So a page ends early and the space under it is simply
    // unused -- which a continuous measurement never accounts for, and which is
    // why it read six tables as seven pages where Word makes eight, and put
    // every break in a prose document one paragraph late.
    //
    // Each page therefore starts where the element that would not fit starts.
    let mut marks: Vec<(usize, usize, usize, f64)> = Vec::new();
    let mut page_top = 0.0f64;
    let mut measured = 0.0f64;
    // Where the text on the page just ended, so the paper left under it can be
    // drawn. A page whose last element moved down whole ends early, and on the
    // page that is white space -- one document's last page holds a single
    // bulleted line and the rest of the sheet is empty.
    let mut ends_the_page = 0.0f64;
    for (at, spot) in flow.iter().take(last + 1).enumerate() {
        measured = spot.bottom;
        ends_the_page = spot.ends;
        // `top > page_top` keeps the first element of a page from starting
        // another one: something taller than a whole page has to sit on one.
        if spot.ends - page_top > height && spot.top > page_top {
            // A heading goes down with the text it introduces. Word keeps them
            // together, so the page ends ABOVE the heading, not between it and
            // its paragraph -- which is why a document whose sections all begin
            // with one came out a page ahead of Word from its third page on.
            let mut starts = at;
            while starts > 0 && flow[starts - 1].keeps_next && flow[starts - 1].top > page_top {
                starts -= 1;
            }
            let begin = &flow[starts];
            // A table ROW is the exception: Word carries the rest of it over
            // rather than moving the whole row, so the page ends where the page
            // ends and the row continues on the next one. Moving it whole left
            // the bottom of a page empty every time, which is a page in nine on
            // a document that is six tables. The mark still goes on the row,
            // because that is where the next page begins to be read -- and only
            // once, for a row tall enough to cross two boundaries.
            let carried = begin.splits && starts == at;
            // Or SPLIT between its own lines, which is what Word does with a
            // paragraph: it leaves as many lines as fit and carries the rest
            // over, keeping two on each side of the break (widow and orphan
            // control, which every one of these documents asks for). Moving the
            // whole paragraph instead left up to a paragraph of empty page at
            // every break -- a page in eight, on a document of prose.
            let split_at = match starts == at {
                true => splits_between_lines(&begin.lines, page_top, height),
                false => None,
            };
            // The paper left at the foot of the page that just ended: the page's
            // own bottom edge, less where its text actually stopped. Nothing,
            // where a row or a paragraph runs over the edge and carries the rest
            // to the next page -- that page is full. Everything from the last
            // line to the edge, where what would not fit moved down whole.
            let floor = page_top + height;
            let text_ended = match (carried, split_at) {
                (true, _) => floor,
                (false, Some(line)) => line,
                (false, None) => match starts.checked_sub(1).and_then(|i| flow.get(i)) {
                    Some(above) => above.ends,
                    // Nothing above it on this page: the element itself is taller
                    // than a page, and there is no paper to spare.
                    None => floor,
                },
            };
            let already = marks.last().is_some_and(|m| (m.0, m.1) == (begin.group, begin.item));
            if !already {
                marks.push((
                    begin.group,
                    begin.item,
                    marks.len() + 2,
                    (floor - text_ended).max(0.0),
                ));
            }
            page_top = match (carried, split_at) {
                (true, _) => page_top + height,
                (false, Some(line)) => line,
                (false, None) => begin.top,
            };
            if marks.len() > 2000 {
                return None;
            }
        }
    }
    // And the paper under the LAST page's text, which no mark inside the
    // document can carry: there is no page after it to be pushed down.
    let last_slack = (page_top + height - ends_the_page).max(0.0);
    // Nothing measured at all (fonts not settled, or an empty document): let
    // the effect try again rather than marking a one-page document.
    (measured > 0.0).then_some((marks.clone(), marks.len() + 1, measured, last_slack))
}

/// Wait until the document has been laid out in the faces it just loaded.
///
/// `document.fonts.ready` resolves when font loading is finished AND the layout
/// that depends on it has been redone. A face whose `load` has resolved is in
/// memory but not necessarily on the page yet: measuring between the two reads
/// the fallback's geometry for some of the document and the real face's for the
/// rest, which is how one document came out eight pages on one visit and nine
/// on the next.
async fn wait_for_the_layout() {
    let Some(fonts) = web_sys::window()
        .and_then(|w| w.document())
        .map(|d| d.fonts())
    else {
        return;
    };
    if let Ok(ready) = fonts.ready() {
        let _ = wasm_bindgen_futures::JsFuture::from(ready).await;
    }
    next_frame().await;
}

/// One turn of the browser's rendering loop.
async fn next_frame() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = window.request_animation_frame(&resolve);
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Wait until the measuring face is loaded, or give up on it.
///
/// `document.fonts.load` both fetches and resolves; it answers straight away
/// once the face is in. Anything that goes wrong here is not worth failing the
/// document for -- the measurement simply happens in whatever the browser has,
/// which is what it did before this font was shipped.
async fn wait_for_the_face(size: f64, face: &str) {
    // The FIRST family, not the stack. `fonts.load` is satisfied by any family
    // in a list that already resolves -- the generic at the end always does --
    // so asking for the stack returns at once and the face this is measured in
    // is still on its way. A Calibri document then measured with the fallback's
    // line box and lost a page.
    let face = first_family(face);
    let Some(fonts) = web_sys::window()
        .and_then(|w| w.document())
        .map(|d| d.fonts())
    else {
        return;
    };
    let _ = wasm_bindgen_futures::JsFuture::from(fonts.load(&format!("{size}pt {face}"))).await;
}

/// The first family named in a font stack, which is the one that decides how
/// the text is laid out when it is there.
fn first_family(stack: &str) -> String {
    stack
        .split(',')
        .next()
        .unwrap_or(stack)
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string()
}

/// The line box of a face, as a multiple of its size: what Word calls SINGLE
/// spacing. Measured by laying one line out in it and asking how tall the line
/// came out, which is the only way to know it for a face rather than assume it.
fn single_spacing(face: &str) -> f64 {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return SINGLE_SPACING;
    };
    // The face itself, quoted, rather than the stack: measuring the stack
    // measures whichever member happens to be resolvable, and the generic at
    // the end always is.
    let face = format!("'{}'", first_family(face));
    let Ok(probe) = document.create_element("div") else {
        return SINGLE_SPACING;
    };
    let _ = probe.set_attribute(
        "style",
        &format!(
            "position:absolute;left:-9999px;top:0;visibility:hidden;\
             font-family:{face};font-size:100px;line-height:normal;white-space:nowrap;"
        ),
    );
    probe.set_text_content(Some("Hxg"));
    let Some(body) = document.body() else {
        return SINGLE_SPACING;
    };
    if body.append_child(&probe).is_err() {
        return SINGLE_SPACING;
    }
    let tall = probe.get_bounding_client_rect().height() / 100.0;
    let _ = body.remove_child(&probe);
    // A face that answered something absurd is not worth believing.
    match (1.0..=2.0).contains(&tall) {
        true => tall,
        false => SINGLE_SPACING,
    }
}

/// Tell the stylesheet what single spacing is for this document, so the line
/// height every paragraph states can be scaled by it.
fn set_single_spacing() {
    let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    else {
        return;
    };
    let Ok(root) = root.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let style = root.style();
    for measured in [
        Measured::Sans,
        Measured::Serif,
        Measured::Cambria,
        Measured::Arial,
    ] {
        // `var(--single-sans)` back to `--single-sans`: the property is named
        // once, where it is read.
        let name = measured.single().trim_start_matches("var(").trim_end_matches(')');
        let _ = style.set_property(name, &format!("{:.4}", single_spacing(measured.family())));
    }
}

/// Where each of an element's LINES begins, in the measuring copy's own
/// coordinates.
///
/// From the element's own line height rather than by asking for its line boxes:
/// the browser will hand those over one rectangle at a time through a range,
/// but the binding is not in this build, and a paragraph whose lines are all
/// the same height needs no such help. Word breaks a page between two lines, so
/// without knowing where they are the whole paragraph has to move and the foot
/// of the page is left empty.
///
/// Empty where the arithmetic cannot be trusted: a table row, whose cells sit
/// side by side and have no shared lines, and a paragraph carrying a run bigger
/// than its own text, whose lines are not all one height.
fn line_tops(element: &web_sys::Element, top_of: f64) -> Vec<f64> {
    if element.tag_name() == "TR" || element.query_selector("table").ok().flatten().is_some() {
        return Vec::new();
    }
    let Some(window) = web_sys::window() else {
        return Vec::new();
    };
    let Ok(Some(style)) = window.get_computed_style(element) else {
        return Vec::new();
    };
    let px = |name: &str| {
        style
            .get_property_value(name)
            .ok()
            .and_then(|v| v.trim().trim_end_matches("px").parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let line = px("line-height");
    if line <= 1.0 {
        return Vec::new();
    }
    let (over, under) = (px("padding-top"), px("padding-bottom"));
    let rect = element.get_bounding_client_rect();
    let text_top = rect.top() - top_of + over;
    let text_height = rect.height() - over - under;
    let count = (text_height / line).round();
    if count < 1.0 || (count * line - text_height).abs() > 1.5 {
        return Vec::new();
    }
    (0..count as usize)
        .map(|i| text_top + i as f64 * line)
        .collect()
}

/// Where a page ends INSIDE a paragraph: the top of the first line that does
/// not fit, or nothing when the paragraph should move whole.
///
/// Word keeps two lines on each side of the break -- one line stranded at the
/// foot of a page, or arriving alone at the head of the next, is what widow and
/// orphan control exists to prevent, and every document in this wiki asks for
/// it. A paragraph of three lines or fewer can never satisfy that, so it moves.
fn splits_between_lines(lines: &[f64], page_top: f64, height: f64) -> Option<f64> {
    if lines.len() < 4 {
        return None;
    }
    let bottom = page_top + height;
    // A line fits when the WHOLE of it does. Counting the ones that merely
    // begin above the boundary puts a line half on each page, which is not
    // something a page can do.
    let tall = lines[1] - lines[0];
    let fits = lines.iter().filter(|top| **top + tall <= bottom).count();
    let left = lines.len() - fits;
    (fits >= 2 && left >= 2).then(|| lines[fits])
}

/// The space UNDER an element, which the measuring copy carries as padding so
/// that it adds to the space over the next one the way Word's does rather than
/// collapsing into it.
///
/// Read back off here because a page's last paragraph does not have to make
/// room for it: Word puts the paragraph on the page if its lines fit and cuts
/// the space at the page edge.
fn space_under(element: &web_sys::Element) -> f64 {
    web_sys::window()
        .and_then(|w| w.get_computed_style(element).ok().flatten())
        .and_then(|style| style.get_property_value("padding-bottom").ok())
        .and_then(|value| value.trim_end_matches("px").parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Whether Word keeps this element on the same page as the one after it.
fn keeps_next(element: &web_sys::Element) -> bool {
    element.get_attribute("data-keep-next").as_deref() == Some("true")
}

/// Whether an element holds no words. `textContent`, NOT `innerText`: the
/// latter is what the browser RENDERS, and this copy is hidden, so it answers
/// empty for everything and the whole measurement gives up. That shipped once
/// and left every Word document with no page marks at all.
fn is_blank(element: &web_sys::Element) -> bool {
    element.text_content().unwrap_or_default().trim().is_empty()
}

/// One of the three faces this app measures documents in.
///
/// Each is metric-compatible with a face Word is set in -- the same widths for
/// the same text -- so a line wraps where Word wraps it whatever the reader has
/// installed. Which one matters: a Cambria document measured in Times metrics
/// held five list items too many on a page, because Times is the narrower face.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Measured {
    /// Carlito, for Calibri and for anything unrecognised -- Word's own default.
    Sans,
    /// Liberation Serif, for Times New Roman and the faces cut to its widths.
    Serif,
    /// Caladea, for Cambria.
    Cambria,
    /// Liberation Sans, for Arial and Helvetica. NOT Calibri's: Arial is the
    /// wider face by about a tenth, so an Arial document measured in Carlito
    /// wraps later than Word wraps it and every table row comes out short.
    Arial,
}

impl Measured {
    /// Which face a document's named one is measured in.
    pub fn of(named: Option<&str>) -> Self {
        let name = named.map(str::trim).unwrap_or("");
        match name {
            n if n.eq_ignore_ascii_case("cambria") => Self::Cambria,
            n if ["arial", "helvetica", "liberation sans"]
                .iter()
                .any(|sans| n.eq_ignore_ascii_case(sans)) =>
            {
                Self::Arial
            }
            n if ["times new roman", "times", "georgia", "garamond"]
                .iter()
                .any(|serif| n.eq_ignore_ascii_case(serif)) =>
            {
                Self::Serif
            }
            _ => Self::Sans,
        }
    }

    /// The family to load and to measure. The substitute itself, never a stack:
    /// `fonts.load` is satisfied by any family in a list that already resolves,
    /// and the generic at the end always does.
    pub fn family(self) -> &'static str {
        match self {
            Self::Sans => "Carlito",
            Self::Serif => "Liberation Serif",
            Self::Cambria => "Caladea",
            Self::Arial => "Liberation Sans",
        }
    }

    /// The custom property holding what SINGLE spacing means in it, measured at
    /// runtime from the face itself.
    pub fn single(self) -> &'static str {
        match self {
            Self::Sans => "var(--single-sans)",
            Self::Serif => "var(--single-serif)",
            Self::Cambria => "var(--single-cambria)",
            Self::Arial => "var(--single-arial)",
        }
    }
}

/// The stack to measure a document set in `named` with.
///
/// A metric-compatible substitute FIRST, and one this app ships, so the answer
/// does not depend on what the reader has installed -- then the document's own
/// name, for a reader who does have it, then the generic.
fn measuring_face(named: Option<&str>) -> String {
    let name = named.map(str::trim).unwrap_or("");
    let face = Measured::of(named);
    let generic = match face {
        Measured::Sans => "Calibri, sans-serif",
        Measured::Arial => "Arial, sans-serif",
        Measured::Serif | Measured::Cambria => "serif",
    };
    match name.is_empty() {
        true => format!("{}, {generic}", face.family()),
        false => format!("'{}', '{name}', {generic}", face.family()),
    }
}

/// What Word means by SINGLE line spacing: the FONT's own line box, not a
/// number. Calibri's is about 1.22 times its size and Times New Roman's about
/// 1.15, and CSS counts line height from the size instead -- so every
/// multiplier a document states has to be scaled by the line box of the face
/// it is actually set in. Measured from the face rather than assumed: assuming
/// Calibri's for a Times document made it a tenth too tall.
///
/// This is the fallback for when the measuring cannot be done at all.
const SINGLE_SPACING: f64 = 1.22;

/// The value that turns up most often, to the nearest hundredth. What "the
/// document's own" means for a measure it states per paragraph rather than once.
fn commonest(values: &[f64]) -> Option<f64> {
    let mut tally: Vec<(f64, usize)> = Vec::new();
    for value in values {
        match tally.iter_mut().find(|(v, _)| (*v - value).abs() < 0.01) {
            Some((_, seen)) => *seen += 1,
            None => tally.push((*value, 1)),
        }
    }
    tally
        .into_iter()
        .max_by_key(|(_, seen)| *seen)
        .map(|(v, _)| v)
}

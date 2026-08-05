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
}

/// A cell's shading, as CSS.
///
/// `auto` means "whatever the reader's theme says", exactly as it does on a
/// run's colour, so it must NOT become a colour — a document that says `auto`
/// on a dark theme wants the dark background, not a white one painted over it.
pub fn cell_style(cell: &Cell) -> String {
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
    marks: Vec<(usize, usize, usize)>,
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
            .find(|(g, i, _)| *g == group && *i == item)
            .map(|(_, _, begins)| *begins)
    };

    rsx! {
        div { class: "docx",
            for (i , group) in groups.into_iter().enumerate() {
                // Where a page ends, when someone has worked out where that is
                // (see `PagedDocx`). Empty for a cell's contents and for a
                // document nobody paginated, which is most of them.
                if let Some(begins) = split_at(i, BEFORE_GROUP) {
                    PageMark { key: "p{i}", begins }
                }
                match group {
                    Group::Single(block) => rsx! {
                        DocxBlock { key: "b{i}", block }
                    },
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
                        let mut runs: Vec<(Option<usize>, Vec<Paragraph>)> = vec![(None, Vec::new())];
                        for (j, item) in items.into_iter().enumerate() {
                            match split_at(i, j) {
                                Some(begins) if j > 0 => runs.push((Some(begins), vec![item])),
                                _ => runs.last_mut().expect("seeded above").1.push(item),
                            }
                        }
                        rsx! {
                            for (r , (begins , run)) in runs.into_iter().enumerate() {
                                if let Some(begins) = begins {
                                    PageMark { key: "lp{i}-{r}", begins }
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
fn PageMark(begins: usize) -> Element {
    rsx! {
        div {
            class: "pdf-page-break",
            role: "separator",
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
fn TableCell(cell: Cell, header: bool) -> Element {
    let shade = cell_style(&cell);
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
            rsx! {
                li { class, {runs_of(&item)} }
            }
        }
    }
}

/// One block: a heading, a paragraph, or a table.
#[component]
fn DocxBlock(block: Block) -> Element {
    match block {
        Block::Paragraph(p) => {
            let style = paragraph_style(&p);
            let inner = runs_of(&p);
            // A dynamic tag name is not expressible in rsx, so the six headings
            // are written out. HTML has six; deeper levels were clamped when the
            // level was worked out.
            match heading_level(p.style_id.as_deref(), p.outline_level) {
                Some(1) => rsx! { h1 { class: "docx-h", style: "{style}", {inner} } },
                Some(2) => rsx! { h2 { class: "docx-h", style: "{style}", {inner} } },
                Some(3) => rsx! { h3 { class: "docx-h", style: "{style}", {inner} } },
                Some(4) => rsx! { h4 { class: "docx-h", style: "{style}", {inner} } },
                Some(5) => rsx! { h5 { class: "docx-h", style: "{style}", {inner} } },
                Some(_) => rsx! { h6 { class: "docx-h", style: "{style}", {inner} } },
                // An empty paragraph is a deliberate blank line in Word, so it
                // keeps its element rather than being dropped.
                None => rsx! { p { class: "docx-p", style: "{style}", {inner} } },
            }
        }
        Block::Table(t) => rsx! {
            // Its own scroll container: a wide table must not widen the page and
            // force the whole document sideways on a phone.
            div { class: "docx-table-wrap",
                table { class: "docx-table",
                    for (r , row) in t.rows.into_iter().enumerate() {
                        tr { key: "r{r}",
                            for (c , cell) in row.cells.into_iter().enumerate() {
                                // A cell covered by a merge from above is not
                                // drawn; drawing it would push the row wider.
                                if !is_merged_away(&cell) {
                                    TableCell { key: "c{c}", cell, header: row.is_header }
                                }
                            }
                        }
                    }
                }
            }
        },
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
        assert_eq!(paragraph_style(p), "text-align:left;");
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
                    }],
                    is_header: false,
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
        assert_eq!(paragraph_style(&flat), "margin-left:0.92rem;");
    }

    #[test]
    fn a_first_line_indent_can_also_be_positive_and_never_escapes_left() {
        // Ordinary prose: the first line pushed in, the rest flush.
        let prose = Paragraph {
            indent_first: Some(16.0),
            ..Default::default()
        };
        assert_eq!(paragraph_style(&prose), "text-indent:1.00rem;");

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
        assert_eq!(paragraph_style(&centred), "text-align:center;");
        let right = Paragraph {
            alignment: Some("right".into()),
            ..Default::default()
        };
        assert_eq!(paragraph_style(&right), "text-align:right;");
        let indented = Paragraph {
            indent_left: Some(32.0),
            ..Default::default()
        };
        assert_eq!(paragraph_style(&indented), "margin-left:2.00rem;");
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
            assert_eq!(paragraph_style(&p), "text-align:left;", "{value}");
        }
        // A paragraph that says nothing still says nothing: the container's
        // `text-align: start` is what saves it, and inline noise on every
        // paragraph of every document is not worth the bytes.
        assert_eq!(paragraph_style(&Paragraph::default()), "");
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
    fn typography(blocks: &[Block]) -> (f64, f64, f64) {
        let mut sizes: Vec<f64> = Vec::new();
        let mut lines: Vec<f64> = Vec::new();
        let mut afters: Vec<f64> = Vec::new();
        for block in blocks {
            let Block::Paragraph(p) = block else { continue };
            if p.outline_level.is_some_and(|l| l < 9) {
                continue;
            }
            if let Some(pt) = dominant_size(p) {
                sizes.push(pt);
            }
            if let Some(spacing) = p.line_spacing.as_ref() {
                // `auto` states a multiplier in 240ths; the other rules state
                // points, which cannot be a multiplier without knowing the size.
                if spacing.rule == "auto" && spacing.value > 0.0 {
                    lines.push(spacing.value);
                }
            }
            if let Some(pt) = p.space_after.filter(|a| *a >= 0.0) {
                afters.push(pt);
            }
        }
        // Word's own defaults, for a document that states none of this.
        //
        // The line multiplier is scaled on the way out. Word's `auto` rule
        // means "this many times SINGLE spacing", and single spacing is the
        // font's own line box -- around 1.22 times its size for the faces these
        // documents are set in. CSS `line-height: 1.08` means 1.08 times the
        // font SIZE, which is a fifth short of what Word lays out.
        (
            commonest(&sizes).unwrap_or(11.0),
            commonest(&lines).unwrap_or(1.08) * SINGLE_SPACING,
            commonest(&afters).unwrap_or(8.0),
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
        let (size, line, after) = Self::typography(blocks);
        (column > 100.0 && height > 100.0).then(|| PageGeometry {
            column,
            height,
            size,
            line,
            after,
            // The document's own body face first, then the metric-compatible
            // substitutes a reader is likely to actually have. The measuring is
            // only as good as this: a face that is not installed is replaced by
            // one with different metrics, the lines wrap elsewhere, and the
            // pages drift.
            font: match font.map(str::trim).filter(|f| !f.is_empty()) {
                Some(named) => format!("{named}, Carlito, Calibri, Liberation Sans, sans-serif"),
                None => "Carlito, Calibri, Liberation Sans, sans-serif".to_string(),
            },
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
pub fn PagedDocx(blocks: Vec<Block>, page: PageGeometry) -> Element {
    let mut marks = use_signal(Vec::<(usize, usize, usize)>::new);
    let mut pages = use_signal(|| 0usize);
    // Whether the off-screen copy is still needed. It is a whole second render
    // of the document, so it goes as soon as it has been measured.
    let mut measuring = use_signal(|| true);

    let height = page.height;
    use_effect(move || {
        if !*measuring.peek() {
            return;
        }
        let Some((found, count)) = measure_pages(height) else {
            return;
        };
        marks.set(found);
        pages.set(count);
        measuring.set(false);
    });

    rsx! {
        if measuring() {
            div {
                id: "docx-measure",
                class: "docx-measure",
                aria_hidden: "true",
                style: "width:{page.column}px;font-family:{page.font};font-size:{page.size}pt;--docx-page-line:{page.line};--docx-page-after:{page.after}pt;",
                DocxBody { blocks: blocks.clone() }
            }
        }
        DocxBody { blocks, marks: marks() }
        if pages() > 1 {
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
fn measure_pages(height: f64) -> Option<(Vec<(usize, usize, usize)>, usize)> {
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
        empty: bool,
    }
    let mut flow: Vec<Spot> = Vec::new();
    for at in 0..groups.length() {
        let Some(group) = groups
            .item(at)
            .and_then(|c| c.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let listish = matches!(group.tag_name().as_str(), "UL" | "OL");
        let items = group.query_selector_all(":scope > li").ok();
        match (listish, items) {
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
                        empty: is_blank(&item),
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
                    empty: is_blank(&group),
                });
            }
        }
    }

    // Where the content actually stops. Trailing empty paragraphs are a blank
    // page nobody wrote.
    let last = flow.iter().rposition(|spot| !spot.empty)?;

    let mut marks: Vec<(usize, usize, usize)> = Vec::new();
    let mut page = 1usize;
    let mut measured = 0.0f64;
    for spot in flow.iter().take(last + 1) {
        measured = spot.bottom;
        let mut began = false;
        while measured > page as f64 * height {
            page += 1;
            if !began {
                // The page that begins here is the one after the marks already
                // placed, NOT the page the height says: anything that swallowed
                // a boundary took its number with it.
                marks.push((spot.group, spot.item, marks.len() + 2));
                began = true;
            }
            // A document whose measuring went wrong should not spin.
            if page > 2000 {
                return None;
            }
        }
        let _ = spot.top;
    }
    // Nothing measured at all (fonts not settled, or an empty document): let
    // the effect try again rather than marking a one-page document.
    (measured > 0.0).then_some((marks.clone(), marks.len() + 1))
}

/// Whether an element holds no words. `textContent`, NOT `innerText`: the
/// latter is what the browser RENDERS, and this copy is hidden, so it answers
/// empty for everything and the whole measurement gives up. That shipped once
/// and left every Word document with no page marks at all.
fn is_blank(element: &web_sys::Element) -> bool {
    element.text_content().unwrap_or_default().trim().is_empty()
}

/// What Word means by SINGLE line spacing: the font's own line box, which for
/// Calibri and the faces that stand in for it is about 1.22 times the size.
/// CSS counts line height from the size instead, so every multiplier the
/// document states has to be scaled by this to mean the same thing.
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

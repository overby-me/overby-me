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
    #[serde(default)]
    pub numbering: Option<Numbering>,
    #[serde(default)]
    pub indent_left: Option<f64>,
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

    /// How to draw that bullet: the size Word recorded for it, which lives in
    /// the numbering definition and is unrelated to the size of the file.
    ///
    /// Driven from the HEIGHT rather than the width, with the width following
    /// from the ratio, because the stylesheet caps a bullet to the line it
    /// leads — Word will happily ask for an 18pt bullet beside 11pt text — and
    /// a cap on the height must be free to take the width down with it.
    pub fn bullet_style(&self) -> String {
        let (w, h) = (
            self.pic_bullet_width_pt.filter(|w| *w > 0.0),
            self.pic_bullet_height_pt.filter(|h| *h > 0.0),
        );
        match (w, h) {
            (Some(w), Some(h)) => {
                format!("height:{h:.2}pt;aspect-ratio:{w:.2}/{h:.2};width:auto;")
            }
            (None, Some(h)) => format!("height:{h:.2}pt;width:auto;"),
            (Some(w), None) => format!("width:{w:.2}pt;"),
            (None, None) => String::new(),
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Run {
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

/// A `data:` url for one picture.
fn data_url(mime: &str, bytes: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
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
pub fn heading_level(style_id: Option<&str>, outline_level: Option<i64>) -> Option<u8> {
    if let Some(style) = style_id {
        let s = style.to_ascii_lowercase().replace([' ', '-', '_'], "");
        if s == "title" {
            return Some(1);
        }
        if let Some(n) = s.strip_prefix("heading").and_then(|n| n.parse::<u8>().ok()) {
            if (1..=9).contains(&n) {
                return Some(n.min(6));
            }
        }
    }
    match outline_level {
        Some(l) if (0..=8).contains(&l) => Some(((l + 1) as u8).min(6)),
        _ => None,
    }
}

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
    if let Some(c) = run.color.as_deref() {
        if c != "auto" && c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
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
    // font size instead of being pinned to Word's.
    if let Some(pt) = p.indent_left.filter(|v| *v > 0.0) {
        css.push_str(&format!("margin-left:{:.2}rem;", pt / 16.0));
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
pub fn DocxBody(blocks: Vec<Block>) -> Element {
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

    rsx! {
        div { class: "docx",
            for (i , group) in groups.into_iter().enumerate() {
                match group {
                    Group::Single(block) => rsx! {
                        DocxBlock { key: "b{i}", block }
                    },
                    Group::List { ordered, items } => rsx! {
                        if ordered {
                            ol { key: "l{i}", class: "docx-list",
                                for (j , item) in items.into_iter().enumerate() {
                                    ListItem { key: "i{j}", item }
                                }
                            }
                        } else {
                            ul { key: "l{i}", class: "docx-list",
                                for (j , item) in items.into_iter().enumerate() {
                                    ListItem { key: "i{j}", item }
                                }
                            }
                        }
                    },
                }
            }
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
        Some((src, style)) => rsx! {
            li { class: "docx-li-pic",
                img { class: "docx-bullet", src: "{src}", style: "{style}", alt: "" }
                span { class: "docx-li-body", {runs_of(&item)} }
            }
        },
        None => rsx! {
            li { {runs_of(&item)} }
        },
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
                                    if row.is_header {
                                        th { key: "c{c}", colspan: "{cell.col_span}",
                                            DocxBody { blocks: cell.content }
                                        }
                                    } else {
                                        td { key: "c{c}", colspan: "{cell.col_span}",
                                            DocxBody { blocks: cell.content }
                                        }
                                    }
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
        // Driven from the height so the stylesheet's cap can take the width
        // down with it: an 18pt bullet beside 11pt text is Word being Word.
        let css = n.bullet_style();
        assert!(css.contains("height:18.75pt;"), "{css}");
        assert!(css.contains("aspect-ratio:18.00/18.75;"), "{css}");
        assert!(css.contains("width:auto;"), "{css}");

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
            numbering: Some(Numbering {
                format: Some("bullet".into()),
                pic_bullet_image_path: Some("word/media/image1.jpeg".into()),
                pic_bullet_mime_type: Some("image/jpeg".into()),
                ..Default::default()
            }),
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
            numbering: Some(Numbering {
                format: Some("bullet".into()),
                level: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(list_kind(&bullet), Some(false));
        let decimal = Paragraph {
            numbering: Some(Numbering {
                format: Some("decimal".into()),
                level: Some(0),
                ..Default::default()
            }),
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

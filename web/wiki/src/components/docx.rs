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
                                    li { key: "i{j}", {runs_of(&item)} }
                                }
                            }
                        } else {
                            ul { key: "l{i}", class: "docx-list",
                                for (j , item) in items.into_iter().enumerate() {
                                    li { key: "i{j}", {runs_of(&item)} }
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

/// A paragraph's runs, as inline elements.
///
/// Bold and italic become `<strong>`/`<em>` rather than CSS: they are meaning,
/// not decoration, and a screen reader announces them.
fn runs_of(p: &Paragraph) -> Element {
    let runs = p.runs.clone();
    rsx! {
        for (i , run) in runs.into_iter().enumerate() {
            RunSpan { key: "r{i}", run }
        }
    }
}

#[component]
fn RunSpan(run: Run) -> Element {
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
            }),
            ..Default::default()
        };
        assert_eq!(list_kind(&bullet), Some(false));
        let decimal = Paragraph {
            numbering: Some(Numbering {
                format: Some("decimal".into()),
                level: Some(0),
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

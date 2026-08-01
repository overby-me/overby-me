//! OpenDocument text, read into the model the Word renderer already draws.
//!
//! ODF is what LibreOffice writes, and what a public body asked for an open
//! format will hand you. Until now this app had nothing to say about one: the
//! Microsoft viewer cannot render ODF at all, so a `.odt` had no preview and no
//! option to get one — just a download button.
//!
//! It is also the easiest of the formats to read. Where WordprocessingML wraps
//! every scrap of text in runs inside runs, ODF says what it means:
//!
//! ```xml
//! <text:h text:outline-level="1">Landsmøde 2026</text:h>
//! <text:p>Dette er <text:span text:style-name="Bold">vigtigt</text:span></text:p>
//! ```
//!
//! So there is no renderer here. This turns ODF into [`super::docx::Block`] —
//! the same model the Word renderer takes — and the whole rendering path,
//! headings and lists and tables and styled runs, comes for free. Anything that
//! model cannot express is not expressible here either, which is the honest
//! outcome: the two formats get exactly the same treatment.
//!
//! Bold and italic live in `<office:automatic-styles>` at the top of the same
//! file, so a run's style name is resolved without reading `styles.xml`.

use std::collections::HashMap;
use std::io::Read;

use super::docx::{Block, Cell, Numbering, Paragraph, Row, Run, Table};

/// The ODF namespaces this reads. Prefixes are conventional but not guaranteed,
/// so elements are matched on the namespace URI and the local name.
const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const NS_STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
const NS_FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";

/// Pull `content.xml` out of an ODF package.
pub fn content_xml(bytes: &[u8]) -> Result<String, String> {
    let reader = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| format!("not a zip: {e}"))?;
    let mut file = zip
        .by_name("content.xml")
        .map_err(|_| "no content.xml: not an OpenDocument file".to_string())?;
    let mut out = String::new();
    file.read_to_string(&mut out)
        .map_err(|e| format!("content.xml unreadable: {e}"))?;
    Ok(out)
}

/// A whole `.odt`, as blocks the Word renderer draws.
pub fn parse_odt(bytes: &[u8]) -> Result<Vec<Block>, String> {
    blocks_from_content(&content_xml(bytes)?)
}

/// Which automatic styles mean bold or italic.
///
/// A run carries a style NAME, and the definition sits in the same document:
/// `<style:style style:name="Bold"><style:text-properties fo:font-weight="bold"/>`.
fn text_styles(doc: &roxmltree::Document) -> HashMap<String, (bool, bool)> {
    let mut out = HashMap::new();
    for style in doc
        .descendants()
        .filter(|n| n.tag_name().namespace() == Some(NS_STYLE) && n.tag_name().name() == "style")
    {
        let Some(name) = style.attribute((NS_STYLE, "name")) else {
            continue;
        };
        for props in style.children().filter(|n| {
            n.tag_name().namespace() == Some(NS_STYLE) && n.tag_name().name() == "text-properties"
        }) {
            let bold = props.attribute((NS_FO, "font-weight")) == Some("bold");
            let italic = props.attribute((NS_FO, "font-style")) == Some("italic");
            if bold || italic {
                out.insert(name.to_string(), (bold, italic));
            }
        }
    }
    out
}

/// The runs of one paragraph or heading: its text nodes and its spans.
fn runs_of(node: roxmltree::Node, styles: &HashMap<String, (bool, bool)>) -> Vec<Run> {
    let mut runs = Vec::new();
    for child in node.descendants() {
        if child.is_text() {
            let text = child.text().unwrap_or_default();
            if text.is_empty() {
                continue;
            }
            // The nearest ancestor span decides how this text is set; text
            // directly under the paragraph is plain.
            let (bold, italic) = child
                .ancestors()
                .find(|a| {
                    a.tag_name().namespace() == Some(NS_TEXT) && a.tag_name().name() == "span"
                })
                .and_then(|s| s.attribute((NS_TEXT, "style-name")))
                .and_then(|name| styles.get(name).copied())
                .unwrap_or((false, false));
            runs.push(Run {
                text: text.to_string(),
                bold,
                italic,
                ..Default::default()
            });
        } else if child.tag_name().namespace() == Some(NS_TEXT)
            && child.tag_name().name() == "line-break"
        {
            runs.push(Run {
                text: "\n".into(),
                ..Default::default()
            });
        }
    }
    runs
}

/// One `<text:p>` or `<text:h>` as a paragraph.
fn paragraph_of(
    node: roxmltree::Node,
    styles: &HashMap<String, (bool, bool)>,
    numbering: Option<Box<Numbering>>,
) -> Paragraph {
    // ODF counts outline levels from 1; the Word model counts from 0, where 0
    // is Heading 1. A `<text:p>` has no level and is body text.
    let outline_level = if node.tag_name().name() == "h" {
        node.attribute((NS_TEXT, "outline-level"))
            .and_then(|v| v.parse::<i64>().ok())
            .map(|l| (l - 1).max(0))
            .or(Some(0))
    } else {
        None
    };
    Paragraph {
        runs: runs_of(node, styles),
        outline_level,
        numbering,
        ..Default::default()
    }
}

/// A `<table:table>` as a table.
fn table_of(node: roxmltree::Node, styles: &HashMap<String, (bool, bool)>) -> Table {
    let mut rows = Vec::new();
    for row in node.children().filter(|n| {
        n.tag_name().namespace() == Some(NS_TABLE) && n.tag_name().name() == "table-row"
    }) {
        let mut cells = Vec::new();
        for cell in row.children().filter(|n| {
            n.tag_name().namespace() == Some(NS_TABLE) && n.tag_name().name() == "table-cell"
        }) {
            let content = cell
                .children()
                .filter(|n| {
                    n.tag_name().namespace() == Some(NS_TEXT)
                        && matches!(n.tag_name().name(), "p" | "h")
                })
                .map(|p| Block::Paragraph(paragraph_of(p, styles, None)))
                .collect();
            cells.push(Cell {
                content,
                // ODF spells a horizontal span `number-columns-spanned`.
                col_span: cell
                    .attribute((NS_TABLE, "number-columns-spanned"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
                v_merge: None,
            });
        }
        rows.push(Row {
            cells,
            is_header: false,
        });
    }
    Table { rows }
}

/// The document body as blocks.
///
/// Pure: takes the XML, returns the model. The zip lives in [`content_xml`], so
/// every mapping decision below is testable without building an archive.
pub fn blocks_from_content(xml: &str) -> Result<Vec<Block>, String> {
    let doc =
        roxmltree::Document::parse(xml).map_err(|e| format!("content.xml is not XML: {e}"))?;
    let styles = text_styles(&doc);

    let Some(body) = doc
        .descendants()
        .find(|n| n.tag_name().name() == "text" && n.tag_name().namespace().is_some())
    else {
        return Ok(Vec::new());
    };

    let mut blocks = Vec::new();
    for node in body.children().filter(|n| n.is_element()) {
        let (ns, name) = (node.tag_name().namespace(), node.tag_name().name());
        match (ns, name) {
            (Some(NS_TEXT), "p") | (Some(NS_TEXT), "h") => {
                blocks.push(Block::Paragraph(paragraph_of(node, &styles, None)));
            }
            // A list's items each hold their own paragraphs. The Word model
            // marks a list item on the paragraph, which is what the renderer
            // groups on, so the nesting is flattened to that.
            (Some(NS_TEXT), "list") => {
                let ordered = node.attribute((NS_TEXT, "style-name")).is_some_and(|s| {
                    let s = s.to_ascii_lowercase();
                    s.contains("number") || s.contains("ordered")
                });
                let numbering = Some(Box::new(Numbering {
                    format: Some(if ordered { "decimal" } else { "bullet" }.to_string()),
                    level: Some(0),
                    ..Default::default()
                }));
                for item in node.children().filter(|n| {
                    n.tag_name().namespace() == Some(NS_TEXT) && n.tag_name().name() == "list-item"
                }) {
                    for p in item.children().filter(|n| {
                        n.tag_name().namespace() == Some(NS_TEXT)
                            && matches!(n.tag_name().name(), "p" | "h")
                    }) {
                        blocks.push(Block::Paragraph(paragraph_of(
                            p,
                            &styles,
                            numbering.clone(),
                        )));
                    }
                }
            }
            (Some(NS_TABLE), "table") => blocks.push(Block::Table(table_of(node, &styles))),
            // Sequence declarations, bookmarks, indexes: nothing to draw.
            _ => {}
        }
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape odfpy writes, which is the shape LibreOffice writes.
    const DOC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
  xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
  xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
  xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
  xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
  xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0">
  <office:automatic-styles>
    <style:style style:name="Bold" style:family="text">
      <style:text-properties fo:font-weight="bold"/>
    </style:style>
    <style:style style:name="It" style:family="text">
      <style:text-properties fo:font-style="italic"/>
    </style:style>
  </office:automatic-styles>
  <office:body><office:text>
    <text:h text:outline-level="1">Landsmøde 2026</text:h>
    <text:p>Dagsorden for mødet.</text:p>
    <text:p>Dette er <text:span text:style-name="Bold">vigtigt</text:span> at læse.</text:p>
    <text:h text:outline-level="2">Punkter</text:h>
    <text:list text:style-name="L1">
      <text:list-item><text:p>Velkomst</text:p></text:list-item>
      <text:list-item><text:p>Beretning</text:p></text:list-item>
    </text:list>
    <table:table>
      <table:table-row>
        <table:table-cell table:number-columns-spanned="2"><text:p>Punkt</text:p></table:table-cell>
        <table:table-cell><text:p>Tid</text:p></table:table-cell>
      </table:table-row>
    </table:table>
  </office:text></office:body>
</office:document-content>"#;

    fn blocks() -> Vec<Block> {
        blocks_from_content(DOC).expect("the document must parse")
    }

    #[test]
    fn headings_keep_their_level() {
        let b = blocks();
        let Block::Paragraph(h1) = &b[0] else {
            panic!("expected a heading")
        };
        // ODF counts from 1, the Word model from 0.
        assert_eq!(h1.outline_level, Some(0));
        assert_eq!(
            super::super::docx::heading_level(None, h1.outline_level),
            Some(1)
        );
        let Block::Paragraph(h2) = &b[3] else {
            panic!("expected the second heading")
        };
        assert_eq!(
            super::super::docx::heading_level(None, h2.outline_level),
            Some(2)
        );
    }

    #[test]
    fn a_paragraph_is_body_text_not_a_heading() {
        let Block::Paragraph(p) = &blocks()[1] else {
            panic!()
        };
        assert_eq!(p.outline_level, None, "a text:p has no level");
        assert_eq!(p.runs.len(), 1);
        assert_eq!(p.runs[0].text, "Dagsorden for mødet.");
    }

    /// The run split, and the style lookup that makes one of them bold.
    #[test]
    fn a_span_is_styled_from_the_documents_own_styles() {
        let Block::Paragraph(p) = &blocks()[2] else {
            panic!()
        };
        let texts: Vec<&str> = p.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["Dette er ", "vigtigt", " at læse."]);
        assert!(!p.runs[0].bold, "text outside the span is plain");
        assert!(p.runs[1].bold, "the span resolves to its style");
        assert!(!p.runs[2].bold, "and the text after it is plain again");
    }

    #[test]
    fn list_items_become_list_paragraphs() {
        let b = blocks();
        let Block::Paragraph(first) = &b[4] else {
            panic!("expected a list item")
        };
        assert_eq!(
            super::super::docx::list_kind(first),
            Some(false),
            "a bullet"
        );
        let Block::Paragraph(second) = &b[5] else {
            panic!()
        };
        assert_eq!(second.runs[0].text, "Beretning");
    }

    #[test]
    fn a_table_keeps_its_spans() {
        let table = blocks()
            .into_iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("a table");
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].cells.len(), 2);
        assert_eq!(table.rows[0].cells[0].col_span, 2);
        assert_eq!(table.rows[0].cells[1].col_span, 1, "absent means one");
    }

    /// Namespaces are matched by URI, not by prefix: a document using
    /// different prefixes for the same namespaces must read identically.
    #[test]
    fn an_unusual_prefix_is_still_read() {
        // Rename only the PREFIX. The namespace URI itself ends in `:text:1.0`,
        // so a blanket replace of `text:` rewrites the URI too and the document
        // stops being OpenDocument — which is what the first version of this
        // test did, and why it failed.
        let odd = DOC
            .replace("<text:", "<t:")
            .replace("</text:", "</t:")
            .replace(" text:", " t:")
            .replace("xmlns:text=", "xmlns:t=");
        let b = blocks_from_content(&odd).expect("prefixes are not the identity");
        assert!(!b.is_empty(), "the body was found under a different prefix");
    }

    #[test]
    fn rubbish_is_an_error_not_a_panic() {
        assert!(blocks_from_content("not xml at all").is_err());
        // Valid XML with no ODF body is an empty document, not a failure.
        assert_eq!(blocks_from_content("<hello/>").unwrap().len(), 0);
    }

    /// The whole path, package and all: a real ODF zip in, blocks out. Built
    /// here rather than committed as a binary fixture, so what it contains is
    /// visible in the test.
    #[test]
    fn a_real_package_round_trips() {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            // `mimetype` first and stored, as the spec asks; this reader does
            // not require it, but a real package has it.
            zip.start_file(
                "mimetype",
                opts.compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(b"application/vnd.oasis.opendocument.text")
                .unwrap();
            zip.start_file("content.xml", opts).unwrap();
            zip.write_all(DOC.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let blocks = parse_odt(&buf).expect("a real package must read");
        let Block::Paragraph(h1) = &blocks[0] else {
            panic!("expected the heading")
        };
        assert_eq!(h1.runs[0].text, "Landsmøde 2026");

        // And the failure modes are errors, not panics.
        assert!(parse_odt(b"not a zip").is_err());
        let mut empty = Vec::new();
        zip::ZipWriter::new(std::io::Cursor::new(&mut empty))
            .finish()
            .unwrap();
        assert!(parse_odt(&empty).is_err(), "a zip with no content.xml");
    }
}

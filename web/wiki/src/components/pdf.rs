//! A PDF, rendered by this app as flowing text rather than as fixed pages.
//!
//! The second option beside the browser's own viewer, which stays the default.
//! That one is exact and it prints; this one reflows, so a hundred-page appendix
//! is readable on a phone instead of being a page-sized image in a scrolling
//! box, and the browser's find works across the whole document.
//!
//! It costs everything a page has and a paragraph does not: no page breaks, no
//! margins, no columns, no figures, no tables yet. What comes out is the text,
//! its headings, its lists and the colours it was written in, in reading order.
//! For the agendas, motions and appendices this wiki holds that is the useful
//! half; for a poster it is not, and the browser's viewer is one tap away.
//!
//! It borrows the Word renderer's classes on purpose, `docx-doc` and `docx-h`
//! and the rest, because it is the same job: a document read on a screen rather
//! than a page reproduced. `docx-doc` in particular is load-bearing. The file
//! viewer around this CENTRES its contents, which is right for an image and
//! wrong for a document, and that class is what sets the text back to `start`.
//!
//! The reconstruction lives in [`crate::pdf_text`], which is pure and testable.
//! This is only the rendering.

use dioxus::prelude::*;

use crate::i18n::t;
use crate::pdf_text::{Block, Extracted, Span};

/// One block's words, keeping the colours the document set.
///
/// A span with no colour of its own gets none here either: the reading surface
/// decides, which is what keeps a black-on-white document legible in the dark
/// theme.
#[component]
fn Spans(spans: Vec<Span>) -> Element {
    rsx! {
        for (i , span) in spans.iter().enumerate() {
            match &span.color {
                Some(color) => rsx! {
                    span { key: "{i}", style: "color:{color};", "{span.text}" }
                },
                None => rsx! { "{span.text}" },
            }
        }
    }
}

/// Render what was read out of a PDF.
#[component]
pub fn PdfDocument(doc: Extracted) -> Element {
    // Consecutive list items become one list, so a bulleted run reads as one
    // rather than as a column of one-item lists.
    let mut groups: Vec<Vec<Block>> = Vec::new();
    for block in doc.blocks.iter() {
        let extend = matches!(block, Block::ListItem(_))
            && groups
                .last()
                .and_then(|g| g.last())
                .is_some_and(|b| matches!(b, Block::ListItem(_)));
        match extend {
            true => groups.last_mut().expect("checked").push(block.clone()),
            false => groups.push(vec![block.clone()]),
        }
    }

    rsx! {
        div { class: "docx-doc",
            for (i , group) in groups.iter().enumerate() {
                match group.first() {
                    Some(Block::ListItem(_)) => rsx! {
                        ul { key: "{i}", class: "docx-list",
                            for (j , item) in group.iter().enumerate() {
                                li { key: "{j}", Spans { spans: item.spans().to_vec() } }
                            }
                        }
                    },
                    Some(Block::Heading { level, spans }) => {
                        // The levels are relative sizes within the document, not
                        // an outline the file declared, so they start at h3:
                        // this sits under the page's own heading.
                        let spans = spans.clone();
                        match level {
                            1 => rsx! { h3 { key: "{i}", class: "docx-h", Spans { spans } } },
                            2 => rsx! { h4 { key: "{i}", class: "docx-h", Spans { spans } } },
                            _ => rsx! { h5 { key: "{i}", class: "docx-h", Spans { spans } } },
                        }
                    }
                    Some(Block::Paragraph(spans)) => rsx! {
                        p { key: "{i}", class: "docx-p", Spans { spans: spans.clone() } }
                    },
                    None => rsx! {},
                }
            }
            // Say what was left behind, in the Word renderer's own gap-notice
            // style rather than a class of this file's invention: a reader
            // deciding whether to open the real thing is owed the reason, and
            // it should look like the other one when they do.
            div { class: "file-gap-notice", role: "note",
                span { class: "material-icons", "info" }
                p { class: "body-small", {t_pages(doc.pages)} }
            }
        }
    }
}

/// "Reflowed from N pages. Layout, figures and tables are not shown."
fn t_pages(pages: usize) -> String {
    crate::i18n::t_with("file.pdfReflowed", &[("pages", &pages.to_string())])
}

/// What to show when a PDF has no text in it at all: a scan, which is an image
/// of a document rather than a document. Nothing here can read it, and saying so
/// is better than an empty page.
#[component]
pub fn PdfHasNoText() -> Element {
    rsx! {
        div { class: "empty-state empty-state-sm",
            div { class: "empty-state-orb empty-state-orb-sm",
                span { class: "material-icons", "image_not_supported" }
            }
            p { class: "empty-state-body", "{t(\"file.pdfNoText\")}" }
        }
    }
}

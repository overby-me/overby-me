//! A PDF, rendered by this app as flowing text rather than as fixed pages.
//!
//! The second option beside the browser's own viewer, which stays the default.
//! That one is exact and it prints; this one reflows, so a hundred-page appendix
//! is readable on a phone instead of being a page-sized image in a scrolling
//! box, and the browser's find works across the whole document.
//!
//! It costs everything a page has and a paragraph does not: no page breaks, no
//! margins, no columns, no figures, no tables yet. What comes out is the text,
//! its headings and its lists, in reading order. For the agendas, motions and
//! appendices this wiki holds that is the useful half; for a poster it is not,
//! and the browser's viewer is one tap away.
//!
//! The reconstruction lives in [`crate::pdf_text`], which is pure and testable.
//! This is only the rendering.

use dioxus::prelude::*;

use crate::i18n::t;
use crate::pdf_text::{Block, Extracted};

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
        div { class: "docx-view",
            for (i , group) in groups.iter().enumerate() {
                match group.first() {
                    Some(Block::ListItem(_)) => rsx! {
                        ul { key: "{i}", class: "docx-list",
                            for (j , item) in group.iter().enumerate() {
                                if let Block::ListItem(text) = item {
                                    li { key: "{j}", "{text}" }
                                }
                            }
                        }
                    },
                    Some(Block::Heading { level, text }) => rsx! {
                        // The levels are relative sizes within the document, not
                        // an outline the file declared, so they start at h3: this
                        // sits under the page's own heading.
                        match level {
                            1 => rsx! { h3 { key: "{i}", class: "docx-h1", "{text}" } },
                            2 => rsx! { h4 { key: "{i}", class: "docx-h2", "{text}" } },
                            _ => rsx! { h5 { key: "{i}", class: "docx-h3", "{text}" } },
                        }
                    },
                    Some(Block::Paragraph(text)) => rsx! {
                        p { key: "{i}", class: "docx-p", "{text}" }
                    },
                    None => rsx! {},
                }
            }
            // Say what was left behind, in the same spirit as the Word renderer's
            // gap notice: a reader deciding whether to open the real thing is
            // owed the reason.
            div { class: "docx-gaps",
                p { class: "body-small",
                    {t_pages(doc.pages)}
                }
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

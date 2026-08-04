//! A PDF, rendered by this app as flowing text rather than as fixed pages.
//!
//! The second option beside the browser's own viewer, which stays the default.
//! That one is exact and it prints; this one reflows, so a hundred-page appendix
//! is readable on a phone instead of being a page-sized image in a scrolling
//! box, and the browser's find works across the whole document.
//!
//! It costs what a page has and a paragraph does not: margins, columns, and
//! tables so far. What comes out is the text, its headings, its lists, the
//! colours and weights it was written in, the pictures it drew, how each block
//! sat across its column, and where the pages ended. For the agendas, motions
//! and appendices this wiki holds that is the useful half; for a poster it is
//! not, and the browser's viewer is one tap away.
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
use crate::pdf_text::{Align, Block, Extracted, Link, Span};

/// One block's words, keeping the colours the document set.
///
/// A span with no colour of its own gets none here either: the reading surface
/// decides, which is what keeps a black-on-white document legible in the dark
/// theme.
#[component]
fn Spans(spans: Vec<Span>) -> Element {
    rsx! {
        for (i , span) in spans.iter().enumerate() {
            {
                let style = match &span.color {
                    Some(color) => format!("color:{color};"),
                    None => String::new(),
                };
                // `strong` and `em` rather than a font-weight style: a document
                // that emphasises a word means it, and the meaning should reach
                // a screen reader too.
                let inner = match (span.bold, span.italic) {
                    (true, true) => rsx! {
                        strong { style: "{style}", em { "{span.text}" } }
                    },
                    (true, false) => rsx! { strong { style: "{style}", "{span.text}" } },
                    (false, true) => rsx! { em { style: "{style}", "{span.text}" } },
                    (false, false) if style.is_empty() => rsx! { "{span.text}" },
                    (false, false) => rsx! { span { style: "{style}", "{span.text}" } },
                };
                match &span.link {
                    Some(link) => rsx! { LinkTo { key: "{i}", link: link.clone(), {inner} } },
                    None => rsx! { Fragment { key: "{i}", {inner} } },
                }
            }
        }
    }
}

/// Wrap something in the link the document put on it.
///
/// A destination inside the document is scrolled to rather than navigated to.
/// The `href` is real, so it focuses, announces and copies like a link; what is
/// suppressed is the fragment landing in the address bar, which this app's
/// router would read as a new place and answer by rebuilding the view: the file
/// would be fetched and reparsed to arrive where a scroll already was.
#[component]
fn LinkTo(
    link: Link,
    /// Extra classes for when the link IS the row rather than words inside one.
    #[props(default)]
    class: String,
    #[props(default)] style: String,
    children: Element,
) -> Element {
    let class = format!("pdf-link {class}");
    match link {
        Link::Url(href) => rsx! {
            a {
                class: "{class}",
                style: "{style}",
                href: "{href}",
                target: "_blank",
                rel: "noopener noreferrer",
                {children}
            }
        },
        Link::Place(id) => rsx! {
            a {
                class: "{class}",
                style: "{style}",
                href: "#{id}",
                onclick: move |e: Event<MouseData>| {
                    e.prevent_default();
                    jump_to(&id);
                },
                {children}
            }
        },
    }
}

/// Bring the anchor with this id into view.
///
/// `scroll-margin-top` on the anchor itself is what keeps the landing clear of
/// the sticky bar; the same arrangement the heading navigation uses.
fn jump_to(id: &str) {
    let Some(target) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
    else {
        return;
    };
    let opts = web_sys::ScrollIntoViewOptions::new();
    opts.set_behavior(web_sys::ScrollBehavior::Smooth);
    opts.set_block(web_sys::ScrollLogicalPosition::Start);
    target.scroll_into_view_with_scroll_into_view_options(&opts);
}

/// The same words with their links taken off, for when something around them is
/// already the link. An anchor inside an anchor is not a thing HTML has.
fn unlinked(spans: &[Span]) -> Vec<Span> {
    spans
        .iter()
        .map(|s| Span {
            link: None,
            ..s.clone()
        })
        .collect()
}

/// Render what was read out of a PDF.
#[component]
pub fn PdfDocument(doc: Extracted) -> Element {
    // Consecutive items of one kind become one group, so a bulleted run reads as
    // a single list rather than a column of one-item lists, and a contents list
    // as one aligned table rather than a stack of unrelated rows.
    fn runs_on(prev: &Block, next: &Block) -> bool {
        matches!(
            (prev, next),
            (Block::ListItem { .. }, Block::ListItem { .. })
                | (Block::IndexEntry { .. }, Block::IndexEntry { .. })
        )
    }
    let mut groups: Vec<Vec<Block>> = Vec::new();
    for block in doc.blocks.iter() {
        let extend = groups
            .last()
            .and_then(|g| g.last())
            .is_some_and(|prev| runs_on(prev, block));
        match extend {
            true => groups.last_mut().expect("checked").push(block.clone()),
            false => groups.push(vec![block.clone()]),
        }
    }

    rsx! {
        div { class: "docx-doc",
            for (i , group) in groups.iter().enumerate() {
                match group.first() {
                    Some(Block::ListItem { .. }) => rsx! {
                        ul { key: "{i}", class: "docx-list",
                            for (j , item) in group.iter().enumerate() {
                                if let Block::ListItem { spans, marker } = item {
                                    match marker {
                                        // The page drew its own bullet, and here
                                        // it is a logo rather than a dot, so it
                                        // is shown rather than stood in for. Sized
                                        // in ems so it follows the text.
                                        Some(src) => rsx! {
                                            li { key: "{j}", class: "pdf-li-drawn",
                                                img {
                                                    class: "pdf-li-mark",
                                                    src: "{src}",
                                                    alt: "",
                                                    aria_hidden: "true",
                                                }
                                                span { Spans { spans: spans.clone() } }
                                            }
                                        },
                                        None => rsx! {
                                            li { key: "{j}", Spans { spans: spans.clone() } }
                                        },
                                    }
                                }
                            }
                        }
                    },
                    Some(Block::Heading { level, spans, .. }) => {
                        // The levels are relative sizes within the document, not
                        // an outline the file declared, so they start at h3:
                        // this sits under the page's own heading.
                        let spans = spans.clone();
                        let st = align_style(group[0].align());
                        match level {
                            1 => rsx! { h3 { key: "{i}", class: "docx-h", style: "{st}", Spans { spans } } },
                            2 => rsx! { h4 { key: "{i}", class: "docx-h", style: "{st}", Spans { spans } } },
                            _ => rsx! { h5 { key: "{i}", class: "docx-h", style: "{st}", Spans { spans } } },
                        }
                    }
                    Some(Block::Paragraph {
                        spans,
                        align,
                        indent,
                    }) => rsx! {
                        p {
                            key: "{i}",
                            class: "docx-p",
                            style: "{align_style(*align)}{indent_style(*indent)}",
                            Spans { spans: spans.clone() }
                        }
                    },
                    Some(Block::IndexEntry { .. }) => rsx! {
                        // A contents list, laid out the way the page laid it
                        // out: the title, a leader carrying the eye across, and
                        // the page number on a single right margin. The leader
                        // is drawn rather than written, so every number lands on
                        // that margin whatever the title's length and whatever
                        // width this is read at.
                        div { key: "{i}", class: "pdf-toc",
                            for (j , entry) in group.iter().enumerate() {
                                if let Block::IndexEntry { spans, page, indent } = entry {
                                    {
                                        // The file draws its link across the whole
                                        // row, number included, so the whole row is
                                        // the link here too: on a phone the number
                                        // is the easiest part of it to hit.
                                        let row = rsx! {
                                            span { class: "pdf-toc-title", Spans { spans: unlinked(spans) } }
                                            // Decoration, and nothing for a screen
                                            // reader to announce: the row already
                                            // says what it points at and where.
                                            span { class: "pdf-toc-leader", aria_hidden: "true" }
                                            span { class: "pdf-toc-page", "{page}" }
                                        };
                                        let style = indent_style(*indent);
                                        match spans.iter().find_map(|s| s.link.clone()) {
                                            Some(link) => rsx! {
                                                LinkTo {
                                                    key: "{j}",
                                                    link,
                                                    class: "pdf-toc-row",
                                                    style: "{style}",
                                                    {row}
                                                }
                                            },
                                            None => rsx! {
                                                div { key: "{j}", class: "pdf-toc-row", style: "{style}", {row} }
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    },
                    Some(Block::Anchor(id)) => rsx! {
                        // Nothing to read: somewhere for a link to land.
                        span { key: "{i}", id: "{id}", class: "pdf-anchor" }
                    },
                    Some(Block::PageBreak { ended, printed }) => rsx! {
                        // Furniture, and marked as such: a separator carries no
                        // meaning to read aloud, and the number is what makes
                        // "see page 12" mean something to someone reading this
                        // rather than the pages. The number the page printed on
                        // itself wins over its position in the file, because
                        // that is the one the document's own index refers to.
                        div { key: "{i}", class: "pdf-page-break", role: "separator",
                            span { {printed.clone().unwrap_or_else(|| ended.to_string())} }
                        }
                    },
                    Some(Block::Image(picture)) => rsx! {
                        // Drawn at the size the page drew it, but never wider
                        // than the column: this reflows, and a banner laid out
                        // for A4 would otherwise push the text sideways.
                        img {
                            key: "{i}",
                            class: "docx-img",
                            src: "{picture.src}",
                            style: "width:{picture.width}px;max-width:100%;height:auto;",
                            alt: "",
                            loading: "lazy",
                        }
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

/// Only a stated alignment is written. Left is the default, and saying so would
/// override a right-to-left document that never asked for it.
fn align_style(align: Align) -> &'static str {
    match align {
        Align::Left => "",
        Align::Center => "text-align:center;",
    }
}

/// How far in a block was set, in steps of the indent token.
///
/// `inline-start` rather than `left`, so a right-to-left document indents away
/// from its own margin rather than into the text.
fn indent_style(indent: u8) -> String {
    match indent {
        0 => String::new(),
        n => format!("margin-inline-start:calc(var(--pdf-indent-step) * {n});"),
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

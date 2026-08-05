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
use wasm_bindgen::JsCast;

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
                let mut style = match &span.color {
                    Some(color) => format!("color:{color};"),
                    None => String::new(),
                };
                // Underline is decoration rather than meaning, so it lives in
                // the style the way the Word renderer keeps it, and every
                // combination of weight and slant survives beside it.
                if span.underline && span.link.is_none() {
                    style.push_str("text-decoration:underline;");
                }
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
                // An anchor is a landing place, not content, and a run of items
                // with one dropped in the middle is still one run. Letting it
                // break the run put a list's whole bottom margin between two
                // rows, which is why the gaps down a contents page came out
                // uneven.
                | (Block::ListItem { .. } | Block::IndexEntry { .. }, Block::Anchor(_))
                | (Block::Anchor(_), Block::ListItem { .. } | Block::IndexEntry { .. })
        )
    }
    // Whether the document numbers its own pages at all. Where it does, a page
    // that prints no number is not given the file's sheet number instead: the
    // songbook ends on an unnumbered back cover, and counting it made the
    // control read 104 on a document whose last page is 99. Where it does not,
    // the sheet numbers are all there is, and they are better than nothing.
    let numbered = doc.blocks.iter().any(|b| {
        matches!(
            b,
            Block::PageBreak {
                printed: Some(_),
                ..
            }
        )
    });
    let mut groups: Vec<Vec<Block>> = Vec::new();
    for block in doc.blocks.iter() {
        // Against the run's KIND rather than its last block, so a run of items
        // survives an anchor landing between two of them.
        let extend = groups
            .last()
            .and_then(|g| g.iter().rev().find(|b| !matches!(b, Block::Anchor(_))))
            .is_some_and(|prev| runs_on(prev, block));
        match extend {
            true => groups.last_mut().expect("checked").push(block.clone()),
            false => groups.push(vec![block.clone()]),
        }
    }

    rsx! {
        div { class: "docx-doc pdf-doc",
            for (i , group) in groups.iter().enumerate() {
                match group.first() {
                    Some(Block::ListItem { .. }) => rsx! {
                        ul { key: "{i}", class: "docx-list",
                            for (j , item) in group.iter().enumerate() {
                                // A landing place among the items: an empty item,
                                // because a list may only hold items.
                                if let Block::Anchor(id) = item {
                                    li { key: "{j}", id: "{id}", class: "pdf-anchor pdf-anchor-item" }
                                }
                                if let Block::ListItem { spans, marker, indent } = item {
                                    match marker {
                                        // The page drew its own bullet, and here
                                        // it is a logo rather than a dot, so it
                                        // is shown rather than stood in for. Sized
                                        // in ems so it follows the text.
                                        Some(src) => rsx! {
                                            li {
                                                key: "{j}",
                                                class: "pdf-li-drawn",
                                                style: "{indent_var(*indent)}",
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
                                            li { key: "{j}", style: "{indent_var(*indent)}",
                                                Spans { spans: spans.clone() }
                                            }
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
                                if let Block::Anchor(id) = entry {
                                    span { key: "{j}", id: "{id}", class: "pdf-anchor" }
                                }
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
                    Some(Block::PageBreak { ended, printed, starts }) => rsx! {
                        // Furniture, and marked as such: a separator carries no
                        // meaning to read aloud, and the number is what makes
                        // "see page 12" mean something to someone reading this
                        // rather than the pages. The number the page printed on
                        // itself wins over its position in the file, because
                        // that is the one the document's own index refers to.
                        div {
                            key: "{i}",
                            // Where the page AFTER this mark begins, so anything
                            // that wants to send a reader to a page has somewhere
                            // to send them: a contents row, or the scrubber.
                            id: "pdf-page-{ended + 1}",
                            class: "pdf-page-break",
                            role: "separator",
                            // The page that BEGINS here, which is the page a
                            // reader arriving at this mark is on. Absent when
                            // that page prints no number in a document that
                            // numbers its pages: an unnumbered back cover is not
                            // page 104 of a book whose last page is 99.
                            "data-page": match (starts.clone(), numbered) {
                                (Some(page), _) => page,
                                (None, false) => (ended + 1).to_string(),
                                (None, true) => String::new(),
                            },
                            span { {printed.clone().unwrap_or_else(|| ended.to_string())} }
                        }
                    },
                    Some(Block::Image(picture)) => match &picture.path {
                        // The page DREW this rather than placing it: a signature
                        // is a thousand line segments and no image at all. Drawn
                        // here too, in the reading colour, so it survives a dark
                        // surface as a black bitmap would not.
                        Some(d) => rsx! {
                            svg {
                                key: "{i}",
                                class: "pdf-drawing",
                                view_box: "0 0 {picture.width} {picture.height}",
                                width: "{picture.width}",
                                height: "{picture.height}",
                                role: "img",
                                path { d: "{d}", fill: "none", stroke: "currentColor", stroke_width: "1" }
                            }
                        },
                        // Drawn at the size the page drew it, but never wider
                        // than the column: this reflows, and a banner laid out
                        // for A4 would otherwise push the text sideways.
                        None => rsx! {
                            img {
                                key: "{i}",
                                class: "docx-img",
                                src: "{picture.src}",
                                style: "width:{picture.width}px;max-width:100%;height:auto;",
                                alt: "",
                                loading: "lazy",
                            }
                        },
                    },
                    None => rsx! {},
                }
            }
            // Where the reader is, and how to go elsewhere. Only for a document
            // with more than one page: a single page has nowhere to go.
            if doc.pages > 1 {
                PageControl { first: first_page(&doc), last: last_page(&doc) }
            }
            // Say what was left behind, in the Word renderer's own gap-notice
            // style rather than a class of this file's invention: a reader
            // deciding whether to open the real thing is owed the reason, and
            // it should look like the other one when they do.
            div { class: "file-gap-notice", role: "note",
                span { class: "material-icons", "info" }
                p { class: "body-small", {t_pages(&last_page(&doc))} }
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

/// The same depth, handed to the stylesheet as a value rather than a margin.
///
/// A list item's inset is not the whole story: a drawn bullet has to hang back
/// out of it into the margin, the way a list marker does. Setting the margin
/// here would overwrite the rule that does that, so the depth is passed in and
/// the stylesheet adds them up.
fn indent_var(indent: u8) -> String {
    match indent {
        0 => String::new(),
        n => format!("--pdf-indent:calc(var(--pdf-indent-step) * {n});"),
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
///
/// Counted the same way the page control counts, because saying "104 pages"
/// under a control that reads "37 / 99" makes a reader wonder which of the two
/// is lying. The document's own numbering wins in both: it is the one a reader
/// is counting, and the one its contents list refers to.
fn t_pages(pages: &str) -> String {
    crate::i18n::t_with("file.pdfReflowed", &[("pages", pages)])
}

/// What the document's first page calls itself. It has no mark of its own, since
/// nothing ended before it, so the first mark carries its number as the page
/// that ENDED there.
fn first_page(doc: &Extracted) -> String {
    doc.blocks
        .iter()
        .find_map(|b| match b {
            Block::PageBreak { printed, .. } => Some(printed.clone()),
            _ => None,
        })
        .flatten()
        .unwrap_or_else(|| "1".into())
}

/// The last page number the document PRINTS.
///
/// Not the number of sheets in the file: the songbook's hundred and four sheets
/// end on an unnumbered back cover, and the page its own last folio calls 99.
/// A reader counting pages is counting the printed ones, so "37 / 99" is the
/// pair that reads in one language.
fn last_page(doc: &Extracted) -> String {
    doc.blocks
        .iter()
        .rev()
        .find_map(|b| match b {
            Block::PageBreak { printed, .. } => printed.clone(),
            _ => None,
        })
        .unwrap_or_else(|| doc.pages.to_string())
}

/// Where the pages of the document on screen begin, and what each calls itself.
///
/// Read off the marks the view draws between its pages, which carry the number
/// the page printed on itself. Measured from the top of the document rather than
/// the window, so the answer does not depend on where the reader is standing.
fn pages_on_screen() -> Vec<(f64, String)> {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return Vec::new();
    };
    let Ok(marks) = document.query_selector_all(".pdf-page-break") else {
        return Vec::new();
    };
    let scrolled = web_sys::window()
        .and_then(|w| w.scroll_y().ok())
        .unwrap_or(0.0);
    let mut out = Vec::new();
    for at in 0..marks.length() {
        let Some(node) = marks.item(at) else { continue };
        let Ok(element) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        // A mark with no number on it is a page the document does not number,
        // and there is nothing for the control to say about it.
        let Some(label) = element.get_attribute("data-page").filter(|l| !l.is_empty()) else {
            continue;
        };
        out.push((element.get_bounding_client_rect().top() + scrolled, label));
    }
    out
}

/// Take the reader to a page by the number the page calls itself.
fn go_to_page(label: &str) {
    if label.is_empty() {
        return;
    }
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(mark) = document
        .query_selector(&format!("[data-page=\"{label}\"]"))
        .ok()
        .flatten()
    else {
        return;
    };
    let opts = web_sys::ScrollIntoViewOptions::new();
    opts.set_behavior(web_sys::ScrollBehavior::Smooth);
    opts.set_block(web_sys::ScrollLogicalPosition::Start);
    mark.scroll_into_view_with_scroll_into_view_options(&opts);
}

/// Which page a step lands on: the one `by` places along from where the reader
/// is, stopping at either end.
///
/// `None` means the top of the document: the first page has no mark of its own,
/// so there is nothing to scroll to, only somewhere to be.
fn step_to(labels: &[String], here: &str, by: i32) -> Option<String> {
    match labels.iter().position(|l| l == here) {
        // Back from the FIRST mark is the top, not the first mark again. The
        // page above it has no mark of its own, because nothing ended before
        // it, so clamping into the list left the control on the second page
        // with a back button that did nothing at all.
        Some(0) if by < 0 => None,
        Some(at) => {
            let next = (at as i64 + by as i64).clamp(0, labels.len() as i64 - 1) as usize;
            labels.get(next).cloned()
        }
        // Before the first mark, which is where the first page is: forward is
        // that mark, and back is the top.
        None if by > 0 => labels.first().cloned(),
        None => None,
    }
}

/// Which page the reader is on, given where they are: the last one that has
/// begun above them.
///
/// `allowance` is the room a jump leaves above the mark it lands on. Without it
/// this reports the page ABOVE the one a jump just landed on -- the mark sits
/// below the top of the window by exactly that much, so a strict comparison
/// says it has not been reached, the control snaps back to where it was, and
/// the next press works out the same answer and goes nowhere. That is the
/// "press it twice" this was reported as.
fn page_at(
    pages: &[(f64, String)],
    first: &str,
    scrolled: f64,
    allowance: f64,
    at_bottom: bool,
) -> String {
    // The end of the document is the last page, whatever the arithmetic says. A
    // final page shorter than the window can never be scrolled to the top of
    // it, so it would otherwise be the one page a reader can reach and never be
    // told they are on.
    if at_bottom {
        if let Some((_, label)) = pages.last() {
            return label.clone();
        }
    }
    match pages
        .iter()
        .take_while(|(top, _)| *top <= scrolled + allowance + 8.0)
        .last()
    {
        None => first.to_string(),
        Some((_, label)) => label.clone(),
    }
}

/// The room a jump leaves above the mark it lands on, read from the mark's own
/// `scroll-margin-top` rather than repeated here, so the stylesheet and this
/// cannot drift apart.
fn landing_allowance() -> f64 {
    let Some(window) = web_sys::window() else {
        return 0.0;
    };
    let Some(document) = window.document() else {
        return 0.0;
    };
    let Ok(Some(mark)) = document.query_selector(".pdf-page-break") else {
        return 0.0;
    };
    let Ok(Some(style)) = window.get_computed_style(&mark) else {
        return 0.0;
    };
    style
        .get_property_value("scroll-margin-top")
        .ok()
        .and_then(|v| v.trim().trim_end_matches("px").parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Which page the reader is on: the last one that has begun above them.
///
/// Above every mark is the first page, which has no mark of its own because
/// nothing ended before it, so it has to be told what it is called.
fn page_here(pages: &[(f64, String)], first: &str) -> String {
    let Some(window) = web_sys::window() else {
        return first.to_string();
    };
    let scrolled = window.scroll_y().unwrap_or(0.0);
    let seen = window
        .inner_height()
        .ok()
        .and_then(|h| h.as_f64())
        .unwrap_or(0.0);
    let tall = window
        .document()
        .and_then(|d| d.document_element())
        .map(|e| e.scroll_height() as f64)
        .unwrap_or(0.0);
    // Within a pixel or two of the end counts as the end: a fractional device
    // pixel ratio leaves the last scroll a hair short of the arithmetic.
    let at_bottom = tall > 0.0 && scrolled + seen >= tall - 2.0;
    page_at(pages, first, scrolled, landing_allowance(), at_bottom)
}

/// Where in the document the reader is, and how to go somewhere else.
///
/// DESIGN (functional: it says what it is). A hundred-page songbook reflows into
/// one long page, and the reader wants the song on page 37. The browser's own PDF
/// viewer — the other choice in this app's own sheet, one tap away — has a page
/// box, so that is what a reader expects a PDF to have, and an invisible gesture
/// on a hairline is not it. This says where you are, moves a page at a time, and
/// takes a number when you tap it.
///
/// The numbers are the pages' OWN, so "37" is the page the document's index calls
/// 37 rather than the thirty-seventh sheet of the file.
#[component]
fn PageControl(first: String, last: String) -> Element {
    let mut pages = use_signal(Vec::<(f64, String)>::new);
    let mut typing = use_signal(|| false);
    let mut typed = use_signal(String::new);
    let mut here = use_signal(String::new);
    // The first page's name, held in a signal so the handlers that need it stay
    // Copy: a closure that captures the String itself can only be given to one
    // of the two buttons.
    let top_page = use_signal(|| first.clone());

    // The marks move as pictures land and fonts settle, so their places are taken
    // again whenever the document's height changes. Scroll progress is what says
    // the reader moved, and it is already being tracked for the progress bar.
    let scrolled = crate::components::back_to_top::progress();
    let mut height = use_signal(|| 0);
    // PEEKED, every one of them, because this effect WRITES all three. Reading
    // them here subscribes the effect to its own output: saying where a jump is
    // going woke this up, and a smooth scroll has not moved yet when it does, so
    // it worked out that the reader was still on the page they were leaving and
    // put that back. The jump then had to be asked for twice. The scroll is the
    // only thing this should wake for.
    use_effect(use_reactive!(|(scrolled,)| {
        let _ = scrolled;
        let tall = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
            .map(|e| e.scroll_height())
            .unwrap_or(0);
        if tall != *height.peek() || pages.peek().is_empty() {
            height.set(tall);
            pages.set(pages_on_screen());
        }
        let now = page_here(&pages.peek(), &first);
        if now != *here.peek() {
            here.set(now);
        }
    }));

    // Where a jump is taking the reader, said straight away rather than waited
    // for. The scroll is what this otherwise learns from, and it is watched a
    // percent at a time: one page of a hundred moves it by about one, so a step
    // often left the control still holding the page it had just left, and the
    // next press worked out the same answer and went nowhere. Pressing forward
    // twice moved one page.
    let mut arrive = move |label: &str| {
        go_to_page(label);
        // Peeked into an OWNED string first. A signal's peek is a live borrow,
        // and this writes the same signal a line later; leaving the borrow in
        // the condition works only because an `if` drops it before the block,
        // which is too subtle to rely on next to a write.
        let at = here.peek().clone();
        if at != label {
            here.set(label.to_string());
        }
    };

    let mut step = move |by: i32| {
        let marks = pages.peek().clone();
        let labels: Vec<String> = marks.iter().map(|(_, l)| l.clone()).collect();
        // The page we are on, TAKEN OUT of the signal before the match. A
        // temporary made in a match's scrutinee lives until the end of the
        // whole match, so `step_to(.., &here.peek(), ..)` held a read borrow of
        // `here` across the arm that writes it, and pressing a page button
        // panicked: it scrolled, then died on the write. That looked from the
        // outside like a control that moved once and then ignored you, which is
        // how it was reported three times and misread as a timing fault.
        let at = here.peek().clone();
        match step_to(&labels, &at, by) {
            Some(label) => arrive(&label),
            None => {
                crate::components::back_to_top::scroll_to_top();
                // Said straight away, like a jump to a mark: the top is the
                // first page, and waiting for the scroll to prove it is what
                // made a press look ignored.
                let top = top_page.peek().clone();
                if *here.peek() != top {
                    here.set(top);
                }
            }
        }
    };

    let mut commit = move || {
        let wanted = typed.read().trim().to_string();
        typing.set(false);
        if !wanted.is_empty() {
            arrive(&wanted);
        }
    };

    rsx! {
        div { class: "pdf-pages", role: "group", aria_label: t("file.goToPage"),
            button {
                class: "pdf-pages-step",
                aria_label: t("file.previousPage"),
                onclick: move |_| step(-1),
                span { class: "material-icons", "chevron_left" }
            }
            if typing() {
                input {
                    class: "pdf-pages-field",
                    r#type: "text",
                    inputmode: "numeric",
                    autofocus: true,
                    value: "{typed}",
                    aria_label: t("file.goToPage"),
                    oninput: move |e| typed.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            commit();
                        }
                    },
                    onblur: move |_| commit(),
                }
            } else {
                button {
                    class: "pdf-pages-here",
                    aria_label: t("file.goToPage"),
                    onclick: move |_| {
                        typed.set(here.read().clone());
                        typing.set(true);
                    },
                    "{here} / {last}"
                }
            }
            button {
                class: "pdf-pages-step",
                aria_label: t("file.nextPage"),
                onclick: move |_| step(1),
                span { class: "material-icons", "chevron_right" }
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{page_at, step_to};

    fn songbook() -> Vec<String> {
        // What the songbook offers: its numbered pages, three to ninety-nine.
        // Its cover and front matter print no numbers and have no marks, and
        // neither does the back cover.
        (3..=99).map(|n| n.to_string()).collect()
    }

    /// One press, one page. This wanted two for a while, and the arithmetic is
    /// worth pinning down away from the scrolling that hid the fault.
    #[test]
    fn a_step_moves_one_page() {
        let pages = songbook();
        assert_eq!(step_to(&pages, "37", 1).as_deref(), Some("38"));
        assert_eq!(step_to(&pages, "37", -1).as_deref(), Some("36"));
    }

    /// Back from the first mark is the top of the document, where the first
    /// page is. It used to clamp into the list and hand back the mark it was
    /// already on, so the back button did nothing on page two.
    #[test]
    fn a_step_back_from_the_first_mark_is_the_top() {
        let pages = songbook();
        assert_eq!(step_to(&pages, "3", -1), None);
    }

    /// The marks of a four-sheet document that prints no numbers, as the DOM
    /// reports them: the first page has no mark, so the list starts at two.
    fn fixture() -> Vec<(f64, String)> {
        vec![
            (464.0, "2".into()),
            (748.0, "3".into()),
            (960.0, "4".into()),
        ]
    }

    /// A jump lands the mark BELOW the top of the window, by exactly the room
    /// the stylesheet leaves above it. Measured in a browser: pressing forward
    /// scrolled to 384 for a mark at 464, and the control then said page one,
    /// which is the fault reported as "you have to press twice".
    #[test]
    fn a_page_jumped_to_is_the_page_you_are_on() {
        let pages = fixture();
        assert_eq!(page_at(&pages, "1", 384.0, 80.0, false), "2");
        // And with no allowance at all it is the old, wrong answer, which is
        // what this test exists to keep from coming back.
        assert_eq!(page_at(&pages, "1", 384.0, 0.0, false), "1");
    }

    /// The top of the document is the first page, which has no mark.
    #[test]
    fn the_top_is_the_first_page() {
        assert_eq!(page_at(&fixture(), "1", 0.0, 80.0, false), "1");
    }

    /// The end of the document is its last page even when that page is shorter
    /// than the window, which is every short document: its last mark sits below
    /// anywhere the scroll can reach.
    #[test]
    fn the_end_of_the_document_is_its_last_page() {
        assert_eq!(page_at(&fixture(), "1", 819.0, 80.0, true), "4");
        // 819 is as far as that document scrolls, and the mark is at 960, so
        // without the end-of-document rule it is unreachable.
        assert_eq!(page_at(&fixture(), "1", 819.0, 80.0, false), "3");
    }

    /// And stops at the end rather than falling off it. The other end is not a
    /// stop but a place: see `a_step_back_from_the_first_mark_is_the_top`.
    #[test]
    fn a_step_stops_at_the_end() {
        let pages = songbook();
        assert_eq!(
            step_to(&pages, "99", 1).as_deref(),
            Some("99"),
            "the last page"
        );
    }

    /// The first page of a document has no mark, since nothing ended before it.
    /// Forward from there is the first mark; back is the top of the document,
    /// which is a scroll rather than a page.
    #[test]
    fn the_first_page_has_nowhere_behind_it() {
        let pages = songbook();
        assert_eq!(step_to(&pages, "1", 1).as_deref(), Some("3"));
        assert_eq!(step_to(&pages, "1", -1), None);
        // And a document with no marks at all goes nowhere in either direction.
        assert_eq!(step_to(&[], "1", 1), None);
    }
}

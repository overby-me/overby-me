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
/// Whether a cell holds a figure rather than words.
///
/// Right-aligned when it does, the way the page set it: a column of amounts
/// reads down its last digit, and ragged-right numbers are a column only by
/// accident. Danish figures carry thousands dots and a decimal comma, an amount
/// may be negative or bracketed, and a note number is a bare digit -- all of
/// them numbers, none of them plain integers.
fn cell_is_number(cell: &[Span]) -> bool {
    let text: String = cell.iter().map(|s| s.text.as_str()).collect();
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let digits = text.chars().filter(char::is_ascii_digit).count();
    digits > 0
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '(' | ')' | '%' | ' ' | '\u{2013}' | '\u{2212}'))
}

fn unlinked(spans: &[Span]) -> Vec<Span> {
    spans
        .iter()
        .map(|s| Span {
            link: None,
            ..s.clone()
        })
        .collect()
}

/// How this app draws a PDF: as the pages were set, or reflowed to the column.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PdfLayout {
    /// Each page at its own proportions, everything where the page put it.
    Page,
    /// The words, in the reading order, wrapped to whatever width there is.
    /// The default: most of what this wiki carries is read on a phone at a
    /// meeting, and there a fixed A4 page is a thing to pinch at.
    Reflow,
}

impl PdfLayout {
    fn key(self) -> &'static str {
        match self {
            PdfLayout::Page => "page",
            PdfLayout::Reflow => "reflow",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            PdfLayout::Page => "file.pdfLayoutPage",
            PdfLayout::Reflow => "file.pdfLayoutReflow",
        }
    }

    /// Anything unrecognised is the default, which is the reading view.
    fn from_key(key: &str) -> Self {
        match key {
            "page" => PdfLayout::Page,
            _ => PdfLayout::Reflow,
        }
    }
}

/// The chosen layout, remembered per device like the viewer choices.
pub static PDF_LAYOUT: GlobalSignal<PdfLayout> = Signal::global(|| {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("wiki_pdf_layout").ok().flatten())
        .map(|v| PdfLayout::from_key(&v))
        .unwrap_or(PdfLayout::Reflow)
});

/// Choose a layout, and remember it.
pub fn set_pdf_layout(layout: PdfLayout) {
    *PDF_LAYOUT.write() = layout;
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("wiki_pdf_layout", layout.key());
    }
}

/// The document as it was laid out: each page at its own size, everything where
/// the page put it.
///
/// Possible because the substitute faces are metric-compatible with what a PDF
/// normally carries (Liberation Sans for Helvetica and Arial, Liberation Serif
/// for Times, Carlito for Calibri): a run placed at its x, at its size, in the
/// stand-in face, is the width the page drew it. Nothing is measured in the
/// browser and nothing is scaled to fit afterwards.
///
/// Sized in container units, so a page is the width it is given and everything
/// on it scales with it: on a phone that is a whole A4 page shrunk to the
/// screen, which is what a PDF looks like everywhere else.
#[component]
fn PdfPages(pages: Vec<crate::pdf_text::PageLayout>) -> Element {
    use crate::pdf_text::{Family, What};
    rsx! {
        // `pdf-sheets`, not `pdf-pages`: the page CONTROL has had that name
        // since it was built, and taking it made this stack `position: fixed`
        // and eight pixels wide, which took every page on it to nothing.
        div { class: "pdf-sheets",
            for (n , page) in pages.iter().enumerate() {
                div {
                    key: "{n}",
                    class: "pdf-page",
                    id: "pdf-page-{n + 1}",
                    "data-page": "{n + 1}",
                    style: "aspect-ratio: {page.width} / {page.height};",
                    for (i , item) in page.items.iter().enumerate() {
                        {
                            // Every measurement as a share of the page, so the
                            // whole thing scales with whatever box it is given.
                            let left = item.x / page.width * 100.0;
                            let top = item.y / page.height * 100.0;
                            let wide = item.width / page.width * 100.0;
                            match &item.what {
                                What::Text { text, size, color, bold, italic, family, .. } => {
                                    let face = match family {
                                        Family::Serif => "var(--pdf-serif)",
                                        Family::Calibri => "var(--pdf-calibri)",
                                        Family::Cambria => "var(--pdf-cambria)",
                                        Family::Mono => "var(--pdf-mono)",
                                        Family::Sans => "var(--pdf-sans)",
                                    };
                                    let ink = color.clone().unwrap_or_else(|| "inherit".to_string());
                                    let em = size / page.width * 100.0;
                                    // Computed here: a format segment in rsx
                                    // takes an expression, not a statement.
                                    let weight = match bold { true => 700, false => 400 };
                                    let slant = match italic { true => "italic", false => "normal" };
                                    rsx! {
                                        span {
                                            key: "{i}",
                                            class: "pdf-run",
                                            style: "left: {left:.3}%; top: {top:.3}%; font-size: {em:.3}cqw; font-family: {face}; color: {ink}; font-weight: {weight}; font-style: {slant};",
                                            "{text}"
                                        }
                                    }
                                }
                                What::Image(picture) => {
                                    let tall = item.height / page.height * 100.0;
                                    match &picture.path {
                                        Some(d) => rsx! {
                                            svg {
                                                key: "{i}",
                                                class: "pdf-page-art",
                                                style: "left: {left:.3}%; top: {top:.3}%; width: {wide:.3}%; height: {tall:.3}%;",
                                                view_box: "0 0 {picture.width} {picture.height}",
                                                path { d: "{d}", fill: "none", stroke: "currentColor", stroke_width: "1" }
                                            }
                                        },
                                        None => rsx! {
                                            img {
                                                key: "{i}",
                                                class: "pdf-page-img",
                                                style: "left: {left:.3}%; top: {top:.3}%; width: {wide:.3}%; height: {tall:.3}%;",
                                                src: "{picture.src}",
                                                alt: "",
                                            }
                                        },
                                    }
                                }
                                What::Rule => {
                                    let thick = item.height.max(0.4) / page.height * 100.0;
                                    rsx! {
                                        div {
                                            key: "{i}",
                                            class: "pdf-page-rule",
                                            style: "left: {left:.3}%; top: {top:.3}%; width: {wide:.3}%; height: {thick:.3}%;",
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

/// Render what was read out of a PDF.
///
/// Reflowed to the reading column by default, because that is how this is read:
/// on a phone, at a meeting, where a fixed A4 page is a thing to pinch at.
///
/// The document as it was SET is one tap away in [`PDF_LAYOUT`], and it is the
/// honest view of a page whose layout carries meaning -- the annual report's
/// cover is nine-tenths deliberate whitespace, which no reflow can keep.
#[component]
pub fn PdfDocument(doc: Extracted) -> Element {
    if PDF_LAYOUT() == PdfLayout::Page && !doc.layout.is_empty() {
        return rsx! { PdfPages { pages: doc.layout.clone() } };
    }
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
    // What an ordinary line weighs in this document: the middle of the weights it
    // draws. Everything is measured against it, so "heavier than the rest" is a
    // property of the document rather than a number of points -- which is the
    // only thing that survives a reflow.
    let ordinary_rule = {
        let mut weights: Vec<f64> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Rule { thickness, .. } => Some(*thickness),
                _ => None,
            })
            .collect();
        weights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        weights.get(weights.len() / 2).copied().unwrap_or(0.0)
    };
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
                            // This mark is a page's ENDING as well as the next
                            // one's start, and it says so on its face -- it
                            // shows the number of the page that finished here.
                            // So a jump to it lands on what comes after (see
                            // `pager::go_to_page`), or a reader turning to 38
                            // arrives looking at a hairline reading 37.
                            "data-page-ends": "true",
                            span { {printed.clone().unwrap_or_else(|| ended.to_string())} }
                        }
                    },
                    // Rows that stood in the same columns. Laid out as a real
                    // table so the columns line up here as they did there: a
                    // name over its role, a figure under its year. Scrolls
                    // inside itself rather than pushing the reading column
                    // wider, which is the one thing a phone cannot give it.
                    Some(Block::Table { rows }) => rsx! {
                        div { key: "{i}", class: "pdf-table-scroll",
                            table { class: "pdf-table",
                                tbody {
                                    for (r , row) in rows.iter().enumerate() {
                                        tr { key: "{r}",
                                            for (c , cell) in row.iter().enumerate() {
                                                td {
                                                    key: "{c}",
                                                    class: if cell_is_number(cell) { "pdf-cell-number" } else { "" },
                                                    Spans { spans: cell.clone() }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    // A line the page drew across itself. It cannot be kept
                    // where it was -- the text under it is a different width
                    // now -- but it separated one thing from another, and drawn
                    // to the share of the width it spanned it still says which.
                    Some(Block::Rule { width, thickness }) => {
                        // Heavy RELATIVE to the document's own lines, not in
                        // points. A reflowed page is re-typeset, and a page's
                        // rules are mostly under a point: converting them to
                        // pixels put every one of them under the floor of one,
                        // so a hairline and a bar came out identical -- which is
                        // what a reader saw. Against the document's ordinary
                        // rule, a bar is a multiple and shows as one.
                        // Not rounded. A browser draws a fractional border as a
                        // lighter or darker line rather than snapping it, so a
                        // document whose weights differ by a fifth still reads
                        // as two weights; rounding to whole pixels put every
                        // rule in this report back on the same 1px line.
                        let heavy = match ordinary_rule > 0.0 {
                            true => (thickness / ordinary_rule).clamp(0.7, 4.0),
                            false => 1.0,
                        };
                        rsx! {
                            hr {
                                key: "{i}",
                                class: "pdf-rule",
                                style: "--rule-width: {(width * 100.0).clamp(8.0, 100.0):.1}%; --rule-thickness: {heavy:.1}px;",
                            }
                        }
                    }
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
            // The last page's number, which the file itself never marks: its
            // breaks sit BETWEEN pages, so the number of the page after the
            // last break is nowhere on the document.
            if doc.pages > 1 {
                super::pager::LastPageMark { page: last_page(&doc) }
            }
            // Where the reader is, and how to go elsewhere. Only for a document
            // with more than one page: a single page has nowhere to go.
            if doc.pages > 1 {
                super::pager::PageControl { first: first_page(&doc), last: last_page(&doc) }
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

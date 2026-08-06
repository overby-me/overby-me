//! Where in a paged document the reader is, and how to go somewhere else.
//!
//! Written for the reflowing PDF reader and shared, because the need is not
//! about PDFs: a long document read as one column has no pages to hold, and
//! "see page 37" is how a room of people refer to one. Anything that can say
//! where its pages begin gets this control by marking them.
//!
//! The contract is one attribute. An element with `data-page="37"` says "page
//! 37 begins here"; the control lists those marks, says which one the reader is
//! below, and scrolls to them. A mark that also carries `data-page-ends` is a
//! page's ENDING rather than its beginning -- the hairline the PDF reader draws
//! between two pages -- so a jump aiming at it lands on what follows instead.
//!
//! What a mark is worth depends on the format, and that belongs to the format:
//! a slide IS a page, so PowerPoint's marks are exact; a PDF's are the pages the
//! file draws; a Word file has no pagination in it at all.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use crate::i18n::t;

/// How long a jump's own answer outranks the scroll's, in milliseconds.
///
/// Long enough to cover a smooth scroll, which the browser runs for a few
/// hundred milliseconds; short enough that a reader who scrolls by hand
/// straight after a jump is followed again almost at once.
const JUMP_HOLD_MS: f64 = 1200.0;

/// Milliseconds on the page's own clock. Only differences matter.
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

/// Every page mark on screen: where it sits in the document, and what it calls
/// its page.
fn pages_on_screen() -> Vec<(f64, String)> {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return Vec::new();
    };
    let Ok(marks) = document.query_selector_all("[data-page]") else {
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
    // A mark that ENDS a page is not where the reader wants to be: it carries
    // the number of the page that finished there, so arriving on it put "37"
    // across the top of the window for a reader who had just turned to 38,
    // which reads as not having moved at all. The page asked for starts on the
    // next line down. A mark that IS the page -- a slide -- is landed on.
    let target = match mark.has_attribute("data-page-ends") {
        true => mark.next_element_sibling().unwrap_or(mark),
        false => mark,
    };
    let opts = web_sys::ScrollIntoViewOptions::new();
    opts.set_behavior(web_sys::ScrollBehavior::Smooth);
    opts.set_block(web_sys::ScrollLogicalPosition::Start);
    target.scroll_into_view_with_scroll_into_view_options(&opts);
}

/// Which page a step lands on: the one `by` places along from where the reader
/// is, stopping at either end.
///
/// `None` means the top of the document: where the first page has no mark of
/// its own, there is nothing to scroll to, only somewhere to be.
fn step_to(labels: &[String], here: &str, by: i32) -> Option<String> {
    match labels.iter().position(|l| l == here) {
        // Back from the FIRST mark is the top, not the first mark again. Where
        // the page above it has no mark -- a PDF's first page, since nothing
        // ended before it -- clamping into the list left the control on the
        // second page with a back button that did nothing at all.
        Some(0) if by < 0 => None,
        Some(at) => {
            let next = (at as i64 + by as i64).clamp(0, labels.len() as i64 - 1) as usize;
            labels.get(next).cloned()
        }
        // Before the first mark: forward is that mark, back is the top.
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
    let Ok(Some(mark)) = document.query_selector("[data-page]") else {
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

/// The rule at the foot of the LAST page, carrying its number.
///
/// Every other page's number is drawn where that page ENDS, on the hairline
/// between it and the next one. The last page has no next one, so its number
/// was the single number a reader could never see on the document itself --
/// they could reach the last page and still not know what it is called, which
/// is the number wanted when someone says "it is on the last page".
///
/// It carries no `data-page`, because nothing begins here: a mark that claimed
/// to would add a page to the control with nothing on it.
#[component]
pub fn LastPageMark(page: String) -> Element {
    rsx! {
        div { class: "pdf-page-break pdf-page-last", role: "separator",
            span { "{page}" }
        }
    }
}

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
/// box, so that is what a reader expects a paged document to have, and an
/// invisible gesture on a hairline is not it. This says where you are, moves a
/// page at a time, and takes a number when you tap it.
///
/// The numbers are the pages' OWN, so "37" is the page the document's index
/// calls 37 rather than the thirty-seventh sheet of the file.
#[component]
pub fn PageControl(first: String, last: String) -> Element {
    let mut pages = use_signal(Vec::<(f64, String)>::new);
    let mut typing = use_signal(|| false);
    let mut typed = use_signal(String::new);
    let mut here = use_signal(String::new);
    // The first page's name, held in a signal so the handlers that need it stay
    // Copy: a closure that captures the String itself can only be given to one
    // of the two buttons.
    let top_page = use_signal(|| first.clone());
    // Until when a jump's own answer outranks the scroll's. See `arrive`.
    let mut jumping_until = use_signal(|| 0.0f64);

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
        // Not while a jump is in flight. The scroll is watched a PERCENT at a
        // time, so on a hundred-page document one page moves it by about one and
        // this runs about once per page -- and the once it runs is in the middle
        // of the smooth scroll, over the page being left. It wrote that back,
        // and no later tick came to correct it, so the reader saw the page they
        // had just left and pressed again. That is the "press it twice" a short
        // document cannot show: there, one page is a quarter of the scroll and
        // the ticks after the landing put it right.
        if now_ms() < *jumping_until.peek() {
            return;
        }
        let now = page_here(&pages.peek(), &first);
        if now != *here.peek() {
            here.set(now);
        }
    }));

    // Where a jump is taking the reader, said straight away rather than waited
    // for, and HELD while the scroll catches up. A smooth scroll reports its
    // position all the way there, and the reckoning above would otherwise read
    // one of those positions -- still over the page being left -- and put it
    // back. The hold is a moment, not a state: nothing has to clear it, and if
    // a jump somehow never lands the control is following the scroll again a
    // second later.
    let mut arrive = move |label: &str| {
        go_to_page(label);
        jumping_until.set(now_ms() + JUMP_HOLD_MS);
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
                jumping_until.set(now_ms() + JUMP_HOLD_MS);
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

    /// The first page of a PDF has no mark, since nothing ended before it.
    /// Forward from there is the first mark; back is the top of the document,
    /// which is where it already is.
    #[test]
    fn the_first_page_has_nowhere_behind_it() {
        let pages = songbook();
        assert_eq!(step_to(&pages, "1", 1).as_deref(), Some("3"));
        assert_eq!(step_to(&pages, "1", -1), None);
        // And a document with no marks at all goes nowhere in either direction.
        assert_eq!(step_to(&[], "1", 1), None);
    }

    /// A slide deck marks its slides, and a slide IS a page: the first has a
    /// mark of its own, so back from it stays put rather than meaning the top.
    #[test]
    fn a_deck_numbers_every_slide_including_the_first() {
        let slides: Vec<String> = (1..=12).map(|n| n.to_string()).collect();
        assert_eq!(step_to(&slides, "1", 1).as_deref(), Some("2"));
        assert_eq!(step_to(&slides, "2", -1).as_deref(), Some("1"));
        assert_eq!(step_to(&slides, "12", 1).as_deref(), Some("12"));
        // Slide one has a mark, so stepping back from it is not the top: the
        // top IS slide one.
        assert_eq!(step_to(&slides, "1", -1), None);
    }
}

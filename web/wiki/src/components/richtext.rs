//! Slate <-> HTML bridge for the rich text editor.
//!
//! The document model stored in `data.content` is Slate JSON (the same shape the
//! old React wiki produced, so content stays interoperable and the ODT export in
//! [`crate::export`] keeps working). The editing surface is a `contenteditable`
//! element driven by the browser's `execCommand`; this module converts the Slate
//! model to the HTML that seeds that element ([`slate_to_html`]) and converts the
//! edited DOM back to Slate on save ([`dom_to_slate`]). It also wraps the
//! `execCommand` / selection plumbing the toolbar needs.

use serde_json::Value;

mod serialize;
pub use serialize::{slate_to_html, strip_leading_empty_paragraph};
// The DOM -> Slate parser and the paste sanitiser (both browser-only) reuse the
// shared HTML escaper.
#[cfg(target_arch = "wasm32")]
use serialize::html_escape;

// ---------------------------------------------------------------------------
// HTML DOM -> Slate (serialize on save)
// ---------------------------------------------------------------------------

/// Accumulated inline marks while walking down the DOM tree.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Default)]
struct Marks {
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    code: bool,
    link: Option<String>,
}

/// Build a Slate text leaf from `text` and the active marks.
#[cfg(target_arch = "wasm32")]
fn make_leaf(text: &str, m: &Marks) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("text".to_string(), Value::from(text));
    if m.bold {
        o.insert("bold".to_string(), Value::Bool(true));
    }
    if m.italic {
        o.insert("italic".to_string(), Value::Bool(true));
    }
    if m.underline {
        o.insert("underline".to_string(), Value::Bool(true));
    }
    if m.strikethrough {
        o.insert("strikethrough".to_string(), Value::Bool(true));
    }
    if m.code {
        o.insert("code".to_string(), Value::Bool(true));
    }
    if let Some(link) = &m.link {
        o.insert("link".to_string(), Value::from(link.clone()));
    }
    Value::Object(o)
}

// ---------------------------------------------------------------------------
// Auto-link detection (#97) — pure helpers, unit-tested on the host. The
// leaf-splitting that consumes them lives in `mod dom` (wasm-only), but the
// tricky part (what counts as a link) is here so it can be tested without a DOM.
// ---------------------------------------------------------------------------

/// If `token` (one whitespace-delimited word, already stripped of surrounding
/// punctuation) is a URL or email, return its `href`.
///
/// Deliberately conservative: only an explicit `http(s)://` or `www.` prefix, or
/// a clear `local@domain.tld` email, links. Bare domains are left alone so prose
/// and code like `main.rs`, `e.g.` or `v1.2` do not turn into links.
#[cfg(any(target_arch = "wasm32", test))]
fn link_href(token: &str) -> Option<String> {
    if let Some(rest) = token
        .strip_prefix("https://")
        .or_else(|| token.strip_prefix("http://"))
    {
        // Need a host (dot-bearing, non-empty) after the scheme.
        return (rest.len() >= 3 && rest.contains('.') && !rest.starts_with('.'))
            .then(|| token.to_string());
    }
    if let Some(rest) = token.strip_prefix("www.") {
        return (rest.contains('.') && rest.len() >= 3).then(|| format!("https://{token}"));
    }
    is_email(token).then(|| format!("mailto:{token}"))
}

/// A conservative `local@domain.tld` check: exactly one `@`, an alnum-ish local
/// part, and a domain whose last label is a 2+ letter alphabetic TLD.
#[cfg(any(target_arch = "wasm32", test))]
fn is_email(token: &str) -> bool {
    let mut parts = token.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if local.is_empty()
        || !local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
    {
        return false;
    }
    let Some(dot) = domain.rfind('.') else {
        return false;
    };
    let (host, tld) = (&domain[..dot], &domain[dot + 1..]);
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return false;
    }
    tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
}

/// Split a word into `(leading_punct, link_text, href, trailing_punct)` when its
/// core is a URL/email, else `None`. So `(https://x.io).` links only the URL and
/// keeps the surrounding `(` and `).` as plain text.
#[cfg(any(target_arch = "wasm32", test))]
fn split_link_word(word: &str) -> Option<(String, String, String, String)> {
    let open: &[char] = &['(', '[', '<', '"', '\'', '{'];
    let close: &[char] = &['.', ',', ')', ']', '>', '!', '?', ';', ':', '"', '\'', '}'];
    let after_open = word.trim_start_matches(open);
    let pre = &word[..word.len() - after_open.len()];
    let core = after_open.trim_end_matches(close);
    let post = &after_open[core.len()..];
    if core.is_empty() {
        return None;
    }
    let href = link_href(core)?;
    Some((pre.to_string(), core.to_string(), href, post.to_string()))
}

/// Split a bare text run into consecutive `(text, Some(href) | None)` segments,
/// linking any URL/email words and preserving all surrounding whitespace and
/// punctuation. Pure, so the splitting is unit-tested without a DOM.
#[cfg(any(target_arch = "wasm32", test))]
fn link_segments(text: &str) -> Vec<(String, Option<String>)> {
    let mut segs: Vec<(String, Option<String>)> = Vec::new();
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            plain.push(c);
            chars.next();
            continue;
        }
        let mut word = String::new();
        while let Some(&c2) = chars.peek() {
            if c2.is_whitespace() {
                break;
            }
            word.push(c2);
            chars.next();
        }
        match split_link_word(&word) {
            Some((pre, link_text, href, post)) => {
                plain.push_str(&pre);
                if !plain.is_empty() {
                    segs.push((std::mem::take(&mut plain), None));
                }
                segs.push((link_text, Some(href)));
                plain.push_str(&post);
            }
            None => plain.push_str(&word),
        }
    }
    if !plain.is_empty() {
        segs.push((plain, None));
    }
    segs
}

#[cfg(target_arch = "wasm32")]
mod dom {
    use super::*;
    use wasm_bindgen::JsCast;
    use web_sys::{Element, HtmlDocument, Node};

    /// The document as an `HtmlDocument` (derefs to `Document`, and carries the
    /// `execCommand` / `queryCommand*` methods the toolbar uses).
    fn document() -> Option<HtmlDocument> {
        web_sys::window()?.document()?.dyn_into().ok()
    }

    fn is_block_tag(tag: &str) -> bool {
        matches!(
            tag,
            "P" | "DIV"
                | "H1"
                | "H2"
                | "H3"
                | "H4"
                | "H5"
                | "H6"
                | "BLOCKQUOTE"
                | "PRE"
                | "UL"
                | "OL"
                | "LI"
                | "IMG"
        )
    }

    /// Fold a `<span style="...">`'s inline styling into the active marks, for
    /// content pasted (or produced) as CSS spans rather than semantic tags.
    fn apply_span_style(el: &Element, m: &mut Marks) {
        let Some(html) = el.dyn_ref::<web_sys::HtmlElement>() else {
            return;
        };
        let style = html.style();
        if let Ok(weight) = style.get_property_value("font-weight") {
            if weight == "bold" || weight == "700" || weight == "bolder" {
                m.bold = true;
            }
        }
        if let Ok(fs) = style.get_property_value("font-style") {
            if fs == "italic" {
                m.italic = true;
            }
        }
        if let Ok(deco) = style.get_property_value("text-decoration") {
            if deco.contains("underline") {
                m.underline = true;
            }
            if deco.contains("line-through") {
                m.strikethrough = true;
            }
        }
    }

    /// The Slate `align` for an element's `text-align`, if any. Firefox's
    /// `justifyCenter` & co (with `styleWithCSS = false`) set the legacy
    /// `align` ATTRIBUTE rather than an inline style, so read both.
    fn read_align(el: &Element) -> Option<String> {
        let ta = el
            .dyn_ref::<web_sys::HtmlElement>()
            .and_then(|h| h.style().get_property_value("text-align").ok())
            .filter(|s| !s.is_empty())
            .or_else(|| el.get_attribute("align"))?;
        match ta.as_str() {
            "center" => Some("center".to_string()),
            "right" => Some("right".to_string()),
            "justify" => Some("justify".to_string()),
            // `left` is the default; storing it would just add noise.
            _ => None,
        }
    }

    /// Append the leaves for a bare (not-already-linked) text run, wrapping any
    /// URL/email words in a `link` mark so typed or pasted links become
    /// clickable on save (#97). Idempotent: text already inside an `<a>` (its
    /// `link` mark set) is emitted unchanged, so re-editing never double-links.
    fn push_autolinked(text: &str, marks: &Marks, out: &mut Vec<Value>) {
        if marks.link.is_some() || !text.contains(|c: char| !c.is_whitespace()) {
            out.push(make_leaf(text, marks));
            return;
        }
        for (seg, href) in link_segments(text) {
            let mut m = marks.clone();
            if href.is_some() {
                m.link = href;
            }
            out.push(make_leaf(&seg, &m));
        }
    }

    /// Walk one node (text or element), accumulating marks, appending leaves.
    /// The last `<br>` of a block is skipped (the browser's bogus trailing
    /// break) via `is_last`.
    fn collect_node(node: &Node, marks: &Marks, out: &mut Vec<Value>, is_last: bool) {
        match node.node_type() {
            Node::TEXT_NODE => {
                if let Some(t) = node.text_content() {
                    if !t.is_empty() {
                        push_autolinked(&t, marks, out);
                    }
                }
            }
            Node::ELEMENT_NODE => {
                let Some(el) = node.dyn_ref::<Element>() else {
                    return;
                };
                let tag = el.tag_name().to_uppercase();
                if tag == "BR" {
                    if !is_last {
                        out.push(make_leaf("\n", marks));
                    }
                    return;
                }
                let mut m = marks.clone();
                match tag.as_str() {
                    "B" | "STRONG" => m.bold = true,
                    "I" | "EM" => m.italic = true,
                    "U" | "INS" => m.underline = true,
                    "S" | "STRIKE" | "DEL" => m.strikethrough = true,
                    "CODE" | "KBD" | "SAMP" => m.code = true,
                    // Drop dangerous link schemes (javascript:, data:, ...) as the
                    // href enters the stored model, so a pasted/edited link can
                    // never persist one. Viewers additionally re-check via
                    // `content::safe_href`.
                    "A" => {
                        m.link = el
                            .get_attribute("href")
                            .filter(|h| !h.is_empty())
                            .filter(|h| crate::components::content::safe_href(h) != "#")
                    }
                    "SPAN" | "FONT" => apply_span_style(el, &mut m),
                    _ => {}
                }
                collect_children(node, &m, out);
            }
            _ => {}
        }
    }

    /// Append the leaves for a node's children, honouring the trailing-`<br>`
    /// rule.
    fn collect_children(node: &Node, marks: &Marks, out: &mut Vec<Value>) {
        let kids = node.child_nodes();
        let len = kids.length();
        for i in 0..len {
            if let Some(child) = kids.get(i) {
                collect_node(&child, marks, out, i + 1 == len);
            }
        }
    }

    /// The inline leaves of a block element (never empty).
    fn block_leaves(el: &Element) -> Vec<Value> {
        let mut leaves = Vec::new();
        collect_children(el.as_ref(), &Marks::default(), &mut leaves);
        if leaves.is_empty() {
            leaves.push(make_leaf("", &Marks::default()));
        }
        leaves
    }

    fn object(pairs: Vec<(&str, Value)>) -> Value {
        let mut o = serde_json::Map::new();
        for (k, v) in pairs {
            o.insert(k.to_string(), v);
        }
        Value::Object(o)
    }

    /// Convert one block-level element to a Slate block.
    fn block_from_element(el: &Element) -> Value {
        let tag = el.tag_name().to_uppercase();

        if tag == "IMG" {
            let url = el.get_attribute("src").unwrap_or_default();
            return object(vec![
                ("type", Value::from("image")),
                ("url", Value::from(url)),
                (
                    "children",
                    Value::Array(vec![make_leaf("", &Marks::default())]),
                ),
            ]);
        }

        if tag == "UL" || tag == "OL" {
            let ty = if tag == "UL" {
                "bulleted-list"
            } else {
                "numbered-list"
            };
            let mut items = Vec::new();
            let kids = el.child_nodes();
            for i in 0..kids.length() {
                let Some(child) = kids.get(i) else { continue };
                if child.node_type() != Node::ELEMENT_NODE {
                    continue;
                }
                let Some(cel) = child.dyn_ref::<Element>() else {
                    continue;
                };
                if cel.tag_name().to_uppercase() == "LI" {
                    items.push(block_from_element(cel));
                }
            }
            if items.is_empty() {
                items.push(object(vec![
                    ("type", Value::from("list-item")),
                    (
                        "children",
                        Value::Array(vec![make_leaf("", &Marks::default())]),
                    ),
                ]));
            }
            return object(vec![
                ("type", Value::from(ty)),
                ("children", Value::Array(items)),
            ]);
        }

        let ty = match tag.as_str() {
            "H1" => "heading-one",
            "H2" => "heading-two",
            "H3" => "heading-three",
            "H4" => "heading-four",
            "H5" => "heading-five",
            "H6" => "heading-six",
            "BLOCKQUOTE" => "block-quote",
            "PRE" => "block-pre",
            "LI" => "list-item",
            _ => "paragraph",
        };
        // Firefox's list commands wrap the list in the line's own <div>
        // (`<div><ul>…</ul></div>`); serializing that wrapper as a paragraph
        // would flatten the list to plain text. When a paragraph-ish element
        // holds exactly one block child and nothing else, unwrap into it.
        if ty == "paragraph" {
            let kids = el.child_nodes();
            let mut only_block: Option<Element> = None;
            let mut extra = false;
            for i in 0..kids.length() {
                let Some(child) = kids.get(i) else { continue };
                match child.node_type() {
                    Node::ELEMENT_NODE => {
                        let Some(cel) = child.dyn_ref::<Element>() else {
                            continue;
                        };
                        if is_block_tag(&cel.tag_name().to_uppercase()) && only_block.is_none() {
                            only_block = Some(cel.clone());
                        } else {
                            extra = true;
                        }
                    }
                    Node::TEXT_NODE
                        if child.text_content().is_some_and(|t| !t.trim().is_empty()) =>
                    {
                        extra = true;
                    }
                    _ => {}
                }
            }
            if let Some(inner) = only_block {
                if !extra {
                    return block_from_element(&inner);
                }
            }
        }
        let mut pairs = vec![("type", Value::from(ty))];
        if let Some(align) = read_align(el) {
            pairs.push(("align", Value::from(align)));
        }
        pairs.push(("children", Value::Array(block_leaves(el))));
        object(pairs)
    }

    /// Serialize the `contenteditable` element's DOM into a Slate content array.
    pub fn dom_to_slate(container: &Element) -> Value {
        let mut blocks: Vec<Value> = Vec::new();
        let mut pending: Vec<Value> = Vec::new();

        let flush = |pending: &mut Vec<Value>, blocks: &mut Vec<Value>| {
            if !pending.is_empty() {
                blocks.push(object(vec![
                    ("type", Value::from("paragraph")),
                    ("children", Value::Array(std::mem::take(pending))),
                ]));
            }
        };

        let kids = container.child_nodes();
        let len = kids.length();
        for i in 0..len {
            let Some(node) = kids.get(i) else { continue };
            match node.node_type() {
                Node::ELEMENT_NODE => {
                    let Some(el) = node.dyn_ref::<Element>() else {
                        continue;
                    };
                    let tag = el.tag_name().to_uppercase();
                    if is_block_tag(&tag) {
                        flush(&mut pending, &mut blocks);
                        blocks.push(block_from_element(el));
                    } else {
                        // Loose inline element at the top level: fold into a
                        // paragraph.
                        collect_node(&node, &Marks::default(), &mut pending, i + 1 == len);
                    }
                }
                Node::TEXT_NODE => {
                    if let Some(t) = node.text_content() {
                        if !t.trim().is_empty() {
                            pending.push(make_leaf(&t, &Marks::default()));
                        }
                    }
                }
                _ => {}
            }
        }
        flush(&mut pending, &mut blocks);

        if blocks.is_empty() {
            blocks.push(object(vec![
                ("type", Value::from("paragraph")),
                (
                    "children",
                    Value::Array(vec![make_leaf("", &Marks::default())]),
                ),
            ]));
        }
        Value::Array(blocks)
    }

    // -----------------------------------------------------------------------
    // execCommand / selection plumbing
    // -----------------------------------------------------------------------
    //
    // NOTE (#144): `document.execCommand` is deprecated but still implemented by
    // every current engine, and it remains the pragmatic way to drive a
    // `contenteditable` surface (bold/italic/lists/alignment/format-block/links
    // + `queryCommandState`). A from-scratch Selection/Range editing engine is a
    // large, high-risk rewrite with poor ROI here: this Dioxus frontend is an
    // interim step before the atproto-based rewrite, which will replace the
    // editor wholesale. The whole dependency is deliberately isolated behind this
    // thin `exec` / `query_*` API so that replacement has a single clean seam.
    // The data-loss half of #144 (autosave + a beforeunload guard) shipped in
    // `components::editor`.

    /// Run a plain `execCommand`.
    pub fn exec(command: &str) -> bool {
        document()
            .map(|d| d.exec_command(command).unwrap_or(false))
            .unwrap_or(false)
    }

    /// Run an `execCommand` that takes a value (e.g. `formatBlock`, `createLink`).
    pub fn exec_value(command: &str, value: &str) -> bool {
        document()
            .map(|d| {
                d.exec_command_with_show_ui_and_value(command, false, value)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    /// Whether a toggle command (`bold`, `italic`, ...) is active at the caret.
    pub fn query_state(command: &str) -> bool {
        document()
            .map(|d| d.query_command_state(command).unwrap_or(false))
            .unwrap_or(false)
    }

    /// The current value of a command (e.g. `formatBlock` -> `h1`), lowercased.
    pub fn query_value(command: &str) -> String {
        document()
            .and_then(|d| d.query_command_value(command).ok())
            .unwrap_or_default()
            .to_lowercase()
    }

    thread_local! {
        /// The editor selection saved when focus moves to a toolbar control that
        /// steals it (the block dropdown, the link input), restored before the
        /// command runs.
        static SAVED_RANGE: std::cell::RefCell<Option<web_sys::Range>> =
            const { std::cell::RefCell::new(None) };
    }

    fn selection() -> Option<web_sys::Selection> {
        web_sys::window()?.get_selection().ok().flatten()
    }

    /// Save the current selection range.
    pub fn save_selection() {
        let Some(sel) = selection() else { return };
        if sel.range_count() > 0 {
            if let Ok(range) = sel.get_range_at(0) {
                SAVED_RANGE.with(|s| *s.borrow_mut() = Some(range));
            }
        }
    }

    /// Restore the previously saved selection range.
    pub fn restore_selection() {
        let Some(sel) = selection() else { return };
        let saved = SAVED_RANGE.with(|s| s.borrow().clone());
        if let Some(range) = saved {
            let _ = sel.remove_all_ranges();
            let _ = sel.add_range(&range);
        }
    }

    /// Toggle a `<code>` span around the current selection (there is no
    /// `execCommand` for it). When the caret already sits inside a `<code>`
    /// element the mark is removed (unwrapped to plain text) rather than nested.
    pub fn wrap_selection_code() {
        let Some(sel) = selection() else { return };
        // Toggle off: select the whole enclosing <code> and replace it with its
        // plain text, so a second click clears the mark instead of nesting it.
        if let Some(code_el) = selection_code_ancestor(&sel) {
            let text = code_el.text_content().unwrap_or_default();
            if let Some(range) = document().and_then(|d| d.create_range().ok()) {
                if range.select_node(&code_el).is_ok() {
                    let _ = sel.remove_all_ranges();
                    let _ = sel.add_range(&range);
                    exec_value("insertHTML", &super::html_escape(&text));
                }
            }
            return;
        }
        let text = sel.to_string().as_string().unwrap_or_default();
        if text.is_empty() {
            return;
        }
        exec_value(
            "insertHTML",
            &format!("<code>{}</code>", super::html_escape(&text)),
        );
    }

    /// The `<code>` element the selection's anchor sits within, if any.
    fn selection_code_ancestor(sel: &web_sys::Selection) -> Option<web_sys::Element> {
        let mut cur = sel.anchor_node();
        while let Some(n) = cur {
            if let Some(el) = n.dyn_ref::<web_sys::Element>() {
                if el.tag_name().eq_ignore_ascii_case("code") {
                    return Some(el.clone());
                }
            }
            cur = n.parent_node();
        }
        None
    }

    /// Make sure `execCommand` emits semantic tags (`<b>`, `<i>`), not styled
    /// spans, so [`dom_to_slate`] round-trips cleanly. Safe to call repeatedly.
    pub fn use_semantic_tags() {
        // `styleWithCSS = false`.
        if let Some(d) = document() {
            let _ = d.exec_command_with_show_ui_and_value("styleWithCSS", false, "false");
        }
    }

    /// Serialize the editor element with the given id into Slate content.
    pub fn serialize_editor(id: &str) -> Option<Value> {
        let el = document()?.get_element_by_id(id)?;
        Some(dom_to_slate(&el))
    }

    /// Set the editor element's initial HTML (called once on mount).
    pub fn seed_editor(id: &str, html: &str) {
        if let Some(el) = document().and_then(|d| d.get_element_by_id(id)) {
            el.set_inner_html(html);
        }
    }

    /// Tags kept when sanitizing pasted HTML; everything else is unwrapped to its
    /// children/text. Matches what [`dom_to_slate`] understands.
    const PASTE_ALLOWED_TAGS: &[&str] = &[
        "p",
        "br",
        "b",
        "strong",
        "i",
        "em",
        "u",
        "s",
        "strike",
        "code",
        "a",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "ul",
        "ol",
        "li",
        "blockquote",
    ];

    /// Tags dropped whole (with their content) rather than unwrapped, so their
    /// text/code never leaks into the document.
    const PASTE_STRIP_TAGS: &[&str] = &[
        "script", "style", "noscript", "head", "meta", "link", "title", "iframe", "object",
    ];

    /// Sanitize pasted HTML to the editor's semantic subset: keep the whitelisted
    /// tags (only `href` on links), unwrap everything else (styled spans/divs,
    /// Office markup, ...), and strip all other attributes. Mirrors React Slate's
    /// `withHtml` / `deserialize`.
    fn sanitize_pasted_html(html: &str) -> Option<String> {
        let doc = web_sys::window()?.document()?;
        let container = doc.create_element("div").ok()?;
        container.set_inner_html(html);
        // Deepest-first, so unwrapping a parent never disturbs children we have
        // not visited yet.
        if let Ok(all) = container.query_selector_all("*") {
            for i in (0..all.length()).rev() {
                let Some(el) = all.item(i).and_then(|n| n.dyn_into::<Element>().ok()) else {
                    continue;
                };
                let tag = el.tag_name().to_lowercase();
                let el_node: &Node = el.unchecked_ref();
                if PASTE_ALLOWED_TAGS.contains(&tag.as_str()) {
                    for name in el
                        .get_attribute_names()
                        .iter()
                        .filter_map(|n| n.as_string())
                    {
                        if !(tag == "a" && name == "href") {
                            let _ = el.remove_attribute(&name);
                        }
                    }
                } else if PASTE_STRIP_TAGS.contains(&tag.as_str()) {
                    // Drop the element and its content entirely.
                    if let Some(parent) = el.parent_node() {
                        let _ = parent.remove_child(el_node);
                    }
                } else if let Some(parent) = el.parent_node() {
                    // Unwrap: move children out, then drop the wrapper.
                    while let Some(child) = el.first_child() {
                        let _ = parent.insert_before(&child, Some(el_node));
                    }
                    let _ = parent.remove_child(el_node);
                }
            }
        }
        Some(container.inner_html())
    }

    /// Intercept paste on the editor and insert sanitized HTML instead of the raw
    /// browser paste (with a plain-text fallback). Attached once on mount.
    pub fn install_paste_handler(id: &str) {
        use wasm_bindgen::closure::Closure;
        let Some(el) = document().and_then(|d| d.get_element_by_id(id)) else {
            return;
        };
        let closure = Closure::wrap(Box::new(move |evt: web_sys::Event| {
            let Some(ce) = evt.dyn_ref::<web_sys::ClipboardEvent>() else {
                return;
            };
            let Some(cd) = ce.clipboard_data() else {
                return;
            };
            evt.prevent_default();
            let html = cd.get_data("text/html").unwrap_or_default();
            if !html.is_empty() {
                if let Some(clean) = sanitize_pasted_html(&html) {
                    exec_value("insertHTML", &clean);
                    return;
                }
            }
            // No HTML on the clipboard: insert plain text, keeping line breaks.
            let text = cd.get_data("text/plain").unwrap_or_default();
            exec_value(
                "insertHTML",
                &super::html_escape(&text).replace('\n', "<br>"),
            );
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = el.add_event_listener_with_callback("paste", closure.as_ref().unchecked_ref());
        // Leak the closure so the listener lives as long as the editor.
        closure.forget();
    }

    /// Focus the editor element (so `execCommand` acts on it).
    pub fn focus_editor(id: &str) {
        if let Some(el) = document()
            .and_then(|d| d.get_element_by_id(id))
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = el.focus();
        }
    }

    /// The caret's current link URL (for pre-filling the link dialog), if the
    /// selection sits inside an `<a>`.
    pub fn current_link() -> Option<String> {
        let sel = web_sys::window()?.get_selection().ok()??;
        let mut node = sel.anchor_node()?;
        loop {
            if node.node_type() == Node::ELEMENT_NODE {
                if let Some(el) = node.dyn_ref::<Element>() {
                    if el.tag_name().to_uppercase() == "A" {
                        return el.get_attribute("href");
                    }
                }
            }
            node = node.parent_node()?;
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use dom::{
    current_link, exec, exec_value, focus_editor, install_paste_handler, query_state, query_value,
    restore_selection, save_selection, seed_editor, serialize_editor, use_semantic_tags,
    wrap_selection_code,
};

/// Non-wasm stubs so the editor component still compiles for host `cargo test`
/// (it never actually runs off the browser).
#[cfg(not(target_arch = "wasm32"))]
mod dom_stub {
    use super::Value;
    pub fn exec(_command: &str) -> bool {
        false
    }
    pub fn exec_value(_command: &str, _value: &str) -> bool {
        false
    }
    pub fn query_state(_command: &str) -> bool {
        false
    }
    pub fn query_value(_command: &str) -> String {
        String::new()
    }
    pub fn use_semantic_tags() {}
    pub fn serialize_editor(_id: &str) -> Option<Value> {
        None
    }
    pub fn seed_editor(_id: &str, _html: &str) {}
    pub fn focus_editor(_id: &str) {}
    pub fn install_paste_handler(_id: &str) {}
    pub fn current_link() -> Option<String> {
        None
    }
    pub fn save_selection() {}
    pub fn restore_selection() {}
    pub fn wrap_selection_code() {}
}

#[cfg(not(target_arch = "wasm32"))]
pub use dom_stub::{
    current_link, exec, exec_value, focus_editor, install_paste_handler, query_state, query_value,
    restore_selection, save_selection, seed_editor, serialize_editor, use_semantic_tags,
    wrap_selection_code,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_email_accepts_and_rejects_precisely() {
        assert!(is_email("niclas@overby.me"));
        assert!(is_email("a.b+tag_1%x@sub.domain.co"));
        // Exactly one '@', a non-empty alnum-ish local, and a 2+ letter TLD.
        assert!(!is_email("no-at-sign.com"));
        assert!(!is_email("two@@x.com"));
        assert!(!is_email("a@b@c.com"));
        assert!(!is_email("@overby.me")); // empty local
        assert!(!is_email("a@b")); // no dot in domain
        assert!(!is_email("a@b.c")); // 1-letter TLD
        assert!(!is_email("a@b.c1")); // non-alpha TLD
        assert!(!is_email("spa ce@x.com")); // space in local
        assert!(!is_email("a@ b.com")); // space in domain host
    }

    #[test]
    fn autolink_links_urls_and_emails_conservatively() {
        // Explicit scheme, www., and emails link (with the right href prefix).
        assert_eq!(
            link_href("https://example.com/a?b=1"),
            Some("https://example.com/a?b=1".to_string())
        );
        assert_eq!(link_href("http://a.bc"), Some("http://a.bc".to_string()));
        assert_eq!(
            link_href("www.example.com"),
            Some("https://www.example.com".to_string())
        );
        assert_eq!(
            link_href("niclas@overby.me"),
            Some("mailto:niclas@overby.me".to_string())
        );
        // Prose, code and partial tokens must NOT become links.
        for plain in [
            "main.rs",
            "example.com",
            "e.g",
            "etc.",
            "v1.2",
            "@handle",
            "http://",
            "a@b",
            "www.x",
        ] {
            assert_eq!(link_href(plain), None, "should not link {plain}");
        }
    }

    #[test]
    fn autolink_word_keeps_surrounding_punctuation() {
        let (pre, text, href, post) = split_link_word("(https://x.io/p).").unwrap();
        assert_eq!(
            (pre.as_str(), text.as_str(), href.as_str(), post.as_str()),
            ("(", "https://x.io/p", "https://x.io/p", ").")
        );
        assert!(split_link_word("hello").is_none());
        assert!(split_link_word("main.rs").is_none());
    }

    #[test]
    fn autolink_segments_preserve_surrounding_text() {
        // A URL mid-sentence splits into plain / link / plain, verbatim around it.
        let segs = link_segments("see https://x.io now");
        assert_eq!(
            segs,
            vec![
                ("see ".to_string(), None),
                ("https://x.io".to_string(), Some("https://x.io".to_string())),
                (" now".to_string(), None),
            ]
        );
        // No link: one plain segment, unchanged.
        assert_eq!(
            link_segments("just main.rs here"),
            vec![("just main.rs here".to_string(), None)]
        );
    }
}

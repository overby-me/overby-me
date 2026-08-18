//! Shared recursion-depth guard for the wild OOXML parsers.
//!
//! Several OOXML grammars nest a container element inside its own descendants,
//! and the natural parser for each is directly recursive:
//!
//!   * DrawingML group shapes — `<p:grpSp>` (pptx `parse_sp_tree_node`),
//!     `<xdr:grpSp>` (xlsx `collect_shapes`);
//!   * WordprocessingML tables — a `<w:tbl>` may live inside a `<w:tc>`
//!     (docx `parse_table` ↔ `parse_table_cell`);
//!   * OMML math — every math container (`<m:f>`, `<m:sSup>`, `<m:d>` …) holds
//!     child math (`parse_omath_nodes`).
//!
//! A hand-crafted document can nest these tens of thousands deep. Because the
//! parser runs in WASM (a fixed, comparatively small linear-memory stack — the
//! default `wasm-pack` build reserves 1 MiB), unbounded recursion overflows the
//! stack and aborts the whole parse with an unrecoverable `RuntimeError:
//! unreachable` **trap** — the browser tab loses the document, not just the
//! pathological subtree.
//!
//! [`DepthGuard`] bounds that recursion. Each recursive call descends the guard
//! by one; when the configured limit is reached the caller stops descending and
//! returns whatever it has parsed so far, so the *rest* of the document still
//! renders (graceful degradation, matching the "skip the blit / drop the
//! picture, keep rendering" contract used elsewhere in these parsers).
//!
//! ## Why 64
//!
//! [`MAX_PARSE_DEPTH`] is 64. That is comfortably above anything a real authoring
//! tool produces yet far below the stack budget:
//!
//!   * Word's own UI caps interactive table nesting well inside this range, and
//!     even programmatically-authored decks/sheets rarely nest groups past a
//!     handful of levels; a genuine document never approaches 64.
//!   * Each recursion frame here is heavy (a `roxmltree::Node` plus several
//!     `&HashMap`/`&[..]` references and, for tables, `String`/`Vec` locals), so
//!     64 frames is a tiny fraction of the 1 MiB WASM stack — there is a wide
//!     safety margin between the guard firing and an actual overflow.
//!
//! The limit is intentionally a single shared constant so the three parsers +
//! the shared OMML grammar behave identically; a document that is "too deep" in
//! one format is "too deep" in all of them.

/// Maximum nesting depth accepted by any guarded OOXML recursion (group shapes,
/// nested tables, OMML math). See the module docs for the rationale.
pub const MAX_PARSE_DEPTH: u32 = 64;

/// A cheap, copyable recursion-depth counter threaded through directly-recursive
/// OOXML parsing functions.
///
/// Callers create one at the top of a recursion with [`DepthGuard::root`], then
/// obtain a child guard for each descent with [`DepthGuard::descend`]. When
/// `descend` returns `None` the limit has been reached and the caller must stop
/// recursing (return the partial result) instead of calling itself again.
///
/// The type is `Copy` and holds a single `u32`, so threading it as a by-value
/// parameter adds no allocation and no borrow-checker friction.
///
/// ```
/// use ooxml_common::depth::{DepthGuard, MAX_PARSE_DEPTH};
///
/// fn walk(depth: DepthGuard, fanout: u32) -> u32 {
///     // Count this node, then descend into `fanout` children if allowed.
///     let mut n = 1;
///     if let Some(child) = depth.descend() {
///         for _ in 0..fanout {
///             n += walk(child, fanout);
///         }
///     }
///     n
/// }
///
/// // A linear chain (fanout 1) visits exactly MAX_PARSE_DEPTH + 1 nodes before
/// // the guard stops the descent — it never traps.
/// assert_eq!(walk(DepthGuard::root(), 1), MAX_PARSE_DEPTH + 1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthGuard {
    depth: u32,
    limit: u32,
}

impl DepthGuard {
    /// A guard at depth 0 with the shared [`MAX_PARSE_DEPTH`] limit. Use this at
    /// every top-level (non-recursive) entry into a guarded recursion.
    #[inline]
    pub const fn root() -> Self {
        Self {
            depth: 0,
            limit: MAX_PARSE_DEPTH,
        }
    }

    /// A guard at depth 0 with an explicit `limit`. Intended for tests; the
    /// wild parsers should use [`DepthGuard::root`] so every format shares the
    /// same ceiling.
    #[inline]
    pub const fn with_limit(limit: u32) -> Self {
        Self { depth: 0, limit }
    }

    /// The current depth (0 at the root).
    #[inline]
    pub const fn depth(&self) -> u32 {
        self.depth
    }

    /// `true` when descending one more level would exceed the limit — i.e. the
    /// caller has hit the floor and must not recurse again.
    #[inline]
    pub const fn is_exhausted(&self) -> bool {
        self.depth >= self.limit
    }

    /// A child guard one level deeper, or `None` when the limit is reached.
    ///
    /// `Some(child)` ⇒ recurse with `child`; `None` ⇒ stop and return the
    /// partial result. The parent guard is unchanged (`Copy`), so sibling
    /// descents each get their own fresh child.
    #[inline]
    pub const fn descend(self) -> Option<Self> {
        if self.depth >= self.limit {
            None
        } else {
            Some(Self {
                depth: self.depth + 1,
                limit: self.limit,
            })
        }
    }
}

impl Default for DepthGuard {
    #[inline]
    fn default() -> Self {
        Self::root()
    }
}

/// Maximum XML element-nesting depth accepted before handing a part off to
/// `roxmltree::Document::parse` (see [`xml_nesting_exceeds`]).
///
/// This is a *separate* concern from [`MAX_PARSE_DEPTH`], which bounds our own
/// recursive descent (group shapes / tables / OMML). `roxmltree::Document::parse`
/// **itself recurses proportionally to element nesting** while building its tree,
/// so a document with a few thousand nested elements overflows the fixed WASM
/// linear-memory stack (~1 MiB) and traps the whole parse *before any of our code
/// runs* — our [`DepthGuard`] alone cannot prevent that. Measured on this
/// roxmltree (0.20) in an optimized build, ~1000-deep nesting is the overflow
/// threshold at a 1 MiB stack.
///
/// 256 keeps a wide safety margin below that threshold while sitting far above
/// any legitimate OOXML part: real documents nest on the order of tens of levels
/// (sectPr › tbl › tr › tc › p › r, group-in-group, etc.), never hundreds. A part
/// deeper than this is rejected so the caller degrades gracefully (skip the part,
/// keep the rest of the document) instead of trapping.
pub const MAX_XML_DEPTH: u32 = 256;

/// Scan raw XML `bytes` for element nesting deeper than `limit`, returning `true`
/// as soon as the limit is exceeded.
///
/// This is a cheap, allocation-free, single-pass pre-check run *before*
/// `roxmltree::Document::parse` so a maliciously deep document is rejected rather
/// than overflowing roxmltree's recursive tree builder (see [`MAX_XML_DEPTH`]).
/// It is deliberately a coarse lexer — it does not validate the XML, only tracks
/// tag nesting well enough to bound depth:
///
///   * `<name …>`   — a start tag: depth + 1.
///   * `</name>`    — an end tag: depth − 1.
///   * `<name … />` — a self-closing tag: no net change (it is a leaf).
///   * `<?…?>` (PI), `<!-- … -->` (comment), `<![CDATA[ … ]]>`, `<!DOCTYPE …>`
///     — skipped without affecting depth; their bodies may contain stray `<`/`>`
///     that must not be miscounted.
///
/// A malformed document can only make this over- or under-count by a little; it
/// exists purely to catch the pathological "thousands deep" case, and roxmltree
/// (or graceful skipping) handles ordinary malformedness afterwards. The scan
/// short-circuits the instant `depth` exceeds `limit`, so a decompression-bomb
/// style input is rejected in O(bytes-until-limit), not O(bytes).
pub fn xml_nesting_exceeds(bytes: &[u8], limit: u32) -> bool {
    let mut depth: u32 = 0;
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // At a '<'. Classify what follows.
        match bytes.get(i + 1) {
            Some(b'/') => {
                // End tag `</…>` — one level up. Saturating so a stray extra end
                // tag in malformed input can't underflow.
                depth = depth.saturating_sub(1);
                i = skip_to_gt(bytes, i + 2);
            }
            Some(b'?') => {
                // Processing instruction `<?…?>` — skip to `?>`.
                i = skip_until(bytes, i + 2, b"?>");
            }
            Some(b'!') => {
                // `<!-- … -->`, `<![CDATA[ … ]]>`, or `<!DOCTYPE …>`.
                if bytes[i + 1..].starts_with(b"!--") {
                    i = skip_until(bytes, i + 4, b"-->");
                } else if bytes[i + 1..].starts_with(b"![CDATA[") {
                    i = skip_until(bytes, i + 9, b"]]>");
                } else {
                    // DOCTYPE / other declaration — skip to the next '>'. (Internal
                    // DTD subsets with nested '>' are not produced by OOXML parts.)
                    i = skip_to_gt(bytes, i + 2);
                }
            }
            Some(_) => {
                // Start tag `<name …>` or self-closing `<name … />`. Find its '>'
                // and inspect the byte just before it to tell them apart.
                let close = skip_to_gt(bytes, i + 2);
                // `close` points just past '>'; the '>' is at close-1, the char
                // before it at close-2.
                let self_closing = close >= 2 && bytes.get(close - 2) == Some(&b'/');
                if !self_closing {
                    depth += 1;
                    if depth > limit {
                        return true;
                    }
                }
                i = close;
            }
            None => break, // trailing '<' at EOF
        }
    }
    false
}

/// Advance past the next `>` (returns the index just after it, or `bytes.len()`
/// if none remains).
#[inline]
fn skip_to_gt(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'>' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// Advance past the next occurrence of `needle` (returns the index just after it,
/// or `bytes.len()` if not found).
#[inline]
fn skip_until(bytes: &[u8], from: usize, needle: &[u8]) -> usize {
    let n = bytes.len();
    if needle.is_empty() || from >= n {
        return n;
    }
    let last = needle.len() - 1;
    let mut i = from;
    while i + last < n {
        if bytes[i] == needle[0] && &bytes[i..i + needle.len()] == needle {
            return i + needle.len();
        }
        i += 1;
    }
    n
}

/// Convenience over [`xml_nesting_exceeds`] using the shared [`MAX_XML_DEPTH`]
/// limit. `true` ⇒ the part is too deeply nested and must NOT be handed to
/// `roxmltree::Document::parse` (parse it would risk a stack-overflow trap).
#[inline]
pub fn xml_too_deep(bytes: &[u8]) -> bool {
    xml_nesting_exceeds(bytes, MAX_XML_DEPTH)
}

/// Error returned by [`parse_guarded`] — either the depth pre-check rejected the
/// input, or `roxmltree` itself failed to parse it.
///
/// It implements [`Display`](std::fmt::Display) so the many parse sites that map
/// a parse failure to a `String` (`.map_err(|e| e.to_string())`) keep working
/// unchanged, while sites that only care whether they got a `Document` can keep
/// using `.ok()` / `let Ok(..) else`.
#[derive(Debug)]
pub enum GuardedParseError {
    /// The raw XML nests deeper than [`MAX_XML_DEPTH`]; handing it to
    /// `roxmltree::Document::parse` would risk a stack-overflow trap, so the
    /// caller must degrade gracefully (skip the part) instead.
    TooDeep,
    /// `roxmltree::Document::parse` rejected the (in-depth-budget) input.
    Xml(roxmltree::Error),
}

impl std::fmt::Display for GuardedParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardedParseError::TooDeep => {
                write!(f, "XML nesting depth exceeds the safe limit")
            }
            GuardedParseError::Xml(e) => std::fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for GuardedParseError {}

/// Parse `xml` into a [`roxmltree::Document`], but **only after** the
/// allocation-free [`xml_too_deep`] pre-check confirms the element nesting is
/// within [`MAX_XML_DEPTH`].
///
/// `roxmltree`'s tree builder recurses per element-nesting level, so a part
/// nested a few thousand deep overflows the fixed WASM linear-memory stack and
/// **traps the whole parse inside `Document::parse`** — before any of our own
/// depth-guarded walks (see [`DepthGuard`]) get a chance to run. Every parse of
/// attacker-controllable XML (any part read out of the untrusted ZIP) must go
/// through this wrapper rather than calling `roxmltree::Document::parse`
/// directly, so the pre-check is impossible to forget at an individual site.
///
/// On success the returned [`roxmltree::Document`] borrows from `xml`, exactly as
/// `Document::parse` does. On failure the caller degrades gracefully (skip the
/// part, keep the rest of the document) — matching the existing
/// "unsupported/oversized ⇒ skip it, keep rendering" contract in these parsers.
#[inline]
pub fn parse_guarded(xml: &str) -> Result<roxmltree::Document<'_>, GuardedParseError> {
    if xml_too_deep(xml.as_bytes()) {
        return Err(GuardedParseError::TooDeep);
    }
    roxmltree::Document::parse(xml).map_err(GuardedParseError::Xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_starts_at_zero_with_shared_limit() {
        let g = DepthGuard::root();
        assert_eq!(g.depth(), 0);
        assert!(!g.is_exhausted());
    }

    #[test]
    fn descend_increments_and_stops_at_limit() {
        // A guard limited to 3 permits descents 0→1→2→3, then refuses.
        let mut g = DepthGuard::with_limit(3);
        for expected in 1..=3 {
            g = g.descend().expect("within limit");
            assert_eq!(g.depth(), expected);
        }
        assert!(g.is_exhausted());
        assert!(g.descend().is_none(), "must refuse past the limit");
    }

    #[test]
    fn siblings_get_independent_children() {
        // `descend` consumes a *copy*, so two children of the same parent are
        // both at depth 1 and neither observes the other's descent.
        let parent = DepthGuard::with_limit(MAX_PARSE_DEPTH);
        let a = parent.descend().unwrap();
        let b = parent.descend().unwrap();
        assert_eq!(a.depth(), 1);
        assert_eq!(b.depth(), 1);
        assert_eq!(a.descend().unwrap().depth(), 2);
        // `b` is still at depth 1 — unaffected by `a`'s further descent.
        assert_eq!(b.depth(), 1);
    }

    #[test]
    fn limit_of_zero_refuses_immediately() {
        let g = DepthGuard::with_limit(0);
        assert!(g.is_exhausted());
        assert!(g.descend().is_none());
    }

    #[test]
    fn default_matches_root() {
        assert_eq!(DepthGuard::default(), DepthGuard::root());
    }

    /// A deep linear chain — the failure this guard exists to prevent — is bounded:
    /// the recursion runs at most `MAX_PARSE_DEPTH` descents and returns rather
    /// than trapping. This mirrors the shape of the real recursive parsers.
    #[test]
    fn deep_chain_is_bounded_not_trapping() {
        fn descend_chain(g: DepthGuard) -> u32 {
            match g.descend() {
                Some(child) => 1 + descend_chain(child),
                None => 0,
            }
        }
        assert_eq!(descend_chain(DepthGuard::root()), MAX_PARSE_DEPTH);
    }

    // ── Raw-XML depth pre-check (guards roxmltree's own recursion) ──────────

    /// `depth` levels of `<a>…</a>` wrapping a text leaf.
    fn nested_xml(depth: usize) -> Vec<u8> {
        let mut s = String::new();
        for _ in 0..depth {
            s.push_str("<a>");
        }
        s.push('x');
        for _ in 0..depth {
            s.push_str("</a>");
        }
        s.into_bytes()
    }

    #[test]
    fn shallow_xml_is_not_flagged() {
        assert!(!xml_nesting_exceeds(&nested_xml(10), 256));
        // A real-ish OOXML snippet nests well under the limit.
        let doc = br#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>hi</w:t></w:r></w:p></w:body></w:document>"#;
        assert!(!xml_too_deep(doc));
    }

    #[test]
    fn exactly_at_the_limit_is_allowed_and_one_past_is_flagged() {
        // `limit` levels deep is fine; `limit + 1` trips the guard.
        assert!(!xml_nesting_exceeds(&nested_xml(32), 32));
        assert!(xml_nesting_exceeds(&nested_xml(33), 32));
    }

    #[test]
    fn pathologically_deep_xml_is_flagged() {
        // The case this pre-check exists for: thousands deep would otherwise
        // overflow roxmltree's recursive tree builder.
        assert!(xml_too_deep(&nested_xml(10_000)));
    }

    #[test]
    fn self_closing_tags_do_not_accumulate_depth() {
        // 1000 sibling self-closing tags are all leaves — net depth stays 1.
        let mut s = String::from("<root>");
        for _ in 0..1000 {
            s.push_str("<a/>");
        }
        s.push_str("</root>");
        assert!(!xml_nesting_exceeds(s.as_bytes(), 8));
        // With whitespace before the '/>', still a leaf.
        assert!(!xml_nesting_exceeds(b"<root><a b=\"1\" /></root>", 4));
    }

    #[test]
    fn comments_pis_and_cdata_do_not_affect_depth() {
        // Stray '<' and '>' inside comments / PIs / CDATA must not be counted as
        // tags (or the guard could be tricked either way).
        let xml =
            br#"<r><!-- <a><b><c> not tags --><?pi <x><y> ?><![CDATA[ </z><w><v> ]]><c/></r>"#;
        // Real nesting is only <r> … </r> plus a leaf <c/>: depth 1.
        assert!(!xml_nesting_exceeds(xml, 3));
    }

    #[test]
    fn unbalanced_end_tags_saturate_not_underflow() {
        // Extra end tags must not underflow (u32) and wrap to a huge depth.
        let xml = b"</a></a></a><b><c></c></b>";
        assert!(!xml_nesting_exceeds(xml, 4));
    }

    #[test]
    fn scan_short_circuits_before_reading_the_whole_input() {
        // The prefix already exceeds the limit; a megabyte of trailing junk after
        // it need not be scanned. (Behavioural: still returns true quickly.)
        let mut v = nested_xml(300);
        v.extend(std::iter::repeat_n(b'x', 1_000_000));
        assert!(xml_nesting_exceeds(&v, 256));
    }

    // ── parse_guarded (the wrapper every attacker-facing parse site must use) ──

    #[test]
    fn parse_guarded_accepts_shallow_xml() {
        let doc = parse_guarded("<r><a>x</a></r>").expect("shallow XML parses");
        assert_eq!(doc.root_element().tag_name().name(), "r");
    }

    #[test]
    fn parse_guarded_rejects_too_deep_before_roxmltree_runs() {
        // The load-bearing guarantee: a part nested far past MAX_XML_DEPTH is
        // rejected by the pre-check and never reaches roxmltree's recursive tree
        // builder (which would overflow the WASM stack and trap). 10_000-deep is
        // well past the ~1000 overflow threshold, so if the pre-check were absent
        // this call would trap instead of returning an Err — this test only runs
        // on the (small) native stack precisely because parse_guarded short-
        // circuits before roxmltree is invoked.
        let deep = String::from_utf8(nested_xml(10_000)).unwrap();
        let err = parse_guarded(&deep).expect_err("over-deep XML is rejected");
        assert!(matches!(err, GuardedParseError::TooDeep));
        assert_eq!(err.to_string(), "XML nesting depth exceeds the safe limit");
    }

    #[test]
    fn parse_guarded_reports_roxmltree_errors_for_in_budget_input() {
        // In-depth-budget but malformed input surfaces roxmltree's own error,
        // Display-forwarded so `.map_err(|e| e.to_string())` sites are unchanged.
        let err = parse_guarded("<r><a></r>").expect_err("malformed XML is rejected");
        assert!(matches!(err, GuardedParseError::Xml(_)));
        // The message is roxmltree's, not the depth message.
        assert_ne!(err.to_string(), "XML nesting depth exceeds the safe limit");
    }

    #[test]
    fn parse_guarded_document_borrows_from_input() {
        // The returned Document borrows `xml`, just like Document::parse — so a
        // caller can read node text without an extra allocation.
        let xml = String::from("<r><t>hello</t></r>");
        let doc = parse_guarded(&xml).unwrap();
        let t = doc
            .descendants()
            .find(|n| n.tag_name().name() == "t")
            .unwrap();
        assert_eq!(t.text(), Some("hello"));
    }
}

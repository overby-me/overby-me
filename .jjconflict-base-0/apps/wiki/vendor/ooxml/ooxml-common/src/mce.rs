//! Markup Compatibility and Extensibility (MCE) — the `<mc:AlternateContent>`
//! reference processing model shared by the docx, pptx and xlsx parsers.
//!
//! ECMA-376 Part 3 (Markup Compatibility and Extensibility), §9.3 "Step 2:
//! Processing the AlternateContent, Choice and Fallback Elements" defines which
//! branch of an `<mc:AlternateContent>` a consumer selects, verbatim:
//!
//! > A Choice element shall be marked as selected if the following conditions
//! > are satisfied:
//! > 1) Each of the namespaces specified by the Requires attribute of this
//! >    element is included in the given application configuration;
//! > 2) No preceding sibling Choice element is marked as selected; and
//! > 3) The element is not a descendant of an application-defined extension
//! >    element.
//! > A Fallback element shall be marked as selected if the following conditions
//! > are satisfied:
//! > 1) No preceding sibling Choice element is marked as selected; and
//! > 2) The element is not a descendant of an application-defined extension
//! >    element.
//!
//! The "given application configuration" is the set of namespaces the consumer
//! understands — i.e. those it has a handler that produces renderable output
//! for. Each parser passes its own membership test as the `understood`
//! predicate; the algorithm here is format-agnostic. Condition (3) concerns
//! MCE's own nested-extension elements, which none of these host schemas emit
//! around Choice/Fallback, so it needs no special handling: we only ever walk
//! the direct Choice/Fallback children of one AlternateContent.
//!
//! This unifies three previously divergent local behaviours (issue #787): docx
//! already implemented §9.3, pptx guessed via output-emptiness (never inspected
//! `Requires`), and xlsx always took the Choice (never the Fallback), so an
//! un-understood Choice with a renderable Fallback silently dropped content.

use roxmltree::Node;

/// Select the active branch of an `<mc:AlternateContent>` per ECMA-376 Part 3
/// §9.3 (Step 2), returning the selected `<mc:Choice>` / `<mc:Fallback>` element
/// node, or `None` when neither a selectable Choice nor a Fallback exists.
///
/// - A `<mc:Choice>` is selected iff EVERY namespace named by its `Requires`
///   attribute is understood AND no preceding sibling Choice was already
///   selected. `Requires` is a whitespace-delimited list of namespace *prefixes*
///   (Part 3 §7.6), each resolved to a URI against the element's in-scope
///   `xmlns` declarations via `lookup_namespace_uri`; the URI is then tested
///   with `understood`. Resolving prefixes (not raw strings) means both the
///   Transitional and Strict conformance classes work, and an arbitrary
///   producer prefix (`cx`, `cx1`, …) binds to the same URI.
/// - The schema requires `Requires` to list ≥1 prefix (§7.6); a missing or
///   whitespace-only value is non-conformant and can never satisfy §9.3(1)
///   ("*each* of the namespaces … is included"), so such a Choice is never
///   selected.
/// - If no Choice is selected, the `<mc:Fallback>` (if present) is selected.
///
/// `understood(ns_uri) -> bool` is the consumer's membership test for its
/// application configuration: return `true` only for namespaces the caller can
/// actually render, so an un-understood Choice correctly yields the Fallback.
pub fn select_alternate_content<'a, 'i>(
    ac: Node<'a, 'i>,
    understood: &dyn Fn(&str) -> bool,
) -> Option<Node<'a, 'i>> {
    for choice in ac
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Choice")
    {
        let requires = choice.attribute("Requires").unwrap_or("");
        let prefixes: Vec<&str> = requires.split_whitespace().collect();
        // §9.3(1): each Requires prefix must resolve, via an in-scope namespace
        // declaration, to an understood namespace. A conformant Choice lists ≥1
        // prefix; an empty list can never be "all understood".
        let all_understood = !prefixes.is_empty()
            && prefixes.iter().all(|prefix| {
                choice
                    .lookup_namespace_uri(Some(*prefix))
                    .is_some_and(understood)
            });
        if all_understood {
            // §9.3(2): first matching Choice wins; later Choices are ignored.
            return Some(choice);
        }
    }
    // §9.3: no Choice selected → the Fallback (if any) is selected.
    ac.children()
        .find(|n| n.is_element() && n.tag_name().name() == "Fallback")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse an `<mc:AlternateContent>` document, run §9.3 selection with the
    /// given understood-URI set, and return the selected branch's `id` attribute
    /// (each Choice/Fallback in the fixtures is tagged with a distinct `id`), or
    /// `None` when nothing is selected.
    fn select_id(xml: &str, understood: &[&str]) -> Option<String> {
        let doc = roxmltree::Document::parse(xml).unwrap();
        let ac = doc.root_element();
        let pred = |ns: &str| understood.contains(&ns);
        select_alternate_content(ac, &pred).and_then(|n| n.attribute("id").map(str::to_string))
    }

    // Fixtures bind prefixes to distinct URIs (mixing the Transitional-style and
    // an unrelated "urn:" form to prove URI — not prefix-string — matching).
    const NS: &str = concat!(
        r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" "#,
        r#"xmlns:k1="urn:known:one" "#,
        r#"xmlns:k2="urn:known:two" "#,
        r#"xmlns:kalt="urn:known:one" "#, // second prefix, SAME URI as k1
        r#"xmlns:u1="urn:unknown:one""#,
    );

    #[test]
    fn understood_single_choice_selected() {
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="k1"><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &["urn:known:one"]).as_deref(), Some("c1"));
    }

    #[test]
    fn unknown_only_choice_falls_back() {
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="u1"><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &["urn:known:one"]).as_deref(), Some("fb"));
    }

    #[test]
    fn multi_namespace_requires_needs_all_understood() {
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="k1 u1"><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        // Only k1 understood → the "k1 u1" Choice is NOT all-understood → Fallback.
        assert_eq!(select_id(&xml, &["urn:known:one"]).as_deref(), Some("fb"));
        // Both understood → the Choice is selected.
        assert_eq!(
            select_id(&xml, &["urn:known:one", "urn:unknown:one"]).as_deref(),
            Some("c1")
        );
    }

    #[test]
    fn second_choice_selected_when_first_not_understood() {
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="u1"><x/></mc:Choice>
                 <mc:Choice id="c2" Requires="k1"><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &["urn:known:one"]).as_deref(), Some("c2"));
    }

    #[test]
    fn first_understood_choice_wins_over_later() {
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="k1"><x/></mc:Choice>
                 <mc:Choice id="c2" Requires="k2"><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        // §9.3(2): the FIRST understood Choice is selected even when a later
        // Choice is also understood.
        assert_eq!(
            select_id(&xml, &["urn:known:one", "urn:known:two"]).as_deref(),
            Some("c1")
        );
    }

    #[test]
    fn prefix_resolves_by_uri_not_spelling() {
        // `kalt` is a different prefix bound to the SAME URI as `k1`. Selection
        // is by resolved URI, so a Choice `Requires="kalt"` is understood.
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="kalt"><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &["urn:known:one"]).as_deref(), Some("c1"));
    }

    #[test]
    fn missing_requires_is_never_selected() {
        // Non-conformant: no Requires attribute at all → can't be all-understood.
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1"><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &["urn:known:one"]).as_deref(), Some("fb"));
    }

    #[test]
    fn blank_requires_is_never_selected() {
        // Non-conformant: whitespace-only Requires → empty prefix list → not
        // selectable (an empty list can never be "each … is included").
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="   "><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &["urn:known:one"]).as_deref(), Some("fb"));
    }

    #[test]
    fn no_selectable_choice_and_no_fallback_returns_none() {
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="u1"><x/></mc:Choice>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &["urn:known:one"]), None);
    }

    #[test]
    fn understood_choice_with_no_fallback_still_selected() {
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="k1"><x/></mc:Choice>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &["urn:known:one"]).as_deref(), Some("c1"));
    }

    #[test]
    fn empty_understood_set_always_falls_back() {
        let xml = format!(
            r#"<mc:AlternateContent {NS}>
                 <mc:Choice id="c1" Requires="k1"><x/></mc:Choice>
                 <mc:Choice id="c2" Requires="k2"><x/></mc:Choice>
                 <mc:Fallback id="fb"><x/></mc:Fallback>
               </mc:AlternateContent>"#
        );
        assert_eq!(select_id(&xml, &[]).as_deref(), Some("fb"));
    }
}

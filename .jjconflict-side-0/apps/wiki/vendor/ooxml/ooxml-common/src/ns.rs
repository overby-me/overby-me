//! OOXML namespace URIs and matching predicates, covering both the
//! Transitional and Strict conformance classes of ECMA-376 / ISO/IEC 29500.
//!
//! A document saved by Office in **Strict** mode (ISO/IEC 29500 Strict) declares
//! its markup under the `http://purl.oclc.org/ooxml/...` namespaces instead of
//! the Transitional `http://schemas.openxmlformats.org/.../2006/...` ones. The
//! element/attribute *local names* are identical between the two classes, so a
//! parser that matches on local name works for both — but any code that pins a
//! namespace URI (`.tag_name().namespace() == Some(W_NS)` or
//! `node.attribute((NS, "attr"))`) silently rejects Strict documents, rendering
//! them blank.
//!
//! This module is the single source of truth for both URI variants of each
//! namespace and for the predicates that accept either. The URI strings are
//! taken verbatim from the `targetNamespace` declarations of the ECMA-376 5th
//! edition XML Schemas:
//! - Transitional: Part 4 `OfficeOpenXML-XMLSchema-Transitional/*.xsd`
//! - Strict: Part 1 `OfficeOpenXML-XMLSchema-Strict/*.xsd`
//!
//! For each namespace we expose `TRANSITIONAL` / `STRICT` constants and an
//! `is_*_ns(ns: Option<&str>) -> bool` predicate that returns true for either
//! variant (and false for `None` / any other URI). Call sites that previously
//! compared against a single `*_NS` constant switch to the predicate; nothing
//! else changes, so behaviour on Transitional input is preserved exactly.

use roxmltree::Node;

/// WordprocessingML main (`w:`). §17.
pub mod wordprocessingml {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
}

/// DrawingML main (`a:`). §20.1.
pub mod drawingml {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/main";
}

/// DrawingML chart (`c:`). §21.2.
pub mod chart {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/chart";
}

/// PresentationML main (`p:`). §19.
pub mod presentationml {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/presentationml/main";
}

/// SpreadsheetML main (`x:` / default). §18.
pub mod spreadsheetml {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
}

/// SpreadsheetDrawingML (`xdr:`), the DrawingML host inside a worksheet. §20.5.
pub mod spreadsheet_drawing {
    pub const TRANSITIONAL: &str =
        "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing";
}

/// WordprocessingDrawingML (`wp:`), the DrawingML host inside a document. §20.4.
pub mod wordprocessing_drawing {
    pub const TRANSITIONAL: &str =
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing";
}

/// DrawingML picture (`pic:`). §20.2.
pub mod picture {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/picture";
}

/// Office Math (OMML, `m:`). §22.1.
pub mod math {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/math";
}

/// Office document relationships (`r:`). Referenced from every host schema for
/// `r:id` / `r:embed` / `r:link`. ECMA-376 shared-relationshipReference.
pub mod relationships {
    pub const TRANSITIONAL: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
}

/// The well-known `<*:graphicData uri="…">` value that marks a graphicFrame's
/// payload as a SmartArt diagram (`dgm:relIds`, ECMA-376 §20.1.2.2.16 /
/// dml-diagram.xsd). Unlike the modules above this is not a namespace bound to
/// an element via `xmlns:` — it is a plain attribute *value* compared as a
/// string — but ECMA-376 defines one literal per conformance class, taken
/// verbatim from `dml-diagram.xsd`'s `targetNamespace`:
/// Transitional confirmed via Part 1 Annex L example markup
/// (`xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"`,
/// e.g. §L.4.3.4); Strict confirmed against the local
/// `spec/ECMA-376-1_5th_edition_december_2016/OfficeOpenXML-XMLSchema-Strict/
/// dml-diagram.xsd` (`targetNamespace="http://purl.oclc.org/ooxml/drawingml/
/// diagram"`) and the Strict Annex L example
/// (`uri="http://purl.oclc.org/ooxml/drawingml/diagram"`).
pub mod diagram_uri {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/diagram";
}

/// The well-known `<a:graphicData uri="…">` value that marks a
/// `<p:graphicFrame>`'s payload as an embedded OLE object (`p:oleObj` child,
/// ECMA-376 §19.3.2.4 CT_OleObject). As with [`diagram_uri`] this is a plain
/// attribute value, not an `xmlns:`-bound namespace, and there is no dedicated
/// `presentationml-ole.xsd` — `oleObj`/`CT_OleObject` live directly in
/// `pml.xsd`'s main namespace in both conformance classes. The literal itself
/// is confirmed by ECMA-376 Part 1 Annex L.7.2.5 "Embeddings in a
/// PresentationML Document", whose Strict example markup is
/// `<a:graphicData uri="http://purl.oclc.org/ooxml/presentationml/ole">`; the
/// Transitional value is this parser's long-standing literal.
pub mod ole_uri {
    pub const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/presentationml/2006/ole";
    pub const STRICT: &str = "http://purl.oclc.org/ooxml/presentationml/ole";
}

/// True when `ns` is either the Transitional or Strict URI of `$module`.
macro_rules! ns_predicate {
    ($(#[$doc:meta])* $fn_name:ident, $module:ident) => {
        $(#[$doc])*
        #[inline]
        pub fn $fn_name(ns: Option<&str>) -> bool {
            matches!(ns, Some($module::TRANSITIONAL) | Some($module::STRICT))
        }
    };
}

ns_predicate!(
    /// WordprocessingML main (`w:`), Transitional or Strict.
    is_w_ns,
    wordprocessingml
);
ns_predicate!(
    /// DrawingML main (`a:`), Transitional or Strict.
    is_a_ns,
    drawingml
);
ns_predicate!(
    /// DrawingML chart (`c:`), Transitional or Strict.
    is_c_ns,
    chart
);
ns_predicate!(
    /// PresentationML main (`p:`), Transitional or Strict.
    is_p_ns,
    presentationml
);
ns_predicate!(
    /// SpreadsheetML main (`x:` / default), Transitional or Strict.
    is_x_ns,
    spreadsheetml
);
ns_predicate!(
    /// SpreadsheetDrawingML (`xdr:`), Transitional or Strict.
    is_xdr_ns,
    spreadsheet_drawing
);
ns_predicate!(
    /// WordprocessingDrawingML (`wp:`), Transitional or Strict.
    is_wp_ns,
    wordprocessing_drawing
);
ns_predicate!(
    /// DrawingML picture (`pic:`), Transitional or Strict.
    is_pic_ns,
    picture
);
ns_predicate!(
    /// Office Math (`m:`), Transitional or Strict.
    is_m_ns,
    math
);
ns_predicate!(
    /// Office document relationships (`r:`), Transitional or Strict.
    is_r_ns,
    relationships
);

/// True when `uri` is the Transitional or Strict `graphicData@uri` value for a
/// SmartArt diagram. See [`diagram_uri`] for the literals and their sources.
#[inline]
pub fn is_diagram_uri(uri: &str) -> bool {
    matches!(uri, diagram_uri::TRANSITIONAL | diagram_uri::STRICT)
}

/// True when `uri` is the Transitional or Strict `graphicData@uri` value for an
/// embedded OLE object. See [`ole_uri`] for the literals and their sources.
#[inline]
pub fn is_pml_ole_uri(uri: &str) -> bool {
    matches!(uri, ole_uri::TRANSITIONAL | ole_uri::STRICT)
}

/// Look up an attribute named `name` that may live under either the Transitional
/// or the Strict URI of the same namespace, falling back to an unqualified
/// (no-namespace) attribute of the same name last.
///
/// This replaces `node.attribute((TRANSITIONAL_URI, name))` at sites that must
/// also accept Strict documents. The unqualified fallback matches producers that
/// emit e.g. a literal `r:embed` without binding the prefix; it mirrors the
/// long-standing tolerance in [`crate::blip::blip_embed_rid`].
#[inline]
pub fn attr_ns<'a>(
    node: &Node<'a, '_>,
    transitional: &str,
    strict: &str,
    name: &str,
) -> Option<&'a str> {
    node.attribute((transitional, name))
        .or_else(|| node.attribute((strict, name)))
        .or_else(|| node.attribute(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (predicate, its Transitional URI, its Strict URI).
    type NsCase = (fn(Option<&str>) -> bool, &'static str, &'static str);

    #[test]
    fn predicates_accept_both_conformance_classes() {
        // Each predicate is true for its Transitional and Strict URI, and false
        // for None, an unrelated OOXML URI, and a foreign string.
        let cases: &[NsCase] = &[
            (
                is_w_ns,
                wordprocessingml::TRANSITIONAL,
                wordprocessingml::STRICT,
            ),
            (is_a_ns, drawingml::TRANSITIONAL, drawingml::STRICT),
            (is_c_ns, chart::TRANSITIONAL, chart::STRICT),
            (
                is_p_ns,
                presentationml::TRANSITIONAL,
                presentationml::STRICT,
            ),
            (is_x_ns, spreadsheetml::TRANSITIONAL, spreadsheetml::STRICT),
            (
                is_xdr_ns,
                spreadsheet_drawing::TRANSITIONAL,
                spreadsheet_drawing::STRICT,
            ),
            (
                is_wp_ns,
                wordprocessing_drawing::TRANSITIONAL,
                wordprocessing_drawing::STRICT,
            ),
            (is_pic_ns, picture::TRANSITIONAL, picture::STRICT),
            (is_m_ns, math::TRANSITIONAL, math::STRICT),
            (is_r_ns, relationships::TRANSITIONAL, relationships::STRICT),
        ];
        for (pred, transitional, strict) in cases {
            assert!(pred(Some(transitional)), "transitional {transitional}");
            assert!(pred(Some(strict)), "strict {strict}");
            assert!(!pred(None), "None for {transitional}");
            assert!(
                !pred(Some("http://example.com/other")),
                "foreign for {transitional}"
            );
        }
    }

    #[test]
    fn predicates_do_not_cross_match() {
        // A different namespace's URI must not satisfy a predicate.
        assert!(!is_w_ns(Some(drawingml::TRANSITIONAL)));
        assert!(!is_a_ns(Some(chart::STRICT)));
        assert!(!is_c_ns(Some(drawingml::STRICT)));
        assert!(!is_r_ns(Some(math::STRICT)));
        // Transitional drawingml is a URL prefix of chart/picture/etc.; ensure
        // exact-match semantics (not prefix) so `a` does not swallow `c`.
        assert!(!is_a_ns(Some(chart::TRANSITIONAL)));
        assert!(!is_a_ns(Some(picture::TRANSITIONAL)));
    }

    #[test]
    fn is_diagram_uri_accepts_both_conformance_classes() {
        assert!(is_diagram_uri(diagram_uri::TRANSITIONAL));
        assert!(is_diagram_uri(diagram_uri::STRICT));
        assert!(!is_diagram_uri(ole_uri::STRICT));
        assert!(!is_diagram_uri("http://example.com/other"));
    }

    #[test]
    fn is_pml_ole_uri_accepts_both_conformance_classes() {
        assert!(is_pml_ole_uri(ole_uri::TRANSITIONAL));
        assert!(is_pml_ole_uri(ole_uri::STRICT));
        assert!(!is_pml_ole_uri(diagram_uri::STRICT));
        assert!(!is_pml_ole_uri("http://example.com/other"));
    }

    #[test]
    fn attr_ns_reads_either_class_and_unqualified() {
        // Transitional-qualified attribute.
        let t = format!(
            r#"<x xmlns:r="{}" r:embed="A"/>"#,
            relationships::TRANSITIONAL
        );
        let doc = roxmltree::Document::parse(&t).unwrap();
        assert_eq!(
            attr_ns(
                &doc.root_element(),
                relationships::TRANSITIONAL,
                relationships::STRICT,
                "embed"
            ),
            Some("A")
        );

        // Strict-qualified attribute.
        let s = format!(r#"<x xmlns:r="{}" r:embed="B"/>"#, relationships::STRICT);
        let doc = roxmltree::Document::parse(&s).unwrap();
        assert_eq!(
            attr_ns(
                &doc.root_element(),
                relationships::TRANSITIONAL,
                relationships::STRICT,
                "embed"
            ),
            Some("B")
        );

        // Unqualified fallback.
        let u = r#"<x embed="C"/>"#;
        let doc = roxmltree::Document::parse(u).unwrap();
        assert_eq!(
            attr_ns(
                &doc.root_element(),
                relationships::TRANSITIONAL,
                relationships::STRICT,
                "embed"
            ),
            Some("C")
        );

        // Absent → None.
        let n = r#"<x/>"#;
        let doc = roxmltree::Document::parse(n).unwrap();
        assert_eq!(
            attr_ns(
                &doc.root_element(),
                relationships::TRANSITIONAL,
                relationships::STRICT,
                "embed"
            ),
            None
        );
    }
}

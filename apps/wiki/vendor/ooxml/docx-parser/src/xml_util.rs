use ooxml_common::ns::{attr_ns, is_w_ns, wordprocessingml};
use roxmltree::Node;

/// Microsoft Word 2010 extension namespace used by paragraph identity and other
/// post-ECMA-376 WordprocessingML extensions.
pub const W14_NS: &str = "http://schemas.microsoft.com/office/word/2010/wordml";

/// Transitional WordprocessingML URI, used only by test fixtures that build
/// `w:`-namespaced XML; runtime matching goes through [`is_w_ns`], which accepts
/// the Strict URI too.
#[cfg(test)]
pub const W_NS: &str = wordprocessingml::TRANSITIONAL;
/// Transitional relationships URI. See [`W_NS`] for why the Transitional value is
/// kept for tests while runtime matching accepts either class.
#[cfg(test)]
pub const R_NS: &str = ooxml_common::ns::relationships::TRANSITIONAL;

/// Find first child in w: namespace.
pub fn child_w<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|n| n.tag_name().name() == name && is_w_ns(n.tag_name().namespace()))
}

/// Collect all children in w: namespace with given name.
pub fn children_w<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Vec<Node<'a, 'input>> {
    node.children()
        .filter(|n| n.tag_name().name() == name && is_w_ns(n.tag_name().namespace()))
        .collect()
}

/// Element children with <w:sdt> wrappers transparently unwrapped. Structured Document
/// Tag (content control) blocks contain their real content inside <w:sdtContent>, and
/// most parsing stages should treat them as inline with the surrounding context.
pub fn element_children_flat<'a, 'input>(node: Node<'a, 'input>) -> Vec<Node<'a, 'input>> {
    let mut out = Vec::new();
    for child in node.children().filter(|n| n.is_element()) {
        let tn = child.tag_name();
        if is_w_ns(tn.namespace()) && tn.name() == "sdt" {
            if let Some(content) = child_w(child, "sdtContent") {
                out.extend(element_children_flat(content));
            }
        } else {
            out.push(child);
        }
    }
    out
}

/// Like children_w but transparently descends into <w:sdt>/<w:sdtContent> wrappers.
pub fn children_w_flat<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Vec<Node<'a, 'input>> {
    element_children_flat(node)
        .into_iter()
        .filter(|n| n.tag_name().name() == name && is_w_ns(n.tag_name().namespace()))
        .collect()
}

/// Get attribute in w: namespace (Transitional or Strict), falling back to
/// no-namespace.
pub fn attr_w(node: Node, name: &str) -> Option<String> {
    attr_ns(
        &node,
        wordprocessingml::TRANSITIONAL,
        wordprocessingml::STRICT,
        name,
    )
    .map(|s| s.to_string())
}

/// Get an attribute in the Microsoft Word 2010 extension namespace.
pub fn attr_w14(node: Node, name: &str) -> Option<String> {
    node.attribute((W14_NS, name)).map(str::to_string)
}

/// Parse a bare decimal or an ECMA-376 universal measure into points.
///
/// Suffixed values use the fixed physical-unit conversions from
/// `ST_UniversalMeasure` (`pt`, `in`, `pc`/`pi`, `mm`, `cm`). The meaning of a
/// bare number is context-dependent, so callers supply how many of that unit
/// make one point: VML shape lengths use `1.0`, while `ST_TwipsMeasure` uses
/// `20.0`. HPS measures use their stricter dedicated parsers below.
pub(crate) fn parse_measure_to_pt(value: &str, unitless_per_pt: f64) -> Option<f64> {
    let value = value.trim();
    let (number, scale) = if let Some(number) = value.strip_suffix("pt") {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix("in") {
        (number, 72.0)
    } else if let Some(number) = value
        .strip_suffix("pc")
        .or_else(|| value.strip_suffix("pi"))
    {
        (number, 12.0)
    } else if let Some(number) = value.strip_suffix("mm") {
        (number, 72.0 / 25.4)
    } else if let Some(number) = value.strip_suffix("cm") {
        (number, 72.0 / 2.54)
    } else {
        // Divide rather than multiply by a reciprocal: 1701 / 20.0 is 85.05,
        // 1701 * (1.0 / 20.0) is 85.05000000000001, and section geometry
        // asserts the exact value.
        return parse_finite(value).map(|number| number / unitless_per_pt);
    };

    parse_finite(number).map(|number| number * scale)
}

/// Parse a decimal, rejecting non-finite values so NaN/inf never reaches layout.
fn parse_finite(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}

/// Parse `ST_TwipsMeasure` (§22.9.2.14) to points. A bare number is twips; a
/// `ST_PositiveUniversalMeasure` suffix is an absolute size.
pub fn twips_to_pt(s: &str) -> f64 {
    parse_measure_to_pt(s, 20.0).unwrap_or(0.0)
}

/// Parse `ST_HpsMeasure` (§17.18.42) to points.
///
/// Its lexical union is deliberately narrower than an `f64`: a bare value is
/// `ST_UnsignedDecimalNumber` (digits only, in half-points), while a suffixed
/// value is `ST_PositiveUniversalMeasure` (an unsigned decimal plus a supported
/// absolute unit). Rejecting signs, fractional/exponent bare numbers, and
/// negative universal measures keeps malformed direct formatting from blocking
/// a valid inherited value.
pub fn half_pt_to_pt(s: &str) -> Option<f64> {
    let value = s.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse::<u64>().ok().map(|number| number as f64 / 2.0);
    }
    universal_measure_to_pt(value, false)
}

/// Parse `ST_SignedHpsMeasure` (§17.18.81) to points.
///
/// The bare member is the XSD `integer` lexical form (optional `+`/`-`, then
/// digits) in half-points. The unit-bearing member is `ST_UniversalMeasure`,
/// whose schema pattern allows an optional minus sign, but not a plus sign.
pub fn signed_half_pt_to_pt(s: &str) -> Option<f64> {
    let value = s.trim();
    if is_xsd_integer_lexeme(value) {
        return parse_finite(value).map(|number| number / 2.0);
    }
    universal_measure_to_pt(value, true)
}

fn is_xsd_integer_lexeme(value: &str) -> bool {
    let digits = value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn universal_measure_to_pt(value: &str, signed: bool) -> Option<f64> {
    let (number, scale) = if let Some(number) = value.strip_suffix("pt") {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix("in") {
        (number, 72.0)
    } else if let Some(number) = value
        .strip_suffix("pc")
        .or_else(|| value.strip_suffix("pi"))
    {
        (number, 12.0)
    } else if let Some(number) = value.strip_suffix("mm") {
        (number, 72.0 / 25.4)
    } else {
        (value.strip_suffix("cm")?, 72.0 / 2.54)
    };

    let unsigned_number = if signed {
        number.strip_prefix('-').unwrap_or(number)
    } else {
        number
    };
    if unsigned_number.is_empty()
        || unsigned_number.starts_with('+')
        || (!signed && number.starts_with('-'))
    {
        return None;
    }

    let mut parts = unsigned_number.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.is_some_and(|digits| {
            digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())
        })
        || parts.next().is_some()
    {
        return None;
    }

    parse_finite(number).map(|number| number * scale)
}

/// Parse a ST_OnOff-style toggle child element. ECMA-376 §17.3.2.22 allows
/// "true"/"false"/"1"/"0"/"on"/"off" (and absent val attribute = true).
/// Returns None if the element itself is absent so the caller can distinguish
/// "explicitly turned off" from "inherited from parent".
pub fn bool_prop(node: Node, tag: &str) -> Option<bool> {
    let child = child_w(node, tag)?;
    let val = attr_w(child, "val");
    Some(!matches!(
        val.as_deref(),
        Some("0") | Some("false") | Some("off")
    ))
}

/// Parse a ST_OnOff-valued ATTRIBUTE (e.g. `<w:eastAsianLayout w:vert="1">`,
/// §17.3.2.10) directly on `node`, using the same "true"/"false"/"1"/"0"/"on"/
/// "off" vocabulary as {@link bool_prop} (§22.9.2.7 ST_OnOff). Unlike `bool_prop`
/// (which reads a CHILD element's `w:val`), the value here is the attribute
/// itself. Returns `None` when the attribute is absent so the caller can
/// distinguish "explicitly off" from "inherited".
pub fn on_off_attr(node: Node, name: &str) -> Option<bool> {
    let val = attr_w(node, name)?;
    Some(!matches!(val.as_str(), "0" | "false" | "off"))
}

#[cfg(test)]
mod measure_tests {
    use super::{half_pt_to_pt, signed_half_pt_to_pt, twips_to_pt};

    #[test]
    fn half_pt_to_pt_accepts_positive_universal_measures() {
        // ST_HpsMeasure (§17.18.42) = ST_UnsignedDecimalNumber |
        // ST_PositiveUniversalMeasure.
        assert_eq!(half_pt_to_pt("20"), Some(10.0), "20 half-points = 10pt");
        assert_eq!(
            half_pt_to_pt("12pt"),
            Some(12.0),
            "12pt is absolute, not 6pt"
        );
        assert_eq!(half_pt_to_pt("1in"), Some(72.0));
        assert_eq!(half_pt_to_pt("1pc"), Some(12.0));
        assert_eq!(half_pt_to_pt("1pi"), Some(12.0));
        assert_eq!(half_pt_to_pt("0"), Some(0.0));
        assert_eq!(half_pt_to_pt("0pt"), Some(0.0));
        assert!((half_pt_to_pt("3.6mm").unwrap() - 72.0 * 3.6 / 25.4).abs() < 1e-9);
        assert!((half_pt_to_pt("2.54cm").unwrap() - 72.0).abs() < 1e-9);
    }

    #[test]
    fn half_pt_to_pt_rejects_values_outside_st_hps_measure_lexical_space() {
        for invalid in ["-0", "-0pt", "-10", "-10pt", "1.5", "1e2", "+10", "+10pt"] {
            assert_eq!(
                half_pt_to_pt(invalid),
                None,
                "{invalid:?} is neither ST_UnsignedDecimalNumber nor ST_PositiveUniversalMeasure"
            );
        }
    }

    #[test]
    fn signed_half_pt_to_pt_matches_st_signed_hps_measure_lexical_space() {
        assert_eq!(signed_half_pt_to_pt("10"), Some(5.0));
        assert_eq!(signed_half_pt_to_pt("-10"), Some(-5.0));
        assert_eq!(signed_half_pt_to_pt("+10"), Some(5.0));
        assert_eq!(signed_half_pt_to_pt("-0"), Some(-0.0));
        assert_eq!(signed_half_pt_to_pt("-0pt"), Some(-0.0));
        assert_eq!(signed_half_pt_to_pt("-12pt"), Some(-12.0));
        assert!((signed_half_pt_to_pt("-3.6mm").unwrap() + 72.0 * 3.6 / 25.4).abs() < 1e-9);

        for invalid in ["1.5", "1e2", "+12pt", "12PT"] {
            assert_eq!(
                signed_half_pt_to_pt(invalid),
                None,
                "{invalid:?} is outside ST_SignedHpsMeasure's lexical union"
            );
        }
    }

    #[test]
    fn twips_to_pt_accepts_positive_universal_measures() {
        // ST_TwipsMeasure (§22.9.2.14) is the same union.
        assert_eq!(twips_to_pt("240"), 12.0, "240 twips = 12pt");
        assert_eq!(twips_to_pt("12pt"), 12.0);
        assert_eq!(twips_to_pt("1in"), 72.0);
    }

    #[test]
    fn measure_parsers_reject_unparseable_and_non_finite_values() {
        assert_eq!(half_pt_to_pt(""), None);
        assert_eq!(half_pt_to_pt("auto"), None);
        assert_eq!(half_pt_to_pt("inf"), None);
        assert_eq!(twips_to_pt("NaN"), 0.0);
    }
}

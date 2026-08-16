//! DrawingML gradient / pattern fill parsing (`<a:gradFill>` / `<a:pattFill>`),
//! shared out of the pptx parser.
//!
//! These two fill kinds carry the same DrawingML grammar in every host that
//! embeds it: a `<a:gradFill>` is a sorted list of `<a:gs pos>` color stops
//! plus a `<a:lin ang>` / `<a:path>` direction (ECMA-376 §20.1.8.33 /
//! §20.1.8.36 / §20.1.8.41), and a `<a:pattFill prst>` is a preset pattern with
//! `<a:fgClr>` / `<a:bgClr>` colors (§20.1.8.40 / §20.1.10.59). The stop /
//! pattern colors resolve through the shared [`crate::color::parse_color_node`]
//! (any srgbClr / schemeClr / sysClr / prstClr + transforms), so a caller
//! supplies its own [`ThemeResolver`](crate::color::ThemeResolver) and the
//! grammar stays identical.
//!
//! Scope is deliberately narrow: only the *parse* moves here. The consuming
//! parser keeps its own `Fill` model (its serde tags, its `Image` / blipFill /
//! grpFill variants, its solidFill handling) and assembles these owned
//! descriptors into it. Currently only the pptx parser implements gradFill /
//! pattFill *rendering*; docx / xlsx have no consumer yet, so they do not call
//! these — a future expansion point when their renderers gain gradient /
//! pattern support.

use crate::color::{parse_color_node, ThemeResolver, TintMode};
use roxmltree::Node;

/// One `<a:gs>` gradient stop: a position in `[0, 1]` and its resolved hex
/// color (6-char opaque, or 8-char RRGGBBAA when an alpha transform applies).
/// The serde shape (`{"position":..,"color":".."}`) matches the pptx parser's
/// former inline `GradStop`, so its JSON stays byte-identical when it stores a
/// `Vec<GradStop>` on its own `Fill::Gradient`.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GradStop {
    /// Stop offset along the gradient axis, `0.0`–`1.0` (raw `pos` / 100000).
    pub position: f64,
    /// Resolved stop color (hex, no `#`).
    pub color: String,
}

/// A parsed `<a:gradFill>`: its sorted stops plus the gradient direction.
/// A plain owned descriptor — the caller maps it onto its own fill model.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientFill {
    /// Color stops, sorted ascending by `position`.
    pub stops: Vec<GradStop>,
    /// Axis angle in degrees (`<a:lin ang>` / 60000; `0` = left→right,
    /// `90` = top→bottom). `0.0` for a radial (`<a:path>`) or directionless
    /// gradient.
    pub angle: f64,
    /// `"linear"` (a `<a:lin>` child or neither child) or `"radial"`
    /// (a `<a:path>` child).
    pub grad_type: String,
}

/// A parsed `<a:pattFill>`: its preset pattern name and fg/bg colors.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternFill {
    /// Foreground color (hex, no `#`) — the pattern's "1" pixels. Defaults to
    /// `000000` when `<a:fgClr>` is missing or unresolved.
    pub fg: String,
    /// Background color (hex, no `#`) — the pattern's "0" pixels. Defaults to
    /// `ffffff` when `<a:bgClr>` is missing or unresolved.
    pub bg: String,
    /// `prst` preset value (`pct5`/…/`horz`/`vert`/`cross`/`diagCross`/…).
    /// Defaults to `pct50` when the attribute is absent.
    pub preset: String,
}

/// Unqualified (no-namespace) attribute lookup, matching the pptx `attr` helper
/// the moved code used.
fn attr(node: Node<'_, '_>, local: &str) -> Option<String> {
    node.attributes()
        .find(|a| a.name() == local && a.namespace().is_none())
        .map(|a| a.value().to_owned())
}

/// First child element with the given local name, matching pptx's `child`.
fn child<'a, 'i>(node: Node<'a, 'i>, local: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == local)
}

/// Parse a located `<a:gradFill>` node (ECMA-376 §20.1.8.33). Reads the
/// `<a:gsLst>` color stops (each `<a:gs pos>` resolved via
/// [`parse_color_node`]), sorts them ascending by position, and derives the
/// direction from `<a:lin ang>` (linear) or `<a:path>` (radial). Returns `None`
/// when there are no resolvable stops, so the caller can keep scanning sibling
/// fill elements (verbatim of the pptx behavior). `resolver` / `tint_mode` are
/// threaded to the stop colors.
pub fn parse_grad_fill<R: ThemeResolver + ?Sized>(
    grad_fill: Node<'_, '_>,
    resolver: &R,
    tint_mode: TintMode,
) -> Option<GradientFill> {
    let mut stops: Vec<GradStop> = child(grad_fill, "gsLst")
        .map(|gs_lst| {
            gs_lst
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "gs")
                .filter_map(|gs| {
                    let position = attr(gs, "pos")
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                        / 100_000.0;
                    let color = parse_color_node(gs, resolver, tint_mode)?;
                    Some(GradStop { position, color })
                })
                .collect()
        })
        .unwrap_or_default();

    if stops.is_empty() {
        return None;
    }
    stops.sort_by(|a, b| {
        a.position
            .partial_cmp(&b.position)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let (grad_type, angle) = if let Some(lin) = child(grad_fill, "lin") {
        // OOXML ang: 60000ths of degree, 0 = left→right, 5400000 = top→bottom
        let ang = attr(lin, "ang")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
            / 60_000.0;
        ("linear".to_owned(), ang)
    } else if child(grad_fill, "path").is_some() {
        ("radial".to_owned(), 0.0)
    } else {
        ("linear".to_owned(), 0.0)
    };
    Some(GradientFill {
        stops,
        angle,
        grad_type,
    })
}

/// Parse a located `<a:pattFill>` node (ECMA-376 §20.1.8.40). Reads the `prst`
/// preset (default `pct50`) and the `<a:fgClr>` / `<a:bgClr>` colors via
/// [`parse_color_node`], defaulting fg→`000000` / bg→`ffffff` when absent or
/// unresolved (verbatim of the pptx behavior). `resolver` / `tint_mode` are
/// threaded to the colors.
pub fn parse_patt_fill<R: ThemeResolver + ?Sized>(
    patt_fill: Node<'_, '_>,
    resolver: &R,
    tint_mode: TintMode,
) -> PatternFill {
    let preset = attr(patt_fill, "prst").unwrap_or_else(|| "pct50".to_owned());
    let fg = child(patt_fill, "fgClr")
        .and_then(|n| parse_color_node(n, resolver, tint_mode))
        .unwrap_or_else(|| "000000".to_owned());
    let bg = child(patt_fill, "bgClr")
        .and_then(|n| parse_color_node(n, resolver, tint_mode))
        .unwrap_or_else(|| "ffffff".to_owned());
    PatternFill { fg, bg, preset }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    const NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

    /// A resolver mapping accent1/accent2 for the scheme-color stops.
    struct MapResolver;
    impl ThemeResolver for MapResolver {
        fn resolve_scheme_color(&self, name: &str) -> Option<String> {
            match name {
                "accent1" => Some("4472C4".to_owned()),
                "accent2" => Some("ED7D31".to_owned()),
                _ => None,
            }
        }
    }

    fn doc(xml: &str) -> Document<'_> {
        Document::parse(xml).unwrap()
    }

    /// gradFill: stops parsed + sorted, linear angle from `<a:lin ang>`.
    #[test]
    fn grad_fill_sorts_stops_and_reads_linear_angle() {
        let xml = format!(
            r#"<a:gradFill xmlns:a="{NS}">
                 <a:gsLst>
                   <a:gs pos="100000"><a:srgbClr val="ffffff"/></a:gs>
                   <a:gs pos="0"><a:schemeClr val="accent1"/></a:gs>
                 </a:gsLst>
                 <a:lin ang="5400000"/>
               </a:gradFill>"#
        );
        let d = doc(&xml);
        let g =
            parse_grad_fill(d.root_element(), &MapResolver, TintMode::PowerPointLinear).unwrap();
        assert_eq!(g.grad_type, "linear");
        assert_eq!(g.angle, 90.0); // 5400000 / 60000
                                   // Sorted ascending by position; colors uppercase, no '#'.
        assert_eq!(g.stops[0].position, 0.0);
        assert_eq!(g.stops[0].color, "4472C4");
        assert_eq!(g.stops[1].position, 1.0);
        assert_eq!(g.stops[1].color, "FFFFFF");
    }

    /// gradFill with a `<a:path>` (no `<a:lin>`) → radial, angle 0.
    #[test]
    fn grad_fill_path_is_radial() {
        let xml = format!(
            r#"<a:gradFill xmlns:a="{NS}">
                 <a:gsLst><a:gs pos="0"><a:srgbClr val="000000"/></a:gs></a:gsLst>
                 <a:path path="circle"/>
               </a:gradFill>"#
        );
        let d = doc(&xml);
        let g =
            parse_grad_fill(d.root_element(), &MapResolver, TintMode::PowerPointLinear).unwrap();
        assert_eq!(g.grad_type, "radial");
        assert_eq!(g.angle, 0.0);
    }

    /// gradFill with no resolvable stops → None (caller keeps scanning).
    #[test]
    fn grad_fill_no_stops_is_none() {
        let xml = format!(r#"<a:gradFill xmlns:a="{NS}"><a:gsLst/></a:gradFill>"#);
        let d = doc(&xml);
        assert_eq!(
            parse_grad_fill(d.root_element(), &MapResolver, TintMode::PowerPointLinear),
            None
        );
        // An unresolvable schemeClr stop is dropped too.
        let xml2 = format!(
            r#"<a:gradFill xmlns:a="{NS}"><a:gsLst><a:gs pos="0"><a:schemeClr val="accent9"/></a:gs></a:gsLst></a:gradFill>"#
        );
        let d2 = doc(&xml2);
        assert_eq!(
            parse_grad_fill(d2.root_element(), &MapResolver, TintMode::PowerPointLinear),
            None
        );
    }

    /// pattFill: preset + fg/bg colors read via the resolver.
    #[test]
    fn patt_fill_reads_preset_and_colors() {
        let xml = format!(
            r#"<a:pattFill xmlns:a="{NS}" prst="ltDnDiag">
                 <a:fgClr><a:schemeClr val="accent1"/></a:fgClr>
                 <a:bgClr><a:srgbClr val="ffffff"/></a:bgClr>
               </a:pattFill>"#
        );
        let d = doc(&xml);
        let p = parse_patt_fill(d.root_element(), &MapResolver, TintMode::PowerPointLinear);
        assert_eq!(p.preset, "ltDnDiag");
        assert_eq!(p.fg, "4472C4");
        assert_eq!(p.bg, "FFFFFF");
    }

    /// pattFill defaults: missing prst → pct50, missing fg → 000000, bg → ffffff.
    #[test]
    fn patt_fill_defaults() {
        let xml = format!(r#"<a:pattFill xmlns:a="{NS}"/>"#);
        let d = doc(&xml);
        let p = parse_patt_fill(d.root_element(), &MapResolver, TintMode::PowerPointLinear);
        assert_eq!(p.preset, "pct50");
        assert_eq!(p.fg, "000000");
        assert_eq!(p.bg, "ffffff");
    }

    /// GradStop serializes as {"position":..,"color":".."} (camelCase),
    /// byte-matching the pptx parser's former inline struct.
    #[test]
    fn grad_stop_serde_shape() {
        let v = serde_json::to_value(GradStop {
            position: 0.5,
            color: "ABCDEF".to_owned(),
        })
        .unwrap();
        assert_eq!(v["position"], 0.5);
        assert_eq!(v["color"], "ABCDEF");
    }
}

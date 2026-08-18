//! Chart element parsing: the legacy DrawingML chart (`c:` namespace) and the
//! newer chartEx (`cx:` namespace) parsers, plus the pptx `ColorResolver` the
//! shared `ooxml_common::chart` helpers use to resolve `<a:solidFill>` colours.
//! Extracted verbatim from `lib.rs`. The general colour grammar
//! (`parse_color_node`) stays in `lib.rs` and is imported here for the
//! `PptxColorResolver`; both chart parsers now delegate their structure walk to
//! `ooxml_common::chart`.

use crate::parse_color_node;
use crate::types::*;
use ooxml_common::depth::parse_guarded;
use std::collections::HashMap;

/// `ooxml_common::chart::ColorResolver` implementation backed by pptx's
/// `HashMap<String, String>` theme palette and PowerPoint's tint formula.
/// Used by chart helpers in ooxml-common that need to resolve
/// `<a:solidFill>` text colors without owning the theme storage.
pub(crate) struct PptxColorResolver<'a> {
    pub(crate) theme: &'a HashMap<String, String>,
}

impl ooxml_common::chart::ColorResolver for PptxColorResolver<'_> {
    fn resolve_solid_fill(&self, node: roxmltree::Node<'_, '_>) -> Option<String> {
        parse_color_node(node, self.theme)
    }

    fn theme_major_font_latin(&self) -> Option<String> {
        // pptx stores the theme major/minor Latin faces under the `+mj-lt` /
        // `+mn-lt` keys of its color+font map (see lib.rs parse_theme_colors).
        self.theme.get("+mj-lt").cloned()
    }

    fn theme_minor_font_latin(&self) -> Option<String> {
        self.theme.get("+mn-lt").cloned()
    }
}

/// Parse a legacy OOXML chart (`c:` namespace) — barChart / lineChart etc.
///
/// Thin pptx adapter over the shared
/// [`ooxml_common::chart::parse_chart_part`]: it builds a [`PptxColorResolver`]
/// from the theme palette, delegates the entire chart-structure parse, and
/// wraps the resulting [`ChartModel`] in a pptx [`ChartElement`] graphic frame.
/// The frame geometry (`x`/`y`/`width`/`height`) is filled in by the caller
/// from the slide's `<p:graphicFrame><a:xfrm>`; here it defaults to 0.
pub(crate) fn parse_legacy_chart(
    xml: &str,
    theme: &HashMap<String, String>,
) -> Option<ChartElement> {
    let doc = parse_guarded(xml).ok()?;
    let root = doc.root_element();
    let resolver = PptxColorResolver { theme };
    let chart = ooxml_common::chart::parse_chart_part(root, &resolver)?;
    Some(ChartElement {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        rotation: 0.0,
        flip_h: false,
        flip_v: false,
        chart,
    })
}

/// Parse a modern chartEx (cx: namespace) — waterfall, treemap, etc.
///
/// Thin pptx adapter over the shared
/// [`ooxml_common::chart::parse_chartex_part`]: it builds a [`PptxColorResolver`]
/// from the theme palette, delegates the entire chartEx-structure parse, and
/// wraps the resulting [`ChartModel`] in a pptx [`ChartElement`] graphic frame.
/// The frame geometry (`x`/`y`/`width`/`height`) is filled in by the caller
/// from the slide's `<p:graphicFrame><a:xfrm>`; here it defaults to 0.
pub(crate) fn parse_chartex(
    xml: &str,
    style_xml: Option<&str>,
    theme: &HashMap<String, String>,
) -> Option<ChartElement> {
    let doc = parse_guarded(xml).ok()?;
    let root = doc.root_element();
    let resolver = PptxColorResolver { theme };
    // chartEx (waterfall/boxWhisker/…) reads its title font size from the
    // associated chartStyle part when the `<cx:title>` itself carries none.
    let chart = ooxml_common::chart::parse_chartex_part(root, &resolver, style_xml)?;
    Some(ChartElement {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        rotation: 0.0,
        flip_h: false,
        flip_v: false,
        chart,
    })
}

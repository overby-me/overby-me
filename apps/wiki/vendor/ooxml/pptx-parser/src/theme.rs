//! Theme + colour-map parsing: `<a:theme>` colour/font scheme extraction, the
//! master `<p:clrMap>` / `<p:clrMapOvr>` logical-name remapping, and the pptx
//! `ThemeResolver` (`PptxSchemeResolver`) that resolves `<a:schemeClr>` names to
//! theme-slot hexes. Extracted verbatim from `lib.rs`. Shared XML helpers
//! (`child`, `attr`) stay in `lib.rs` and are imported here.

use crate::{attr, child};
use ooxml_common::depth::parse_guarded;
use std::collections::HashMap;

/// Parse the color scheme from a theme XML file.
/// Returns a map: scheme slot name (e.g. "dk1", "lt1", "acc1") → hex string.
///
/// The clrScheme and fontScheme are parsed by the shared
/// [`ooxml_common::theme`] grammar; this function keeps pptx's flat merged-map
/// storage (colors, `+mj-lt`/`+mn-*` font keys, `+lnRef-N` line widths and
/// `+txDef`/`+spDef` object defaults all in one `HashMap<String, String>`)
/// because ~30 call sites look these up by string key. The lnStyleLst and
/// objectDefaults handling stays local — the former's "record only entries with
/// an explicit `w`, as a raw string" contract differs from the shared
/// default-filling `parse_ln_style_widths`, and the latter is pptx-specific.
pub(crate) fn parse_theme_colors(xml: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    // Color slots: shared parse, kept RAW (pptx applies no case-folding, unlike
    // docx/xlsx). prstClr is now resolved via the shared preset table.
    for (slot, hex) in ooxml_common::theme::ThemeColorScheme::parse(xml).iter() {
        map.insert(slot.to_owned(), hex.to_owned());
    }

    // Font scheme: shared parse, mapped onto pptx's `+mj-*` / `+mn-*` keys.
    let fonts = ooxml_common::theme::ThemeFonts::parse(xml);
    for (group, prefix) in [(&fonts.major, "+mj"), (&fonts.minor, "+mn")] {
        for (face, axis) in [(&group.latin, "lt"), (&group.ea, "ea"), (&group.cs, "cs")] {
            if let Some(typeface) = face {
                map.insert(format!("{prefix}-{axis}"), typeface.clone());
            }
        }
    }

    let doc = match parse_guarded(xml) {
        Ok(d) => d,
        Err(_) => return map,
    };
    let root = doc.root_element();

    // Parse fmtScheme > lnStyleLst so lnRef idx="N" can resolve to the theme's
    // canonical stroke width (9525 is wrong; theme defines 12700 / 19050 / 25400).
    // Stored under "+lnRef-1", "+lnRef-2", "+lnRef-3". Kept local: only entries
    // that declare an explicit `w` get a key (a bare `<a:ln/>` is skipped, unlike
    // the shared helper which fills the CT_LineProperties 9525 default), and the
    // value is the raw `w` string the consumer re-parses.
    if let Some(fmt_scheme) = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "fmtScheme")
    {
        if let Some(ln_style_lst) = child(fmt_scheme, "lnStyleLst") {
            for (i, ln) in ln_style_lst
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "ln")
                .enumerate()
            {
                if let Some(w) = attr(&ln, "w") {
                    map.insert(format!("+lnRef-{}", i + 1), w);
                }
            }
        }
    }

    // Parse <a:objectDefaults> per ECMA-376 §20.1.6.7. PowerPoint stores
    // `<a:txDef>` (text-box default), `<a:spDef>` (shape default) and
    // `<a:lnDef>` (line default) here; their `<a:bodyPr>` settings are the
    // last-resort fallback below master/layout/slide for any text body that
    // doesn't override the attribute. Sample-2 hides a `<a:spAutoFit/>`
    // inside its txDef — without inheriting it, every text box in the
    // template defaults to "noAutofit + wrap=square" (the spec literal),
    // which makes mixed-size runs like "20代" wrap unnecessarily and
    // reproduces the slide-13 regression on every similar deck. We parse
    // the bodyPr attributes (and the autoFit child element) into namespaced
    // theme keys so `parse_text_body` can fall back through them without
    // changing function signatures.
    if let Some(obj_defaults) = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "objectDefaults")
    {
        let read_def = |def_name: &str, key_prefix: &str, map: &mut HashMap<String, String>| {
            let Some(def) = child(obj_defaults, def_name) else {
                return;
            };
            let Some(body_pr) = child(def, "bodyPr") else {
                return;
            };
            // Plain attributes — copy verbatim; consumers parse the strings.
            for attr_name in [
                "wrap",
                "anchor",
                "anchorCtr",
                "vert",
                "rtlCol",
                "lIns",
                "rIns",
                "tIns",
                "bIns",
                "numCol",
                "spcCol",
                "vertOverflow",
                "horzOverflow",
                "spcFirstLastPara",
                "rot",
                "upright",
                "fromWordArt",
                "forceAA",
                "compatLnSpc",
            ] {
                if let Some(v) = attr(&body_pr, attr_name) {
                    map.insert(format!("{key_prefix}-bodyPr-{attr_name}"), v);
                }
            }
            // Auto-fit is a child element, not an attribute. Encode as
            // `{prefix}-autoFit` → "sp" | "norm" | "none".
            let auto_fit = if child(body_pr, "spAutoFit").is_some() {
                "sp"
            } else if child(body_pr, "normAutofit").is_some() {
                "norm"
            } else {
                "none"
            };
            // Only record when explicit; "none" is the spec default and
            // recording it would make every consumer see `Some("none")` even
            // for themes that didn't say anything.
            if auto_fit != "none" {
                map.insert(format!("{key_prefix}-autoFit"), auto_fit.to_owned());
            }
        };
        read_def("txDef", "+txDef", &mut map);
        read_def("spDef", "+spDef", &mut map);
    }

    map
}

/// Bake a slide master's `<p:clrMap>` (ECMA-376 §19.3.1.6) into a theme map so
/// that logical scheme names (bg1/tx1/bg2/tx2/accent1..6/hlink/folHlink) can be
/// resolved by a direct `theme.get(name)` lookup later.
///
/// `<p:clrMap>` maps each logical name to a theme color-scheme slot
/// (dk1/lt1/dk2/lt2/accent1..6/hlink/folHlink). We resolve that indirection
/// here and insert `theme[logical] = theme[slot]` for every logical name. This
/// keeps `parse_color_node_tint`'s `schemeClr` handling a single map lookup.
///
/// When `<p:clrMap>` is absent (or an attribute is missing) the PowerPoint
/// default mapping is applied: bg1=lt1, tx1=dk1, bg2=lt2, tx2=dk2, accentN
/// identity, hlink/folHlink identity. The raw slot keys (dk1, lt1, …) added by
/// `parse_theme_colors` are left untouched, so canonical lookups still work.
/// The 12 logical scheme names of a `CT_ColorMapping` (ECMA-376 §19.3.1.6),
/// paired with the scheme slot each maps to by default when a `<p:clrMap>`
/// attribute is absent: bg1=lt1, tx1=dk1, bg2=lt2, tx2=dk2, accentN identity,
/// hlink/folHlink identity. Shared by `<p:clrMap>` (§19.3.1.6) and
/// `<a:overrideClrMapping>` (§20.1.6.8), which carry the identical attribute set.
/// Aliases `ooxml_common::color::SCHEME_DEFAULT_SLOTS` so the default §19.3.1.6
/// table lives in exactly one place across the workspace.
pub(crate) const CLR_MAP_LOGICALS: &[(&str, &str)] = ooxml_common::color::SCHEME_DEFAULT_SLOTS;

/// Read the 12 `CT_ColorMapping` attributes (§19.3.1.6) from `node` into an owned
/// `{logical → slot}` map. Works for both `<p:clrMap>` and
/// `<a:overrideClrMapping>` (same attribute set). roxmltree borrows the doc, so
/// we resolve into an owned map here rather than return the node.
pub(crate) fn parse_clr_map_node(node: roxmltree::Node<'_, '_>) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    for (logical, _) in CLR_MAP_LOGICALS {
        if let Some(slot) = attr(&node, logical) {
            m.insert((*logical).to_owned(), slot);
        }
    }
    m
}

/// Apply a `{logical → slot}` color mapping to `theme`, inserting
/// `theme[logical] = theme[slot]` for every logical name (falling back to the
/// default slot from `CLR_MAP_LOGICALS` when the mapping omits an attr). Reads
/// the raw scheme slot keys (dk1/lt1/dk2/lt2/accent1..6/hlink/folHlink) and
/// writes the logical keys, leaving the raw slots untouched — so the same
/// `theme` can be re-baked later with an override mapping that again resolves
/// against the intact raw slots. `clr_map` = `None` applies the all-default
/// PowerPoint mapping.
pub(crate) fn apply_clr_map(
    theme: &mut HashMap<String, String>,
    clr_map: Option<&HashMap<String, String>>,
) {
    for (logical, default_slot) in CLR_MAP_LOGICALS {
        // Resolve the slot this logical name points at (mapping value, else default).
        let slot = clr_map
            .and_then(|m| m.get(*logical).cloned())
            .unwrap_or_else(|| (*default_slot).to_owned());
        // theme[logical] = theme[slot] when the slot has a hex; otherwise skip
        // (leaves any prior value, and the canonical fallback still applies).
        if let Some(hex) = theme.get(&slot).cloned() {
            theme.insert((*logical).to_owned(), hex);
        }
    }
}

pub(crate) fn bake_clr_map(theme: &mut HashMap<String, String>, master_xml: Option<&str>) {
    // Find the master's <p:clrMap> element (direct child of <p:sldMaster>) and
    // resolve its 12 logical→slot attrs, then apply.
    let clr_map = master_xml.and_then(|xml| {
        let doc = parse_guarded(xml).ok()?;
        let node = child(doc.root_element(), "clrMap")?;
        Some(parse_clr_map_node(node))
    });
    apply_clr_map(theme, clr_map.as_ref());
}

/// Resolve a slide-or-layout's `<p:clrMapOvr>` color-mapping override
/// (ECMA-376 §19.3.1.7 CT_ColorMappingOverride). Returns:
/// - `Some(map)` when `<a:overrideClrMapping>` is present — its 12 logical→slot
///   attrs replace the master's mapping for this slide/layout (§20.1.6.8).
/// - `None` when `<a:masterClrMapping/>` is present (§20.1.6.6) or there is no
///   `<p:clrMapOvr>` at all — both mean "inherit the master's clrMap".
pub(crate) fn parse_clr_map_ovr(xml: &str) -> Option<HashMap<String, String>> {
    // Fast reject: `<p:clrMapOvr>` is absent on the vast majority of slides, so
    // skip a full second parse of the (often largest) slide XML part when the
    // element name does not even appear. A substring false-positive is harmless
    // — we then parse and find no `overrideClrMapping`, returning `None` as usual.
    if !xml.contains("clrMapOvr") {
        return None;
    }
    let doc = parse_guarded(xml).ok()?;
    // <p:clrMapOvr> is a direct child of <p:sld> / <p:sldLayout> (right after
    // <p:cSld>); the choice inside is masterClrMapping XOR overrideClrMapping.
    let ovr = child(doc.root_element(), "clrMapOvr")?;
    let override_node = child(ovr, "overrideClrMapping")?;
    Some(parse_clr_map_node(override_node))
}

/// Resolve a theme typeface reference (e.g. "+mj-lt") to the actual font family name.
/// If the typeface starts with '+' and has a matching entry in the theme map (added by
/// parse_theme_colors from the fontScheme), returns the resolved name; otherwise returns
/// the original string unchanged.
pub(crate) fn resolve_theme_typeface(typeface: &str, theme: &HashMap<String, String>) -> String {
    if typeface.starts_with('+') {
        if let Some(resolved) = theme.get(typeface) {
            return resolved.clone();
        }
    }
    typeface.to_string()
}

/// Resolves a `<a:schemeClr val>` name to its base theme hex the PowerPoint
/// way, for the shared [`ooxml_common::color::parse_color_node`]. The color
/// grammar (srgbClr/sysClr/prstClr/schemeClr + transforms) is shared; only this
/// theme-slot lookup is pptx-specific.
pub(crate) struct PptxSchemeResolver<'a> {
    pub(crate) theme: &'a HashMap<String, String>,
}

impl ooxml_common::color::ThemeResolver for PptxSchemeResolver<'_> {
    fn resolve_scheme_color(&self, name: &str) -> Option<String> {
        // Per ECMA-376 §19.3.1.6 the master's <p:clrMap> remaps logical
        // names (bg1/tx1/bg2/tx2/accentN/hlink/folHlink) to theme slots.
        // `bake_clr_map` pre-bakes those logical names into the theme
        // map, so try a direct lookup FIRST — this honors clrMap (e.g.
        // tx1="lt1"). Fall back to the canonical alias only when the
        // logical name was not baked (no master / unmapped name), so a
        // missing clrMap still resolves tx1→dk1, bg1→lt1, etc.
        if let Some(hex) = self.theme.get(name) {
            return Some(hex.clone());
        }
        // Canonical logical→slot fallback, per the default §19.3.1.6
        // clrMap (shared table: ooxml_common::color::SCHEME_DEFAULT_SLOTS).
        // The helper also passes raw slot names (dk1/lt1/…) and accents
        // through unchanged.
        let canonical: &str = match name {
            // phClr = "placeholder color" (inherits from layout).
            // Approximate as the primary dark text color. Not part of
            // §19.3.1.6, so it stays a local special case.
            "phClr" => "dk1",
            other => ooxml_common::color::default_scheme_slot(other),
        };
        self.theme.get(canonical).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shallow_theme_xml_parses_colors() {
        // A normal theme parses through `parse_guarded` unchanged (sanity: the
        // guard does not reject legitimate parts).
        let xml = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <a:themeElements><a:clrScheme name="Office">
              <a:dk1><a:srgbClr val="000000"/></a:dk1>
              <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
            </a:clrScheme></a:themeElements></a:theme>"#;
        let map = parse_theme_colors(xml);
        assert_eq!(map.get("dk1").map(String::as_str), Some("000000"));
        assert_eq!(map.get("lt1").map(String::as_str), Some("FFFFFF"));
    }

    // ── RB2 neutralization (roxmltree layer): a pathologically deep theme/part
    //    XML is rejected by the depth pre-check in `parse_guarded` BEFORE
    //    roxmltree's recursive tree builder runs, so `parse_theme_colors` returns
    //    gracefully instead of trapping. This runs on the DEFAULT (small)
    //    test-thread stack ON PURPOSE: handing 5 000-deep XML straight to
    //    roxmltree here would overflow and abort the process. That it returns is
    //    the guarantee. (The shape-walk DepthGuard is covered separately in
    //    shape.rs on a generous stack.)
    #[test]
    fn deeply_nested_theme_xml_is_rejected_not_trapped() {
        let mut xml = String::from(
            r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">"#,
        );
        // 5 000 levels — ~20× MAX_XML_DEPTH and far past roxmltree's ~1 000-deep
        // small-stack overflow threshold.
        for _ in 0..5_000 {
            xml.push_str("<a:x>");
        }
        for _ in 0..5_000 {
            xml.push_str("</a:x>");
        }
        xml.push_str("</a:theme>");

        // Must return (not trap); the rejected part yields no colors.
        let map = parse_theme_colors(&xml);
        assert!(
            map.is_empty(),
            "an over-deep theme XML must be rejected, yielding no colors"
        );
    }
}

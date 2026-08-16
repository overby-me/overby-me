//! DrawingML theme parsing (`<a:theme>`) shared by the docx, pptx and xlsx
//! parsers.
//!
//! Every OOXML host embeds the same theme grammar (ECMA-376 §20.1.6 /
//! §14.2.7 / §20.1.4.1): a `<a:clrScheme>` of twelve named color slots, a
//! `<a:fontScheme>` with major/minor typefaces per script, and a
//! `<a:fmtScheme><a:lnStyleLst>` of reference line widths. The three parsers had
//! three partial, drifting copies — pptx resolved `<a:prstClr>` preset names
//! while docx and xlsx silently dropped them; xlsx never read the font scheme;
//! docx never read line styles. Consolidating the *parse* here fixes the prstClr
//! gap uniformly (a preset color now resolves in all three formats) while each
//! parser keeps its own thin key-format adapter and color casing.
//!
//! Scope is "types + parse + pure predicate". This module reads the theme XML
//! into owned structs and stores each color slot's hex **exactly as authored**
//! (the `srgbClr@val` / `sysClr@lastClr` string verbatim, or a preset's
//! canonical hex). Case-folding, `#` prefixing, logical-name (`clrMap`)
//! resolution and the runtime color transforms (lumMod/tint/…) are NOT here —
//! they diverge per host and stay in each parser / the renderer.

use crate::ns::is_a_ns;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The twelve `<a:clrScheme>` slot names in ECMA-376 §20.1.6.2 declaration
/// order: `dk1`, `lt1`, `dk2`, `lt2`, `accent1`..`accent6`, `hlink`,
/// `folHlink`. Exposed so a consumer that needs positional order (xlsx indexes
/// its palette by slot ordinal) can iterate without hard-coding the list.
pub const CLR_SCHEME_SLOTS: [&str; 12] = [
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
    "hlink", "folHlink",
];

/// The parsed `<a:clrScheme>`: slot name → hex string, stored **raw** (as
/// authored — no `#`, no case-folding). Keys are the twelve slot names present
/// in the theme; a slot whose color could not be read (unsupported child, or a
/// `prstClr` name outside [`preset_color`]) is simply absent.
///
/// A [`BTreeMap`] backs it so any serialized form is deterministic; lookups by
/// slot name are the common case ([`ThemeColorScheme::get`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeColorScheme {
    colors: BTreeMap<String, String>,
}

impl ThemeColorScheme {
    /// Parse the `<a:clrScheme>` from a theme XML document. Each slot child holds
    /// one color element; `srgbClr` contributes its `val`, `sysClr` its
    /// `lastClr`, and `prstClr` its [`preset_color`] hex. The stored string is
    /// verbatim (the caller applies any casing / `#` prefix). Missing scheme or
    /// malformed XML yields an empty scheme.
    pub fn parse(xml: &str) -> Self {
        let mut colors = BTreeMap::new();
        let Ok(doc) = crate::depth::parse_guarded(xml) else {
            return Self { colors };
        };
        let Some(scheme) = doc
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "clrScheme")
        else {
            return Self { colors };
        };
        for slot in scheme.children().filter(|n| n.is_element()) {
            let name = slot.tag_name().name().to_owned();
            for c in slot.children().filter(|n| n.is_element()) {
                let hex = color_node_hex(c);
                if let Some(h) = hex {
                    colors.insert(name, h);
                    break;
                }
            }
        }
        Self { colors }
    }

    /// Look up a slot's raw hex by name (`dk1`, `accent1`, `hlink`, …).
    pub fn get(&self, slot: &str) -> Option<&str> {
        self.colors.get(slot).map(String::as_str)
    }

    /// The twelve slots in spec order (`CLR_SCHEME_SLOTS`), each `Some(raw hex)`
    /// when present. Lets a positional consumer (xlsx builds a `Vec` indexed by
    /// ordinal) reconstruct its ordered palette while dropping absent slots as it
    /// sees fit.
    pub fn slots_in_order(&self) -> [Option<&str>; 12] {
        CLR_SCHEME_SLOTS.map(|slot| self.get(slot))
    }

    /// Iterate slot-name → raw-hex pairs (sorted by slot name). For a host that
    /// stores the palette in its own string-keyed map.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.colors.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// True when no slot color was parsed.
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }
}

/// Resolve a single DrawingML color element to its raw hex, covering the three
/// forms a `<a:clrScheme>` slot uses: `srgbClr@val`, `sysClr@lastClr`, and
/// `prstClr@val` (via [`preset_color`]). Other elements (e.g. `scheme`-relative
/// colors, which never appear inside a theme's own scheme) yield `None`. The
/// returned string is verbatim — no casing or `#` is applied.
fn color_node_hex(node: roxmltree::Node<'_, '_>) -> Option<String> {
    match node.tag_name().name() {
        "srgbClr" => node.attribute("val").map(str::to_owned),
        "sysClr" => node.attribute("lastClr").map(str::to_owned),
        "prstClr" => preset_color(node.attribute("val").unwrap_or_default()),
        _ => None,
    }
}

/// The parsed `<a:fontScheme>`: the major (heading) and minor (body) typeface
/// for each script axis. Stored as owned strings; a script with no `typeface`
/// (or an empty one) is `None`. Each parser maps these onto its own key format
/// (pptx `+mj-lt`, docx `major/latin`, …) in its adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFonts {
    /// `<a:majorFont>` typefaces (latin / ea / cs).
    pub major: ThemeFontGroup,
    /// `<a:minorFont>` typefaces (latin / ea / cs).
    pub minor: ThemeFontGroup,
}

/// The three script typefaces of one font group (`<a:majorFont>` or
/// `<a:minorFont>`): Latin (`<a:latin>`), East-Asian (`<a:ea>`) and
/// complex-script (`<a:cs>`). Empty `typeface=""` (common for `ea`/`cs`) is
/// normalized to `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeFontGroup {
    pub latin: Option<String>,
    pub ea: Option<String>,
    pub cs: Option<String>,
}

impl ThemeFonts {
    /// Parse the `<a:fontScheme>` (major + minor × latin/ea/cs). Missing scheme
    /// or malformed XML yields all-`None`.
    pub fn parse(xml: &str) -> Self {
        let Ok(doc) = crate::depth::parse_guarded(xml) else {
            return Self::default();
        };
        let Some(scheme) = doc
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "fontScheme")
        else {
            return Self::default();
        };
        Self {
            major: parse_font_group(scheme, "majorFont"),
            minor: parse_font_group(scheme, "minorFont"),
        }
    }
}

/// Read one `<a:majorFont>` / `<a:minorFont>` child's latin/ea/cs typefaces.
fn parse_font_group(scheme: roxmltree::Node<'_, '_>, group_name: &str) -> ThemeFontGroup {
    let Some(group) = scheme
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == group_name)
    else {
        return ThemeFontGroup::default();
    };
    let read = |axis: &str| -> Option<String> {
        group
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == axis)
            .and_then(|n| n.attribute("typeface"))
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    ThemeFontGroup {
        latin: read("latin"),
        ea: read("ea"),
        cs: read("cs"),
    }
}

/// Parse `<a:fmtScheme><a:lnStyleLst>` line-reference widths (EMU), in
/// declaration order. A drawing shape's `<a:style><a:lnRef idx="N">` resolves
/// its outline width from entry N (1-based) of this list (ECMA-376 §20.1.4.2.19);
/// an `<a:ln>` without an explicit `w` uses the CT_LineProperties default 9525
/// EMU = 0.75 pt (§20.1.2.2.24). Missing scheme or malformed XML yields an empty
/// list.
pub fn parse_ln_style_widths(xml: &str) -> Vec<i64> {
    let Ok(doc) = crate::depth::parse_guarded(xml) else {
        return Vec::new();
    };
    for node in doc.descendants() {
        if node.tag_name().name() == "lnStyleLst" && is_a_ns(node.tag_name().namespace()) {
            return node
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "ln")
                .map(|ln| {
                    ln.attribute("w")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(9525)
                })
                .collect();
        }
    }
    Vec::new()
}

/// Resolve a DrawingML `<a:prstClr>` preset color name (ECMA-376 §20.1.10.48
/// ST_PresetColorVal) to its canonical uppercase hex. Covers the **complete**
/// 190-value enumeration (each `<xsd:enumeration>` of `ST_PresetColorVal` in
/// `dml-main.xsd`), with the RGB values taken verbatim from the §20.1.10.48
/// value table. Unrecognized names yield `None` (the caller leaves the slot /
/// fill unset). Single source of truth for the three parsers, which previously
/// handled only a subset (pptx) or none (docx/xlsx).
///
/// The 190 enum values collapse to 140 distinct hexes: the `dk*`/`lt*` Office
/// shorthands are *usually* exact aliases of the corresponding `dark*`/`light*`
/// CSS names, so they share an arm — but the spec table lists two deliberate
/// exceptions that must NOT be merged:
///
/// - `dkSeaGreen` = 8FBC8B vs `darkSeaGreen` = 8FBC8F
/// - `ltGoldenrodYellow` = FAFA78 vs `lightGoldenrodYellow` = FAFAD2
///
/// These are spec-authored differences (verified against the §20.1.10.48 table),
/// not typos, so they get their own arms.
pub fn preset_color(name: &str) -> Option<String> {
    let hex = match name {
        "aliceBlue" => "F0F8FF",
        "antiqueWhite" => "FAEBD7",
        "aqua" | "cyan" => "00FFFF",
        "aquamarine" => "7FFFD4",
        "azure" => "F0FFFF",
        "beige" => "F5F5DC",
        "bisque" => "FFE4C4",
        "black" => "000000",
        "blanchedAlmond" => "FFEBCD",
        "blue" => "0000FF",
        "blueViolet" => "8A2BE2",
        "brown" => "A52A2A",
        "burlyWood" => "DEB887",
        "cadetBlue" => "5F9EA0",
        "chartreuse" => "7FFF00",
        "chocolate" => "D2691E",
        "coral" => "FF7F50",
        "cornflowerBlue" => "6495ED",
        "cornsilk" => "FFF8DC",
        "crimson" => "DC143C",
        "darkBlue" | "dkBlue" => "00008B",
        "darkCyan" | "dkCyan" => "008B8B",
        "darkGoldenrod" | "dkGoldenrod" => "B8860B",
        "darkGray" | "darkGrey" | "dkGray" | "dkGrey" => "A9A9A9",
        "darkGreen" | "dkGreen" => "006400",
        "darkKhaki" | "dkKhaki" => "BDB76B",
        "darkMagenta" | "dkMagenta" => "8B008B",
        "darkOliveGreen" | "dkOliveGreen" => "556B2F",
        "darkOrange" | "dkOrange" => "FF8C00",
        "darkOrchid" | "dkOrchid" => "9932CC",
        "darkRed" | "dkRed" => "8B0000",
        "darkSalmon" | "dkSalmon" => "E9967A",
        "darkSeaGreen" => "8FBC8F",
        "dkSeaGreen" => "8FBC8B",
        "darkSlateBlue" | "dkSlateBlue" => "483D8B",
        "darkSlateGray" | "darkSlateGrey" | "dkSlateGray" | "dkSlateGrey" => "2F4F4F",
        "darkTurquoise" | "dkTurquoise" => "00CED1",
        "darkViolet" | "dkViolet" => "9400D3",
        "deepPink" => "FF1493",
        "deepSkyBlue" => "00BFFF",
        "dimGray" | "dimGrey" => "696969",
        "dodgerBlue" => "1E90FF",
        "firebrick" => "B22222",
        "floralWhite" => "FFFAF0",
        "forestGreen" => "228B22",
        "fuchsia" | "magenta" => "FF00FF",
        "gainsboro" => "DCDCDC",
        "ghostWhite" => "F8F8FF",
        "gold" => "FFD700",
        "goldenrod" => "DAA520",
        "gray" | "grey" => "808080",
        "green" => "008000",
        "greenYellow" => "ADFF2F",
        "honeydew" => "F0FFF0",
        "hotPink" => "FF69B4",
        "indianRed" => "CD5C5C",
        "indigo" => "4B0082",
        "ivory" => "FFFFF0",
        "khaki" => "F0E68C",
        "lavender" => "E6E6FA",
        "lavenderBlush" => "FFF0F5",
        "lawnGreen" => "7CFC00",
        "lemonChiffon" => "FFFACD",
        "lightBlue" | "ltBlue" => "ADD8E6",
        "lightCoral" | "ltCoral" => "F08080",
        "lightCyan" | "ltCyan" => "E0FFFF",
        "lightGoldenrodYellow" => "FAFAD2",
        "ltGoldenrodYellow" => "FAFA78",
        "lightGray" | "lightGrey" | "ltGray" | "ltGrey" => "D3D3D3",
        "lightGreen" | "ltGreen" => "90EE90",
        "lightPink" | "ltPink" => "FFB6C1",
        "lightSalmon" | "ltSalmon" => "FFA07A",
        "lightSeaGreen" | "ltSeaGreen" => "20B2AA",
        "lightSkyBlue" | "ltSkyBlue" => "87CEFA",
        "lightSlateGray" | "lightSlateGrey" | "ltSlateGray" | "ltSlateGrey" => "778899",
        "lightSteelBlue" | "ltSteelBlue" => "B0C4DE",
        "lightYellow" | "ltYellow" => "FFFFE0",
        "lime" => "00FF00",
        "limeGreen" => "32CD32",
        "linen" => "FAF0E6",
        "maroon" => "800000",
        "medAquamarine" | "mediumAquamarine" => "66CDAA",
        "medBlue" | "mediumBlue" => "0000CD",
        "medOrchid" | "mediumOrchid" => "BA55D3",
        "medPurple" | "mediumPurple" => "9370DB",
        "medSeaGreen" | "mediumSeaGreen" => "3CB371",
        "medSlateBlue" | "mediumSlateBlue" => "7B68EE",
        "medSpringGreen" | "mediumSpringGreen" => "00FA9A",
        "medTurquoise" | "mediumTurquoise" => "48D1CC",
        "medVioletRed" | "mediumVioletRed" => "C71585",
        "midnightBlue" => "191970",
        "mintCream" => "F5FFFA",
        "mistyRose" => "FFE4E1",
        "moccasin" => "FFE4B5",
        "navajoWhite" => "FFDEAD",
        "navy" => "000080",
        "oldLace" => "FDF5E6",
        "olive" => "808000",
        "oliveDrab" => "6B8E23",
        "orange" => "FFA500",
        "orangeRed" => "FF4500",
        "orchid" => "DA70D6",
        "paleGoldenrod" => "EEE8AA",
        "paleGreen" => "98FB98",
        "paleTurquoise" => "AFEEEE",
        "paleVioletRed" => "DB7093",
        "papayaWhip" => "FFEFD5",
        "peachPuff" => "FFDAB9",
        "peru" => "CD853F",
        "pink" => "FFC0CB",
        "plum" => "DDA0DD",
        "powderBlue" => "B0E0E6",
        "purple" => "800080",
        "red" => "FF0000",
        "rosyBrown" => "BC8F8F",
        "royalBlue" => "4169E1",
        "saddleBrown" => "8B4513",
        "salmon" => "FA8072",
        "sandyBrown" => "F4A460",
        "seaGreen" => "2E8B57",
        "seaShell" => "FFF5EE",
        "sienna" => "A0522D",
        "silver" => "C0C0C0",
        "skyBlue" => "87CEEB",
        "slateBlue" => "6A5ACD",
        "slateGray" | "slateGrey" => "708090",
        "snow" => "FFFAFA",
        "springGreen" => "00FF7F",
        "steelBlue" => "4682B4",
        "tan" => "D2B48C",
        "teal" => "008080",
        "thistle" => "D8BFD8",
        "tomato" => "FF6347",
        "turquoise" => "40E0D0",
        "violet" => "EE82EE",
        "wheat" => "F5DEB3",
        "white" => "FFFFFF",
        "whiteSmoke" => "F5F5F5",
        "yellow" => "FFFF00",
        "yellowGreen" => "9ACD32",
        _ => return None,
    };
    Some(hex.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 190 `ST_PresetColorVal` enum values (in `dml-main.xsd` order). Pins
    /// that `preset_color` resolves the complete enumeration — a drift guard
    /// against the XSD.
    const PRESET_ENUM_NAMES: [&str; 190] = [
        "aliceBlue",
        "antiqueWhite",
        "aqua",
        "aquamarine",
        "azure",
        "beige",
        "bisque",
        "black",
        "blanchedAlmond",
        "blue",
        "blueViolet",
        "brown",
        "burlyWood",
        "cadetBlue",
        "chartreuse",
        "chocolate",
        "coral",
        "cornflowerBlue",
        "cornsilk",
        "crimson",
        "cyan",
        "darkBlue",
        "darkCyan",
        "darkGoldenrod",
        "darkGray",
        "darkGrey",
        "darkGreen",
        "darkKhaki",
        "darkMagenta",
        "darkOliveGreen",
        "darkOrange",
        "darkOrchid",
        "darkRed",
        "darkSalmon",
        "darkSeaGreen",
        "darkSlateBlue",
        "darkSlateGray",
        "darkSlateGrey",
        "darkTurquoise",
        "darkViolet",
        "dkBlue",
        "dkCyan",
        "dkGoldenrod",
        "dkGray",
        "dkGrey",
        "dkGreen",
        "dkKhaki",
        "dkMagenta",
        "dkOliveGreen",
        "dkOrange",
        "dkOrchid",
        "dkRed",
        "dkSalmon",
        "dkSeaGreen",
        "dkSlateBlue",
        "dkSlateGray",
        "dkSlateGrey",
        "dkTurquoise",
        "dkViolet",
        "deepPink",
        "deepSkyBlue",
        "dimGray",
        "dimGrey",
        "dodgerBlue",
        "firebrick",
        "floralWhite",
        "forestGreen",
        "fuchsia",
        "gainsboro",
        "ghostWhite",
        "gold",
        "goldenrod",
        "gray",
        "grey",
        "green",
        "greenYellow",
        "honeydew",
        "hotPink",
        "indianRed",
        "indigo",
        "ivory",
        "khaki",
        "lavender",
        "lavenderBlush",
        "lawnGreen",
        "lemonChiffon",
        "lightBlue",
        "lightCoral",
        "lightCyan",
        "lightGoldenrodYellow",
        "lightGray",
        "lightGrey",
        "lightGreen",
        "lightPink",
        "lightSalmon",
        "lightSeaGreen",
        "lightSkyBlue",
        "lightSlateGray",
        "lightSlateGrey",
        "lightSteelBlue",
        "lightYellow",
        "ltBlue",
        "ltCoral",
        "ltCyan",
        "ltGoldenrodYellow",
        "ltGray",
        "ltGrey",
        "ltGreen",
        "ltPink",
        "ltSalmon",
        "ltSeaGreen",
        "ltSkyBlue",
        "ltSlateGray",
        "ltSlateGrey",
        "ltSteelBlue",
        "ltYellow",
        "lime",
        "limeGreen",
        "linen",
        "magenta",
        "maroon",
        "medAquamarine",
        "medBlue",
        "medOrchid",
        "medPurple",
        "medSeaGreen",
        "medSlateBlue",
        "medSpringGreen",
        "medTurquoise",
        "medVioletRed",
        "mediumAquamarine",
        "mediumBlue",
        "mediumOrchid",
        "mediumPurple",
        "mediumSeaGreen",
        "mediumSlateBlue",
        "mediumSpringGreen",
        "mediumTurquoise",
        "mediumVioletRed",
        "midnightBlue",
        "mintCream",
        "mistyRose",
        "moccasin",
        "navajoWhite",
        "navy",
        "oldLace",
        "olive",
        "oliveDrab",
        "orange",
        "orangeRed",
        "orchid",
        "paleGoldenrod",
        "paleGreen",
        "paleTurquoise",
        "paleVioletRed",
        "papayaWhip",
        "peachPuff",
        "peru",
        "pink",
        "plum",
        "powderBlue",
        "purple",
        "red",
        "rosyBrown",
        "royalBlue",
        "saddleBrown",
        "salmon",
        "sandyBrown",
        "seaGreen",
        "seaShell",
        "sienna",
        "silver",
        "skyBlue",
        "slateBlue",
        "slateGray",
        "slateGrey",
        "snow",
        "springGreen",
        "steelBlue",
        "tan",
        "teal",
        "thistle",
        "tomato",
        "turquoise",
        "violet",
        "wheat",
        "white",
        "whiteSmoke",
        "yellow",
        "yellowGreen",
    ];

    const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
      <a:themeElements>
        <a:clrScheme name="Office">
          <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
          <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
          <a:dk2><a:srgbClr val="44546A"/></a:dk2>
          <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
          <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
          <a:accent2><a:prstClr val="orange"/></a:accent2>
          <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
          <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
        </a:clrScheme>
        <a:fontScheme name="Office">
          <a:majorFont>
            <a:latin typeface="Aptos Display"/>
            <a:ea typeface="Yu Gothic"/>
            <a:cs typeface=""/>
          </a:majorFont>
          <a:minorFont>
            <a:latin typeface="Aptos"/>
            <a:ea typeface=""/>
            <a:cs typeface=""/>
          </a:minorFont>
        </a:fontScheme>
        <a:fmtScheme name="Office">
          <a:lnStyleLst>
            <a:ln w="6350"/>
            <a:ln w="12700"/>
            <a:ln/>
          </a:lnStyleLst>
        </a:fmtScheme>
      </a:themeElements>
    </a:theme>"#;

    #[test]
    fn clr_scheme_reads_srgb_sys_and_preset_raw() {
        let s = ThemeColorScheme::parse(THEME);
        // sysClr uses lastClr, srgbClr uses val — both raw (as authored).
        assert_eq!(s.get("dk1"), Some("000000"));
        assert_eq!(s.get("lt1"), Some("FFFFFF"));
        assert_eq!(s.get("dk2"), Some("44546A"));
        assert_eq!(s.get("accent1"), Some("4472C4"));
        // prstClr resolves through preset_color (uniform across formats).
        assert_eq!(s.get("accent2"), Some("FFA500"));
        assert_eq!(s.get("hlink"), Some("0563C1"));
        // Slots not present in the XML are absent (not empty strings).
        assert_eq!(s.get("accent3"), None);
    }

    #[test]
    fn clr_scheme_slots_in_order_matches_spec_positions() {
        let s = ThemeColorScheme::parse(THEME);
        let ordered = s.slots_in_order();
        assert_eq!(ordered[0], Some("000000")); // dk1
        assert_eq!(ordered[1], Some("FFFFFF")); // lt1
        assert_eq!(ordered[4], Some("4472C4")); // accent1
        assert_eq!(ordered[5], Some("FFA500")); // accent2 (preset)
        assert_eq!(ordered[6], None); // accent3 absent
        assert_eq!(ordered[10], Some("0563C1")); // hlink
    }

    #[test]
    fn font_scheme_reads_axes_and_drops_empty() {
        let f = ThemeFonts::parse(THEME);
        assert_eq!(f.major.latin.as_deref(), Some("Aptos Display"));
        assert_eq!(f.major.ea.as_deref(), Some("Yu Gothic"));
        // Empty typeface="" normalizes to None (not Some("")).
        assert_eq!(f.major.cs, None);
        assert_eq!(f.minor.latin.as_deref(), Some("Aptos"));
        assert_eq!(f.minor.ea, None);
        assert_eq!(f.minor.cs, None);
    }

    #[test]
    fn ln_style_widths_reads_list_with_default() {
        // Third <a:ln> has no w → CT_LineProperties default 9525.
        assert_eq!(parse_ln_style_widths(THEME), vec![6350, 12700, 9525]);
    }

    /// Same fixture as [`ln_style_widths_reads_list_with_default`], but declared
    /// under the ISO/IEC 29500 Strict `a:` URI
    /// (`http://purl.oclc.org/ooxml/drawingml/main`) instead of the Transitional
    /// one. `parse_ln_style_widths` must accept both — a document saved by
    /// Office in Strict conformance still has a `<a:fmtScheme><a:lnStyleLst>` to
    /// resolve `<a:lnRef idx="N">` line widths from.
    #[test]
    fn ln_style_widths_reads_list_with_default_strict_ns() {
        const STRICT_THEME: &str = r#"<a:theme xmlns:a="http://purl.oclc.org/ooxml/drawingml/main">
          <a:themeElements>
            <a:fmtScheme name="Office">
              <a:lnStyleLst>
                <a:ln w="6350"/>
                <a:ln w="12700"/>
                <a:ln/>
              </a:lnStyleLst>
            </a:fmtScheme>
          </a:themeElements>
        </a:theme>"#;
        assert_eq!(parse_ln_style_widths(STRICT_THEME), vec![6350, 12700, 9525]);
    }

    #[test]
    fn preset_color_covers_named_presets() {
        assert_eq!(preset_color("black").as_deref(), Some("000000"));
        assert_eq!(preset_color("white").as_deref(), Some("FFFFFF"));
        assert_eq!(preset_color("gray").as_deref(), Some("808080"));
        assert_eq!(preset_color("grey").as_deref(), Some("808080"));
        assert_eq!(preset_color("orange").as_deref(), Some("FFA500"));
        // Previously unrecognized names now resolve (full 190-value table).
        assert_eq!(preset_color("chartreuse").as_deref(), Some("7FFF00"));
        assert_eq!(preset_color("cornflowerBlue").as_deref(), Some("6495ED"));
        assert_eq!(preset_color("rebeccaPurple"), None); // truly absent name
    }

    /// The full §20.1.10.48 table resolves all 190 enum values, and the
    /// dk*/lt* Office shorthands alias their dark*/light* CSS names *except*
    /// for two spec-authored exceptions.
    #[test]
    fn preset_color_full_table_and_alias_exceptions() {
        // dk*/lt* usual aliases.
        assert_eq!(preset_color("dkBlue"), preset_color("darkBlue"));
        assert_eq!(preset_color("ltGray"), preset_color("lightGray"));
        assert_eq!(preset_color("medBlue"), preset_color("mediumBlue"));
        // Two deliberate spec exceptions — must NOT be equal.
        assert_eq!(preset_color("darkSeaGreen").as_deref(), Some("8FBC8F"));
        assert_eq!(preset_color("dkSeaGreen").as_deref(), Some("8FBC8B"));
        assert_ne!(preset_color("dkSeaGreen"), preset_color("darkSeaGreen"));
        assert_eq!(
            preset_color("lightGoldenrodYellow").as_deref(),
            Some("FAFAD2")
        );
        assert_eq!(preset_color("ltGoldenrodYellow").as_deref(), Some("FAFA78"));
        assert_ne!(
            preset_color("ltGoldenrodYellow"),
            preset_color("lightGoldenrodYellow")
        );
        // Every one of the 190 enum values resolves to some hex.
        for name in PRESET_ENUM_NAMES {
            assert!(
                preset_color(name).is_some(),
                "preset_color({name}) returned None"
            );
        }
        assert_eq!(PRESET_ENUM_NAMES.len(), 190);
    }

    #[test]
    fn empty_and_malformed_yield_empty() {
        assert!(ThemeColorScheme::parse("").is_empty());
        assert!(ThemeColorScheme::parse("<not xml").is_empty());
        assert_eq!(ThemeFonts::parse(""), ThemeFonts::default());
        assert!(parse_ln_style_widths("").is_empty());
    }
}

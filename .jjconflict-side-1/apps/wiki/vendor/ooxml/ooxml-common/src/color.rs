//! OOXML color transforms (lumMod, lumOff, satMod, satOff, hueMod, hueOff,
//! shade, tint, alpha and friends) shared between the docx and pptx parsers.
//!
//! Word and PowerPoint diverge on the `tint` transform — Word reads `val` as
//! the *retained fraction of the input color* (the literal ECMA-376
//! §20.1.2.3.34 reading: `result = val·input + (1-val)·white`), while
//! PowerPoint applies it as a `lerp(input, white, val)` in linear sRGB. Empirical
//! comparison against PDF exports confirms each app does its own thing — see
//! `TintMode` and the per-app `apply_color_transforms_with` flag.
//!
//! Everything else (shade, lumMod/Off, satMod/Off, hueMod/Off, alpha
//! family) is identical between the two and lives here uncopied.

use roxmltree::Node;

/// Default DrawingML logical-color → theme scheme-slot mapping
/// (ECMA-376 §19.3.1.6 `clrMap` / CT_ColorMapping, with the standard
/// PowerPoint default attribute values).
///
/// A theme's `<a:clrScheme>` stores twelve named slots — `dk1`, `lt1`, `dk2`,
/// `lt2`, `accent1`..`accent6`, `hlink`, `folHlink`. Documents reference colors
/// by *logical* name (`bg1`, `tx1`, `bg2`, `tx2`, the accents, and the two
/// hyperlink names) and a `clrMap` indirection layer resolves each logical name
/// to a slot. When no explicit `clrMap` override is present the default mapping
/// applies:
///
/// - `bg1` → `lt1`, `tx1` → `dk1` (background 1 is the light slot, text 1 the
///   dark slot)
/// - `bg2` → `lt2`, `tx2` → `dk2`
/// - `accent1`..`accent6`, `hlink`, `folHlink` map to the identically named slot
///
/// This is the single source of truth for that default table. Each parser keeps
/// its own *resolution* (storage layout and per-app tint), but the logical→slot
/// names live here. See also [`default_scheme_slot`] for a lookup that also
/// accepts a raw slot name and returns it unchanged.
pub const SCHEME_DEFAULT_SLOTS: &[(&str, &str)] = &[
    ("bg1", "lt1"),
    ("tx1", "dk1"),
    ("bg2", "lt2"),
    ("tx2", "dk2"),
    ("accent1", "accent1"),
    ("accent2", "accent2"),
    ("accent3", "accent3"),
    ("accent4", "accent4"),
    ("accent5", "accent5"),
    ("accent6", "accent6"),
    ("hlink", "hlink"),
    ("folHlink", "folHlink"),
];

/// Resolve a DrawingML color name to its default theme scheme slot
/// (ECMA-376 §19.3.1.6, default `clrMap`).
///
/// Logical names listed in [`SCHEME_DEFAULT_SLOTS`] (`bg1`/`tx1`/`bg2`/`tx2`)
/// map to their slot; every other input — including the raw slot names
/// (`dk1`, `lt1`, …), the accents and the hyperlink names — is returned
/// unchanged. This mirrors how a parser walks a `schemeClr@val` that may carry
/// either a logical name *or* a slot name and wants the underlying slot.
pub fn default_scheme_slot(name: &str) -> &str {
    SCHEME_DEFAULT_SLOTS
        .iter()
        .find(|(logical, _)| *logical == name)
        .map(|(_, slot)| *slot)
        .unwrap_or(name)
}

/// Selects the formula applied to `<a:tint val>` modifiers. The OOXML spec
/// is consistent (val = retained input), but the two desktop apps render
/// templates differently in practice — see ECMA-376 §20.1.2.3.34 and the
/// commit history of pptx-parser for the linear-sRGB derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintMode {
    /// Word: `result = val·input + (1-val)·white` in sRGB. Matches Word's
    /// rendering of resume / cover templates that use accent recolors with
    /// tint values.
    WordLiteral,
    /// PowerPoint: `lerp(input, white, val)` in linear sRGB. Matches
    /// PowerPoint's rendering of SmartArt accent recolors pixel-for-pixel.
    PowerPointLinear,
}

/// Apply OOXML color transforms to `hex` based on the modifier elements
/// declared as direct children of `node`. Returns 6-char hex when fully
/// opaque, or 8-char hex (RRGGBBAA) when alpha < 1.
pub fn apply_color_transforms(hex: &str, node: Node, tint_mode: TintMode) -> String {
    if hex.len() < 6 {
        return hex.to_owned();
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);

    let mut rf = r as f64 / 255.0;
    let mut gf = g as f64 / 255.0;
    let mut bf = b as f64 / 255.0;
    let mut alpha = if hex.len() >= 8 {
        u8::from_str_radix(&hex[6..8], 16).unwrap_or(255) as f64 / 255.0
    } else {
        1.0
    };

    let attr_pct = |t: &Node, name: &str, default: f64| -> f64 {
        t.attribute(name)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(default)
            / 100_000.0
    };

    for t in node.children().filter(|n| n.is_element()) {
        match t.tag_name().name() {
            "lumMod" => {
                let val = attr_pct(&t, "val", 100_000.0);
                let (h, l, s) = rgb_to_hls(rf, gf, bf);
                let (nr, ng, nb) = hls_to_rgb(h, (l * val).min(1.0), s);
                rf = nr;
                gf = ng;
                bf = nb;
            }
            "lumOff" => {
                let val = attr_pct(&t, "val", 0.0);
                let (h, l, s) = rgb_to_hls(rf, gf, bf);
                let (nr, ng, nb) = hls_to_rgb(h, (l + val).clamp(0.0, 1.0), s);
                rf = nr;
                gf = ng;
                bf = nb;
            }
            "satMod" => {
                let val = attr_pct(&t, "val", 100_000.0);
                let (h, l, s) = rgb_to_hls(rf, gf, bf);
                let (nr, ng, nb) = hls_to_rgb(h, l, (s * val).clamp(0.0, 1.0));
                rf = nr;
                gf = ng;
                bf = nb;
            }
            "satOff" => {
                let val = attr_pct(&t, "val", 0.0);
                let (h, l, s) = rgb_to_hls(rf, gf, bf);
                let (nr, ng, nb) = hls_to_rgb(h, l, (s + val).clamp(0.0, 1.0));
                rf = nr;
                gf = ng;
                bf = nb;
            }
            "hueMod" => {
                let val = attr_pct(&t, "val", 100_000.0);
                let (h, l, s) = rgb_to_hls(rf, gf, bf);
                let (nr, ng, nb) = hls_to_rgb((h * val).rem_euclid(1.0), l, s);
                rf = nr;
                gf = ng;
                bf = nb;
            }
            "hueOff" => {
                // hueOff is in 60000ths of a degree per ECMA-376 §20.1.2.3.16.
                let val_deg = t
                    .attribute("val")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    / 60_000.0;
                let (h, l, s) = rgb_to_hls(rf, gf, bf);
                let (nr, ng, nb) = hls_to_rgb((h + val_deg / 360.0).rem_euclid(1.0), l, s);
                rf = nr;
                gf = ng;
                bf = nb;
            }
            "shade" => {
                // ECMA-376 §20.1.2.3.31: result = val·input + (1-val)·black.
                let val = attr_pct(&t, "val", 100_000.0);
                rf *= val;
                gf *= val;
                bf *= val;
            }
            "tint" => {
                let val = attr_pct(&t, "val", 0.0);
                match tint_mode {
                    TintMode::WordLiteral => {
                        // `result = val·input + (1-val)·white` per literal spec.
                        rf = val * rf + (1.0 - val);
                        gf = val * gf + (1.0 - val);
                        bf = val * bf + (1.0 - val);
                    }
                    TintMode::PowerPointLinear => {
                        // PowerPoint reads val as the lerp fraction toward
                        // white in LINEAR sRGB. Verified against PDF
                        // exports of SmartArt accent recolors.
                        let lr = srgb_to_linear(rf);
                        let lg = srgb_to_linear(gf);
                        let lb = srgb_to_linear(bf);
                        rf = linear_to_srgb((lr + (1.0 - lr) * val).clamp(0.0, 1.0));
                        gf = linear_to_srgb((lg + (1.0 - lg) * val).clamp(0.0, 1.0));
                        bf = linear_to_srgb((lb + (1.0 - lb) * val).clamp(0.0, 1.0));
                    }
                }
            }
            "alpha" => {
                // ECMA-376 §20.1.2.3.1 — sets absolute alpha.
                alpha = attr_pct(&t, "val", 100_000.0);
            }
            "alphaModFix" => {
                // ECMA-376 §20.1.8.4 — fixed (absolute) alpha modulation.
                alpha = attr_pct(&t, "amt", 100_000.0);
            }
            "alphaMod" => {
                // ECMA-376 §20.1.2.3.2 — multiply current alpha by val/100000.
                alpha *= attr_pct(&t, "val", 100_000.0);
            }
            "alphaOff" => {
                // ECMA-376 §20.1.2.3.3 — additive offset to alpha.
                alpha += attr_pct(&t, "val", 0.0);
            }
            "comp" => {
                // ECMA-376 §20.1.2.3.7 — the *complement*: the color which,
                // mixed with the input, yields a shade of grey. The spec's
                // worked example is red RGB(255,0,0) → cyan RGB(0,255,255),
                // i.e. a 180° hue rotation with saturation and luminance held
                // constant (red is HSL(0°,100%,50%); cyan is HSL(180°,100%,50%)).
                // Rotating hue by half the wheel is the definition that both
                // reproduces the spec example and keeps the "mix → grey"
                // property (two hues 180° apart average to a neutral).
                let (h, l, s) = rgb_to_hls(rf, gf, bf);
                let (nr, ng, nb) = hls_to_rgb((h + 0.5).rem_euclid(1.0), l, s);
                rf = nr;
                gf = ng;
                bf = nb;
            }
            "inv" => {
                // ECMA-376 §20.1.2.3.17 — the per-channel RGB inverse. The
                // spec example is red(1,0,0) → cyan(0,1,1), i.e. 1 − channel.
                // (Distinct from `comp`: `inv` is a straight channel negation,
                // which happens to also map red→cyan but differs for
                // non-primary colors.)
                rf = 1.0 - rf;
                gf = 1.0 - gf;
                bf = 1.0 - bf;
            }
            "gray" => {
                // ECMA-376 §20.1.2.3.9 — grayscale weighted by the relative
                // intensities of the RGB primaries (Rec. 601 luma coefficients,
                // the standard perceptual weighting for sRGB primaries).
                let y = 0.299 * rf + 0.587 * gf + 0.114 * bf;
                rf = y;
                gf = y;
                bf = y;
            }
            "gamma" => {
                // ECMA-376 §20.1.2.3.8 — the sRGB gamma shift of the input.
                // Applying the sRGB *encoding* transfer function (linear→sRGB)
                // to the channel values.
                rf = linear_to_srgb(rf.clamp(0.0, 1.0));
                gf = linear_to_srgb(gf.clamp(0.0, 1.0));
                bf = linear_to_srgb(bf.clamp(0.0, 1.0));
            }
            "invGamma" => {
                // ECMA-376 §20.1.2.3.18 — the inverse sRGB gamma shift: the
                // sRGB *decoding* transfer function (sRGB→linear).
                rf = srgb_to_linear(rf.clamp(0.0, 1.0));
                gf = srgb_to_linear(gf.clamp(0.0, 1.0));
                bf = srgb_to_linear(bf.clamp(0.0, 1.0));
            }
            _ => {}
        }
    }

    let r = (rf.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (gf.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (bf.clamp(0.0, 1.0) * 255.0).round() as u8;
    if (alpha - 1.0).abs() < 0.004 {
        format!("{:02X}{:02X}{:02X}", r, g, b)
    } else {
        let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("{:02X}{:02X}{:02X}{:02X}", r, g, b, a)
    }
}

/// A DrawingML color element (`<a:srgbClr>` / `<a:sysClr>` / `<a:prstClr>` /
/// `<a:schemeClr>`), extracted from a color container's first color child but
/// **not yet resolved to a hex string**. This is the format-agnostic middle
/// layer between [`extract_color_source`] (which finds the element) and
/// [`parse_color_node`] (which resolves + transforms it). The three parsers
/// share the *grammar* here; theme-slot resolution (which differs per host —
/// pptx bakes a `clrMap`, xlsx indexes a positional palette, docx keys a slot
/// map) stays behind the [`ThemeResolver`] trait.
///
/// The wrapped `Node` is retained so the caller can apply the color-transform
/// children (lumMod/tint/…) via [`apply_color_transforms`] on the resolved base.
#[derive(Debug, Clone)]
pub enum ColorSource<'a, 'input> {
    /// `<a:srgbClr val="RRGGBB">` — an explicit hex (ECMA-376 §20.1.2.3.32).
    /// `val` is the raw authored string (case as-authored). "A perceptual gamma
    /// of 2.2 is used" — the hex is already the display value, so no gamma
    /// conversion is applied to the base.
    SrgbClr { val: String, node: Node<'a, 'input> },
    /// `<a:scrgbClr r="50000" g="50000" b="50000">` — an RGB color in the
    /// percentage variant (ECMA-376 §20.1.2.3.30). "A linear gamma of 1.0 is
    /// assumed": r/g/b are *linear-light* percentages (ST_Percentage, 1/1000 %,
    /// 0..100000). The base hex is obtained by gamma-*encoding* each linear
    /// channel to sRGB. The spec's own example pins this — `r=g=b=50%` equals
    /// `<a:srgbClr val="BCBCBC">` (0xBC = round(linear_to_srgb(0.5)·255)), NOT
    /// the naive 0x80.
    ScrgbClr {
        r: f64,
        g: f64,
        b: f64,
        node: Node<'a, 'input>,
    },
    /// `<a:hslClr hue="14400000" sat="100000" lum="50000">` — a color in the
    /// HSL model (ECMA-376 §20.1.2.3.13). `hue` is ST_PositiveFixedAngle in
    /// 1/60000 of a degree (0..21600000); `sat`/`lum` are ST_Percentage
    /// (1/1000 %). Resolved via the shared HLS→RGB conversion.
    HslClr {
        hue: f64,
        sat: f64,
        lum: f64,
        node: Node<'a, 'input>,
    },
    /// `<a:sysClr val="windowText" lastClr="RRGGBB">` — a system color
    /// (§20.1.2.3.33). `last_clr` is the cached resolved hex; `val` is the
    /// system-color enum name, retained only as a last-ditch fallback (matches
    /// docx's historical behavior — pptx/xlsx never authored a val-only sysClr).
    SysClr {
        last_clr: Option<String>,
        val: Option<String>,
        node: Node<'a, 'input>,
    },
    /// `<a:prstClr val="black">` — a named preset color (§20.1.2.3.22 /
    /// §20.1.10.48). Resolved through [`preset_color`](crate::theme::preset_color).
    /// The node is retained so color-transform children apply on the resolved
    /// preset base (per CT_PresetColor's `EG_ColorTransform` content model).
    PrstClr { val: String, node: Node<'a, 'input> },
    /// `<a:schemeClr val="accent1">` — a theme-slot reference (§20.1.2.3.29).
    /// `val` is the logical/slot name; the [`ThemeResolver`] maps it to a base
    /// hex. The node is retained for the transform children.
    SchemeClr { val: String, node: Node<'a, 'input> },
}

/// Locate the first DrawingML color element among a container's element
/// children and return it as a [`ColorSource`]. Covers the four color-choice
/// members a `<a:solidFill>` / `<a:gs>` / run `<a:rPr>` / shadow etc. carry:
/// `srgbClr`, `sysClr`, `prstClr`, `schemeClr`. Returns `None` when no such
/// child exists (e.g. a `<a:noFill>` sibling, or a `<a:solidFill>` with no
/// color child). The caller supplies the *container* node (the same node its
/// prior inline `for child` loop walked).
pub fn extract_color_source<'a, 'input>(
    container: Node<'a, 'input>,
) -> Option<ColorSource<'a, 'input>> {
    for c in container.children().filter(|n| n.is_element()) {
        match c.tag_name().name() {
            "srgbClr" => {
                return Some(ColorSource::SrgbClr {
                    val: c.attribute("val")?.to_owned(),
                    node: c,
                });
            }
            "scrgbClr" => {
                // r/g/b are ST_Percentage (1/1000 of a percent). Missing/invalid
                // attrs default to 0 — treat as black-linear rather than dropping
                // the whole color (a required attr per XSD, but be lenient).
                let pct = |name: &str| -> f64 {
                    c.attribute(name)
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                        / 100_000.0
                };
                return Some(ColorSource::ScrgbClr {
                    r: pct("r"),
                    g: pct("g"),
                    b: pct("b"),
                    node: c,
                });
            }
            "hslClr" => {
                return Some(ColorSource::HslClr {
                    // hue: 1/60000 degree → [0,1) turns.
                    hue: c
                        .attribute("hue")
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                        / 60_000.0
                        / 360.0,
                    // sat/lum: ST_Percentage (1/1000 %).
                    sat: c
                        .attribute("sat")
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                        / 100_000.0,
                    lum: c
                        .attribute("lum")
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                        / 100_000.0,
                    node: c,
                });
            }
            "sysClr" => {
                return Some(ColorSource::SysClr {
                    last_clr: c.attribute("lastClr").map(str::to_owned),
                    val: c.attribute("val").map(str::to_owned),
                    node: c,
                });
            }
            "prstClr" => {
                return Some(ColorSource::PrstClr {
                    val: c.attribute("val")?.to_owned(),
                    node: c,
                });
            }
            "schemeClr" => {
                return Some(ColorSource::SchemeClr {
                    val: c.attribute("val")?.to_owned(),
                    node: c,
                });
            }
            _ => {}
        }
    }
    None
}

/// Host-specific resolution of a `<a:schemeClr val>` name to its **base hex**
/// (6-char, no `#`, ready for [`apply_color_transforms`]). Each parser keeps
/// its own theme storage and logical-name handling:
///
/// - pptx bakes the master's `<p:clrMap>` into its theme map and falls back to
///   the default §19.3.1.6 slot table (plus a `phClr`→dk1 approximation);
/// - xlsx indexes a positional `dk1,lt1,dk2,lt2,accent1..6,hlink,folHlink`
///   palette (with the well-known dk/lt index swap);
/// - docx keys a slot map and substitutes a caller-provided name for `phClr`.
///
/// Returning `None` leaves the color unresolved (the caller then falls back —
/// e.g. to a shape's style color). Implementations must return the base
/// **without** a leading `#` so the shared transform sees clean input.
pub trait ThemeResolver {
    /// Resolve a scheme/logical color name to its base hex (no `#`). `name` is
    /// the raw `<a:schemeClr val>` string.
    fn resolve_scheme_color(&self, name: &str) -> Option<String>;
}

/// Resolve a located color container to a hex string, sharing the DrawingML
/// color grammar across the docx/pptx/xlsx parsers. Finds the first color child
/// ([`extract_color_source`]), resolves it, and applies any color-transform
/// children (lumMod/lumOff/shade/tint/… via [`apply_color_transforms`]):
///
/// - `srgbClr` / `sysClr` → their hex + transforms;
/// - `scrgbClr` → linear r/g/b gamma-encoded to sRGB (§20.1.2.3.30) + transforms;
/// - `hslClr` → HLS→RGB (§20.1.2.3.13) + transforms;
/// - `prstClr` → [`preset_color`](crate::theme::preset_color) + transforms
///   (CT_PresetColor carries an `EG_ColorTransform` list — the prior no-transform
///   behavior was a documented latent gap, now closed);
/// - `schemeClr` → `resolver.resolve_scheme_color(val)` + transforms.
///
/// The returned hex is uppercase and has **no** `#` prefix (that is what
/// [`apply_color_transforms`] emits: `{:02X}`); each caller re-applies its own
/// casing / `#` convention in a thin adapter. `tint_mode` selects the Word vs
/// PowerPoint `<a:tint>` interpretation ([`TintMode`]). Returns `None` when no
/// color child is present or a `schemeClr` fails to resolve.
pub fn parse_color_node<R: ThemeResolver + ?Sized>(
    container: Node<'_, '_>,
    resolver: &R,
    tint_mode: TintMode,
) -> Option<String> {
    resolve_color_source(extract_color_source(container)?, resolver, tint_mode)
}

/// Build a [`ColorSource`] from a node that IS itself a colour element
/// (`<a:srgbClr>`, `<a:schemeClr>`, `<a:prstClr>`, …), rather than a container
/// wrapping one. [`extract_color_source`] scans a container's *children*; this
/// classifies the node directly. Needed where the color elements appear as a
/// bare list — e.g. `<a:duotone>`'s two `EG_ColorChoice` children (§20.1.8.23),
/// which are the colour elements themselves. Returns `None` for a non-colour
/// element (or a `srgbClr`/`prstClr`/`schemeClr` missing its required `val`).
pub fn color_source_from_element<'a, 'input>(
    el: Node<'a, 'input>,
) -> Option<ColorSource<'a, 'input>> {
    match el.tag_name().name() {
        "srgbClr" => Some(ColorSource::SrgbClr {
            val: el.attribute("val")?.to_owned(),
            node: el,
        }),
        "scrgbClr" => {
            let pct = |name: &str| -> f64 {
                el.attribute(name)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    / 100_000.0
            };
            Some(ColorSource::ScrgbClr {
                r: pct("r"),
                g: pct("g"),
                b: pct("b"),
                node: el,
            })
        }
        "hslClr" => Some(ColorSource::HslClr {
            hue: el
                .attribute("hue")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 60_000.0
                / 360.0,
            sat: el
                .attribute("sat")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 100_000.0,
            lum: el
                .attribute("lum")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 100_000.0,
            node: el,
        }),
        "sysClr" => Some(ColorSource::SysClr {
            last_clr: el.attribute("lastClr").map(str::to_owned),
            val: el.attribute("val").map(str::to_owned),
            node: el,
        }),
        "prstClr" => Some(ColorSource::PrstClr {
            val: el.attribute("val")?.to_owned(),
            node: el,
        }),
        "schemeClr" => Some(ColorSource::SchemeClr {
            val: el.attribute("val")?.to_owned(),
            node: el,
        }),
        _ => None,
    }
}

/// Resolve a located [`ColorSource`] to a hex string (uppercase, no `#`),
/// applying its EG_ColorTransform children. Split out of [`parse_color_node`] so
/// callers that already hold a `ColorSource` (e.g. from
/// [`color_source_from_element`]) can resolve it without re-scanning.
pub fn resolve_color_source<R: ThemeResolver + ?Sized>(
    source: ColorSource<'_, '_>,
    resolver: &R,
    tint_mode: TintMode,
) -> Option<String> {
    match source {
        ColorSource::SrgbClr { val, node } => Some(apply_color_transforms(&val, node, tint_mode)),
        ColorSource::ScrgbClr { r, g, b, node } => {
            // Linear-light r/g/b (0..1) → sRGB display hex, then transforms.
            let base = format!(
                "{:02X}{:02X}{:02X}",
                (linear_to_srgb(r.clamp(0.0, 1.0)) * 255.0).round() as u8,
                (linear_to_srgb(g.clamp(0.0, 1.0)) * 255.0).round() as u8,
                (linear_to_srgb(b.clamp(0.0, 1.0)) * 255.0).round() as u8,
            );
            Some(apply_color_transforms(&base, node, tint_mode))
        }
        ColorSource::HslClr {
            hue,
            sat,
            lum,
            node,
        } => {
            // HLS→RGB (note the shared helper takes h, l, s order).
            let (r, g, b) = hls_to_rgb(
                hue.rem_euclid(1.0),
                lum.clamp(0.0, 1.0),
                sat.clamp(0.0, 1.0),
            );
            let base = format!(
                "{:02X}{:02X}{:02X}",
                (r.clamp(0.0, 1.0) * 255.0).round() as u8,
                (g.clamp(0.0, 1.0) * 255.0).round() as u8,
                (b.clamp(0.0, 1.0) * 255.0).round() as u8,
            );
            Some(apply_color_transforms(&base, node, tint_mode))
        }
        ColorSource::SysClr {
            last_clr,
            val,
            node,
        } => {
            // Prefer the cached lastClr; fall back to the enum name only when
            // lastClr is absent (docx's historical behavior — see ColorSource).
            let hex = last_clr.or(val)?;
            Some(apply_color_transforms(&hex, node, tint_mode))
        }
        ColorSource::PrstClr { val, node } => {
            // Resolve the preset base, then apply any EG_ColorTransform children.
            let base = crate::theme::preset_color(&val)?;
            Some(apply_color_transforms(&base, node, tint_mode))
        }
        ColorSource::SchemeClr { val, node } => {
            let base = resolver.resolve_scheme_color(&val)?;
            Some(apply_color_transforms(&base, node, tint_mode))
        }
    }
}

/// sRGB → linear light. IEC 61966-2-1 transfer function.
pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light → sRGB.
pub fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// RGB → HLS conversion (lightness in the middle, matches Python's colorsys).
/// Returns (h, l, s) with each component in [0, 1].
pub fn rgb_to_hls(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-10 {
        return (0.0, l, 0.0);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < 1e-10 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1e-10 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, l, s)
}

/// HLS → RGB conversion. (h, l, s) are each in [0, 1]; returns linear-RGB
/// triple in [0, 1].
pub fn hls_to_rgb(h: f64, l: f64, s: f64) -> (f64, f64, f64) {
    if s < 1e-10 {
        return (l, l, l);
    }
    fn hue2rgb(p: f64, q: f64, mut t: f64) -> f64 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 0.5 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    (
        hue2rgb(p, q, h + 1.0 / 3.0),
        hue2rgb(p, q, h),
        hue2rgb(p, q, h - 1.0 / 3.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the default §19.3.1.6 logical→slot table so the three parsers that
    /// consume it can't drift. The twelve pairs are exactly the PowerPoint
    /// default `clrMap`.
    #[test]
    fn scheme_default_slots_match_spec() {
        assert_eq!(
            SCHEME_DEFAULT_SLOTS,
            &[
                ("bg1", "lt1"),
                ("tx1", "dk1"),
                ("bg2", "lt2"),
                ("tx2", "dk2"),
                ("accent1", "accent1"),
                ("accent2", "accent2"),
                ("accent3", "accent3"),
                ("accent4", "accent4"),
                ("accent5", "accent5"),
                ("accent6", "accent6"),
                ("hlink", "hlink"),
                ("folHlink", "folHlink"),
            ]
        );
    }

    // ── parse_color_node / extract_color_source ─────────────────────────────

    use roxmltree::Document;

    /// A minimal ThemeResolver mapping a couple of slot names to base hex,
    /// letting the scheme-color path be exercised without a full parser.
    struct MapResolver;
    impl ThemeResolver for MapResolver {
        fn resolve_scheme_color(&self, name: &str) -> Option<String> {
            match name {
                "accent1" => Some("4472C4".to_owned()),
                "dk1" => Some("000000".to_owned()),
                _ => None,
            }
        }
    }

    fn parse(xml: &str, mode: TintMode) -> Option<String> {
        // Every fixture wraps the color element in a `<a:solidFill>` container,
        // matching how the parsers pass the located fill node.
        let doc = Document::parse(xml).unwrap();
        parse_color_node(doc.root_element(), &MapResolver, mode)
    }

    const NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

    /// srgbClr resolves to its (uppercased) hex, no `#`, under either TintMode.
    #[test]
    fn parse_color_node_srgb_plain() {
        let xml = format!(r#"<a:solidFill xmlns:a="{NS}"><a:srgbClr val="ff8000"/></a:solidFill>"#);
        assert_eq!(
            parse(&xml, TintMode::WordLiteral).as_deref(),
            Some("FF8000")
        );
        assert_eq!(
            parse(&xml, TintMode::PowerPointLinear).as_deref(),
            Some("FF8000")
        );
    }

    /// sysClr uses lastClr; a transform child (lumMod) is applied on top.
    #[test]
    fn parse_color_node_sysclr_lastclr_with_lummod() {
        let xml = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:sysClr val="windowText" lastClr="FFFFFF"><a:lumMod val="50000"/></a:sysClr></a:solidFill>"#
        );
        // 50% luminance of white → mid gray (uppercase, no #).
        assert_eq!(
            parse(&xml, TintMode::WordLiteral).as_deref(),
            Some("808080")
        );
    }

    /// sysClr with no lastClr falls back to `val` (docx-historical). "windowText"
    /// is not valid hex, but the fallback path must still fire (produces 000000).
    #[test]
    fn parse_color_node_sysclr_val_fallback() {
        let xml = format!(r#"<a:solidFill xmlns:a="{NS}"><a:sysClr val="000000"/></a:solidFill>"#);
        assert_eq!(
            parse(&xml, TintMode::WordLiteral).as_deref(),
            Some("000000")
        );
    }

    /// prstClr resolves via the shared preset table AND applies color-transform
    /// children (§20.1.2.3.22 / CT_PresetColor carries EG_ColorTransform).
    #[test]
    fn parse_color_node_prstclr_via_preset_table() {
        let xml = format!(r#"<a:solidFill xmlns:a="{NS}"><a:prstClr val="orange"/></a:solidFill>"#);
        assert_eq!(
            parse(&xml, TintMode::WordLiteral).as_deref(),
            Some("FFA500")
        );
        // A lumMod=50% transform on white now darkens it to mid gray, proving
        // transforms are applied to the preset base (was a latent gap).
        let xml2 = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:prstClr val="white"><a:lumMod val="50000"/></a:prstClr></a:solidFill>"#
        );
        assert_eq!(
            parse(&xml2, TintMode::WordLiteral).as_deref(),
            Some("808080")
        );
        // A full-range preset from the newly-complete table resolves too.
        let xml3 =
            format!(r#"<a:solidFill xmlns:a="{NS}"><a:prstClr val="chartreuse"/></a:solidFill>"#);
        assert_eq!(
            parse(&xml3, TintMode::WordLiteral).as_deref(),
            Some("7FFF00")
        );
    }

    /// scrgbClr's r/g/b are LINEAR percentages; the base must be gamma-encoded
    /// to sRGB. The spec example pins r=g=b=50% == srgbClr "BCBCBC".
    #[test]
    fn parse_color_node_scrgbclr_linear_gamma_encoded() {
        let xml = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:scrgbClr r="50000" g="50000" b="50000"/></a:solidFill>"#
        );
        // 0xBC, NOT the naive 0x80 — proves linear→sRGB encoding.
        assert_eq!(
            parse(&xml, TintMode::WordLiteral).as_deref(),
            Some("BCBCBC")
        );
        // Endpoints round-trip exactly.
        let black =
            format!(r#"<a:solidFill xmlns:a="{NS}"><a:scrgbClr r="0" g="0" b="0"/></a:solidFill>"#);
        assert_eq!(
            parse(&black, TintMode::WordLiteral).as_deref(),
            Some("000000")
        );
        let white = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:scrgbClr r="100000" g="100000" b="100000"/></a:solidFill>"#
        );
        assert_eq!(
            parse(&white, TintMode::WordLiteral).as_deref(),
            Some("FFFFFF")
        );
    }

    /// hslClr resolves through HLS→RGB. hue in 1/60000 degree, sat/lum in
    /// ST_Percentage. hue=0, sat=100%, lum=50% is pure red.
    #[test]
    fn parse_color_node_hslclr_to_rgb() {
        // Pure red: hue=0, sat=100%, lum=50%.
        let red = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:hslClr hue="0" sat="100000" lum="50000"/></a:solidFill>"#
        );
        assert_eq!(
            parse(&red, TintMode::WordLiteral).as_deref(),
            Some("FF0000")
        );
        // hue=120° (=7200000) green.
        let green = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:hslClr hue="7200000" sat="100000" lum="50000"/></a:solidFill>"#
        );
        assert_eq!(
            parse(&green, TintMode::WordLiteral).as_deref(),
            Some("00FF00")
        );
        // sat=0 → gray at the given luminance (50% → 0x80).
        let gray = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:hslClr hue="0" sat="0" lum="50000"/></a:solidFill>"#
        );
        assert_eq!(
            parse(&gray, TintMode::WordLiteral).as_deref(),
            Some("808080")
        );
    }

    /// The five newly-added transforms behave per their §20.1.2.3 definitions.
    #[test]
    fn parse_color_node_new_transforms() {
        // inv: red → cyan (1−channel), spec §20.1.2.3.17 example.
        let inv = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:srgbClr val="FF0000"><a:inv/></a:srgbClr></a:solidFill>"#
        );
        assert_eq!(
            parse(&inv, TintMode::WordLiteral).as_deref(),
            Some("00FFFF")
        );
        // comp: red → cyan (180° hue rotation), spec §20.1.2.3.7 example.
        let comp = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:srgbClr val="FF0000"><a:comp/></a:srgbClr></a:solidFill>"#
        );
        assert_eq!(
            parse(&comp, TintMode::WordLiteral).as_deref(),
            Some("00FFFF")
        );
        // gray: a saturated color collapses to its luma (equal channels).
        let gray = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:srgbClr val="FF0000"><a:gray/></a:srgbClr></a:solidFill>"#
        );
        let g = parse(&gray, TintMode::WordLiteral).unwrap();
        assert_eq!(&g[0..2], &g[2..4]);
        assert_eq!(&g[2..4], &g[4..6]);
        // 0.299·255 ≈ 76 = 0x4C.
        assert_eq!(g.as_str(), "4C4C4C");
        // gamma then invGamma round-trips (within rounding).
        let round = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:srgbClr val="336699"><a:gamma/><a:invGamma/></a:srgbClr></a:solidFill>"#
        );
        let r = parse(&round, TintMode::WordLiteral).unwrap();
        // Allow ±1 per channel for the double 8-bit round-trip.
        for (i, exp) in [0x33u8, 0x66, 0x99].iter().enumerate() {
            let got = u8::from_str_radix(&r[i * 2..i * 2 + 2], 16).unwrap();
            assert!(
                (got as i16 - *exp as i16).abs() <= 1,
                "channel {i}: got {got:02X} exp {exp:02X}"
            );
        }
    }

    /// schemeClr resolves through the injected ThemeResolver, then transforms.
    #[test]
    fn parse_color_node_schemeclr_via_resolver() {
        let xml =
            format!(r#"<a:solidFill xmlns:a="{NS}"><a:schemeClr val="accent1"/></a:solidFill>"#);
        assert_eq!(
            parse(&xml, TintMode::WordLiteral).as_deref(),
            Some("4472C4")
        );
        // Unknown slot → None (caller falls back).
        let unknown =
            format!(r#"<a:solidFill xmlns:a="{NS}"><a:schemeClr val="accent9"/></a:solidFill>"#);
        assert_eq!(parse(&unknown, TintMode::WordLiteral), None);
    }

    /// The two TintModes diverge on `<a:tint>`: Word reads val as retained input
    /// (a near-white wash at 20%), PowerPoint lerps toward white in linear sRGB.
    /// The two must produce DIFFERENT hex for the same input, proving the mode
    /// is threaded through parse_color_node.
    #[test]
    fn parse_color_node_tint_mode_diverges() {
        let xml = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:schemeClr val="accent1"><a:tint val="20000"/></a:schemeClr></a:solidFill>"#
        );
        let word = parse(&xml, TintMode::WordLiteral).unwrap();
        let ppt = parse(&xml, TintMode::PowerPointLinear).unwrap();
        assert_ne!(word, ppt);
        // Both are 6-char uppercase hex with no '#'.
        assert_eq!(word.len(), 6);
        assert!(!word.contains('#'));
        assert!(word.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    /// alpha < 1 yields an 8-char RRGGBBAA hex (transform emits the alpha byte).
    #[test]
    fn parse_color_node_alpha_yields_eight_hex() {
        let xml = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:srgbClr val="112233"><a:alpha val="50000"/></a:srgbClr></a:solidFill>"#
        );
        let out = parse(&xml, TintMode::WordLiteral).unwrap();
        assert_eq!(out.len(), 8);
        assert!(out.starts_with("112233"));
    }

    /// A container with no color child (e.g. a lone noFill sibling) → None.
    #[test]
    fn parse_color_node_none_when_no_color_child() {
        let xml = format!(r#"<a:solidFill xmlns:a="{NS}"><a:noFill/></a:solidFill>"#);
        assert_eq!(parse(&xml, TintMode::WordLiteral), None);
    }

    /// extract_color_source returns the first color element and skips others.
    #[test]
    fn extract_color_source_picks_first_color_child() {
        let xml = format!(
            r#"<a:solidFill xmlns:a="{NS}"><a:srgbClr val="ABCDEF"/><a:schemeClr val="accent1"/></a:solidFill>"#
        );
        let doc = Document::parse(&xml).unwrap();
        match extract_color_source(doc.root_element()) {
            Some(ColorSource::SrgbClr { val, .. }) => assert_eq!(val, "ABCDEF"),
            other => panic!("expected SrgbClr first, got {other:?}"),
        }
    }

    #[test]
    fn default_scheme_slot_maps_logicals_and_passes_through_slots() {
        // Logical names resolve to their slot.
        assert_eq!(default_scheme_slot("bg1"), "lt1");
        assert_eq!(default_scheme_slot("tx1"), "dk1");
        assert_eq!(default_scheme_slot("bg2"), "lt2");
        assert_eq!(default_scheme_slot("tx2"), "dk2");
        // Raw slot names and accents/hyperlinks pass through unchanged.
        assert_eq!(default_scheme_slot("dk1"), "dk1");
        assert_eq!(default_scheme_slot("lt1"), "lt1");
        assert_eq!(default_scheme_slot("accent3"), "accent3");
        assert_eq!(default_scheme_slot("hlink"), "hlink");
        // Unknown input is returned verbatim (caller decides what to do).
        assert_eq!(default_scheme_slot("phClr"), "phClr");
    }
}

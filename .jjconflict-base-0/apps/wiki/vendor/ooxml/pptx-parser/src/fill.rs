//! Fill / colour / stroke / effect / 3-D scene / custom-geometry parsing
//! (pptx-specific DrawingML). Extracted verbatim from `lib.rs`. The general
//! colour-node grammar (`parse_color_node` / `parse_color_node_tint`) lives here;
//! it uses the `PptxSchemeResolver` (from `theme`) for `<a:schemeClr>` lookups.
//! Shared XML helpers (`child`, `children_vec`, `attr`, `attr_r`, `attr_i64`,
//! `attr_f64`) stay in `lib.rs` and are imported here.

use crate::theme::PptxSchemeResolver;
use crate::types::*;
use crate::{attr, attr_f64, attr_i64, attr_r, child, children_vec};
use ooxml_common::blip::{mime_from_ext, parse_blip_duotone};
use std::collections::HashMap;

/// Parse `<a:blip><a:alphaModFix amt="..."/></a:blip>` from a blipFill node
/// (ECMA-376 §20.1.8.6). Thin re-export of the shared
/// [`ooxml_common::blip::parse_blip_alpha`] so the three formats read the blip
/// alpha identically (previously a pptx-local copy). Returns the fraction
/// `amt/100000` when present and < 1.0; `None` otherwise.
pub(crate) use ooxml_common::blip::parse_blip_alpha;

/// Parse `<a:stretch><a:fillRect l t r b>` (ECMA-376 §20.1.8.58 / §20.1.8.30).
/// Edge attributes are ST_Percentage (1000ths of a percent → /100000 gives a
/// fraction). Negative values are valid (overscan). Returns None when there is
/// no fillRect or all four edges are zero (= the source fills the whole box).
pub(crate) fn parse_fill_rect(stretch: roxmltree::Node<'_, '_>) -> Option<FillRect> {
    let fr = child(stretch, "fillRect")?;
    let read = |name: &str| -> f64 {
        attr(&fr, name)
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v / 100_000.0)
            .unwrap_or(0.0)
    };
    let rect = FillRect {
        l: read("l"),
        t: read("t"),
        r: read("r"),
        b: read("b"),
    };
    if is_zero_f64(&rect.l) && is_zero_f64(&rect.t) && is_zero_f64(&rect.r) && is_zero_f64(&rect.b)
    {
        None
    } else {
        Some(rect)
    }
}

/// Parse `<a:tile tx ty sx sy flip algn>` (ECMA-376 §20.1.8.58 CT_TileInfoProperties).
///
/// - `tx` / `ty`: ST_Coordinate offset of the first tile, in EMU. Default 0.
/// - `sx` / `sy`: ST_Percentage scale of the tile (1000ths of a percent →
///   `/100000` gives a fraction). Absent → `1.0` (100% = native blip size).
/// - `flip`: ST_TileFlipMode (none|x|y|xy). Default "none".
/// - `algn`: ST_RectAlignment (tl|t|tr|l|ctr|r|bl|b|br) — the anchor corner the
///   tile grid is registered against. The schema gives no default; PowerPoint
///   treats an absent `algn` as "tl" (the first tile sits at the box origin
///   plus tx/ty). We carry it verbatim and default to "tl".
pub(crate) fn parse_tile(tile: roxmltree::Node<'_, '_>) -> TileInfo {
    let scale = |name: &str| -> f64 {
        attr(&tile, name)
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v / 100_000.0)
            .unwrap_or(1.0)
    };
    TileInfo {
        tx: attr_i64(&tile, "tx").unwrap_or(0),
        ty: attr_i64(&tile, "ty").unwrap_or(0),
        sx: scale("sx"),
        sy: scale("sy"),
        flip: attr(&tile, "flip").unwrap_or_else(|| "none".to_owned()),
        algn: attr(&tile, "algn").unwrap_or_else(|| "tl".to_owned()),
    }
}

// ===========================
//  Color parsing
// ===========================

/// Resolve a color node (solidFill child / run rPr child) to a hex string.
/// Handles srgbClr, sysClr, prstClr, and schemeClr (with transform support).
pub(crate) fn parse_color_node(
    node: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
) -> Option<String> {
    parse_color_node_tint(node, theme, ooxml_common::color::TintMode::PowerPointLinear)
}

/// Like `parse_color_node`, but lets the caller pick how `<a:tint>` is interpreted.
/// Table styles (`<a:tcStyle>` band fills) use `TintMode::WordLiteral` — the literal
/// ECMA-376 §20.1.2.3.34 definition (`val·input + (1-val)·white`, so a 20% tint is a
/// near-white wash) — which is how PowerPoint renders table band tints. The SmartArt
/// accent-recolor path keeps `PowerPointLinear` (see `apply_color_transforms`).
///
/// Thin wrapper over the shared [`ooxml_common::color::parse_color_node`]: the
/// grammar + transforms live there; [`PptxSchemeResolver`] supplies the
/// pptx-specific theme-slot lookup. Output is unchanged (uppercase hex, no `#`).
pub(crate) fn parse_color_node_tint(
    node: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
    tint_mode: ooxml_common::color::TintMode,
) -> Option<String> {
    ooxml_common::color::parse_color_node(node, &PptxSchemeResolver { theme }, tint_mode)
}

// ===========================
//  Fill / Stroke parsing
// ===========================

pub(crate) fn parse_fill(
    node: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
) -> Option<Fill> {
    for c in node.children().filter(|n| n.is_element()) {
        match c.tag_name().name() {
            "solidFill" => {
                // If the color resolves, use it. If not (e.g. phClr with no theme slot),
                // return None so the caller can fall back to the shape style color.
                if let Some(color) = parse_color_node(c, theme) {
                    return Some(Fill::Solid { color });
                }
                // Unresolvable → don't default to black; let fallback logic handle it
            }
            "noFill" => return Some(Fill::None),
            "pattFill" => {
                // ECMA-376 §20.1.8.40 — preset pattern with fg/bg colours.
                // Shared parse (ooxml_common::fill); colors resolve with pptx's
                // PowerPointLinear tint via PptxSchemeResolver.
                let ooxml_common::fill::PatternFill { fg, bg, preset } =
                    ooxml_common::fill::parse_patt_fill(
                        c,
                        &PptxSchemeResolver { theme },
                        ooxml_common::color::TintMode::PowerPointLinear,
                    );
                return Some(Fill::Pattern { fg, bg, preset });
            }
            "gradFill" => {
                // Shared parse (ooxml_common::fill). Returns None when there are
                // no resolvable stops, so we keep scanning sibling fill elements.
                if let Some(g) = ooxml_common::fill::parse_grad_fill(
                    c,
                    &PptxSchemeResolver { theme },
                    ooxml_common::color::TintMode::PowerPointLinear,
                ) {
                    return Some(Fill::Gradient {
                        stops: g.stops,
                        angle: g.angle,
                        grad_type: g.grad_type,
                    });
                }
            }
            _ => {}
        }
    }
    None
}

/// ECMA-376 §20.1.8.14 `a:blipFill` → `Fill::Image`. The `resolve_blip`
/// closure maps the `<a:blip r:embed>` rId to the blip's embedded **zip path**
/// using the caller's rels (each inheritance level resolves against its own
/// part); the mime is derived from that path. The renderer fetches the bytes
/// lazily by path rather than from an inlined data URL.
///
/// Both fill-modes are honoured and mutually exclusive:
/// - `stretch` (§20.1.8.56): the `fillRect` (§20.1.8.30) is captured so the
///   renderer can place the (possibly overscanned) image into the box.
/// - `tile` (§20.1.8.58): the tile offset/scale/flip/align descriptor is
///   captured so the renderer can repeat the blip at its native (scaled) size.
///
/// When neither child is present the blip defaults to full-box placement
/// (stretch with no fillRect).
///
/// `theme` resolves the `<a:duotone>` (§20.1.8.23) endpoint colours through the
/// slide palette (PowerPoint linear tint), so a picture FILL recolours exactly
/// like a `<p:pic>` picture element does.
pub(crate) fn parse_blip_fill<F: FnMut(&str) -> Option<String>>(
    blip_fill: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
    resolve_blip: &mut F,
) -> Option<Fill> {
    let r_id = child(blip_fill, "blip").and_then(|b| attr_r(&b, "embed"))?;
    let image_path = resolve_blip(&r_id)?;
    let mime_type = mime_from_ext(&image_path).to_owned();
    let alpha = parse_blip_alpha(blip_fill);
    // §20.1.8.23 duotone recolour, resolved through the theme with PowerPoint's
    // linear tint (same call the `<p:pic>` paths use). `None` ⇒ no effect.
    let duotone = parse_blip_duotone(
        blip_fill,
        &PptxSchemeResolver { theme },
        ooxml_common::color::TintMode::PowerPointLinear,
    );
    // §20.1.8.58 tile takes precedence when present (stretch/tile are an
    // either-or choice in CT_BlipFillProperties).
    if let Some(tile_node) = child(blip_fill, "tile") {
        return Some(Fill::Image {
            image_path,
            mime_type,
            fill_rect: None,
            tile: Some(parse_tile(tile_node)),
            alpha,
            duotone,
        });
    }
    let fill_rect = child(blip_fill, "stretch").and_then(parse_fill_rect);
    Some(Fill::Image {
        image_path,
        mime_type,
        fill_rect,
        tile: None,
        alpha,
        duotone,
    })
}

pub(crate) fn parse_arrow_end(node: roxmltree::Node<'_, '_>) -> ArrowEnd {
    let kind = attr(&node, "type").unwrap_or_else(|| "none".to_owned());
    let w = attr(&node, "w").unwrap_or_else(|| "med".to_owned());
    let len = attr(&node, "len").unwrap_or_else(|| "med".to_owned());
    ArrowEnd { kind, w, len }
}

pub(crate) fn parse_stroke(
    ln_node: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
) -> Option<Stroke> {
    if child(ln_node, "noFill").is_some() {
        return None;
    }
    let width = attr_i64(&ln_node, "w").unwrap_or(9525);
    // CT_LineProperties uses EG_LineFillProperties (§20.1.8.38), so a line can
    // carry the same solid/gradient/pattern paints as a shape. Keep a solid
    // fallback colour for arrowheads and consumers that do not yet understand
    // non-solid line paint.
    let parsed_fill = parse_fill(ln_node, theme)?;
    let color = match &parsed_fill {
        Fill::Solid { color } => color.clone(),
        Fill::Gradient { stops, .. } => stops
            .iter()
            .rev()
            .find(|stop| !stop.color.ends_with("00"))
            .or_else(|| stops.last())
            .map(|stop| stop.color.clone())?,
        Fill::Pattern { fg, .. } => fg.clone(),
        Fill::None | Fill::Image { .. } => return None,
    };
    let fill = match parsed_fill {
        Fill::Gradient { .. } | Fill::Pattern { .. } => Some(parsed_fill),
        Fill::Solid { .. } => None,
        Fill::None | Fill::Image { .. } => unreachable!(),
    };
    let dash_style = child(ln_node, "prstDash")
        .and_then(|n| attr(&n, "val"))
        .filter(|v| v != "solid");
    let line_cap = attr(&ln_node, "cap").and_then(|cap| match cap.as_str() {
        "rnd" => Some("round".to_owned()),
        "sq" => Some("square".to_owned()),
        "flat" => Some("butt".to_owned()),
        _ => None,
    });
    // Arrow ends — only emit when type != "none"
    let head_end = child(ln_node, "headEnd")
        .map(parse_arrow_end)
        .filter(|a| a.kind != "none");
    let tail_end = child(ln_node, "tailEnd")
        .map(parse_arrow_end)
        .filter(|a| a.kind != "none");
    // ECMA-376 §20.1.8.42 ST_CompoundLine. Default "sng" stays absent so the
    // renderer keeps its single-stroke fast path.
    let cmpd = attr(&ln_node, "cmpd").filter(|v| v != "sng");
    Some(Stroke {
        color,
        width,
        fill,
        dash_style,
        line_cap,
        head_end,
        tail_end,
        cmpd,
    })
}

// ===========================
//  Shadow parsing
// ===========================

/// Parse spPr > effectLst > outerShdw into a Shadow.
pub(crate) fn parse_shadow(
    effect_lst: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
) -> Option<Shadow> {
    parse_shadow_node(child(effect_lst, "outerShdw")?, theme)
}

/// Parse spPr > effectLst > innerShdw into a Shadow. ECMA-376 §20.1.8.21
/// (CT_InnerShadowEffect) — same field shape as outerShdw, semantics differ
/// at render time (cast inward).
pub(crate) fn parse_inner_shadow(
    effect_lst: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
) -> Option<Shadow> {
    parse_shadow_node(child(effect_lst, "innerShdw")?, theme)
}

/// Shared field reader for innerShdw / outerShdw. Both elements expose
/// blurRad, dist, dir, and a color child with optional alphaModFix.
pub(crate) fn parse_shadow_node(
    n: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
) -> Option<Shadow> {
    let blur = attr_i64(&n, "blurRad").unwrap_or(0);
    let dist = attr_i64(&n, "dist").unwrap_or(0);
    let dir = attr_f64(&n, "dir").unwrap_or(0.0) / 60_000.0;

    let color_str = parse_color_node(n, theme).unwrap_or_else(|| "000000".to_owned());
    let (color, alpha) = if color_str.len() >= 8 {
        let a = u8::from_str_radix(&color_str[6..8], 16).unwrap_or(255) as f64 / 255.0;
        (color_str[..6].to_owned(), a)
    } else {
        (color_str, 1.0)
    };

    Some(Shadow {
        color,
        alpha,
        blur,
        dist,
        dir,
    })
}

/// Parse spPr > effectLst > glow into a Glow effect — ECMA-376 §20.1.8.17
/// (CT_GlowEffect): a coloured halo with a blur radius, no offset.
pub(crate) fn parse_glow(
    effect_lst: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
) -> Option<Glow> {
    let g = child(effect_lst, "glow")?;
    let radius = attr_i64(&g, "rad").unwrap_or(0);
    let color_str = parse_color_node(g, theme).unwrap_or_else(|| "000000".to_owned());
    let (color, alpha) = if color_str.len() >= 8 {
        let a = u8::from_str_radix(&color_str[6..8], 16).unwrap_or(255) as f64 / 255.0;
        (color_str[..6].to_owned(), a)
    } else {
        (color_str, 1.0)
    };
    Some(Glow {
        color,
        alpha,
        radius,
    })
}

/// Parse spPr > effectLst > softEdge into a SoftEdge — ECMA-376 §20.1.8.31.
pub(crate) fn parse_soft_edge(effect_lst: roxmltree::Node<'_, '_>) -> Option<SoftEdge> {
    let n = child(effect_lst, "softEdge")?;
    let radius = attr_i64(&n, "rad").unwrap_or(0);
    Some(SoftEdge { radius })
}

/// Parse spPr > effectLst > reflection — ECMA-376 §20.1.8.27. Defaults
/// follow the spec table: blur=0, dist=0, dir=0, stA=100000 (=1.0),
/// stPos=0, endA=0, endPos=100000 (=1.0), sx=100000, sy=-100000.
pub(crate) fn parse_reflection(effect_lst: roxmltree::Node<'_, '_>) -> Option<Reflection> {
    let r = child(effect_lst, "reflection")?;
    let pct = |name: &str, default: f64| -> f64 {
        attr_f64(&r, name).map(|v| v / 100_000.0).unwrap_or(default)
    };
    Some(Reflection {
        blur: attr_i64(&r, "blurRad").unwrap_or(0),
        dist: attr_i64(&r, "dist").unwrap_or(0),
        dir: attr_f64(&r, "dir").unwrap_or(0.0) / 60_000.0,
        st_a: pct("stA", 1.0),
        st_pos: pct("stPos", 0.0),
        end_a: pct("endA", 0.0),
        end_pos: pct("endPos", 1.0),
        sx: pct("sx", 1.0),
        sy: pct("sy", -1.0),
    })
}

/// Effects pulled from `spPr > effectLst`. The five members are independent
/// siblings inside `CT_EffectList` — ECMA-376 §20.1.8.16. Used by both shapes
/// (`p:sp`) and pictures (`p:pic`): `p:spPr` is `CT_ShapeProperties` in both
/// cases (§19.3.1.37), so `effectLst` applies equally to images.
pub(crate) struct EffectLst {
    pub(crate) shadow: Option<Shadow>,
    pub(crate) inner_shadow: Option<Shadow>,
    pub(crate) glow: Option<Glow>,
    pub(crate) soft_edge: Option<SoftEdge>,
    pub(crate) reflection: Option<Reflection>,
}

/// Read every `effectLst` child shapes and pictures share. `effect_lst` is the
/// optional `<a:effectLst>` node; missing nodes yield an all-`None` result.
pub(crate) fn parse_effect_lst(
    effect_lst: Option<roxmltree::Node<'_, '_>>,
    theme: &HashMap<String, String>,
) -> EffectLst {
    EffectLst {
        shadow: effect_lst.and_then(|n| parse_shadow(n, theme)),
        inner_shadow: effect_lst.and_then(|n| parse_inner_shadow(n, theme)),
        glow: effect_lst.and_then(|n| parse_glow(n, theme)),
        soft_edge: effect_lst.and_then(parse_soft_edge),
        reflection: effect_lst.and_then(parse_reflection),
    }
}

// ===========================
//  3D scene parsing (scene3d / sp3d)
// ===========================

/// Parse `<a:rot>` (`CT_SphereCoords`, ECMA-376 §20.1.5.11). Angles are stored
/// in the XML as 60000ths of a degree; we convert to degrees. All three
/// attributes are required by the schema, but we default missing ones to 0 to
/// stay tolerant of malformed input.
pub(crate) fn parse_rot3d(rot: roxmltree::Node<'_, '_>) -> Rot3d {
    let deg = |name: &str| attr_f64(&rot, name).unwrap_or(0.0) / 60_000.0;
    Rot3d {
        lat: deg("lat"),
        lon: deg("lon"),
        rev: deg("rev"),
    }
}

/// Parse `<a:scene3d>` (`CT_Scene3D`, ECMA-376 §20.1.4.1.41). Requires a
/// `<a:camera>` child (§20.1.5.5); `<a:lightRig>` is optional for our purposes
/// (Phase A renders the camera only). Returns None when no camera is present.
pub(crate) fn parse_scene3d(sppr: roxmltree::Node<'_, '_>) -> Option<Scene3d> {
    let scene = child(sppr, "scene3d")?;
    let cam = child(scene, "camera")?;
    let camera = Camera3d {
        prst: attr(&cam, "prst")?,
        // §20.1.5.5: fov is an ST_FOVAngle in 60000ths of a degree.
        fov: attr_f64(&cam, "fov").map(|v| v / 60_000.0),
        // zoom is an ST_PositivePercentage (100000 = 100%).
        zoom: attr_f64(&cam, "zoom").map(|v| v / 100_000.0),
        rot: child(cam, "rot").map(parse_rot3d),
    };
    let light_rig = child(scene, "lightRig").and_then(|lr| {
        Some(LightRig {
            rig: attr(&lr, "rig")?,
            dir: attr(&lr, "dir")?,
            rot: child(lr, "rot").map(parse_rot3d),
        })
    });
    Some(Scene3d { camera, light_rig })
}

/// Parse `<a:bevel>` (`CT_Bevel`, ECMA-376 §20.1.5.3). `w`/`h` default to
/// 76200 EMU and `prst` to "circle" per the schema.
pub(crate) fn parse_bevel3d(bevel: roxmltree::Node<'_, '_>) -> Bevel3d {
    Bevel3d {
        w: attr_i64(&bevel, "w").unwrap_or(76_200),
        h: attr_i64(&bevel, "h").unwrap_or(76_200),
        prst: attr(&bevel, "prst").unwrap_or_else(|| "circle".into()),
    }
}

/// Parse `<a:sp3d>` (`CT_Shape3D`, ECMA-376 §20.1.5.12). Defaults follow the
/// schema: z=0, extrusionH=0, contourW=0, prstMaterial="warmMatte". Parsed in
/// full but not rendered in Phase A.
pub(crate) fn parse_sp3d(sppr: roxmltree::Node<'_, '_>) -> Option<Sp3d> {
    let n = child(sppr, "sp3d")?;
    // contourClr is colour-only here; pass an empty theme map because sp3d
    // contour colours in practice are srgbClr (no theme lookup needed) and this
    // parser has the theme threaded only into the line/fill paths.
    let contour_clr = child(n, "contourClr").and_then(|c| parse_color_node(c, &HashMap::new()));
    Some(Sp3d {
        z: attr_i64(&n, "z").unwrap_or(0),
        extrusion_h: attr_i64(&n, "extrusionH").unwrap_or(0),
        contour_w: attr_i64(&n, "contourW").unwrap_or(0),
        contour_clr,
        prst_material: attr(&n, "prstMaterial").unwrap_or_else(|| "warmMatte".into()),
        bevel_t: child(n, "bevelT").map(parse_bevel3d),
        bevel_b: child(n, "bevelB").map(parse_bevel3d),
    })
}

// ===========================
//  Custom geometry parsing
// ===========================

/// Parse a single path command node; coordinates are normalised to [0,1].
pub(crate) fn parse_path_cmd(
    cmd_node: roxmltree::Node<'_, '_>,
    path_w: f64,
    path_h: f64,
) -> Option<PathCmd> {
    match cmd_node.tag_name().name() {
        "moveTo" => {
            let pt = child(cmd_node, "pt")?;
            let x = attr_f64(&pt, "x")? / path_w;
            let y = attr_f64(&pt, "y")? / path_h;
            Some(PathCmd::MoveTo { x, y })
        }
        "lnTo" => {
            let pt = child(cmd_node, "pt")?;
            let x = attr_f64(&pt, "x")? / path_w;
            let y = attr_f64(&pt, "y")? / path_h;
            Some(PathCmd::LineTo { x, y })
        }
        "cubicBezTo" => {
            let pts: Vec<_> = children_vec(cmd_node, "pt");
            if pts.len() < 3 {
                return None;
            }
            let x1 = attr_f64(&pts[0], "x")? / path_w;
            let y1 = attr_f64(&pts[0], "y")? / path_h;
            let x2 = attr_f64(&pts[1], "x")? / path_w;
            let y2 = attr_f64(&pts[1], "y")? / path_h;
            let x = attr_f64(&pts[2], "x")? / path_w;
            let y = attr_f64(&pts[2], "y")? / path_h;
            Some(PathCmd::CubicBezTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            })
        }
        "arcTo" => {
            // wR/hR are radii in path-local units; stAng/swAng in 60000ths of a degree
            let wr = attr_f64(&cmd_node, "wR").unwrap_or(0.0) / path_w;
            let hr = attr_f64(&cmd_node, "hR").unwrap_or(0.0) / path_h;
            let st_ang = attr_f64(&cmd_node, "stAng").unwrap_or(0.0) / 60000.0;
            let sw_ang = attr_f64(&cmd_node, "swAng").unwrap_or(0.0) / 60000.0;
            Some(PathCmd::ArcTo {
                wr,
                hr,
                st_ang,
                sw_ang,
            })
        }
        "close" => Some(PathCmd::Close),
        _ => None,
    }
}

/// Parse custGeom > pathLst into a list of sub-paths (one per <a:path> element).
pub(crate) fn parse_cust_geom(cust_geom: roxmltree::Node<'_, '_>) -> Vec<Vec<PathCmd>> {
    let path_lst = match child(cust_geom, "pathLst") {
        Some(n) => n,
        None => return vec![],
    };

    path_lst
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "path")
        .map(|path_node| {
            let path_w = attr_f64(&path_node, "w").unwrap_or(1.0).max(1.0);
            let path_h = attr_f64(&path_node, "h").unwrap_or(1.0).max(1.0);
            path_node
                .children()
                .filter(|n| n.is_element())
                .filter_map(|cmd| parse_path_cmd(cmd, path_w, path_h))
                .collect()
        })
        .collect()
}

// ===========================
//  Transform (a:xfrm)
// ===========================

pub(crate) fn parse_xfrm(xfrm: roxmltree::Node<'_, '_>) -> Transform {
    let rot = attr_f64(&xfrm, "rot").unwrap_or(0.0) / 60000.0;
    let flip_h = attr(&xfrm, "flipH")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);
    let flip_v = attr(&xfrm, "flipV")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);
    let off = child(xfrm, "off");
    let ext = child(xfrm, "ext");
    Transform {
        x: off.and_then(|n| attr_i64(&n, "x")).unwrap_or(0),
        y: off.and_then(|n| attr_i64(&n, "y")).unwrap_or(0),
        cx: ext.and_then(|n| attr_i64(&n, "cx")).unwrap_or(0),
        cy: ext.and_then(|n| attr_i64(&n, "cy")).unwrap_or(0),
        rot,
        flip_h,
        flip_v,
    }
}

// ===========================
//  Slide background
// ===========================

/// ECMA-376 §19.3.1.1 `p:bg`. `resolve_blip` maps a `<a:blip r:embed>` rId to a
/// base64 data URL using the rels + zip of the part this `c_sld` belongs to
/// (slide / layout / master), so an image background (§20.1.8.14) is resolved
/// against the correct relationship base.
pub(crate) fn parse_background<F: FnMut(&str) -> Option<String>>(
    c_sld: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
    resolve_blip: &mut F,
) -> Option<Fill> {
    let bg = child(c_sld, "bg")?;
    // bgPr contains an explicit fill specification
    if let Some(bg_pr) = child(bg, "bgPr") {
        // §20.1.8.14 — an image background lives in `bgPr > blipFill`. Try it
        // first so the embedded blip is resolved; fall back to the generic
        // solid/gradient/pattern parser for non-image bgPr fills.
        if let Some(blip_fill) = child(bg_pr, "blipFill") {
            if let Some(fill) = parse_blip_fill(blip_fill, theme, resolve_blip) {
                return Some(fill);
            }
        }
        return parse_fill(bg_pr, theme);
    }
    // bgRef references a theme background style; its child is a color element
    if let Some(bg_ref) = child(bg, "bgRef") {
        return parse_color_node(bg_ref, theme).map(|c| Fill::Solid { color: c });
    }
    None
}

/// Resolve a table-style `<a:fill>` wrapper's colour. Identical to `parse_fill`
/// for the common solid/no-fill cases, except `<a:tint>` uses the literal
/// ECMA-376 §20.1.2.3.34 formula (`TintMode::WordLiteral`) so a band's
/// `accent + tint 20%` renders as the near-white wash PowerPoint draws, rather
/// than the saturated linear-lerp used for SmartArt accents. Gradient/pattern/
/// blip fills (rare in table styles) defer to the generic `parse_fill`.
pub(crate) fn parse_table_style_fill(
    fill_wrapper: roxmltree::Node<'_, '_>,
    theme: &HashMap<String, String>,
) -> Option<Fill> {
    use ooxml_common::color::TintMode::WordLiteral;
    for c in fill_wrapper.children().filter(|n| n.is_element()) {
        match c.tag_name().name() {
            "noFill" => return Some(Fill::None),
            "solidFill" => {
                return parse_color_node_tint(c, theme, WordLiteral)
                    .map(|color| Fill::Solid { color });
            }
            _ => {}
        }
    }
    parse_fill(fill_wrapper, theme)
}

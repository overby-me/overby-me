use crate::types::*;
use crate::worksheet_reference::{
    resolve_worksheet_reference, ReferencedCellValue, WorksheetReferenceSession,
};
use crate::{find_rel_target_by_type, parse_rels_map, resolve_fill_color, resolve_zip_path};
use ooxml_common::depth::parse_guarded;
use ooxml_common::ns::{is_c_ns, is_r_ns, is_xdr_ns};
use ooxml_common::zip::read_zip_string;

pub(crate) struct ChartReferenceContext<'a, 'input, 'session> {
    pub(crate) sheet_xml: &'a str,
    pub(crate) sheet_name: &'a str,
    pub(crate) sheets: &'a [SheetMeta],
    pub(crate) workbook_rels: &'a roxmltree::Document<'input>,
    pub(crate) shared_strings: &'a [SharedString],
    pub(crate) session: &'session mut WorksheetReferenceSession,
}

struct XlsxChartReferenceResolver<'archive, 'data, 'input, 'session> {
    archive: &'archive mut crate::XlsxZip,
    sheet_xml: &'data str,
    sheet_name: &'data str,
    sheets: &'data [SheetMeta],
    workbook_rels: &'data roxmltree::Document<'input>,
    shared_strings: &'data [SharedString],
    session: &'session mut WorksheetReferenceSession,
}

impl ooxml_common::chart::ChartReferenceResolver for XlsxChartReferenceResolver<'_, '_, '_, '_> {
    fn resolve_strings(&mut self, formula: &str) -> Option<Vec<String>> {
        resolve_worksheet_reference(
            self.archive,
            formula,
            self.sheet_xml,
            self.sheet_name,
            self.sheets,
            self.workbook_rels,
            self.shared_strings,
            self.session,
        )
        .map(|values| {
            values
                .into_iter()
                .map(|value| match value {
                    ReferencedCellValue::Text(text) => text,
                    ReferencedCellValue::Number(number) => number.to_string(),
                    ReferencedCellValue::Empty => String::new(),
                })
                .collect()
        })
    }

    fn resolve_numbers(&mut self, formula: &str) -> Option<Vec<Option<f64>>> {
        resolve_worksheet_reference(
            self.archive,
            formula,
            self.sheet_xml,
            self.sheet_name,
            self.sheets,
            self.workbook_rels,
            self.shared_strings,
            self.session,
        )
        .map(|values| {
            values
                .into_iter()
                .map(|value| match value {
                    ReferencedCellValue::Number(number) => Some(number),
                    _ => None,
                })
                .collect()
        })
    }
}

/// Read the chartStyle part (`styleN.xml`) associated with a chart part at
/// `chart_path` (e.g. `xl/charts/chart1.xml`), following that part's own
/// relationships (`xl/charts/_rels/chart1.xml.rels`) to the
/// `.../2011/relationships/chartStyle` target. Returns `None` when the chart
/// has no chartStyle relationship or the part cannot be read (the chartEx
/// title then falls back to its inline size, or the renderer's default).
fn load_chart_style_xml(archive: &mut crate::XlsxZip, chart_path: &str) -> Option<String> {
    let (dir, file) = chart_path.rsplit_once('/')?;
    let rels_path = format!("{}/_rels/{}.rels", dir, file);
    let rels_xml = read_zip_string(archive, &rels_path).ok()?;
    let target =
        find_rel_target_by_type(&rels_xml, ooxml_common::chart::CHART_STYLE_REL_TYPE_SUFFIX)?;
    let style_path = resolve_zip_path(dir, &target);
    read_zip_string(archive, &style_path).ok()
}

/// Given a sheet path (e.g. "worksheets/sheet1.xml"), locate and parse
/// its drawing(s) for chart anchors (`<xdr:graphicFrame>` elements).
pub(crate) fn load_sheet_charts(
    archive: &mut crate::XlsxZip,
    sheet_path: &str,
    mut reference_context: Option<ChartReferenceContext<'_, '_, '_>>,
    theme_colors: &[String],
    theme_fonts: (Option<&str>, Option<&str>),
) -> Vec<ChartAnchor> {
    let Some((sheet_dir, sheet_file)) = sheet_path.rsplit_once('/') else {
        return Vec::new();
    };
    let sheet_rels_path = format!("xl/{}/_rels/{}.rels", sheet_dir, sheet_file);
    let Ok(sheet_rels_xml) = read_zip_string(archive, &sheet_rels_path) else {
        return Vec::new();
    };
    let Ok(rels_doc) = parse_guarded(&sheet_rels_xml) else {
        return Vec::new();
    };

    // Collect all drawing relationship targets
    let mut drawing_targets: Vec<String> = Vec::new();
    for rel in rels_doc
        .root_element()
        .children()
        .filter(|n| n.is_element())
    {
        if rel.attribute("Type").unwrap_or("").ends_with("/drawing") {
            if let Some(t) = rel.attribute("Target") {
                drawing_targets.push(t.to_string());
            }
        }
    }
    if drawing_targets.is_empty() {
        return Vec::new();
    }

    let mut all_charts: Vec<ChartAnchor> = Vec::new();

    for target in drawing_targets {
        // Resolve drawing path relative to the sheet directory
        let drawing_path = resolve_zip_path(&format!("xl/{}", sheet_dir), &target);
        let Ok(drawing_xml) = read_zip_string(archive, &drawing_path) else {
            continue;
        };
        let Ok(draw_doc) = parse_guarded(&drawing_xml) else {
            continue;
        };

        // Load drawing rels (to resolve chart rIds)
        let Some((drawing_dir, drawing_file)) = drawing_path.rsplit_once('/') else {
            continue;
        };
        let drawing_rels_path = format!("{}/_rels/{}.rels", drawing_dir, drawing_file);
        let drawing_rels = read_zip_string(archive, &drawing_rels_path)
            .ok()
            .map(|xml| parse_rels_map(&xml))
            .unwrap_or_default();

        // Iterate over chart anchors. Charts may be saved either as a
        // `<xdr:twoCellAnchor>` (from + to cells — Excel's default) or a
        // `<xdr:oneCellAnchor>` (from cell + a saved `<xdr:ext cx cy>` EMU
        // size, ECMA-376 §20.5.2.24). openpyxl and some other writers emit
        // oneCellAnchor; both must produce a chart.
        for anchor in draw_doc
            .root_element()
            .children()
            .filter(|n| n.is_element())
        {
            let anchor_tag = anchor.tag_name().name();
            let is_one_cell = anchor_tag == "oneCellAnchor";
            if (anchor_tag != "twoCellAnchor" && !is_one_cell)
                || !is_xdr_ns(anchor.tag_name().namespace())
            {
                continue;
            }

            let (mut from_col, mut from_col_off, mut from_row, mut from_row_off) =
                (0u32, 0i64, 0u32, 0i64);
            let (mut to_col, mut to_col_off, mut to_row, mut to_row_off) = (0u32, 0i64, 0u32, 0i64);
            // oneCellAnchor size: `<xdr:ext cx cy>` in EMU.
            let (mut ext_cx, mut ext_cy) = (0i64, 0i64);
            let mut chart_rid: Option<String> = None;
            // Modern chartEx (Microsoft `cx:` namespace, 2014) parts are wired the
            // same way as a legacy chart — `<a:graphicData>` still carries a
            // `<c:chart r:id>` child (the transitional `c:` local name, not `cx:`)
            // — but the `graphicData@uri` is the chartex extension URI instead of
            // the DrawingML chart URI. Track it alongside `chart_rid` so the part
            // is dispatched to `parse_chartex_part`, matching pptx's
            // `uri.contains("chartex")` detection in shape.rs.
            let mut is_chartex = false;

            for child in anchor.children() {
                if !child.is_element() {
                    continue;
                }
                match child.tag_name().name() {
                    "from" | "to" => {
                        let is_from = child.tag_name().name() == "from";
                        let mut col: u32 = 0;
                        let mut col_off: i64 = 0;
                        let mut row: u32 = 0;
                        let mut row_off: i64 = 0;
                        for c in child.children() {
                            match (c.tag_name().name(), c.text()) {
                                ("col", Some(t)) => col = t.trim().parse().unwrap_or(0),
                                ("colOff", Some(t)) => col_off = t.trim().parse().unwrap_or(0),
                                ("row", Some(t)) => row = t.trim().parse().unwrap_or(0),
                                ("rowOff", Some(t)) => row_off = t.trim().parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                        if is_from {
                            from_col = col;
                            from_col_off = col_off;
                            from_row = row;
                            from_row_off = row_off;
                        } else {
                            to_col = col;
                            to_col_off = col_off;
                            to_row = row;
                            to_row_off = row_off;
                        }
                    }
                    "ext" => {
                        // oneCellAnchor's `<xdr:ext cx cy>` size in EMU.
                        ext_cx = child
                            .attribute("cx")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        ext_cy = child
                            .attribute("cy")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                    }
                    "graphicFrame" => {
                        // ECMA-376 §20.1.2.2.8 CT_NonVisualDrawingProps@hidden:
                        // a hidden chart's own `<xdr:nvGraphicFramePr>/<xdr:cNvPr
                        // hidden="1">` marks it not rendered. This is the chart's
                        // own graphicFrame walk (independent of the shared shape
                        // walker in drawing.rs::collect_shapes), so it needs its
                        // own check.
                        if crate::drawing::xdr_node_hidden(&child) {
                            continue;
                        }
                        // `<a:graphicData uri>` distinguishes a chartEx part
                        // (`http://schemas.microsoft.com/office/drawing/2014/chartex`)
                        // from a legacy DrawingML chart
                        // (`http://schemas.openxmlformats.org/drawingml/2006/chart`).
                        // Both wire the part through a `<c:chart r:id>` child, so
                        // the rId resolution below is unchanged either way.
                        if let Some(gd) = child
                            .descendants()
                            .find(|n| n.tag_name().name() == "graphicData")
                        {
                            if let Some(uri) = gd.attribute("uri") {
                                is_chartex = uri.contains("chartex") || uri.contains("chartEx");
                            }
                        }
                        // Look for a:graphic/a:graphicData/c:chart[@r:id]
                        for gf_child in child.descendants() {
                            if gf_child.tag_name().name() == "chart"
                                && is_c_ns(gf_child.tag_name().namespace())
                            {
                                if let Some(rid) = gf_child
                                    .attributes()
                                    .find(|a| a.name() == "id" && is_r_ns(a.namespace()))
                                    .map(|a| a.value().to_string())
                                {
                                    chart_rid = Some(rid);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            // For a oneCellAnchor the `<to>` corner is absent; the chart's
            // size is the saved EMU extent (ECMA-376 §20.5.2.24). Encode it as
            // a `to` corner pinned to the `from` cell plus the extent offset so
            // the renderer's from/to → pixel math yields exactly `ext` px.
            if is_one_cell {
                to_col = from_col;
                to_row = from_row;
                to_col_off = from_col_off + ext_cx;
                to_row_off = from_row_off + ext_cy;
            }

            let Some(rid) = chart_rid else {
                continue;
            };
            let Some(chart_target) = drawing_rels.get(&rid) else {
                continue;
            };
            let chart_path = resolve_zip_path(drawing_dir, chart_target);
            let Ok(chart_xml) = read_zip_string(archive, &chart_path) else {
                continue;
            };
            // A chartEx part reads its title font size from the associated
            // chartStyle sidecar (`styleN.xml`), reached via the chart part's
            // OWN rels (`xl/charts/_rels/chartN.xml.rels`,
            // `.../2011/relationships/chartStyle`). Read it best-effort now
            // (before the chart doc is parsed, since both borrow `archive`);
            // legacy `<c:>` charts ignore it (their title size is inline).
            let style_xml = load_chart_style_xml(archive, &chart_path);
            // Parse the chart directly through the shared `parse_chart_part`
            // (the single superset parser for pptx + xlsx). The xlsx theme
            // palette + major/minor Latin faces ride on the `XlsxColorResolver`,
            // so no `ChartData` intermediate / `From` adapter is needed.
            //
            // A chartEx part (`is_chartex`) has a `<cx:chartSpace>` root instead
            // of `<c:chartSpace>` and uses the shared
            // `parse_chartex_part` structure walk (waterfall / boxWhisker /
            // treemap / sunburst / … — ECMA-376 does not cover these; they are
            // the Microsoft 2014 chartex extension). Same `ColorResolver`.
            let Ok(chart_doc) = parse_guarded(&chart_xml) else {
                continue;
            };
            let resolver = XlsxColorResolver {
                theme_colors,
                theme_major_font_latin: theme_fonts.0,
                theme_minor_font_latin: theme_fonts.1,
            };
            let chart_opt = if is_chartex {
                ooxml_common::chart::parse_chartex_part(
                    chart_doc.root_element(),
                    &resolver,
                    style_xml.as_deref(),
                )
            } else if let Some(context) = reference_context.as_mut() {
                let mut references = XlsxChartReferenceResolver {
                    archive,
                    sheet_xml: context.sheet_xml,
                    sheet_name: context.sheet_name,
                    sheets: context.sheets,
                    workbook_rels: context.workbook_rels,
                    shared_strings: context.shared_strings,
                    session: context.session,
                };
                ooxml_common::chart::parse_chart_part_with_references(
                    chart_doc.root_element(),
                    &resolver,
                    &mut references,
                )
            } else {
                ooxml_common::chart::parse_chart_part(chart_doc.root_element(), &resolver)
            };
            let Some(chart) = chart_opt else {
                continue;
            };

            all_charts.push(ChartAnchor {
                from_col,
                from_col_off,
                from_row,
                from_row_off,
                to_col,
                to_col_off,
                to_row,
                to_row_off,
                chart,
            });
        }
    }
    all_charts
}

/// xlsx `ColorResolver` used by the shared [`ooxml_common::chart::parse_chart_part`].
/// Carries the workbook theme palette (clrScheme document order) plus the
/// theme's major/minor Latin font faces so the shared parser can supply the
/// chart-text font fallbacks without a separate `theme_fonts` parameter.
struct XlsxColorResolver<'a> {
    theme_colors: &'a [String],
    theme_major_font_latin: Option<&'a str>,
    theme_minor_font_latin: Option<&'a str>,
}

impl ooxml_common::chart::ColorResolver for XlsxColorResolver<'_> {
    fn resolve_solid_fill(&self, node: roxmltree::Node<'_, '_>) -> Option<String> {
        resolve_fill_color(&node, self.theme_colors)
    }

    /// Shape fills (series / marker / dPt / errBars) resolve through the FULL
    /// DrawingML color grammar (transforms included), matching the historical
    /// `extract_solid_fill_in_drawingml` path so a scheme-color marker with a
    /// `lumMod`/`lumOff` tint renders at the right strength. This is deliberately
    /// heavier than [`Self::resolve_solid_fill`] (which xlsx keeps for callers
    /// that pass an already-selected solidFill node).
    fn resolve_shape_fill(&self, parent: roxmltree::Node<'_, '_>) -> Option<String> {
        extract_solid_fill_in_drawingml(&parent, self.theme_colors)
    }

    /// Default series fill: `theme.accent[(idx % 6) + 1]`. The palette is stored
    /// in clrScheme document order (dk1@0, lt1@1, dk2@2, lt2@3, accent1@4 …
    /// accent6@9), so accent1 is index 4.
    fn resolve_series_accent(&self, idx: usize) -> Option<String> {
        self.theme_colors
            .get(4 + (idx % 6))
            .map(|c| c.trim_start_matches('#').to_lowercase())
    }

    fn theme_major_font_latin(&self) -> Option<String> {
        self.theme_major_font_latin.map(|s| s.to_string())
    }

    fn theme_minor_font_latin(&self) -> Option<String> {
        self.theme_minor_font_latin.map(|s| s.to_string())
    }

    /// Excel paints an opaque-white chart area when the file omits
    /// `<c:chartSpace><c:spPr>` entirely (the historical `has_chart_sp_pr=false`
    /// white default).
    fn default_chart_bg(&self) -> Option<String> {
        Some("FFFFFF".to_string())
    }
}
/// Locate the first resolvable `<a:solidFill>` among `parent`'s direct children
/// (children only, not deep descendants — chart spPr is structured shallowly)
/// and resolve its color to hex **without** `#` (uppercase). The chart wire
/// model prepends `#` on the TS side, so this matches every other chart color
/// field.
///
/// Delegates the DrawingML color grammar (`srgbClr`/`sysClr`/`prstClr`/
/// `schemeClr` + `lumMod`/`lumOff`/`tint`/`shade` transforms) to the
/// shared [`ooxml_common::color::parse_color_node`] via the crate-wide
/// [`XlsxSchemeResolver`], so scheme slots resolve through the §20.1.6.2 default
/// clrMap (`tx2`→`dk2`, `bg2`→`lt2`) and luminance transforms apply in HLS space
/// (§20.1.2.3.20/.21). The prior private copy in this module mapped `bg2`/`tx2`
/// to the wrong slots and multiplied `lumMod`/`lumOff` in RGB space.
pub(crate) fn extract_solid_fill_in_drawingml(
    parent: &roxmltree::Node,
    theme_colors: &[String],
) -> Option<String> {
    parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "solidFill")
        .find_map(|fill| {
            ooxml_common::color::parse_color_node(
                fill,
                &crate::drawing::XlsxSchemeResolver { theme_colors },
                ooxml_common::color::TintMode::PowerPointLinear,
            )
        })
}

#[cfg(test)]
mod solid_fill_color_tests {
    use super::*;
    use roxmltree::Document;

    const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

    // Theme in clrScheme document order: dk1@0, lt1@1, dk2@2, lt2@3,
    // accent1@4 … folHlink@11. Distinct hexes so a mis-index is obvious.
    fn theme() -> Vec<String> {
        vec![
            "#111111".into(), // dk1 @0
            "#FEFEFE".into(), // lt1 @1
            "#222222".into(), // dk2 @2
            "#EEEEEE".into(), // lt2 @3
            "#4472C4".into(), // accent1 @4
            "#00AA00".into(), // accent2 @5
            "#0000AA".into(), // accent3 @6
            "#AAAA00".into(), // accent4 @7
            "#00AAAA".into(), // accent5 @8
            "#AA00AA".into(), // accent6 @9
            "#0563C1".into(), // hlink @10
            "#954F72".into(), // folHlink @11
        ]
    }

    fn solid_fill(inner: &str) -> String {
        format!(r#"<a:spPr xmlns:a="{A_NS}"><a:solidFill>{inner}</a:solidFill></a:spPr>"#)
    }

    /// §20.1.6.2 default clrMap: `tx2` → `dk2` (theme slot 2), NOT `lt1`.
    #[test]
    fn scheme_tx2_resolves_to_dk2_slot() {
        let xml = solid_fill(r#"<a:schemeClr val="tx2"/>"#);
        let doc = Document::parse(&xml).unwrap();
        let out = extract_solid_fill_in_drawingml(&doc.root_element(), &theme());
        // tx2 → dk2 → theme[2] = "222222" (uppercase, no `#`).
        assert_eq!(out.as_deref(), Some("222222"));
    }

    /// §20.1.6.2 default clrMap: `bg2` → `lt2` (theme slot 3), NOT `dk1`.
    #[test]
    fn scheme_bg2_resolves_to_lt2_slot() {
        let xml = solid_fill(r#"<a:schemeClr val="bg2"/>"#);
        let doc = Document::parse(&xml).unwrap();
        let out = extract_solid_fill_in_drawingml(&doc.root_element(), &theme());
        // bg2 → lt2 → theme[3] = "EEEEEE".
        assert_eq!(out.as_deref(), Some("EEEEEE"));
    }

    /// tx1 → dk1 and bg1 → lt1 (unchanged, but pinned so a refactor can't drift).
    #[test]
    fn scheme_tx1_bg1_resolve_to_dk1_lt1() {
        let tx1 = solid_fill(r#"<a:schemeClr val="tx1"/>"#);
        let doc = Document::parse(&tx1).unwrap();
        assert_eq!(
            extract_solid_fill_in_drawingml(&doc.root_element(), &theme()).as_deref(),
            Some("111111") // dk1 @0
        );
        let bg1 = solid_fill(r#"<a:schemeClr val="bg1"/>"#);
        let doc = Document::parse(&bg1).unwrap();
        assert_eq!(
            extract_solid_fill_in_drawingml(&doc.root_element(), &theme()).as_deref(),
            Some("FEFEFE") // lt1 @1
        );
    }

    /// `lumMod` is a luminance modulation applied to the HLS `L` channel
    /// (§20.1.2.3.20), NOT a per-RGB-component multiply. For `4472C4` at
    /// `lumMod 50000`, the HLS result is `203864`; the (wrong) RGB-space
    /// multiply would give `223962`.
    #[test]
    fn lummod_applies_in_hls_space_not_rgb() {
        let xml = solid_fill(r#"<a:srgbClr val="4472C4"><a:lumMod val="50000"/></a:srgbClr>"#);
        let doc = Document::parse(&xml).unwrap();
        let out = extract_solid_fill_in_drawingml(&doc.root_element(), &theme());
        assert_eq!(out.as_deref(), Some("203864"));
    }

    /// A plain srgbClr with no transforms passes through (uppercased, no `#`).
    #[test]
    fn plain_srgb_passthrough() {
        let xml = solid_fill(r#"<a:srgbClr val="ff8000"/>"#);
        let doc = Document::parse(&xml).unwrap();
        let out = extract_solid_fill_in_drawingml(&doc.root_element(), &theme());
        assert_eq!(out.as_deref(), Some("FF8000"));
    }

    /// A chart series is a DrawingML shape too: its `<c:spPr>` fill must retain
    /// color transforms such as `lumMod` (§20.1.2.3.20). Excel commonly writes
    /// a grey series as `bg1` (white) with a 65% luminance modulation.
    #[test]
    fn chart_series_scheme_fill_applies_lummod() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
                              xmlns:a="{A_NS}">
                 <c:chart><c:plotArea><c:barChart>
                   <c:barDir val="col"/>
                   <c:grouping val="clustered"/>
                   <c:ser>
                     <c:idx val="0"/><c:order val="0"/>
                     <c:spPr><a:solidFill><a:schemeClr val="bg1">
                       <a:lumMod val="65000"/>
                     </a:schemeClr></a:solidFill></c:spPr>
                     <c:val><c:numLit><c:ptCount val="1"/>
                       <c:pt idx="0"><c:v>1</c:v></c:pt>
                     </c:numLit></c:val>
                   </c:ser>
                 </c:barChart></c:plotArea></c:chart>
               </c:chartSpace>"#
        );
        let doc = Document::parse(&xml).unwrap();
        let mut colors = theme();
        colors[1] = "#FFFFFF".into();
        let resolver = XlsxColorResolver {
            theme_colors: &colors,
            theme_major_font_latin: None,
            theme_minor_font_latin: None,
        };
        let chart = ooxml_common::chart::parse_chart_part(doc.root_element(), &resolver).unwrap();
        assert_eq!(chart.series[0].color.as_deref(), Some("A6A6A6"));
    }
}

/// §20.1.2.2.8 — an `<xdr:cNvPr hidden="1">` graphicFrame is not rendered.
/// `load_sheet_charts` walks `<xdr:graphicFrame>` independently of the shared
/// shape walker in `drawing.rs::collect_shapes`, so it needs its own hidden
/// check — this covers that walk specifically (full sheet → drawing → chart
/// zip round trip, since `load_sheet_charts` reads from the archive).
#[cfg(test)]
mod hidden_tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    const NS: &str = concat!(
        r#"xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" "#,
        r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
        r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#,
    );

    fn theme() -> Vec<String> {
        vec!["#111111".into(); 12]
    }

    fn minimal_chart_xml() -> String {
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <c:chart>
    <c:plotArea>
      <c:layout/>
      <c:barChart>
        <c:barDir val="col"/>
        <c:grouping val="clustered"/>
        <c:ser>
          <c:idx val="0"/><c:order val="0"/>
          <c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Q1</c:v></c:pt></c:strCache></c:strRef></c:cat>
          <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt></c:numCache></c:numRef></c:val>
        </c:ser>
        <c:axId val="1"/><c:axId val="2"/>
      </c:barChart>
      <c:valAx><c:axId val="1"/><c:axPos val="l"/></c:valAx>
      <c:catAx><c:axId val="2"/><c:axPos val="b"/></c:catAx>
    </c:plotArea>
  </c:chart>
</c:chartSpace>"#
            .to_string()
    }

    fn drawing_xml(hidden_attr: &str) -> String {
        format!(
            r#"<xdr:wsDr {NS}><xdr:twoCellAnchor>
              <xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
              <xdr:to><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
              <xdr:graphicFrame>
                <xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"{hidden}/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>
                <xdr:xfrm><a:off x="0" y="0"/><a:ext cx="4000000" cy="3000000"/></xdr:xfrm>
                <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
                  <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rIdChart"/>
                </a:graphicData></a:graphic>
              </xdr:graphicFrame>
              <xdr:clientData/>
            </xdr:twoCellAnchor></xdr:wsDr>"#,
            NS = NS,
            hidden = hidden_attr,
        )
    }

    /// Builds a minimal zip archive wiring `xl/worksheets/sheet1.xml`'s rels →
    /// `xl/drawings/drawing1.xml` → its own rels → `xl/charts/chart1.xml`, the
    /// same part chain `load_sheet_charts` walks in production.
    fn archive_with_chart(hidden_attr: &str) -> crate::XlsxZip {
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
            let o = SimpleFileOptions::default();

            zw.start_file("xl/worksheets/_rels/sheet1.xml.rels", o)
                .unwrap();
            zw.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdDrawing" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#).unwrap();

            zw.start_file("xl/drawings/drawing1.xml", o).unwrap();
            zw.write_all(drawing_xml(hidden_attr).as_bytes()).unwrap();

            zw.start_file("xl/drawings/_rels/drawing1.xml.rels", o)
                .unwrap();
            zw.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#).unwrap();

            zw.start_file("xl/charts/chart1.xml", o).unwrap();
            zw.write_all(minimal_chart_xml().as_bytes()).unwrap();

            zw.finish().unwrap();
        }
        zip::ZipArchive::new(Cursor::new(buf)).unwrap()
    }

    #[test]
    fn hidden_chart_graphicframe_is_not_emitted() {
        for attr in [r#" hidden="1""#, r#" hidden="true""#] {
            let mut archive = archive_with_chart(attr);
            let charts = load_sheet_charts(
                &mut archive,
                "worksheets/sheet1.xml",
                None,
                &theme(),
                (None, None),
            );
            assert!(charts.is_empty(), "hidden chart emitted (attr={attr})");
        }
    }

    #[test]
    fn visible_chart_graphicframe_is_emitted_unchanged() {
        for attr in ["", r#" hidden="0""#, r#" hidden="false""#] {
            let mut archive = archive_with_chart(attr);
            let charts = load_sheet_charts(
                &mut archive,
                "worksheets/sheet1.xml",
                None,
                &theme(),
                (None, None),
            );
            assert_eq!(charts.len(), 1, "visible chart dropped (attr={attr})");
        }
    }
}

#[cfg(test)]
mod worksheet_reference_tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    const DASHBOARD_XML: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#;
    const DATA_XML: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="C1" t="inlineStr"><is><t>المبيعات</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>أحمد</t></is></c><c r="C2"><v>5000</v></c><c r="D2"><v>10</v></c><c r="E2"><v>3</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>سارة</t></is></c><c r="C3"><v>6200</v></c><c r="D3"><v>20</v></c><c r="E3"><v>5</v></c></row><row r="4"><c r="A4" t="inlineStr"><is><t>خالد</t></is></c><c r="C4"><v>7500</v></c><c r="D4"><v>30</v></c><c r="E4"><v>7</v></c></row></sheetData></worksheet>"#;

    fn chart_xml(with_cache: bool) -> String {
        let name_cache = if with_cache {
            r#"<c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Cached</c:v></c:pt></c:strCache>"#
        } else {
            ""
        };
        let category_cache = if with_cache {
            r#"<c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Cached category</c:v></c:pt></c:strCache>"#
        } else {
            ""
        };
        let value_cache = if with_cache {
            r#"<c:numCache><c:ptCount val="1"/><c:pt idx="0"><c:v>99</c:v></c:pt></c:numCache>"#
        } else {
            ""
        };
        format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>'التقرير'!C1</c:f>{name_cache}</c:strRef></c:tx><c:cat><c:strRef><c:f>'التقرير'!$A$2:$A$4</c:f>{category_cache}</c:strRef></c:cat><c:val><c:numRef><c:f>'التقرير'!$C$2:$C$4</c:f>{value_cache}</c:numRef></c:val></c:ser><c:axId val="10"/><c:axId val="100"/></c:barChart><c:catAx><c:axId val="10"/><c:axPos val="b"/></c:catAx><c:valAx><c:axId val="100"/><c:axPos val="l"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#,
        )
    }

    fn cacheless_bubble_chart_xml() -> String {
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:bubbleChart><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>'التقرير'!C1</c:f></c:strRef></c:tx><c:xVal><c:numRef><c:f>'التقرير'!C2:C4</c:f></c:numRef></c:xVal><c:yVal><c:numRef><c:f>'التقرير'!D2:D4</c:f></c:numRef></c:yVal><c:bubbleSize><c:numRef><c:f>'التقرير'!E2:E4</c:f></c:numRef></c:bubbleSize></c:ser></c:bubbleChart></c:plotArea></c:chart></c:chartSpace>"#.into()
    }

    fn archive_with_chart_and_data(chart_xml: &str) -> crate::XlsxZip {
        let mut bytes = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut bytes));
            let options = SimpleFileOptions::default();
            writer
                .start_file("xl/worksheets/_rels/sheet1.xml.rels", options)
                .unwrap();
            writer.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rDrawing" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#).unwrap();
            writer
                .start_file("xl/drawings/drawing1.xml", options)
                .unwrap();
            writer.write_all(br#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:twoCellAnchor><xdr:from><xdr:col>6</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>12</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>15</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="1" name="Chart 1"/></xdr:nvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rChart"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#).unwrap();
            writer
                .start_file("xl/drawings/_rels/drawing1.xml.rels", options)
                .unwrap();
            writer.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#).unwrap();
            writer.start_file("xl/charts/chart1.xml", options).unwrap();
            writer.write_all(chart_xml.as_bytes()).unwrap();
            writer
                .start_file("xl/worksheets/sheet2.xml", options)
                .unwrap();
            writer.write_all(DATA_XML.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        zip::ZipArchive::new(Cursor::new(bytes)).unwrap()
    }

    fn sheets() -> Vec<SheetMeta> {
        vec![
            SheetMeta {
                name: "Dashboard".into(),
                sheet_id: 1,
                r_id: "rDashboard".into(),
                tab_color: None,
                visibility: SheetVisibility::Visible,
            },
            SheetMeta {
                name: "التقرير".into(),
                sheet_id: 2,
                r_id: "rData".into(),
                tab_color: None,
                visibility: SheetVisibility::Visible,
            },
        ]
    }

    fn workbook_rels_xml() -> &'static str {
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rDashboard" Target="worksheets/sheet1.xml"/><Relationship Id="rData" Target="worksheets/sheet2.xml"/></Relationships>"#
    }

    fn load_model(xml: &str) -> ooxml_common::chart::ChartModel {
        let mut archive = archive_with_chart_and_data(xml);
        let rels = parse_guarded(workbook_rels_xml()).unwrap();
        let sheet_metas = sheets();
        let theme = vec!["#4472C4".into(); 12];
        let mut session = WorksheetReferenceSession::default();
        let charts = load_sheet_charts(
            &mut archive,
            "worksheets/sheet1.xml",
            Some(ChartReferenceContext {
                sheet_xml: DASHBOARD_XML,
                sheet_name: "Dashboard",
                sheets: &sheet_metas,
                workbook_rels: &rels,
                shared_strings: &[],
                session: &mut session,
            }),
            &theme,
            (None, None),
        );
        assert_eq!(charts.len(), 1);
        charts.into_iter().next().unwrap().chart
    }

    #[test]
    fn cacheless_unicode_chart_resolves_cross_sheet_series() {
        let xml = chart_xml(false);
        let chart = load_model(&xml);

        assert_eq!(chart.series[0].name, "المبيعات");
        assert_eq!(chart.categories, vec!["أحمد", "سارة", "خالد"]);
        assert_eq!(chart.series[0].categories, None);
        assert_eq!(
            chart.series[0].values,
            vec![Some(5000.0), Some(6200.0), Some(7500.0)],
        );
    }

    #[test]
    fn authored_chart_caches_take_precedence_over_live_cells() {
        let xml = chart_xml(true);
        let chart = load_model(&xml);

        assert_eq!(chart.series[0].name, "Cached");
        assert_eq!(chart.categories, vec!["Cached category"]);
        assert_eq!(chart.series[0].values, vec![Some(99.0)]);
    }

    #[test]
    fn cacheless_bubble_chart_resolves_all_series_fields_through_loader() {
        let chart = load_model(&cacheless_bubble_chart_xml());

        assert_eq!(chart.categories, vec!["5000", "6200", "7500"]);
        assert_eq!(chart.series[0].name, "المبيعات");
        assert_eq!(
            chart.series[0].values,
            vec![Some(10.0), Some(20.0), Some(30.0)]
        );
        assert_eq!(
            chart.series[0].bubble_sizes,
            Some(vec![Some(3.0), Some(5.0), Some(7.0)])
        );
    }
}

/// CH14 — chartEx (Microsoft 2014 `cx:` namespace) recognition for xlsx.
/// `xdr:graphicFrame` wires a chartEx part through the SAME `<c:chart r:id>`
/// child a legacy chart uses (the transitional `c:` local name, not `cx:`); only
/// `<a:graphicData@uri>` distinguishes it
/// (`http://schemas.microsoft.com/office/drawing/2014/chartex` vs the
/// DrawingML chart URI). No private xlsx fixture currently contains a chartEx
/// part (`unzip -l ... | grep -i chartex` across every `packages/xlsx/public/
/// private/*.xlsx` sample found none — all still use `<c:chartSpace>`), so this
/// exercises the full zip → drawing → chartEx-part round trip with an inline
/// waterfall fixture, mirroring `parse_chartex_part_waterfall_full_contract`
/// in `ooxml-common`'s chart tests.
#[cfg(test)]
mod chartex_tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    const NS: &str = concat!(
        r#"xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" "#,
        r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
        r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#,
    );
    const CX_NS: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";

    fn theme() -> Vec<String> {
        vec!["#111111".into(); 12]
    }

    /// A minimal waterfall chartEx part: one category dimension, one value
    /// dimension with a negative point, and the `cx:series layoutId` that
    /// `parse_chartex_part` reads as the chart type.
    fn waterfall_chartex_xml() -> String {
        format!(
            r#"<cx:chartSpace xmlns:cx="{CX_NS}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
              <cx:chartData>
                <cx:data id="0">
                  <cx:strDim type="cat">
                    <cx:lvl ptCount="3">
                      <cx:pt idx="0">Start</cx:pt>
                      <cx:pt idx="1">Change</cx:pt>
                      <cx:pt idx="2">End</cx:pt>
                    </cx:lvl>
                  </cx:strDim>
                  <cx:numDim type="val">
                    <cx:lvl ptCount="3">
                      <cx:pt idx="0">50</cx:pt>
                      <cx:pt idx="1">-15</cx:pt>
                      <cx:pt idx="2">35</cx:pt>
                    </cx:lvl>
                  </cx:numDim>
                </cx:data>
              </cx:chartData>
              <cx:chart>
                <cx:plotArea>
                  <cx:plotAreaRegion>
                    <cx:series layoutId="waterfall"/>
                  </cx:plotAreaRegion>
                </cx:plotArea>
              </cx:chart>
            </cx:chartSpace>"#
        )
    }

    /// `<xdr:graphicFrame>` for a chartEx part. Structurally identical to the
    /// legacy `drawing_xml` fixture in `hidden_tests` except for the
    /// `graphicData@uri`, which is exactly the wire-format signal
    /// `load_sheet_charts` now checks.
    fn chartex_drawing_xml() -> String {
        format!(
            r#"<xdr:wsDr {NS}><xdr:twoCellAnchor>
              <xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
              <xdr:to><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
              <xdr:graphicFrame>
                <xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>
                <xdr:xfrm><a:off x="0" y="0"/><a:ext cx="4000000" cy="3000000"/></xdr:xfrm>
                <a:graphic><a:graphicData uri="{CX_NS}">
                  <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rIdChart"/>
                </a:graphicData></a:graphic>
              </xdr:graphicFrame>
              <xdr:clientData/>
            </xdr:twoCellAnchor></xdr:wsDr>"#,
            NS = NS,
            CX_NS = CX_NS,
        )
    }

    /// Builds the same `sheet1.xml.rels` → `drawing1.xml` → `drawing1.xml.rels`
    /// → `charts/chart1.xml` chain as `hidden_tests::archive_with_chart`, but
    /// with a chartEx part at the end instead of a legacy one.
    fn archive_with_chartex_chart() -> crate::XlsxZip {
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
            let o = SimpleFileOptions::default();

            zw.start_file("xl/worksheets/_rels/sheet1.xml.rels", o)
                .unwrap();
            zw.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdDrawing" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#).unwrap();

            zw.start_file("xl/drawings/drawing1.xml", o).unwrap();
            zw.write_all(chartex_drawing_xml().as_bytes()).unwrap();

            zw.start_file("xl/drawings/_rels/drawing1.xml.rels", o)
                .unwrap();
            zw.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#).unwrap();

            zw.start_file("xl/charts/chart1.xml", o).unwrap();
            zw.write_all(waterfall_chartex_xml().as_bytes()).unwrap();

            zw.finish().unwrap();
        }
        zip::ZipArchive::new(Cursor::new(buf)).unwrap()
    }

    #[test]
    fn chartex_graphicframe_parses_through_parse_chartex_part() {
        let mut archive = archive_with_chartex_chart();
        let charts = load_sheet_charts(
            &mut archive,
            "worksheets/sheet1.xml",
            None,
            &theme(),
            (None, None),
        );
        assert_eq!(
            charts.len(),
            1,
            "chartEx graphicFrame did not produce a chart"
        );
        let chart = &charts[0].chart;
        assert_eq!(chart.chart_type, "waterfall");
        assert_eq!(chart.series.len(), 1, "expected exactly one chartEx series");
        assert_eq!(
            chart.categories,
            vec!["Start".to_string(), "Change".to_string(), "End".to_string()]
        );
    }
}

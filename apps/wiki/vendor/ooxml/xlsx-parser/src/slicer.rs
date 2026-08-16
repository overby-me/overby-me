use crate::types::*;
use crate::{parse_color, resolve_zip_path};
use ooxml_common::depth::parse_guarded;
use ooxml_common::ns::{is_x_ns, is_xdr_ns};
use ooxml_common::zip::read_zip_string;
use std::collections::HashMap;

// ─── Slicer loading ─────────────────────────────────────────────────────────
//
// Office 2010+ extension (`sle:slicer` inside `<mc:AlternateContent>`).
// Resolving one slicer graphicFrame into a drawable anchor takes four
// XML files:
//   1. The sheet's drawing (for the anchor rect + graphicFrame name).
//   2. `xl/slicers/slicerN.xml` — slicer definition: graphicFrame name →
//      caption + cache name.
//   3. `xl/slicerCaches/slicerCacheN.xml` — cache definition: cache name →
//      source field + list of (item index, selected?).
//   4. `xl/pivotCache/pivotCacheDefinitionN.xml` — pivot cache: field name →
//      ordered string values.
// Excel also allows slicers bound to Excel Tables (`tableSlicerCache`), but
// the present sample is pivot-only; we only implement the pivot path.

#[derive(Default)]
pub(crate) struct SlicerCacheInfo {
    source_name: String,
    items: Vec<(u32, bool)>, // (index into pivot field, selected)
}

#[derive(Default)]
pub(crate) struct PivotCacheFields {
    by_name: HashMap<String, Vec<String>>, // field name → ordered string items
}

/// Parse every `xl/pivotCache/pivotCacheDefinition*.xml` and merge its
/// cacheFields (indexed by `@name`) into a single map. Sample workbooks
/// typically have one pivotCache but the loop keeps the code general.
pub(crate) fn load_all_pivot_cache_fields(archive: &mut crate::XlsxZip) -> PivotCacheFields {
    let mut out = PivotCacheFields::default();
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with("xl/pivotCache/pivotCacheDefinition") && n.ends_with(".xml"))
        .collect();
    for name in names {
        let Ok(xml) = read_zip_string(archive, &name) else {
            continue;
        };
        let Ok(doc) = parse_guarded(&xml) else {
            continue;
        };
        for field in doc.descendants() {
            if field.tag_name().name() != "cacheField" || !is_x_ns(field.tag_name().namespace()) {
                continue;
            }
            let Some(field_name) = field.attribute("name") else {
                continue;
            };
            let mut items: Vec<String> = Vec::new();
            for shared in field
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "sharedItems")
            {
                for item in shared.children().filter(|n| n.is_element()) {
                    match item.tag_name().name() {
                        "s" => items.push(item.attribute("v").unwrap_or("").to_string()),
                        "n" => items.push(item.attribute("v").unwrap_or("").to_string()),
                        "d" => items.push(item.attribute("v").unwrap_or("").to_string()),
                        "b" => items.push(item.attribute("v").unwrap_or("").to_string()),
                        "m" => items.push(String::new()),
                        _ => {}
                    }
                }
            }
            if !items.is_empty() {
                out.by_name.insert(field_name.to_string(), items);
            }
        }
    }
    out
}

/// Parse every `xl/slicerCaches/slicerCache*.xml` and build a map keyed by
/// the slicerCache's `@name` attribute (e.g. `"スライサー_贈答相手1"`). That
/// name is what `<slicer cache="…"/>` in `xl/slicers/slicerN.xml` references.
pub(crate) fn load_all_slicer_caches(
    archive: &mut crate::XlsxZip,
) -> HashMap<String, SlicerCacheInfo> {
    let mut out: HashMap<String, SlicerCacheInfo> = HashMap::new();
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with("xl/slicerCaches/slicerCache") && n.ends_with(".xml"))
        .collect();
    for path in names {
        let Ok(xml) = read_zip_string(archive, &path) else {
            continue;
        };
        let Ok(doc) = parse_guarded(&xml) else {
            continue;
        };
        let root = doc.root_element();
        let cache_name = root.attribute("name").unwrap_or("").to_string();
        let source_name = root.attribute("sourceName").unwrap_or("").to_string();
        let mut items: Vec<(u32, bool)> = Vec::new();
        for tabular in doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "tabular")
        {
            for i_el in tabular
                .descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "i")
            {
                let x: u32 = i_el
                    .attribute("x")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                // `s` defaults to "1" (selected) when absent — ECMA-376
                // extension schema for slicer caches.
                let selected = i_el.attribute("s").map(|v| v != "0").unwrap_or(true);
                items.push((x, selected));
            }
        }
        if !cache_name.is_empty() {
            out.insert(cache_name, SlicerCacheInfo { source_name, items });
        }
    }
    out
}

/// Slicer definition (`xl/slicers/slicerN.xml`): maps each graphicFrame name
/// on the sheet to its display caption and the slicerCache it's backed by.
#[derive(Default)]
pub(crate) struct SlicerDef {
    caption: String,
    cache: String,
    style: String,
}

pub(crate) fn parse_slicers_xml(xml: &str) -> HashMap<String, SlicerDef> {
    let mut out: HashMap<String, SlicerDef> = HashMap::new();
    let Ok(doc) = parse_guarded(xml) else {
        return out;
    };
    for slicer in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "slicer")
    {
        let name = slicer.attribute("name").unwrap_or("").to_string();
        let caption = slicer.attribute("caption").unwrap_or("").to_string();
        let cache = slicer.attribute("cache").unwrap_or("").to_string();
        let style = slicer.attribute("style").unwrap_or("").to_string();
        if !name.is_empty() {
            out.insert(
                name,
                SlicerDef {
                    caption,
                    cache,
                    style,
                },
            );
        }
    }
    out
}

pub(crate) fn load_sheet_slicers(
    archive: &mut crate::XlsxZip,
    sheet_path: &str, // e.g. "worksheets/sheet1.xml"
    theme_colors: &[String],
) -> Vec<SlicerAnchor> {
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

    // 1. Collect slicer-definition and drawing targets from the sheet rels.
    let mut drawing_targets: Vec<String> = Vec::new();
    let mut slicer_targets: Vec<String> = Vec::new();
    for rel in rels_doc
        .root_element()
        .children()
        .filter(|n| n.is_element())
    {
        let rel_type = rel.attribute("Type").unwrap_or("");
        let Some(target) = rel.attribute("Target") else {
            continue;
        };
        if rel_type.ends_with("/drawing") {
            drawing_targets.push(target.to_string());
        } else if rel_type.ends_with("/slicer") {
            slicer_targets.push(target.to_string());
        }
    }
    if drawing_targets.is_empty() || slicer_targets.is_empty() {
        return Vec::new();
    }

    // 2. Parse all slicer definitions referenced by this sheet, keyed by
    //    graphicFrame name.
    let mut slicer_defs: HashMap<String, SlicerDef> = HashMap::new();
    for target in &slicer_targets {
        let slicer_path = resolve_zip_path(&format!("xl/{}", sheet_dir), target);
        let Ok(xml) = read_zip_string(archive, &slicer_path) else {
            continue;
        };
        for (k, v) in parse_slicers_xml(&xml) {
            slicer_defs.insert(k, v);
        }
    }
    if slicer_defs.is_empty() {
        return Vec::new();
    }

    // 3. Resolve caches (and their backing pivot fields) once.
    let slicer_caches = load_all_slicer_caches(archive);
    let pivot_fields = load_all_pivot_cache_fields(archive);
    let slicer_styles = load_slicer_styles(archive, theme_colors);

    // 4. Walk each drawing and pick up slicer graphicFrames.
    let mut out: Vec<SlicerAnchor> = Vec::new();
    for target in drawing_targets {
        let drawing_path = resolve_zip_path(&format!("xl/{}", sheet_dir), &target);
        let Ok(drawing_xml) = read_zip_string(archive, &drawing_path) else {
            continue;
        };
        out.extend(parse_slicer_anchors(
            &drawing_xml,
            &slicer_defs,
            &slicer_caches,
            &pivot_fields,
            &slicer_styles,
        ));
    }
    out
}

pub(crate) fn parse_slicer_anchors(
    drawing_xml: &str,
    slicer_defs: &HashMap<String, SlicerDef>,
    slicer_caches: &HashMap<String, SlicerCacheInfo>,
    pivot_fields: &PivotCacheFields,
    slicer_styles: &HashMap<String, SlicerStyle>,
) -> Vec<SlicerAnchor> {
    let Ok(doc) = parse_guarded(drawing_xml) else {
        return Vec::new();
    };
    let mc_ns = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    let slicer_uri = "http://schemas.microsoft.com/office/drawing/2010/slicer";
    let mut out: Vec<SlicerAnchor> = Vec::new();

    for anchor in doc.descendants() {
        if anchor.tag_name().name() != "twoCellAnchor" || !is_xdr_ns(anchor.tag_name().namespace())
        {
            continue;
        }

        // Anchor rect.
        let mut from = (0u32, 0i64, 0u32, 0i64);
        let mut to = (0u32, 0i64, 0u32, 0i64);
        for child in anchor.children().filter(|n| n.is_element()) {
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
                        from = (col, col_off, row, row_off);
                    } else {
                        to = (col, col_off, row, row_off);
                    }
                }
                _ => {}
            }
        }

        // Slicers live inside `<mc:AlternateContent><mc:Choice>` — descend
        // until we find a `<xdr:graphicFrame>` whose graphicData uri is the
        // 2010 slicer namespace, then harvest the graphicFrame's cNvPr name.
        let Some(frame_name) = anchor
            .descendants()
            .filter(|n| {
                n.is_element()
                    && n.tag_name().name() == "Choice"
                    && n.tag_name().namespace() == Some(mc_ns)
            })
            .flat_map(|choice| choice.descendants())
            .find_map(|n| {
                if n.is_element()
                    && n.tag_name().name() == "graphicData"
                    && n.attribute("uri") == Some(slicer_uri)
                {
                    // graphicData → ancestor graphicFrame → nvGraphicFramePr → cNvPr
                    let mut p = n.parent();
                    while let Some(pp) = p {
                        if pp.tag_name().name() == "graphicFrame" {
                            break;
                        }
                        p = pp.parent();
                    }
                    let frame = p?;
                    // ECMA-376 §20.1.2.2.8 CT_NonVisualDrawingProps@hidden: a
                    // hidden slicer graphicFrame's own `<xdr:nvGraphicFramePr>/
                    // <xdr:cNvPr hidden="1">` marks it not rendered. This walk
                    // is independent of the shared shape walker in
                    // drawing.rs::collect_shapes, so it needs its own check.
                    if crate::drawing::xdr_node_hidden(&frame) {
                        return None;
                    }
                    let cnvpr = frame
                        .descendants()
                        .find(|d| d.is_element() && d.tag_name().name() == "cNvPr")?;
                    cnvpr.attribute("name").map(|s| s.to_string())
                } else {
                    None
                }
            })
        else {
            continue;
        };

        let Some(slicer_def) = slicer_defs.get(&frame_name) else {
            continue;
        };

        // Resolve items via cache → pivot field; fall back to an empty list
        // if any link is broken (still renders the header and box).
        let items: Vec<SlicerItem> = slicer_caches
            .get(&slicer_def.cache)
            .map(|cache| {
                let field_items = pivot_fields.by_name.get(&cache.source_name);
                cache
                    .items
                    .iter()
                    .map(|(x, selected)| {
                        let name = field_items
                            .and_then(|list| list.get(*x as usize))
                            .cloned()
                            .unwrap_or_default();
                        SlicerItem {
                            name,
                            selected: *selected,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let caption = if !slicer_def.caption.is_empty() {
            slicer_def.caption.clone()
        } else {
            frame_name.clone()
        };

        out.push(SlicerAnchor {
            from_col: from.0,
            from_col_off: from.1,
            from_row: from.2,
            from_row_off: from.3,
            to_col: to.0,
            to_col_off: to.1,
            to_row: to.2,
            to_row_off: to.3,
            caption,
            items,
            style: slicer_styles.get(&slicer_def.style).cloned(),
        });
    }
    out
}

fn load_slicer_styles(
    archive: &mut crate::XlsxZip,
    theme_colors: &[String],
) -> HashMap<String, SlicerStyle> {
    let Ok(xml) = read_zip_string(archive, "xl/styles.xml") else {
        return HashMap::new();
    };
    parse_slicer_styles_xml(&xml, theme_colors)
}

/// Resolve the custom slicer skin split across SpreadsheetML
/// `<tableStyles>` (whole/header) and the Office 2010 `x14:slicerStyles`
/// extension (item states). Both sets reference different dxf collections.
fn parse_slicer_styles_xml(xml: &str, theme_colors: &[String]) -> HashMap<String, SlicerStyle> {
    let Ok(doc) = parse_guarded(xml) else {
        return HashMap::new();
    };
    let root = doc.root_element();
    let standard_dxfs: Vec<_> = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "dxfs")
        .map(|n| {
            n.children()
                .filter(|c| c.is_element() && c.tag_name().name() == "dxf")
                .collect()
        })
        .unwrap_or_default();
    let extension_dxfs: Vec<_> = doc
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "dxfs"
                && n.tag_name()
                    .namespace()
                    .is_some_and(|ns| ns.contains("/spreadsheetml/2009/9/"))
        })
        .map(|n| {
            n.children()
                .filter(|c| c.is_element() && c.tag_name().name() == "dxf")
                .collect()
        })
        .unwrap_or_default();

    let mut out: HashMap<String, SlicerStyle> = HashMap::new();
    for table_style in root.descendants().filter(|n| {
        n.is_element() && n.tag_name().name() == "tableStyle" && is_x_ns(n.tag_name().namespace())
    }) {
        let Some(name) = table_style.attribute("name") else {
            continue;
        };
        for element in table_style
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "tableStyleElement")
        {
            let Some(dxf_id) = element
                .attribute("dxfId")
                .and_then(|value| value.parse::<usize>().ok())
            else {
                continue;
            };
            let Some(dxf) = standard_dxfs.get(dxf_id) else {
                continue;
            };
            let style = parse_slicer_element_style(*dxf, theme_colors);
            let entry = out.entry(name.to_string()).or_default();
            match element.attribute("type") {
                Some("wholeTable") => entry.whole = Some(style),
                Some("headerRow") => entry.header = Some(style),
                _ => {}
            }
        }
    }

    for slicer_style in doc.descendants().filter(|n| {
        n.is_element()
            && n.tag_name().name() == "slicerStyle"
            && n.tag_name()
                .namespace()
                .is_some_and(|ns| ns.contains("/spreadsheetml/2009/9/"))
    }) {
        let Some(name) = slicer_style.attribute("name") else {
            continue;
        };
        for element in slicer_style
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "slicerStyleElement")
        {
            let Some(dxf_id) = element
                .attribute("dxfId")
                .and_then(|value| value.parse::<usize>().ok())
            else {
                continue;
            };
            let Some(dxf) = extension_dxfs.get(dxf_id) else {
                continue;
            };
            let style = parse_slicer_element_style(*dxf, theme_colors);
            let entry = out.entry(name.to_string()).or_default();
            match element.attribute("type") {
                Some("selectedItemWithData") => entry.selected_item_with_data = Some(style),
                Some("unselectedItemWithData") => entry.unselected_item_with_data = Some(style),
                _ => {}
            }
        }
    }
    out
}

fn parse_slicer_element_style(
    dxf: roxmltree::Node<'_, '_>,
    theme_colors: &[String],
) -> SlicerElementStyle {
    let mut style = SlicerElementStyle::default();
    for child in dxf.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "font" => {
                for property in child.children().filter(|n| n.is_element()) {
                    match property.tag_name().name() {
                        "color" => style.font_color = parse_color(&property, theme_colors),
                        "sz" => {
                            style.font_size = property
                                .attribute("val")
                                .and_then(|value| value.parse().ok())
                        }
                        "b" => style.font_bold = Some(crate::styles::parse_st_on_off(&property)),
                        "name" => {
                            style.font_family = property.attribute("val").map(ToString::to_string)
                        }
                        _ => {}
                    }
                }
            }
            "fill" => {
                if let Some(pattern) = child
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == "patternFill")
                {
                    if pattern.attribute("patternType") == Some("solid") {
                        style.fill_color = pattern
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "fgColor")
                            .and_then(|color| parse_color(&color, theme_colors));
                    }
                }
            }
            "border" => {
                style.border_color = child
                    .children()
                    .filter(|n| {
                        n.is_element()
                            && matches!(n.tag_name().name(), "left" | "right" | "top" | "bottom")
                            && n.attribute("style").is_some()
                    })
                    .find_map(|edge| {
                        edge.children()
                            .find(|n| n.is_element() && n.tag_name().name() == "color")
                            .and_then(|color| parse_color(&color, theme_colors))
                    });
            }
            _ => {}
        }
    }
    style
}

/// §20.1.2.2.8 — an `<xdr:cNvPr hidden="1">` slicer graphicFrame is not
/// rendered. `parse_slicer_anchors` resolves its graphicFrame via a manual
/// `mc:AlternateContent` descent independent of the shared shape walker in
/// `drawing.rs::collect_shapes`, so it needs its own hidden check.
#[cfg(test)]
mod hidden_tests {
    use super::*;

    const NS: &str = concat!(
        r#"xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" "#,
        r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
        r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" "#,
        r#"xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main""#,
    );

    fn drawing_xml(hidden_attr: &str) -> String {
        format!(
            r#"<xdr:wsDr {NS}><xdr:twoCellAnchor>
              <xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
              <xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>10</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
              <mc:AlternateContent>
                <mc:Choice Requires="x14">
                  <xdr:graphicFrame>
                    <xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Slicer_Region"{hidden}/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>
                    <xdr:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="2743200"/></xdr:xfrm>
                    <a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/drawing/2010/slicer">
                      <x14:slicer name="Region"/>
                    </a:graphicData></a:graphic>
                  </xdr:graphicFrame>
                </mc:Choice>
              </mc:AlternateContent>
              <xdr:clientData/>
            </xdr:twoCellAnchor></xdr:wsDr>"#,
            NS = NS,
            hidden = hidden_attr,
        )
    }

    fn slicer_defs() -> HashMap<String, SlicerDef> {
        // Keyed by the graphicFrame's own cNvPr@name (the "Slicer_Region" in
        // the fixture below), matching how parse_slicer_anchors resolves it.
        let mut m = HashMap::new();
        m.insert(
            "Slicer_Region".to_string(),
            SlicerDef {
                caption: "Region".to_string(),
                cache: "Slicer_Region".to_string(),
                style: String::new(),
            },
        );
        m
    }

    #[test]
    fn hidden_slicer_graphicframe_is_not_emitted() {
        for attr in [r#" hidden="1""#, r#" hidden="true""#] {
            let out = parse_slicer_anchors(
                &drawing_xml(attr),
                &slicer_defs(),
                &HashMap::new(),
                &PivotCacheFields::default(),
                &HashMap::new(),
            );
            assert!(out.is_empty(), "hidden slicer emitted (attr={attr})");
        }
    }

    #[test]
    fn visible_slicer_graphicframe_is_emitted_unchanged() {
        for attr in ["", r#" hidden="0""#, r#" hidden="false""#] {
            let out = parse_slicer_anchors(
                &drawing_xml(attr),
                &slicer_defs(),
                &HashMap::new(),
                &PivotCacheFields::default(),
                &HashMap::new(),
            );
            assert_eq!(out.len(), 1, "visible slicer dropped (attr={attr})");
        }
    }

    #[test]
    fn custom_slicer_style_resolves_header_and_item_dxfs() {
        let xml = r#"
          <styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                      xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main">
            <dxfs count="1"><dxf>
              <font><b val="0"/><sz val="12"/><color theme="4"/><name val="Meiryo UI"/></font>
            </dxf></dxfs>
            <tableStyles count="1">
              <tableStyle name="CustomSlicer" table="0">
                <tableStyleElement type="headerRow" dxfId="0"/>
              </tableStyle>
            </tableStyles>
            <extLst><ext uri="custom">
              <x14:dxfs count="1"><x14:dxf>
                <x14:font><x14:color theme="4"/></x14:font>
                <x14:fill><x14:patternFill patternType="solid"><x14:fgColor auto="1"/></x14:patternFill></x14:fill>
                <x14:border>
                  <x14:left style="thin"><x14:color theme="4"/></x14:left>
                  <x14:right style="thin"><x14:color theme="4"/></x14:right>
                  <x14:top style="thin"><x14:color theme="4"/></x14:top>
                  <x14:bottom style="thin"><x14:color theme="4"/></x14:bottom>
                </x14:border>
              </x14:dxf></x14:dxfs>
              <x14:slicerStyles><x14:slicerStyle name="CustomSlicer">
                <x14:slicerStyleElements>
                  <x14:slicerStyleElement type="selectedItemWithData" dxfId="0"/>
                </x14:slicerStyleElements>
              </x14:slicerStyle></x14:slicerStyles>
            </ext></extLst>
          </styleSheet>"#;
        let theme = vec![
            "#000000".into(),
            "#FFFFFF".into(),
            "#222222".into(),
            "#EEEEEE".into(),
            "#5C7D21".into(),
        ];
        let styles = parse_slicer_styles_xml(xml, &theme);
        let style = styles.get("CustomSlicer").expect("custom slicer style");
        let header = style.header.as_ref().expect("header dxf");
        assert_eq!(header.font_color.as_deref(), Some("#5C7D21"));
        assert_eq!(header.font_size, Some(12.0));
        assert_eq!(header.font_bold, Some(false));
        assert_eq!(header.font_family.as_deref(), Some("Meiryo UI"));
        let selected = style
            .selected_item_with_data
            .as_ref()
            .expect("selected item dxf");
        assert_eq!(selected.font_color.as_deref(), Some("#5C7D21"));
        assert_eq!(selected.border_color.as_deref(), Some("#5C7D21"));
        assert_eq!(
            selected.fill_color, None,
            "auto fill retains white fallback"
        );
    }
}

// ─── Chart loading ──────────────────────────────────────────────────────────

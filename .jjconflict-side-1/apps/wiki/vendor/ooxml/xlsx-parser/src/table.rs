use crate::types::*;
use crate::{parse_cell_ref, resolve_zip_path};
use ooxml_common::depth::parse_guarded;
use ooxml_common::ns::is_x_ns;
use ooxml_common::zip::read_zip_string;

/// Parse `xl/tables/tableN.xml` files referenced from the sheet rels and
/// collect them for the renderer. Each table carries a ref range, style name
/// (e.g. "TableStyleLight18"), and the banded-rows / banded-cols flags from
/// `<tableStyleInfo>` (ECMA-376 §18.5).
/// Resolve a built-in table style's accent color from the theme.
///
/// Built-in style names follow the pattern `TableStyle{Light|Medium|Dark}{N}`
/// (ECMA-376 §18.5.1.4). Excel's UI lays the 21/28/11 built-ins out in a grid
/// of rows × 7 columns: column 0 is a "none" style (no accent), columns 1–6
/// map to accent1–accent6. So the accent index is `(N - 1) mod 7` where 0
/// means "no accent" and 1..=6 map to the theme's accent slots.
///
/// `theme_colors` is in OOXML natural order — accent1 lives at index 4, so
/// accent_n is at `theme_colors[3 + n]`. Falls back to a neutral gray when
/// the style name is unrecognised or the theme is missing accents.
/// dxf indices for the ECMA-376 §18.8.83 `<tableStyleElement>` roles we
/// render. The presence of an entry in the file's `<tableStyles>` block marks
/// a *custom* style: built-in styles (`TableStyleLight18`, etc.) have no entry
/// here and fall through to accent-based rendering, whereas custom styles
/// (`"予算"`, `"交通費"`) reference dxfs from `<dxfs>` and must be drawn from
/// those dxfs alone — a custom style contributes ONLY what its declared
/// elements define (§18.5.1.2 tableStyleInfo). If none of its elements carry a
/// border dxf, Excel draws no table-level border at all.
///
/// `ST_TableStyleType` (§18.18.93) defines many element types. We cover the
/// region-level ones that affect rendering here: wholeTable, headerRow,
/// totalRow, firstColumn, lastColumn, and the two horizontal banding stripes
/// (firstRowStripe / secondRowStripe). Vertical banding stripes and the corner
/// cell types (first/last header/total cell) are out of scope for now.
#[derive(Debug, Clone, Default)]
pub(crate) struct TableStyleElements {
    whole_table: Option<u32>,
    header_row: Option<u32>,
    total_row: Option<u32>,
    first_column: Option<u32>,
    last_column: Option<u32>,
    /// `firstRowStripe` — band1 horizontal stripe dxf (§18.18.93).
    band1_horizontal: Option<u32>,
    /// `secondRowStripe` — band2 horizontal stripe dxf.
    band2_horizontal: Option<u32>,
}

/// Parse `<tableStyles><tableStyle name="…"><tableStyleElement type="…" dxfId="…"/>`
/// into a lookup keyed by table-style name.
pub(crate) fn parse_table_styles_map(
    archive: &mut crate::XlsxZip,
) -> std::collections::HashMap<String, TableStyleElements> {
    use std::collections::HashMap;
    let mut map: HashMap<String, TableStyleElements> = HashMap::new();
    let Ok(xml) = read_zip_string(archive, "xl/styles.xml") else {
        return map;
    };
    let Ok(doc) = parse_guarded(&xml) else {
        return map;
    };
    for n in doc.descendants() {
        if n.tag_name().name() != "tableStyles" || !is_x_ns(n.tag_name().namespace()) {
            continue;
        }
        for ts in n
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "tableStyle")
        {
            let Some(name) = ts.attribute("name") else {
                continue;
            };
            let mut elems = TableStyleElements::default();
            for el in ts
                .children()
                .filter(|c| c.is_element() && c.tag_name().name() == "tableStyleElement")
            {
                let t = el.attribute("type").unwrap_or("");
                let dxf: Option<u32> = el.attribute("dxfId").and_then(|s| s.parse().ok());
                // §18.18.93 ST_TableStyleType. firstRowStripe/secondRowStripe
                // are the horizontal banding stripes (row banding).
                match t {
                    "wholeTable" => elems.whole_table = dxf,
                    "headerRow" => elems.header_row = dxf,
                    "totalRow" => elems.total_row = dxf,
                    "firstColumn" => elems.first_column = dxf,
                    "lastColumn" => elems.last_column = dxf,
                    "firstRowStripe" => elems.band1_horizontal = dxf,
                    "secondRowStripe" => elems.band2_horizontal = dxf,
                    _ => {}
                }
            }
            map.insert(name.to_string(), elems);
        }
    }
    map
}

pub(crate) fn resolve_table_style_accent(style_name: &str, theme_colors: &[String]) -> String {
    let fallback = "#808080".to_string();
    let Some(rest) = style_name.strip_prefix("TableStyle") else {
        return fallback;
    };
    let digits_start = rest.find(|c: char| c.is_ascii_digit());
    let Some(start) = digits_start else {
        return fallback;
    };
    let Ok(n) = rest[start..].parse::<u32>() else {
        return fallback;
    };
    if n == 0 {
        return fallback;
    }
    let slot = ((n - 1) % 7) as usize;
    if slot == 0 {
        return fallback;
    }
    theme_colors.get(3 + slot).cloned().unwrap_or(fallback)
}

pub(crate) fn load_sheet_tables(
    archive: &mut crate::XlsxZip,
    sheet_path: &str,
    theme_colors: &[String],
) -> Vec<TableInfo> {
    let custom_styles = parse_table_styles_map(archive);
    let Some((sheet_dir, sheet_file)) = sheet_path.rsplit_once('/') else {
        return Vec::new();
    };
    let sheet_rels_path = format!("xl/{}/_rels/{}.rels", sheet_dir, sheet_file);
    let Ok(rels_xml) = read_zip_string(archive, &sheet_rels_path) else {
        return Vec::new();
    };
    let Ok(rels_doc) = parse_guarded(&rels_xml) else {
        return Vec::new();
    };

    let mut table_targets: Vec<String> = Vec::new();
    for rel in rels_doc
        .root_element()
        .children()
        .filter(|n| n.is_element())
    {
        if rel.attribute("Type").unwrap_or("").ends_with("/table") {
            if let Some(t) = rel.attribute("Target") {
                table_targets.push(t.to_string());
            }
        }
    }

    let mut tables: Vec<TableInfo> = Vec::new();
    for target in table_targets {
        let table_path = resolve_zip_path(&format!("xl/{}", sheet_dir), &target);
        let Ok(xml) = read_zip_string(archive, &table_path) else {
            continue;
        };
        let Ok(doc) = parse_guarded(&xml) else {
            continue;
        };
        let root = doc.root_element();
        let Some(ref_attr) = root.attribute("ref") else {
            continue;
        };
        let parts: Vec<&str> = ref_attr.split(':').collect();
        let range = if parts.len() == 2 {
            let (left, top) = parse_cell_ref(parts[0]);
            let (right, bottom) = parse_cell_ref(parts[1]);
            CellRange {
                top,
                left,
                bottom,
                right,
            }
        } else {
            let (col, row) = parse_cell_ref(parts[0]);
            CellRange {
                top: row,
                left: col,
                bottom: row,
                right: col,
            }
        };
        let header_row_count: u32 = root
            .attribute("headerRowCount")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let totals_row_count: u32 = root
            .attribute("totalsRowCount")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let style_info = root
            .children()
            .find(|n| n.tag_name().name() == "tableStyleInfo");
        // ECMA-376 §18.5.1.4: when `name` is absent the table has "None" style —
        // no visual table formatting. Default to "" rather than a named style so
        // the renderer can skip table-style overlay for these cells.
        let style_name = style_info
            .and_then(|n| n.attribute("name"))
            .unwrap_or("")
            .to_string();
        let bool_attr = |n: &roxmltree::Node, key: &str| {
            n.attribute(key)
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false)
        };
        let (show_row_stripes, show_column_stripes, show_first_column, show_last_column) =
            match style_info {
                Some(n) => (
                    bool_attr(&n, "showRowStripes"),
                    bool_attr(&n, "showColumnStripes"),
                    bool_attr(&n, "showFirstColumn"),
                    bool_attr(&n, "showLastColumn"),
                ),
                None => (false, false, false, false),
            };
        let accent_color = resolve_table_style_accent(&style_name, theme_colors);
        // §18.5.1.2: a style is *custom* iff it is defined in the file's
        // `<tableStyles>` block. Custom styles render strictly from their
        // declared element dxfs (no accent approximation); built-in style
        // names (absent from the block) keep the accent-based fallback.
        let style_elems = custom_styles.get(&style_name);
        let is_custom = style_elems.is_some();
        let whole_table_dxf = style_elems.and_then(|e| e.whole_table);
        let header_row_dxf = style_elems.and_then(|e| e.header_row);
        let total_row_dxf = style_elems.and_then(|e| e.total_row);
        let first_column_dxf = style_elems.and_then(|e| e.first_column);
        let last_column_dxf = style_elems.and_then(|e| e.last_column);
        let band1_horizontal_dxf = style_elems.and_then(|e| e.band1_horizontal);
        let band2_horizontal_dxf = style_elems.and_then(|e| e.band2_horizontal);
        // ECMA-376 §18.5.1.3: each `<tableColumn>` may carry its own
        // `dataDxfId`, `headerRowDxfId`, `totalsRowDxfId`. We collect them in
        // document order so the renderer can index them via
        // `columns[cellCol - range.left]`.
        let columns: Vec<TableColumnInfo> = root
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "tableColumn")
            .map(|tc| TableColumnInfo {
                data_dxf_id: tc.attribute("dataDxfId").and_then(|s| s.parse().ok()),
                header_row_dxf_id: tc.attribute("headerRowDxfId").and_then(|s| s.parse().ok()),
                totals_row_dxf_id: tc.attribute("totalsRowDxfId").and_then(|s| s.parse().ok()),
            })
            .collect();
        tables.push(TableInfo {
            range,
            style_name,
            header_row_count,
            totals_row_count,
            show_row_stripes,
            show_column_stripes,
            show_first_column,
            show_last_column,
            accent_color,
            is_custom,
            whole_table_dxf,
            header_row_dxf,
            total_row_dxf,
            first_column_dxf,
            last_column_dxf,
            band1_horizontal_dxf,
            band2_horizontal_dxf,
            columns,
        });
    }
    tables
}

use crate::{parse_cell_ref, resolve_implicit_ordinal, resolve_sheet_path};
use crate::{CellRange, SharedString, SheetMeta, XlsxZip};
use ooxml_common::depth::parse_guarded;
use ooxml_common::ns::is_x_ns;
use ooxml_common::zip::read_zip_string;
use std::collections::HashMap;

pub(crate) const MAX_REFERENCE_CELLS: usize = 1_000_000;
pub(crate) const MAX_TOTAL_REFERENCE_CELLS: usize = 1_000_000;
const MAX_TOTAL_REFERENCE_STRING_BYTES: usize = 64 * 1024 * 1024;
const MAX_TOTAL_INDEXED_CELLS: usize = MAX_TOTAL_REFERENCE_CELLS;
const MAX_TOTAL_INDEXED_STRING_BYTES: usize = MAX_TOTAL_REFERENCE_STRING_BYTES;
const MAX_COL: u32 = 16_384;
const MAX_ROW: u32 = 1_048_576;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ReferencedCellValue {
    Empty,
    Text(String),
    Number(f64),
}

impl ReferencedCellValue {
    fn string_bytes(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            _ => 0,
        }
    }
}

struct IndexedWorksheet {
    cells: HashMap<(u32, u32), ReferencedCellValue>,
    string_bytes: usize,
}

/// Per-worksheet-parse state shared by charts and sparklines. Source sheets
/// are parsed once into sparse non-empty-cell maps. Independent cumulative
/// cell and UTF-8 byte budgets bound both those indexes and the dense reference
/// vectors retained by the resulting model.
pub(crate) struct WorksheetReferenceSession {
    sheets: HashMap<String, Option<IndexedWorksheet>>,
    remaining_cells: usize,
    remaining_string_bytes: usize,
    remaining_indexed_cells: usize,
    remaining_indexed_string_bytes: usize,
}

impl Default for WorksheetReferenceSession {
    fn default() -> Self {
        Self {
            sheets: HashMap::new(),
            remaining_cells: MAX_TOTAL_REFERENCE_CELLS,
            remaining_string_bytes: MAX_TOTAL_REFERENCE_STRING_BYTES,
            remaining_indexed_cells: MAX_TOTAL_INDEXED_CELLS,
            remaining_indexed_string_bytes: MAX_TOTAL_INDEXED_STRING_BYTES,
        }
    }
}

impl WorksheetReferenceSession {
    fn reservable_cell_count(&self, range: &CellRange) -> Option<usize> {
        let total = reference_cell_count(range)?;
        if total > MAX_REFERENCE_CELLS || total > self.remaining_cells {
            return None;
        }
        Some(total)
    }

    fn consume_result(&mut self, cell_count: usize, string_bytes: usize) {
        self.remaining_cells -= cell_count;
        self.remaining_string_bytes -= string_bytes;
    }

    fn consume_index(&mut self, worksheet: &IndexedWorksheet) {
        self.remaining_indexed_cells -= worksheet.cells.len();
        self.remaining_indexed_string_bytes -= worksheet.string_bytes;
    }
}

pub(crate) fn split_sheet_ref(formula: &str) -> Option<(Option<String>, String)> {
    let formula = formula.trim();
    if formula.is_empty() {
        return None;
    }

    let (sheet_name, reference) = match formula.rfind('!') {
        Some(bang) => {
            let raw_sheet = formula[..bang].trim();
            let reference = formula[bang + 1..].trim();
            if raw_sheet.is_empty() || reference.is_empty() {
                return None;
            }
            let sheet = if raw_sheet.starts_with('\'') && raw_sheet.ends_with('\'') {
                if raw_sheet.len() < 2 {
                    return None;
                }
                raw_sheet[1..raw_sheet.len() - 1].replace("''", "'")
            } else {
                if raw_sheet.contains(['\'', '!', '[', ']']) {
                    return None;
                }
                raw_sheet.to_string()
            };
            (Some(sheet), reference)
        }
        None => (None, formula),
    };

    Some((sheet_name, reference.replace('$', "")))
}

fn parse_a1_cell(reference: &str) -> Option<(u32, u32)> {
    let split = reference
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(reference.len());
    let (col_text, row_text) = reference.split_at(split);
    if col_text.is_empty() || row_text.is_empty() || !row_text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let col = col_text.chars().try_fold(0u32, |value, c| {
        value
            .checked_mul(26)?
            .checked_add(c.to_ascii_uppercase() as u32 - 'A' as u32 + 1)
    })?;
    let row = row_text.parse::<u32>().ok()?;
    if !(1..=MAX_COL).contains(&col) || !(1..=MAX_ROW).contains(&row) {
        return None;
    }
    Some((col, row))
}

pub(crate) fn parse_a1_range(reference: &str) -> Option<CellRange> {
    let mut parts = reference.trim().split(':');
    let first = parts.next()?;
    let second = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let (col_a, row_a) = parse_a1_cell(first)?;
    let (col_b, row_b) = match second {
        Some(cell) => parse_a1_cell(cell)?,
        None => (col_a, row_a),
    };
    Some(CellRange {
        top: row_a.min(row_b),
        left: col_a.min(col_b),
        bottom: row_a.max(row_b),
        right: col_a.max(col_b),
    })
}

fn inline_string_text(cell: roxmltree::Node<'_, '_>) -> String {
    let Some(inline) = cell.children().find(|node| {
        node.is_element() && node.tag_name().name() == "is" && is_x_ns(node.tag_name().namespace())
    }) else {
        return String::new();
    };
    let mut text = String::new();
    for child in inline.children().filter(|node| node.is_element()) {
        match child.tag_name().name() {
            "t" if is_x_ns(child.tag_name().namespace()) => {
                text.push_str(child.text().unwrap_or(""));
            }
            "r" if is_x_ns(child.tag_name().namespace()) => {
                for run_text in child.children().filter(|node| {
                    node.is_element()
                        && node.tag_name().name() == "t"
                        && is_x_ns(node.tag_name().namespace())
                }) {
                    text.push_str(run_text.text().unwrap_or(""));
                }
            }
            // `<rPh>` is phonetic guidance for the base text, not part of the
            // cell's displayed value (ECMA-376 §18.4.6).
            _ => {}
        }
    }
    text
}

fn cell_value(
    cell: roxmltree::Node<'_, '_>,
    shared_strings: &[SharedString],
) -> ReferencedCellValue {
    let cell_type = cell.attribute("t").unwrap_or("");
    if cell_type == "inlineStr" {
        return ReferencedCellValue::Text(inline_string_text(cell));
    }
    let value = cell
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "v"
                && is_x_ns(node.tag_name().namespace())
        })
        .and_then(|node| node.text())
        .unwrap_or("");

    match cell_type {
        "s" => value
            .parse::<usize>()
            .ok()
            .and_then(|index| shared_strings.get(index))
            .map(|string| ReferencedCellValue::Text(string.text.clone()))
            .unwrap_or(ReferencedCellValue::Empty),
        "str" => ReferencedCellValue::Text(value.to_string()),
        "" | "n" => value
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(ReferencedCellValue::Number)
            .unwrap_or(ReferencedCellValue::Empty),
        _ => ReferencedCellValue::Empty,
    }
}

fn parse_worksheet_cells(
    sheet_xml: &str,
    shared_strings: &[SharedString],
    max_cells: usize,
    max_string_bytes: usize,
) -> Option<IndexedWorksheet> {
    let document = parse_guarded(sheet_xml).ok()?;
    let mut values = HashMap::new();
    let mut string_bytes = 0usize;
    let mut previous_row = 0;
    for row_node in document.descendants().filter(|node| {
        node.is_element() && node.tag_name().name() == "row" && is_x_ns(node.tag_name().namespace())
    }) {
        let row = resolve_implicit_ordinal(
            row_node
                .attribute("r")
                .and_then(|value| value.parse::<u32>().ok()),
            &mut previous_row,
        );
        let mut previous_col = 0;
        for cell in row_node.children().filter(|node| {
            node.is_element()
                && node.tag_name().name() == "c"
                && is_x_ns(node.tag_name().namespace())
        }) {
            let (col, cell_row) = match cell.attribute("r") {
                Some(reference) => {
                    let (col, row) = parse_cell_ref(reference);
                    previous_col = col;
                    (col, row)
                }
                None => (resolve_implicit_ordinal(None, &mut previous_col), row),
            };
            if !(1..=MAX_COL).contains(&col) || !(1..=MAX_ROW).contains(&cell_row) {
                continue;
            }
            let value = cell_value(cell, shared_strings);
            if value == ReferencedCellValue::Empty {
                continue;
            }
            let key = (cell_row, col);
            let old_string_bytes = values
                .get(&key)
                .map(ReferencedCellValue::string_bytes)
                .unwrap_or(0);
            let next_cell_count = values.len() + usize::from(!values.contains_key(&key));
            let next_string_bytes = string_bytes
                .checked_sub(old_string_bytes)?
                .checked_add(value.string_bytes())?;
            if next_cell_count > max_cells || next_string_bytes > max_string_bytes {
                return None;
            }
            values.insert(key, value);
            string_bytes = next_string_bytes;
        }
    }
    Some(IndexedWorksheet {
        cells: values,
        string_bytes,
    })
}

fn reference_cell_count(range: &CellRange) -> Option<usize> {
    let row_count = range
        .bottom
        .checked_sub(range.top)
        .and_then(|value| value.checked_add(1))?;
    let col_count = range
        .right
        .checked_sub(range.left)
        .and_then(|value| value.checked_add(1))?;
    let total = (row_count as usize).checked_mul(col_count as usize)?;
    Some(total)
}

fn extract_from_sparse_cells(
    cells: &HashMap<(u32, u32), ReferencedCellValue>,
    range: &CellRange,
) -> Option<(Vec<ReferencedCellValue>, usize)> {
    let total = reference_cell_count(range)?;
    if total > MAX_REFERENCE_CELLS {
        return None;
    }
    let col_count = (range.right - range.left + 1) as usize;
    let mut values = vec![ReferencedCellValue::Empty; total];
    let mut string_bytes = 0usize;
    for row in range.top..=range.bottom {
        for col in range.left..=range.right {
            let Some(value) = cells.get(&(row, col)) else {
                continue;
            };
            string_bytes = string_bytes.checked_add(value.string_bytes())?;
            let index = (row - range.top) as usize * col_count + (col - range.left) as usize;
            values[index] = value.clone();
        }
    }
    Some((values, string_bytes))
}

#[cfg(test)]
pub(crate) fn extract_reference_cells(
    sheet_xml: &str,
    range: &CellRange,
    shared_strings: &[SharedString],
) -> Vec<ReferencedCellValue> {
    if reference_cell_count(range).is_none_or(|total| total > MAX_REFERENCE_CELLS) {
        return Vec::new();
    }
    parse_worksheet_cells(
        sheet_xml,
        shared_strings,
        MAX_TOTAL_INDEXED_CELLS,
        MAX_TOTAL_INDEXED_STRING_BYTES,
    )
    .as_ref()
    .and_then(|worksheet| extract_from_sparse_cells(&worksheet.cells, range))
    .map(|(values, _)| values)
    .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_worksheet_reference(
    archive: &mut XlsxZip,
    formula: &str,
    current_sheet_xml: &str,
    current_sheet_name: &str,
    sheets: &[SheetMeta],
    workbook_rels: &roxmltree::Document<'_>,
    shared_strings: &[SharedString],
    session: &mut WorksheetReferenceSession,
) -> Option<Vec<ReferencedCellValue>> {
    let (source_sheet, reference) = split_sheet_ref(formula)?;
    let range = parse_a1_range(&reference)?;
    let cell_count = session.reservable_cell_count(&range)?;
    let sheet_name = source_sheet.as_deref().unwrap_or(current_sheet_name);
    if !session.sheets.contains_key(sheet_name) {
        let worksheet = if sheet_name == current_sheet_name {
            parse_worksheet_cells(
                current_sheet_xml,
                shared_strings,
                session.remaining_indexed_cells,
                session.remaining_indexed_string_bytes,
            )
        } else {
            sheets
                .iter()
                .find(|sheet| sheet.name == sheet_name)
                .and_then(|sheet| resolve_sheet_path(workbook_rels, &sheet.r_id))
                .and_then(|path| read_zip_string(archive, &format!("xl/{path}")).ok())
                .and_then(|xml| {
                    parse_worksheet_cells(
                        &xml,
                        shared_strings,
                        session.remaining_indexed_cells,
                        session.remaining_indexed_string_bytes,
                    )
                })
        };
        if let Some(worksheet) = worksheet.as_ref() {
            session.consume_index(worksheet);
        }
        session.sheets.insert(sheet_name.to_string(), worksheet);
    }
    session.sheets.get(sheet_name).and_then(Option::as_ref)?;
    // Only successful source resolution consumes the cumulative dense-output
    // budget. Broken sheet names and unreadable parts must not starve later,
    // valid references in the same worksheet parse.
    let (values, string_bytes) = session
        .sheets
        .get(sheet_name)
        .and_then(Option::as_ref)
        .and_then(|worksheet| extract_from_sparse_cells(&worksheet.cells, &range))?;
    if string_bytes > session.remaining_string_bytes {
        return None;
    }
    session.consume_result(cell_count, string_bytes);
    Some(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CellRange, SharedString};

    #[test]
    fn quoted_unicode_sheet_reference_is_split_and_unescaped() {
        assert_eq!(
            split_sheet_ref("'التقرير'!$A$2:$A$5"),
            Some((Some("التقرير".into()), "A2:A5".into())),
        );
        assert_eq!(
            split_sheet_ref("'Bob''s data'!C1"),
            Some((Some("Bob's data".into()), "C1".into())),
        );
        assert_eq!(split_sheet_ref("A1:A3"), Some((None, "A1:A3".into())));
    }

    #[test]
    fn direct_a1_range_is_normalized() {
        let range = parse_a1_range("C5:A2").expect("direct A1 range parses");
        assert_eq!(
            (range.top, range.left, range.bottom, range.right),
            (2, 1, 5, 3),
        );
        assert!(parse_a1_range("Sales").is_none());
        assert!(parse_a1_range("A1+1").is_none());
    }

    #[test]
    fn worksheet_cells_resolve_inline_shared_formula_string_and_numbers() {
        let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Inline</t></is></c><c r="B1" t="s"><v>0</v></c><c r="C1" t="str"><f>UPPER(&quot;x&quot;)</f><v>X</v></c><c r="D1"><v>42</v></c></row></sheetData></worksheet>"#;
        let shared = vec![SharedString {
            text: "Shared".into(),
            ..Default::default()
        }];
        let range = CellRange {
            top: 1,
            left: 1,
            bottom: 1,
            right: 4,
        };
        assert_eq!(
            extract_reference_cells(xml, &range, &shared),
            vec![
                ReferencedCellValue::Text("Inline".into()),
                ReferencedCellValue::Text("Shared".into()),
                ReferencedCellValue::Text("X".into()),
                ReferencedCellValue::Number(42.0),
            ],
        );
    }

    #[test]
    fn oversized_range_is_rejected_before_allocation() {
        let range = CellRange {
            top: 1,
            left: 1,
            bottom: 1_048_576,
            right: 16_384,
        };
        assert!(extract_reference_cells("<worksheet/>", &range, &[]).is_empty());
    }

    #[test]
    fn cumulative_budget_rejects_individually_valid_ranges() {
        let mut session = WorksheetReferenceSession {
            remaining_cells: 4,
            ..Default::default()
        };
        let two_cells = CellRange {
            top: 1,
            left: 1,
            bottom: 1,
            right: 2,
        };
        let three_cells = CellRange {
            top: 1,
            left: 1,
            bottom: 1,
            right: 3,
        };

        let first_count = session.reservable_cell_count(&two_cells).unwrap();
        session.consume_result(first_count, 0);
        assert!(session.reservable_cell_count(&three_cells).is_none());
        assert_eq!(session.remaining_cells, 2);
    }

    #[test]
    fn sparse_index_has_cell_and_string_budgets() {
        let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" s="1"/><c r="B1" t="inlineStr"><is><t>alpha</t></is></c><c r="C1"><v>42</v></c></row></sheetData></worksheet>"#;

        let indexed = parse_worksheet_cells(xml, &[], 2, 5).expect("two non-empty cells fit");
        assert_eq!(indexed.cells.len(), 2);
        assert_eq!(indexed.string_bytes, 5);
        assert!(!indexed.cells.contains_key(&(1, 1)));
        assert!(parse_worksheet_cells(xml, &[], 1, 5).is_none());
        assert!(parse_worksheet_cells(xml, &[], 2, 4).is_none());
    }

    #[test]
    fn inline_string_excludes_phonetic_guidance() {
        let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>漢字</t><rPh sb="0" eb="2"><t>かんじ</t></rPh></is></c></row></sheetData></worksheet>"#;
        let range = CellRange {
            top: 1,
            left: 1,
            bottom: 1,
            right: 1,
        };

        assert_eq!(
            extract_reference_cells(xml, &range, &[]),
            vec![ReferencedCellValue::Text("漢字".into())]
        );
    }
}

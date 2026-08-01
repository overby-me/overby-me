//! A spreadsheet, rendered by this app.
//!
//! The sibling of [`super::docx`], and the same trade: the file is parsed in the
//! browser and rendered as a real `<table>`, so nothing is sent anywhere, the
//! cells are selectable, and the browser's own search finds them.
//!
//! A sheet is not a document. Two things follow, and both shape this module:
//!
//! * The grid is SPARSE. A row carries its `index` and a cell its `col`, and
//!   empty ones are simply absent — a sheet with a value in A1 and another in
//!   Z400 has two cells, not ten thousand. Rendering has to put them back on a
//!   grid, or every row collapses left and the columns stop lining up.
//! * A number is not its text. `45678` is a date, a price or a count depending
//!   on the format its style points at, and Excel stores only the number. The
//!   formats this understands are in [`cell_text`]; everything else falls back
//!   to the number as written, which is wrong-looking but never a lie.
//!
//! Parsing is `xlsx-parser` (MIT, Yuki Yokotani), which is a two-step API: the
//! workbook (shared strings, styles, sheet list) once, then each sheet on
//! demand.

use dioxus::prelude::*;
use serde::Deserialize;

/// A parsed sheet: the rows that have anything in them.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    #[serde(default)]
    pub rows: Vec<SheetRow>,
    #[serde(default)]
    pub merge_cells: Vec<Merge>,
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SheetRow {
    /// 1-based, and NOT the position in `rows`: empty rows are absent.
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub cells: Vec<SheetCell>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SheetCell {
    /// 1-based column.
    #[serde(default)]
    pub col: u32,
    #[serde(default)]
    pub value: Option<CellValue>,
    /// Index into `cellXfs`, which is how a number learns it is a date.
    #[serde(default)]
    pub style_index: Option<usize>,
}

/// What a cell holds. Text is not stored in the cell: a string lives once in the
/// workbook's shared table and the cell points at it by index.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CellValue {
    Shared {
        si: usize,
    },
    Number {
        number: f64,
    },
    #[serde(rename = "inlineStr")]
    Inline {
        text: String,
    },
    Boolean {
        #[serde(default)]
        boolean: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Merge {
    #[serde(default)]
    pub start_row: u32,
    #[serde(default)]
    pub end_row: u32,
    #[serde(default)]
    pub start_col: u32,
    #[serde(default)]
    pub end_col: u32,
}

/// The workbook parts every sheet needs: the strings and the number formats.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Workbook {
    #[serde(default)]
    pub shared_strings: Vec<SharedString>,
    #[serde(default)]
    pub styles: Styles,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SharedString {
    #[serde(default)]
    pub text: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Styles {
    #[serde(default)]
    pub cell_xfs: Vec<Xf>,
    /// Custom formats, by id. The built-in ids are not listed here — they are
    /// defined by the spec, which is why [`is_date_format`] knows them.
    #[serde(default)]
    pub num_fmts: std::collections::HashMap<String, String>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Xf {
    #[serde(default)]
    pub num_fmt_id: Option<u32>,
}

/// A spreadsheet column's letter: 1 → A, 26 → Z, 27 → AA.
///
/// Bijective base-26, which is not ordinary base-26: there is no zero digit, so
/// the usual algorithm puts a `@` where `Z` belongs at every multiple of 26.
pub fn column_label(mut col: u32) -> String {
    let mut out = Vec::new();
    while col > 0 {
        let rem = (col - 1) % 26;
        out.push(b'A' + rem as u8);
        col = (col - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Whether a number formatted by this id is a date rather than a quantity.
///
/// The built-in ids come from ECMA-376 §18.8.30; a custom format is a date if
/// its pattern mentions a date field, which is how Excel itself decides.
pub fn is_date_format(num_fmt_id: u32, custom: Option<&str>) -> bool {
    if matches!(num_fmt_id, 14..=22 | 45..=47) {
        return true;
    }
    match custom {
        // `m` is minutes as well as months, so a bare `m` is not enough; `y`
        // and `d` are unambiguous, and `mm-dd` style patterns carry one.
        Some(p) => {
            let p = p.to_ascii_lowercase();
            let stripped = p.replace("\\", "");
            stripped.contains('y') || stripped.contains('d')
        }
        None => false,
    }
}

/// An Excel serial date as `YYYY-MM-DD`.
///
/// Day 1 is 1900-01-01, so the epoch is 1899-12-30 — Excel deliberately
/// reproduces a Lotus bug in which 1900 is a leap year, and serials below 61
/// are off by one against a real calendar. Serials at or above 61 (1 March
/// 1900) are correct, which is every date anybody in this wiki will have.
pub fn excel_serial_to_date(serial: f64) -> Option<String> {
    if !serial.is_finite() || !(1.0..=2_958_465.0).contains(&serial) {
        return None;
    }
    // Days from the 1899-12-30 epoch to the Unix epoch.
    let days = serial.trunc() as i64 - 25_569;
    let (y, m, d) = civil_from_days(days);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Days since 1970-01-01 as a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// What a cell should read as.
///
/// The one place the sparse model, the shared strings and the number formats
/// come together, and the only part of this module that can be wrong in a way a
/// reader would notice — hence the tests.
pub fn cell_text(cell: &SheetCell, wb: &Workbook) -> String {
    let Some(value) = cell.value.as_ref() else {
        return String::new();
    };
    match value {
        CellValue::Inline { text } => text.clone(),
        CellValue::Boolean { boolean } => if *boolean { "TRUE" } else { "FALSE" }.to_string(),
        // A dangling index is empty rather than a panic: a workbook whose
        // shared table failed to parse still shows its numbers.
        CellValue::Shared { si } => wb
            .shared_strings
            .get(*si)
            .map(|s| s.text.clone())
            .unwrap_or_default(),
        CellValue::Number { number } => {
            let fmt = cell
                .style_index
                .and_then(|i| wb.styles.cell_xfs.get(i))
                .and_then(|xf| xf.num_fmt_id);
            if let Some(id) = fmt {
                let custom = wb.styles.num_fmts.get(&id.to_string()).map(String::as_str);
                if is_date_format(id, custom) {
                    if let Some(date) = excel_serial_to_date(*number) {
                        return date;
                    }
                }
            }
            // Trailing zeros are noise: 12.0 is 12, 12.50 is 12.5.
            let s = format!("{number}");
            s
        }
        CellValue::Other => String::new(),
    }
}

/// The sparse rows as a dense grid of text, plus how wide the widest row is.
///
/// Rendering straight from the sparse model puts every value hard against the
/// left edge and loses the columns entirely; this is what puts them back.
pub fn to_grid(sheet: &Sheet, wb: &Workbook) -> (Vec<Vec<String>>, usize) {
    let width = sheet
        .rows
        .iter()
        .flat_map(|r| r.cells.iter().map(|c| c.col as usize))
        .max()
        .unwrap_or(0);
    let last_row = sheet
        .rows
        .iter()
        .map(|r| r.index as usize)
        .max()
        .unwrap_or(0);
    let mut grid = vec![vec![String::new(); width]; last_row];
    for row in &sheet.rows {
        let Some(r) = (row.index as usize).checked_sub(1) else {
            continue;
        };
        for cell in &row.cells {
            let Some(c) = (cell.col as usize).checked_sub(1) else {
                continue;
            };
            if r < grid.len() && c < width {
                grid[r][c] = cell_text(cell, wb);
            }
        }
    }
    (grid, width)
}

/// A sheet as a table, with the spreadsheet's own row numbers and column
/// letters so a reader can talk about a cell by name.
#[component]
pub fn SheetTable(sheet: Sheet, workbook: Workbook) -> Element {
    let (grid, width) = to_grid(&sheet, &workbook);
    rsx! {
        div { class: "xlsx-wrap",
            table { class: "xlsx-table",
                thead {
                    tr {
                        th { class: "xlsx-corner" }
                        for c in 1..=width as u32 {
                            th { key: "c{c}", class: "xlsx-colhead", "{column_label(c)}" }
                        }
                    }
                }
                tbody {
                    for (r , row) in grid.into_iter().enumerate() {
                        tr { key: "r{r}",
                            th { class: "xlsx-rowhead", "{r + 1}" }
                            for (c , text) in row.into_iter().enumerate() {
                                td { key: "c{c}", "{text}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wb() -> Workbook {
        Workbook {
            shared_strings: vec![
                SharedString {
                    text: "Year".into(),
                },
                SharedString {
                    text: "Segment".into(),
                },
            ],
            styles: Styles {
                // 0: general, 1: a built-in date, 2: a custom date, 3: currency
                cell_xfs: vec![
                    Xf {
                        num_fmt_id: Some(0),
                    },
                    Xf {
                        num_fmt_id: Some(14),
                    },
                    Xf {
                        num_fmt_id: Some(165),
                    },
                    Xf {
                        num_fmt_id: Some(44),
                    },
                ],
                num_fmts: [("165".to_string(), "dd/mm/yyyy".to_string())]
                    .into_iter()
                    .collect(),
            },
        }
    }

    fn cell(col: u32, value: CellValue, style: Option<usize>) -> SheetCell {
        SheetCell {
            col,
            value: Some(value),
            style_index: style,
        }
    }

    /// Bijective base-26: the multiples of 26 are where the naive version breaks.
    #[test]
    fn columns_are_lettered_like_a_spreadsheet() {
        assert_eq!(column_label(1), "A");
        assert_eq!(column_label(26), "Z");
        assert_eq!(column_label(27), "AA");
        assert_eq!(column_label(52), "AZ");
        assert_eq!(column_label(53), "BA");
        assert_eq!(column_label(702), "ZZ");
        assert_eq!(column_label(703), "AAA");
    }

    #[test]
    fn a_shared_string_is_looked_up_and_a_dangling_one_is_empty() {
        let w = wb();
        assert_eq!(
            cell_text(&cell(1, CellValue::Shared { si: 0 }, None), &w),
            "Year"
        );
        assert_eq!(
            cell_text(&cell(1, CellValue::Shared { si: 99 }, None), &w),
            "",
            "a dangling index must not panic"
        );
    }

    /// The one that matters: the same number is a date or a quantity depending
    /// on the style it points at.
    #[test]
    fn a_number_reads_as_its_format() {
        let w = wb();
        let n = |style| cell_text(&cell(1, CellValue::Number { number: 45678.0 }, style), &w);
        assert_eq!(n(Some(0)), "45678", "general is just the number");
        assert_eq!(n(None), "45678", "no style is just the number");
        assert_eq!(n(Some(1)), "2025-01-21", "built-in date format 14");
        assert_eq!(n(Some(2)), "2025-01-21", "custom dd/mm/yyyy");
        assert_eq!(n(Some(3)), "45678", "currency is a quantity, not a date");
    }

    #[test]
    fn date_formats_are_recognised_by_id_or_pattern() {
        assert!(is_date_format(14, None));
        assert!(is_date_format(22, None));
        assert!(!is_date_format(0, None));
        assert!(!is_date_format(44, None), "currency");
        assert!(is_date_format(165, Some("yyyy-mm-dd")));
        assert!(is_date_format(166, Some("d mmm")));
        // `m` alone is minutes as often as months, so it is not enough.
        assert!(!is_date_format(167, Some("h:mm")));
        assert!(!is_date_format(168, Some("0.00")));
    }

    #[test]
    fn the_serial_epoch_is_right() {
        // 1900-03-01 is serial 61, the first day Excel and the calendar agree.
        assert_eq!(excel_serial_to_date(61.0).as_deref(), Some("1900-03-01"));
        assert_eq!(excel_serial_to_date(45678.0).as_deref(), Some("2025-01-21"));
        // A fraction is a time of day on the same date.
        assert_eq!(
            excel_serial_to_date(45678.75).as_deref(),
            Some("2025-01-21")
        );
        // Checked against an independent calculation (1899-12-30 + n days),
        // not against this implementation.
        assert_eq!(
            excel_serial_to_date(2_958_465.0).as_deref(),
            Some("9999-12-31")
        );
        // Out of range is not a date.
        assert_eq!(excel_serial_to_date(0.0), None);
        assert_eq!(excel_serial_to_date(f64::NAN), None);
        assert_eq!(excel_serial_to_date(1e9), None);
    }

    /// The sparse model put back on a grid. Without this every row collapses
    /// left and the columns stop meaning anything.
    #[test]
    fn holes_in_the_sheet_keep_their_place() {
        let w = wb();
        let sheet = Sheet {
            rows: vec![
                SheetRow {
                    index: 1,
                    cells: vec![
                        cell(1, CellValue::Shared { si: 0 }, None),
                        // C1, with nothing in B1.
                        cell(3, CellValue::Number { number: 7.0 }, None),
                    ],
                },
                // Row 2 is empty and absent; row 3 has one cell far right.
                SheetRow {
                    index: 3,
                    cells: vec![cell(4, CellValue::Inline { text: "x".into() }, None)],
                },
            ],
            ..Default::default()
        };
        let (grid, width) = to_grid(&sheet, &w);
        assert_eq!(width, 4, "widest row decides the column count");
        assert_eq!(grid.len(), 3, "the absent row 2 is still a row");
        assert_eq!(grid[0], vec!["Year", "", "7", ""], "B1 stays a hole");
        assert_eq!(grid[1], vec!["", "", "", ""], "the empty row is empty");
        assert_eq!(grid[2], vec!["", "", "", "x"], "D3 stays in column D");
    }

    /// The parser's real wire shape, keys and all.
    #[test]
    fn the_parsers_sheet_json_deserialises() {
        let json = r#"{
            "name":"Sheet1",
            "rows":[{"index":1,"height":15.0,"cells":[
                {"col":1,"row":1,"value":{"si":6,"type":"shared"}},
                {"col":3,"row":1,"styleIndex":5,"value":{"number":1618.5,"type":"number"}}
            ]}],
            "mergeCells":[],
            "colWidths":{"1":16.28}
        }"#;
        let sheet: Sheet = serde_json::from_str(json).expect("the sheet model must parse");
        assert_eq!(sheet.name, "Sheet1");
        assert_eq!(sheet.rows[0].cells.len(), 2);
        assert_eq!(sheet.rows[0].cells[1].style_index, Some(5));
        assert!(matches!(
            sheet.rows[0].cells[1].value,
            Some(CellValue::Number { number }) if number == 1618.5
        ));
    }
}

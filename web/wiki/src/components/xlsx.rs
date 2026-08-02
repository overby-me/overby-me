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
    /// The style the row states, which its cells inherit unless they state one.
    #[serde(default)]
    pub style_index: Option<usize>,
    /// The height the author set, in POINTS, when they set one. Absent means
    /// the sheet's default, which most rows are. Worth honouring because it is
    /// used to give a section its heading: a banner row set to 34pt against a
    /// 14pt default is doing the work a heading style would in a document.
    #[serde(default)]
    pub height: Option<f64>,
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
    /// Lenient on purpose. Styles decide whether a number is a date, and
    /// nothing else; a shape this does not recognise should cost the dates, not
    /// the whole spreadsheet. That is exactly what went wrong once already —
    /// `numFmts` turned out to be a list where this expected a map, and the
    /// error took every word of the document with it.
    #[serde(default, deserialize_with = "lenient_styles")]
    pub styles: Styles,
}

fn lenient_styles<'de, D>(d: D) -> Result<Styles, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(raw).unwrap_or_default())
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
    /// Custom formats. A LIST, carrying its own ids — not a map keyed by them,
    /// which is what this was and what silently failed the whole workbook. The
    /// built-in ids are not listed here at all; they are defined by the spec,
    /// which is why [`is_date_format`] knows them.
    #[serde(default)]
    pub num_fmts: Vec<NumFmt>,
    /// The line styles cells draw their edges with, indexed by `border_id`.
    ///
    /// In a set of accounts these ARE the structure: a medium rule under a
    /// subtotal, a double rule under the result. Without them the numbers run
    /// together as one undivided block.
    #[serde(default)]
    pub borders: Vec<Border>,
    /// The typefaces cells are set in, indexed by `font_id`.
    #[serde(default)]
    pub fonts: Vec<Font>,
    /// The backgrounds cells are painted with, indexed by `fill_id`.
    #[serde(default)]
    pub fills: Vec<Fill>,
}

/// A cell's background.
///
/// Note which colour that is: for a solid pattern it is the FOREGROUND. In
/// OOXML a pattern paints `fgColor` over `bgColor`, and "solid" is the pattern
/// that covers everything — so `fgColor` is the flat colour a reader sees, and
/// `bgColor` is what would show between the dots of a hatch. Reading the
/// obvious-looking field would paint nearly every filled cell the wrong colour.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    #[serde(default)]
    pub pattern_type: String,
    #[serde(default)]
    pub fg_color: Option<String>,
}

impl Fill {
    /// The flat colour this paints, if it paints one. Only `solid` is taken:
    /// a hatch drawn as its foreground colour would be far heavier than the
    /// sparse pattern the author chose, and `none` is the unfilled default that
    /// most cells point at.
    pub fn colour(&self) -> Option<&str> {
        if self.pattern_type.trim() != "solid" {
            return None;
        }
        self.fg_color
            .as_deref()
            .filter(|c| super::docx::is_real_colour(c))
    }
}

/// A cell's typeface. Colour and family are parsed by the vendored crate but not
/// read here: the app sets both, and a workbook's own black on the app's own
/// surface is a contrast decision this is not the place to make.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Font {
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    /// In points, as the workbook states it.
    #[serde(default)]
    pub size: f64,
}

impl Styles {
    /// The pattern for a format id, if the workbook defines one.
    pub fn format_code(&self, id: u32) -> Option<&str> {
        self.num_fmts
            .iter()
            .find(|f| f.num_fmt_id == id)
            .map(|f| f.format_code.as_str())
    }

    /// The border a cell of this style draws, if it draws one at all.
    ///
    /// Index 0 is the workbook's "no border" entry and every unstyled cell
    /// points at it, so an edgeless border is treated as none rather than as a
    /// rule to draw.
    pub fn border_of(&self, style_index: Option<usize>) -> Option<&Border> {
        let xf = self.cell_xfs.get(style_index?)?;
        let border = self.borders.get(xf.border_id? as usize)?;
        border.draws().then_some(border)
    }

    /// The font a cell of this style is set in.
    pub fn font_of(&self, style_index: Option<usize>) -> Option<&Font> {
        let xf = self.cell_xfs.get(style_index?)?;
        self.fonts.get(xf.font_id? as usize)
    }

    /// The background a cell of this style is painted, if any.
    pub fn fill_of(&self, style_index: Option<usize>) -> Option<&str> {
        let xf = self.cell_xfs.get(style_index?)?;
        self.fills.get(xf.fill_id? as usize)?.colour()
    }

    /// The size the sheet is mostly set in, which every other size is read
    /// against. Font 0 is the workbook's default by definition (ECMA-376), and
    /// 11pt is Excel's out-of-the-box default when a workbook states nothing.
    pub fn base_font_size(&self) -> f64 {
        self.fonts
            .first()
            .map(|f| f.size)
            .filter(|s| *s > 0.0)
            .unwrap_or(11.0)
    }
}

/// A cell's typeface as inline CSS, relative to what the sheet is mostly set in.
///
/// The size is a RATIO, not the workbook's points. A sheet is rendered at this
/// app's own density, and 11pt Arial would be half again the size of everything
/// around it; what carries meaning is that a heading is bigger than its rows,
/// not that it is 14 points. Anything within a whisker of the base size says
/// nothing and is left off.
pub fn font_css(font: &Font, base: f64) -> String {
    let mut css = String::new();
    if font.bold {
        css.push_str("font-weight:700;");
    }
    if font.italic {
        css.push_str("font-style:italic;");
    }
    if font.underline {
        css.push_str("text-decoration:underline;");
    }
    if base > 0.0 && font.size > 0.0 {
        let ratio = font.size / base;
        if !(0.96..=1.04).contains(&ratio) {
            css.push_str(&format!("font-size:{ratio:.3}em;"));
        }
    }
    css
}

/// One cell's four edges. Diagonals and the table-style inner rules are parsed
/// by the vendored crate but not read here: neither appears on a cell border.
#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Border {
    #[serde(default)]
    pub left: Option<BorderEdge>,
    #[serde(default)]
    pub right: Option<BorderEdge>,
    #[serde(default)]
    pub top: Option<BorderEdge>,
    #[serde(default)]
    pub bottom: Option<BorderEdge>,
}

impl Border {
    /// Whether any edge actually draws. Excel writes a full `<border>` with four
    /// empty edges for "no border", and every plain cell refers to it.
    pub fn draws(&self) -> bool {
        [&self.left, &self.right, &self.top, &self.bottom]
            .into_iter()
            .flatten()
            .any(|e| edge_css(e).is_some())
    }
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BorderEdge {
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub color: Option<String>,
}

/// One edge as a CSS `border-*` value, or `None` when it draws nothing.
///
/// The colour is deliberately dropped for `currentColor`. Excel writes these as
/// `auto` or `indexed="64"` — "whatever the text colour is" — which is exactly
/// what `currentColor` means, and it keeps the rules legible in dark mode where
/// a literal black would vanish.
pub fn edge_css(edge: &BorderEdge) -> Option<&'static str> {
    match edge.style.trim() {
        "thin" | "hair" => Some("1px solid currentColor"),
        "medium" => Some("2px solid currentColor"),
        "thick" => Some("3px solid currentColor"),
        // The accountant's grand-total rule, and the one CSS spells the same.
        "double" => Some("3px double currentColor"),
        "dotted" => Some("1px dotted currentColor"),
        "dashed" | "dashDot" | "dashDotDot" | "slantDashDot" => Some("1px dashed currentColor"),
        "mediumDashed" | "mediumDashDot" | "mediumDashDotDot" => Some("2px dashed currentColor"),
        // "none", "", and anything a future Excel invents.
        _ => None,
    }
}

/// A cell's borders as inline CSS. Empty when it draws none, so the common cell
/// carries no style attribute at all.
pub fn border_css(border: &Border) -> String {
    let mut css = String::new();
    for (side, edge) in [
        ("top", &border.top),
        ("right", &border.right),
        ("bottom", &border.bottom),
        ("left", &border.left),
    ] {
        if let Some(value) = edge.as_ref().and_then(edge_css) {
            css.push_str(&format!("border-{side}:{value};"));
        }
    }
    css
}

/// Everything one style says about how a cell looks, as inline CSS.
///
/// Shared by cells and by ROWS: a row states a style its cells inherit, and
/// Excel then omits the cells that hold nothing but that style — so for those
/// positions the row is the only record that they are styled at all.
pub fn style_css(styles: &Styles, style_index: Option<usize>, base: f64) -> String {
    let mut css = styles
        .border_of(style_index)
        .map(border_css)
        .unwrap_or_default();
    if let Some(font) = styles.font_of(style_index) {
        css.push_str(&font_css(font, base));
    }
    if let Some(fill) = styles.fill_of(style_index) {
        // Ink to match, worked out from the fill: the background is the
        // WORKBOOK'S and the theme's on-surface colour knows nothing about it,
        // so a pale fill in dark mode would otherwise carry near-white text.
        // Same reasoning, and the same helper, as a Word table's shaded cell.
        let hex = fill.trim().trim_start_matches('#');
        css.push_str(&format!(
            "background:#{hex};color:{};",
            super::docx::readable_ink(hex)
        ));
    }
    css
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NumFmt {
    #[serde(default)]
    pub num_fmt_id: u32,
    #[serde(default)]
    pub format_code: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Xf {
    #[serde(default)]
    pub num_fmt_id: Option<u32>,
    /// Which entry of [`Styles::borders`] this style draws its edges with.
    #[serde(default)]
    pub border_id: Option<u32>,
    /// Which entry of [`Styles::fonts`] this style is set in.
    #[serde(default)]
    pub font_id: Option<u32>,
    /// Which entry of [`Styles::fills`] this style is painted with.
    #[serde(default)]
    pub fill_id: Option<u32>,
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
                let custom = wb.styles.format_code(id);
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
pub fn to_grid(sheet: &Sheet, wb: &Workbook) -> (Vec<Vec<GridCell>>, usize) {
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
    let base = wb.styles.base_font_size();
    let mut grid = vec![vec![GridCell::default(); width]; last_row];
    for row in &sheet.rows {
        let Some(r) = (row.index as usize).checked_sub(1) else {
            continue;
        };
        if r >= grid.len() {
            continue;
        }
        // The ROW's own style across the whole width first. Excel omits the
        // cells that hold nothing but the row's format, so a rule under a
        // heading is recorded here and nowhere else for those columns — it was
        // simply missing from every column with no cell of its own.
        let from_row = row
            .style_index
            .map(|s| style_css(&wb.styles, Some(s), base))
            .unwrap_or_default();
        if !from_row.is_empty() {
            for cell in grid[r].iter_mut() {
                cell.style = from_row.clone();
            }
        }
        // Then the cells, which override it: a cell that states a style wins
        // over the row's, and a cell with no text can still carry a rule (the
        // underline beneath a total sits on the empty cells beside it).
        for cell in &row.cells {
            let Some(c) = (cell.col as usize).checked_sub(1) else {
                continue;
            };
            if c < width {
                grid[r][c] = GridCell {
                    text: cell_text(cell, wb),
                    style: style_css(&wb.styles, cell.style_index, base),
                };
            }
        }
    }
    (grid, width)
}

/// One cell of the dense grid: what it says, and how it is set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridCell {
    pub text: String,
    /// Inline CSS: its rules and its typeface. Empty for a plain cell, which is
    /// most of them, so the common cell carries no style attribute at all.
    pub style: String,
}

/// A row's height as inline CSS, when the author set one — points, as the
/// workbook states them.
///
/// This raises a row and cannot lower one: `height` is a minimum for a table
/// row, and a padded cell here is ~16.5pt against Excel's 14pt default. So a
/// row stated at 15pt still renders at ~16.5, and that is deliberate. Rows
/// around Excel's default are not shrunk rows, they ARE the default; trimming
/// the padding to reach them would make most of a sheet cramped to honour a
/// number the author never chose. What is worth honouring is the row the author
/// made deliberately TALL — a 34pt banner heading — and that works.
pub fn row_style(height: Option<f64>) -> String {
    match height {
        Some(h) if h > 0.0 => format!("height:{h}pt;"),
        _ => String::new(),
    }
}

/// A sheet as a table, with the spreadsheet's own row numbers and column
/// letters so a reader can talk about a cell by name.
#[component]
pub fn SheetTable(sheet: Sheet, workbook: Workbook) -> Element {
    let (grid, width) = to_grid(&sheet, &workbook);
    // Heights by the same dense 0-based position the grid uses, so a row that
    // states one can be found while rendering. Absent rows keep the default.
    let mut heights: Vec<Option<f64>> = vec![None; grid.len()];
    for row in &sheet.rows {
        if let Some(r) = (row.index as usize).checked_sub(1) {
            if r < heights.len() {
                heights[r] = row.height;
            }
        }
    }
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
                        tr {
                            key: "r{r}",
                            style: row_style(heights.get(r).copied().flatten()),
                            th { class: "xlsx-rowhead", "{r + 1}" }
                            for (c , cell) in row.into_iter().enumerate() {
                                td {
                                    key: "c{c}",
                                    // Only styled cells carry the attribute at
                                    // all; a sheet is mostly plain cells.
                                    style: cell.style,
                                    "{cell.text}"
                                }
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

    /// The custom number formats are a LIST carrying their own ids, not a map
    /// keyed by them. Guessing that wrong failed the whole workbook and blanked
    /// a document — so this is the shape the parser actually emits, pasted.
    #[test]
    fn custom_number_formats_are_a_list() {
        let json = serde_json::json!({
            "cellXfs": [
                {"alignH": null, "alignV": null, "borderId": 0, "fillId": 0,
                 "fontId": 0, "numFmtId": 0, "wrapText": false},
                {"alignH": null, "alignV": null, "borderId": 1, "fillId": 0,
                 "fontId": 3, "numFmtId": 164, "wrapText": false}
            ],
            "numFmts": [
                {"formatCode": "_-* #,##0.00\\ \"kr.\"_-", "numFmtId": 44},
                {"formatCode": "dd/mm/yyyy", "numFmtId": 164}
            ]
        });
        let styles: Styles = serde_json::from_value(json).expect("the parser's own shape");
        assert_eq!(styles.cell_xfs.len(), 2);
        assert_eq!(styles.cell_xfs[1].num_fmt_id, Some(164));
        assert_eq!(styles.format_code(164), Some("dd/mm/yyyy"));
        assert_eq!(styles.format_code(9999), None, "an id nobody defined");
        assert!(is_date_format(164, styles.format_code(164)));
    }

    /// A styles block this cannot read must cost the DATES, not the document.
    /// The workbook is what carries every string in the file.
    #[test]
    fn unreadable_styles_do_not_take_the_words_with_them() {
        let json = serde_json::json!({
            "sharedStrings": [{"text": "Kontonr."}, {"text": "Kontonavn"}],
            // Whatever this is, it is not the shape above.
            "styles": {"numFmts": {"164": "dd/mm/yyyy"}, "cellXfs": "surprise"}
        });
        let wb: Workbook = serde_json::from_value(json).expect("must not fail");
        assert_eq!(wb.shared_strings.len(), 2, "the words survive");
        assert!(wb.styles.num_fmts.is_empty(), "the styles are given up on");
    }

    /// Reported: a 351-row spreadsheet rendered as one empty row.
    ///
    /// The workbook was being read with `parse_workbook_native`, which
    /// serialises only the sheet LIST. It is named after the workbook and it
    /// deserialises into [`Workbook`] without complaint — every field simply
    /// takes its default — so the shared-string table was always empty. A cell
    /// holding text holds an INDEX into that table, so every string in every
    /// real spreadsheet resolved to nothing.
    ///
    /// Both shapes are pinned here: the one that silently says nothing, and the
    /// one that carries the strings.
    #[test]
    fn the_workbook_must_be_the_whole_workbook() {
        // What `parse_workbook_native` returns. Deserialises fine; says nothing.
        let list_only = serde_json::json!({
            "sheets": [{"name": "Ark1", "sheetId": 1, "rId": "rId1"}]
        });
        let wb: Workbook = serde_json::from_value(list_only).unwrap();
        assert!(
            wb.shared_strings.is_empty(),
            "this is the trap: it parses, and it is empty"
        );

        // What `parse_xlsx` returns, which is what the renderer needs.
        let whole = serde_json::json!({
            "workbook": {"sheets": [{"name": "Ark1", "sheetId": 1, "rId": "rId1"}]},
            "sharedStrings": [{"text": "Kontonr."}, {"text": "Kontonavn"}],
            "styles": {"cellXfs": [{"numFmtId": 0}], "numFmts": {}}
        });
        let wb: Workbook = serde_json::from_value(whole).unwrap();
        assert_eq!(wb.shared_strings.len(), 2);

        // And a cell pointing into it reads back as its text.
        let cell = SheetCell {
            value: Some(CellValue::Shared { si: 1 }),
            ..Default::default()
        };
        assert_eq!(cell_text(&cell, &wb), "Kontonavn");

        // An index past the end is blank, not a panic: a corrupt table must not
        // take the page down.
        let past_end = SheetCell {
            value: Some(CellValue::Shared { si: 99 }),
            ..Default::default()
        };
        assert_eq!(cell_text(&past_end, &wb), "");
    }

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
                        ..Default::default()
                    },
                    Xf {
                        num_fmt_id: Some(14),
                        ..Default::default()
                    },
                    Xf {
                        num_fmt_id: Some(165),
                        ..Default::default()
                    },
                    Xf {
                        num_fmt_id: Some(44),
                        ..Default::default()
                    },
                ],
                num_fmts: vec![NumFmt {
                    num_fmt_id: 165,
                    format_code: "dd/mm/yyyy".to_string(),
                }],
                ..Default::default()
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
                    ..Default::default()
                },
                // Row 2 is empty and absent; row 3 has one cell far right.
                SheetRow {
                    index: 3,
                    cells: vec![cell(4, CellValue::Inline { text: "x".into() }, None)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let (grid, width) = to_grid(&sheet, &w);
        let text = |r: usize| -> Vec<String> { grid[r].iter().map(|c| c.text.clone()).collect() };
        assert_eq!(width, 4, "widest row decides the column count");
        assert_eq!(grid.len(), 3, "the absent row 2 is still a row");
        assert_eq!(text(0), vec!["Year", "", "7", ""], "B1 stays a hole");
        assert_eq!(text(1), vec!["", "", "", ""], "the empty row is empty");
        assert_eq!(text(2), vec!["", "", "", "x"], "D3 stays in column D");
    }

    /// The rules a set of accounts is read by, in the shape the parser emits
    /// them: pasted from the styles of a real `regnskabsark`. Every edge is
    /// present as a key and `null` when it draws nothing, and index 0 is the
    /// all-empty border that every plain cell in the workbook points at.
    #[test]
    fn a_workbooks_rules_become_css_edges() {
        let wb: Workbook = serde_json::from_value(serde_json::json!({
            "sharedStrings": [],
            "styles": {
                "cellXfs": [
                    {"numFmtId": 0, "borderId": 0},
                    {"numFmtId": 0, "borderId": 1},
                    {"numFmtId": 0, "borderId": 2},
                    {"numFmtId": 0, "borderId": 3}
                ],
                "numFmts": [],
                "borders": [
                    {"left": null, "right": null, "top": null, "bottom": null},
                    {"left": null, "right": null,
                     "top": {"style": "thin", "color": "#000000"},
                     "bottom": {"style": "double", "color": "#000000"}},
                    {"left": null, "right": null, "top": null,
                     "bottom": {"style": "thick", "color": null}},
                    {"left": null, "right": null, "top": null,
                     "bottom": {"style": "medium", "color": "#000000"}}
                ]
            }
        }))
        .expect("the parser's own shape");

        // The plain cell draws nothing, which is most of the sheet.
        assert!(wb.styles.border_of(Some(0)).is_none());
        // ...and so does a cell with no style at all, or one off the end.
        assert!(wb.styles.border_of(None).is_none());
        assert!(wb.styles.border_of(Some(99)).is_none());

        // The grand total: a thin rule above, a double rule below.
        let total = wb.styles.border_of(Some(1)).expect("border 1 draws");
        assert_eq!(
            border_css(total),
            "border-top:1px solid currentColor;border-bottom:3px double currentColor;"
        );
        // The heavy rule, and the subtotal's.
        assert_eq!(
            border_css(wb.styles.border_of(Some(2)).expect("border 2 draws")),
            "border-bottom:3px solid currentColor;"
        );
        assert_eq!(
            border_css(wb.styles.border_of(Some(3)).expect("border 3 draws")),
            "border-bottom:2px solid currentColor;"
        );
    }

    /// A rule can sit on a cell with nothing in it — the underline beside a
    /// total usually does — so the grid has to carry style, not just text.
    #[test]
    fn an_empty_cell_still_carries_its_rule() {
        let wb: Workbook = serde_json::from_value(serde_json::json!({
            "sharedStrings": [],
            "styles": {
                "cellXfs": [{"numFmtId": 0, "borderId": 0}, {"numFmtId": 0, "borderId": 1}],
                "numFmts": [],
                "borders": [
                    {"left": null, "right": null, "top": null, "bottom": null},
                    {"left": null, "right": null, "top": null,
                     "bottom": {"style": "medium", "color": null}}
                ]
            }
        }))
        .expect("the parser's own shape");
        let sheet = Sheet {
            rows: vec![SheetRow {
                index: 1,
                cells: vec![SheetCell {
                    col: 2,
                    value: None,
                    style_index: Some(1),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let (grid, _) = to_grid(&sheet, &wb);
        assert_eq!(grid[0][1].text, "", "nothing written in it");
        assert_eq!(
            grid[0][1].style, "border-bottom:2px solid currentColor;",
            "but the rule under it is the point"
        );
    }

    /// Sizes are read against what the sheet is mostly set in, not in points:
    /// the sheet is drawn at this app's density, and what carries meaning is
    /// that a heading is bigger than its rows.
    #[test]
    fn a_cells_typeface_is_relative_to_the_sheets_own() {
        let plain = Font {
            size: 11.0,
            ..Default::default()
        };
        assert_eq!(font_css(&plain, 11.0), "", "the body size says nothing");

        let heading = Font {
            bold: true,
            size: 14.0,
            ..Default::default()
        };
        assert_eq!(
            font_css(&heading, 11.0),
            "font-weight:700;font-size:1.273em;",
            "14 over 11, not 14 points"
        );

        let emphasis = Font {
            bold: true,
            italic: true,
            size: 11.0,
            ..Default::default()
        };
        assert_eq!(
            font_css(&emphasis, 11.0),
            "font-weight:700;font-style:italic;"
        );

        // A workbook that states nothing is Excel's own 11pt.
        assert_eq!(Styles::default().base_font_size(), 11.0);
    }

    /// For a solid pattern the colour a reader sees is the FOREGROUND: OOXML
    /// paints fg over bg, and solid covers everything. Reading `bgColor` would
    /// paint nearly every filled cell the wrong colour.
    #[test]
    fn a_solid_fill_is_its_foreground_and_a_pattern_is_not_taken() {
        let solid = Fill {
            pattern_type: "solid".into(),
            fg_color: Some("#C9C9C9".into()),
        };
        assert_eq!(solid.colour(), Some("#C9C9C9"));

        // The two every unfilled cell points at.
        assert_eq!(
            Fill {
                pattern_type: "none".into(),
                fg_color: None
            }
            .colour(),
            None
        );
        assert_eq!(
            Fill {
                pattern_type: "gray125".into(),
                fg_color: Some("#000000".into())
            }
            .colour(),
            None,
            "a hatch drawn solid would be far heavier than the author chose"
        );
    }

    /// The heights this workbook actually states: a 34pt banner row, and a
    /// handful around Excel's 14pt default. Only the tall ones change anything,
    /// which is the point — `height` on a table row is a minimum.
    #[test]
    fn a_row_states_the_height_its_author_set() {
        assert_eq!(row_style(Some(34.0)), "height:34pt;", "the banner heading");
        assert_eq!(row_style(Some(18.0)), "height:18pt;");
        assert_eq!(row_style(Some(15.0)), "height:15pt;");

        // A row that states nothing keeps the sheet's default.
        assert_eq!(row_style(None), "");
        assert_eq!(row_style(Some(0.0)), "");
    }

    /// Row 1 of the reported sheet: the row states the rule, and Excel then
    /// writes no cell at all for column A. The rule has to come from the row or
    /// it is lost exactly where there is nothing else to draw.
    #[test]
    fn a_rule_stated_by_the_row_reaches_the_columns_with_no_cell() {
        let wb: Workbook = serde_json::from_value(serde_json::json!({
            "sharedStrings": [],
            "styles": {
                "cellXfs": [
                    {"numFmtId": 0, "borderId": 0, "fontId": 0, "fillId": 0},
                    {"numFmtId": 0, "borderId": 1, "fontId": 0, "fillId": 0},
                    {"numFmtId": 0, "borderId": 2, "fontId": 0, "fillId": 0}
                ],
                "numFmts": [],
                "fonts": [{"bold": false, "italic": false, "underline": false, "size": 11.0}],
                "fills": [],
                "borders": [
                    {"left": null, "right": null, "top": null, "bottom": null},
                    {"left": null, "right": null, "top": null,
                     "bottom": {"style": "medium", "color": null}},
                    {"left": null, "right": null, "top": null,
                     "bottom": {"style": "thick", "color": null}}
                ]
            }
        }))
        .expect("the parser's own shape");

        let sheet = Sheet {
            rows: vec![SheetRow {
                index: 1,
                // The row says medium; column A has no cell of its own.
                style_index: Some(1),
                cells: vec![
                    SheetCell {
                        col: 2,
                        value: None,
                        style_index: Some(1),
                    },
                    // ...and one cell overrides the row with its own.
                    SheetCell {
                        col: 3,
                        value: None,
                        style_index: Some(2),
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let (grid, width) = to_grid(&sheet, &wb);
        assert_eq!(width, 3);
        assert_eq!(
            grid[0][0].style, "border-bottom:2px solid currentColor;",
            "column A has no cell, so the ROW is the only record of its rule"
        );
        assert_eq!(grid[0][1].style, "border-bottom:2px solid currentColor;");
        assert_eq!(
            grid[0][2].style, "border-bottom:3px solid currentColor;",
            "a cell that states a style wins over its row's"
        );
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

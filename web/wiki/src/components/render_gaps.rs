//! What this app's own renderers cannot draw, found before they draw it.
//!
//! The native viewers ([`super::docx`], [`super::xlsx`], [`super::pptx`]) render
//! text, structure and tables. A file can hold a great deal more — images,
//! charts, footnotes, merged cells, slide pictures — and the renderers simply
//! skip what they do not understand. Skipping quietly is the problem: a report
//! whose two charts are its whole point renders as a page of captions, and
//! nothing says so.
//!
//! So the model is inspected first, and what is missing is counted. Two things
//! come of that:
//!
//! * A **minor** gap is shown as a note above the document, naming what is not
//!   there and offering the viewers that can show it. The reader decides.
//! * A **major** gap does not render natively at all. If most of what a file
//!   holds cannot be drawn, showing the remainder is worse than useless — it
//!   looks complete, and it is not. Those go straight to an embedded viewer,
//!   with the reason said out loud.
//!
//! The counting reads the parser's JSON rather than the render models, because
//! the render models deliberately drop what they cannot use — by the time a
//! document is a `Vec<Block>`, the evidence is gone.

use serde_json::Value;

/// Something in the file that the native renderers do not draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    Image(usize),
    Chart(usize),
    /// A shape that is not a plain text box: a picture, a diagram, a connector.
    Graphic(usize),
    Footnote(usize),
    /// Running heads and feet. Counted as one whether there are one or six —
    /// nobody needs to know the number, only that they are not shown.
    HeaderFooter,
    /// Cells joined across rows or columns. The grid still renders; the joins
    /// do not, so a merged title row reads as one value and several blanks.
    MergedCells(usize),
}

impl Gap {
    /// The translation key naming this gap to a reader.
    pub fn label_key(self) -> &'static str {
        match self {
            Gap::Image(_) => "file.gapImages",
            Gap::Chart(_) => "file.gapCharts",
            Gap::Graphic(_) => "file.gapGraphics",
            Gap::Footnote(_) => "file.gapFootnotes",
            Gap::HeaderFooter => "file.gapHeaders",
            Gap::MergedCells(_) => "file.gapMerges",
        }
    }

    /// How many, for the ones worth counting.
    pub fn count(self) -> usize {
        match self {
            Gap::Image(n)
            | Gap::Chart(n)
            | Gap::Graphic(n)
            | Gap::Footnote(n)
            | Gap::MergedCells(n) => n,
            Gap::HeaderFooter => 0,
        }
    }
}

/// What was found, and whether the native renderer should be trusted with it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GapReport {
    pub gaps: Vec<Gap>,
    /// Blocks of text the renderer CAN draw. The denominator for the judgement:
    /// four missing images matter differently in a two-page memo and in a
    /// forty-page report.
    pub text_blocks: usize,
}

impl GapReport {
    pub fn is_empty(&self) -> bool {
        self.gaps.is_empty()
    }

    /// Everything countable that will not be drawn.
    pub fn missing(&self) -> usize {
        self.gaps.iter().map(|g| g.count()).sum::<usize>()
            + self
                .gaps
                .iter()
                .filter(|g| matches!(g, Gap::HeaderFooter))
                .count()
    }

    /// Whether rendering this natively would misrepresent the file.
    ///
    /// The rule: at least three things missing, AND the text that survives is
    /// not clearly the bulk of it (fewer than four text blocks for each missing
    /// piece). A long report with a few figures stays native and says what is
    /// absent; a poster with two lines of text and six images does not, because
    /// the two lines would look like the whole document.
    ///
    /// Deliberately a judgement, and deliberately in one place with a name, so
    /// it can be argued with and changed. It errs toward rendering natively:
    /// being shown a document with a note about its figures beats being sent to
    /// Microsoft for a file that would have read perfectly well.
    pub fn is_major(&self) -> bool {
        let missing = self.missing();
        missing >= 3 && self.text_blocks < missing * 4
    }
}

/// Walk a JSON tree, counting objects whose `type` is one of `wanted`.
fn count_typed(value: &Value, wanted: &[&str]) -> usize {
    match value {
        Value::Array(items) => items.iter().map(|v| count_typed(v, wanted)).sum(),
        Value::Object(map) => {
            let here = map
                .get("type")
                .and_then(|t| t.as_str())
                .is_some_and(|t| wanted.contains(&t)) as usize;
            here + map.values().map(|v| count_typed(v, wanted)).sum::<usize>()
        }
        _ => 0,
    }
}

fn len_of(value: &Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

/// What a Word document holds that this app will not draw.
pub fn docx_gaps(model: &Value) -> GapReport {
    let body = model.get("body").cloned().unwrap_or(Value::Null);
    let mut gaps = Vec::new();

    // The parser tags a picture run `image`, and the frame an anchored (floating)
    // one hangs from `anchorHost`. Both draw nothing here.
    let images = count_typed(&body, &["image", "anchorHost"]);
    if images > 0 {
        gaps.push(Gap::Image(images));
    }
    let charts = count_typed(&body, &["chart", "diagram", "smartArt"]);
    if charts > 0 {
        gaps.push(Gap::Chart(charts));
    }
    let notes = len_of(model, "footnotes") + len_of(model, "endnotes");
    if notes > 0 {
        gaps.push(Gap::Footnote(notes));
    }
    // `headers`/`footers` are maps keyed by relationship id, not arrays.
    let heads = ["headers", "footers"]
        .iter()
        .filter_map(|k| model.get(*k))
        .filter_map(|v| v.as_object())
        .any(|m| !m.is_empty());
    if heads {
        gaps.push(Gap::HeaderFooter);
    }

    GapReport {
        gaps,
        text_blocks: count_typed(&body, &["paragraph", "table"]),
    }
}

/// What a sheet holds that this app will not draw.
///
/// Takes the SHEET, not the workbook: charts and images live on the sheet, and
/// so do the merges the grid renderer flattens.
pub fn xlsx_gaps(sheet: &Value) -> GapReport {
    let mut gaps = Vec::new();
    let charts = len_of(sheet, "charts") + len_of(sheet, "sparklineGroups");
    if charts > 0 {
        gaps.push(Gap::Chart(charts));
    }
    let images = len_of(sheet, "images");
    if images > 0 {
        gaps.push(Gap::Image(images));
    }
    let shapes = len_of(sheet, "shapeGroups") + len_of(sheet, "slicers");
    if shapes > 0 {
        gaps.push(Gap::Graphic(shapes));
    }
    let merges = len_of(sheet, "mergeCells");
    if merges > 0 {
        gaps.push(Gap::MergedCells(merges));
    }

    // Rows of cells are what survives, and they are the denominator: a sheet of
    // eight hundred rows with one logo is not "mostly missing".
    GapReport {
        gaps,
        text_blocks: len_of(sheet, "rows"),
    }
}

/// What a deck holds that this app will not draw.
pub fn pptx_gaps(model: &Value) -> GapReport {
    let slides = model.get("slides").cloned().unwrap_or(Value::Null);
    let mut gaps = Vec::new();

    let pictures = count_typed(&slides, &["picture", "image", "media", "video"]);
    if pictures > 0 {
        gaps.push(Gap::Image(pictures));
    }
    let charts = count_typed(&slides, &["chart", "graphicFrame", "diagram", "table"]);
    if charts > 0 {
        gaps.push(Gap::Chart(charts));
    }

    // A deck's text lives in shapes that have a text body; a shape without one
    // draws nothing here either way.
    let text_shapes = match &slides {
        Value::Array(items) => items
            .iter()
            .flat_map(|s| s.get("elements").and_then(|e| e.as_array()).cloned())
            .flatten()
            .filter(|e| e.get("textBody").is_some_and(|t| !t.is_null()))
            .count(),
        _ => 0,
    };
    GapReport {
        gaps,
        text_blocks: text_shapes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_document_of_plain_text_has_no_gaps() {
        let model = json!({
            "body": [
                {"type":"paragraph","runs":[{"type":"text","text":"Dagsorden"}]},
                {"type":"paragraph","runs":[{"type":"text","text":"Velkomst"}]}
            ],
            "footnotes": [], "endnotes": [], "headers": {}, "footers": {}
        });
        let report = docx_gaps(&model);
        assert!(report.is_empty());
        assert!(!report.is_major());
        assert_eq!(report.text_blocks, 2);
    }

    /// The real shape: the parser tags picture runs `image`, and an anchored
    /// one hangs off an `anchorHost`. Counted from a nested walk, because a run
    /// lives inside a paragraph inside the body — and inside table cells too.
    #[test]
    fn images_are_found_wherever_they_are_nested() {
        let model = json!({
            "body": [
                {"type":"paragraph","runs":[
                    {"type":"text","text":"See figure"},
                    {"type":"image","path":"media/image1.png"}
                ]},
                {"type":"table","rows":[{"cells":[{"content":[
                    {"type":"paragraph","runs":[{"type":"anchorHost"}]}
                ]}]}]}
            ]
        });
        let report = docx_gaps(&model);
        assert_eq!(
            report.gaps,
            vec![Gap::Image(2)],
            "one nested in a table cell"
        );
        assert_eq!(report.text_blocks, 3, "two paragraphs and a table");
    }

    #[test]
    fn footnotes_and_running_heads_are_reported() {
        let model = json!({
            "body": [{"type":"paragraph","runs":[]}],
            "footnotes": [{"id":"1"}, {"id":"2"}],
            "endnotes": [{"id":"3"}],
            "headers": {"rId4": {}}
        });
        let report = docx_gaps(&model);
        assert!(report.gaps.contains(&Gap::Footnote(3)), "{:?}", report.gaps);
        assert!(report.gaps.contains(&Gap::HeaderFooter));
    }

    /// The judgement call, from both sides. A long document with a few figures
    /// stays native; a poster does not.
    #[test]
    fn a_report_with_figures_still_renders_but_a_poster_does_not() {
        let long = GapReport {
            gaps: vec![Gap::Image(4)],
            text_blocks: 200,
        };
        assert!(!long.is_major(), "four figures in two hundred blocks");

        let poster = GapReport {
            gaps: vec![Gap::Image(6)],
            text_blocks: 2,
        };
        assert!(poster.is_major(), "two lines of text and six images");

        // Under the floor, nothing is major however thin the text: one missing
        // image is not worth sending somebody to Microsoft.
        let thin = GapReport {
            gaps: vec![Gap::Image(2)],
            text_blocks: 0,
        };
        assert!(!thin.is_major(), "below the three-item floor");
    }

    #[test]
    fn a_sheet_reports_its_charts_and_merges() {
        let sheet = json!({
            "rows": [{"index":1,"cells":[]}, {"index":2,"cells":[]}],
            "charts": [{"id":1}],
            "images": [],
            "mergeCells": [{"startRow":1,"endRow":1,"startCol":1,"endCol":3}],
            "shapeGroups": [], "slicers": [], "sparklineGroups": []
        });
        let report = xlsx_gaps(&sheet);
        assert!(report.gaps.contains(&Gap::Chart(1)));
        assert!(report.gaps.contains(&Gap::MergedCells(1)));
        assert_eq!(report.text_blocks, 2);
        // Two rows and two missing pieces is under the floor.
        assert!(!report.is_major());
    }

    /// A deck of photographs is the case this exists for: the text that
    /// survives would look like the whole deck.
    #[test]
    fn a_deck_of_pictures_is_major() {
        let model = json!({
            "slides": [
                {"elements":[
                    {"type":"picture"}, {"type":"picture"},
                    {"type":"shape","textBody":{"paragraphs":[]}}
                ]},
                {"elements":[{"type":"picture"}, {"type":"chart"}]}
            ]
        });
        let report = pptx_gaps(&model);
        assert_eq!(report.text_blocks, 1);
        assert!(report.gaps.contains(&Gap::Image(3)));
        assert!(report.gaps.contains(&Gap::Chart(1)));
        assert!(report.is_major(), "one text shape against four pictures");
    }

    #[test]
    fn a_deck_of_bullet_slides_is_not_major() {
        let elements: Vec<_> = (0..12)
            .map(|_| json!({"type":"shape","textBody":{"paragraphs":[]}}))
            .collect();
        let model = json!({"slides":[{"elements": elements}]});
        let report = pptx_gaps(&model);
        assert!(report.is_empty());
        assert!(!report.is_major());
    }
}

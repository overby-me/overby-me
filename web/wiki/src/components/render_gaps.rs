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

/// Walk a JSON tree, counting nodes a predicate accepts.
fn count_where(value: &Value, want: &dyn Fn(&Value) -> bool) -> usize {
    match value {
        Value::Array(items) => items.iter().map(|v| count_where(v, want)).sum(),
        Value::Object(map) => {
            want(value) as usize + map.values().map(|v| count_where(v, want)).sum::<usize>()
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

/// Whether one header or footer slot holds anything a reader would miss.
///
/// The presence of the slot says nothing. `headers` and `footers` are objects
/// with a fixed three keys — `default`, `first`, `even` — and the parser writes
/// `null` into the ones the document does not have, so the object is never
/// empty even in a document with no header at all. Measured on 23 documents
/// from the wiki: 14 had no header or footer part whatsoever, and every one of
/// them would have been told its headers were missing.
///
/// Word also leaves behind slots holding a single empty paragraph. There is
/// nothing to miss in those either, so the test is for content, not structure:
///
/// * any text that is not just whitespace, or
/// * any run that is not text — a `field` is how a page number is written, and
///   a footer that is only a page number is still a real footer, or
/// * any block that is not a paragraph, such as a table.
fn has_running_content(slot: &Value) -> bool {
    let Some(body) = slot.get("body").and_then(|b| b.as_array()) else {
        return false;
    };
    body.iter().any(|block| {
        if block.get("type").and_then(|t| t.as_str()) != Some("paragraph") {
            // A table or anything else structural is content by existing.
            return !block.is_null();
        }
        block
            .get("runs")
            .and_then(|r| r.as_array())
            .is_some_and(|runs| {
                runs.iter().any(|run| {
                    let kind = run.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                    let has_text = run
                        .get("text")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| !t.trim().is_empty());
                    has_text || !matches!(kind, "text" | "break" | "tab")
                })
            })
    })
}

/// What a Word document holds that this app will not draw.
pub fn docx_gaps(model: &Value) -> GapReport {
    let body = model.get("body").cloned().unwrap_or(Value::Null);
    let mut gaps = Vec::new();

    // Pictures ARE drawn now, so only the ones that cannot be are counted:
    // a format no browser decodes (Word embeds EMF and WMF), or an effect that
    // would need the pixels reworked before drawing. An image shown uncropped
    // or un-recoloured is shown wrong, and that is worth saying.
    let images = count_where(&body, &|node| {
        if node.get("type").and_then(|t| t.as_str()) != Some("image") {
            return false;
        }
        let vector = node
            .get("svgImagePath")
            .and_then(|p| p.as_str())
            .is_some_and(|p| !p.is_empty());
        let drawable = vector
            || super::docx::is_drawable(
                node.get("mimeType").and_then(|m| m.as_str()).unwrap_or(""),
            );
        let reworked = ["srcRect", "duotone", "colorReplaceFrom", "alpha"]
            .iter()
            .any(|k| node.get(*k).is_some_and(|v| !v.is_null()));
        !drawable || reworked
    });
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
    let heads = ["headers", "footers"]
        .iter()
        .filter_map(|k| model.get(*k))
        .filter_map(|v| v.as_object())
        .flat_map(|m| m.values())
        .any(has_running_content);
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

    // Pictures ARE drawn now. What is still counted is a picture the browser
    // cannot decode, on the same rule the Word renderer uses; video is never
    // drawn, so it stays counted whatever its format.
    let pictures = count_where(
        &slides,
        &|node| match node.get("type").and_then(|t| t.as_str()) {
            Some("media") | Some("video") => true,
            Some("picture") | Some("image") => {
                let vector = node
                    .get("svgImagePath")
                    .and_then(|p| p.as_str())
                    .is_some_and(|p| !p.is_empty());
                !vector
                    && !super::docx::is_drawable(
                        node.get("mimeType").and_then(|m| m.as_str()).unwrap_or(""),
                    )
            }
            _ => false,
        },
    );
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

    /// Pictures are drawn now, so a picture is not a gap. Only one that cannot
    /// be drawn is: a format no browser decodes. Counted from a nested walk,
    /// because a run lives inside a paragraph inside the body — and inside
    /// table cells too.
    #[test]
    fn only_undrawable_images_are_counted_wherever_they_are_nested() {
        // Field names as the parser writes them.
        let model = json!({
            "body": [
                {"type":"paragraph","runs":[
                    {"type":"text","text":"See figure"},
                    {"type":"image","imagePath":"word/media/image1.png","mimeType":"image/png"}
                ]},
                {"type":"table","rows":[{"cells":[{"content":[
                    {"type":"paragraph","runs":[
                        {"type":"image","imagePath":"word/media/logo.emf","mimeType":"image/x-emf"}
                    ]}
                ]}]}]}
            ]
        });
        let report = docx_gaps(&model);
        assert_eq!(
            report.gaps,
            vec![Gap::Image(1)],
            "the png draws; the emf, nested in a table cell, does not"
        );
        assert_eq!(report.text_blocks, 3, "two paragraphs and a table");
    }

    /// A picture that draws but draws WRONG is still worth saying. Word can ask
    /// for a crop or a recolour, and this renderer hands the browser the whole
    /// original: right pixels, wrong picture.
    #[test]
    fn a_picture_needing_rework_is_still_a_gap() {
        let cropped = json!({"body": [{"type":"paragraph","runs":[
            {"type":"image","imagePath":"word/media/image1.png","mimeType":"image/png",
             "srcRect":{"l":0.1,"t":0.0,"r":0.1,"b":0.0}}
        ]}]});
        assert_eq!(docx_gaps(&cropped).gaps, vec![Gap::Image(1)]);

        // The same fields present and null are the ordinary case, not a gap.
        let plain = json!({"body": [{"type":"paragraph","runs":[
            {"type":"image","imagePath":"word/media/image1.png","mimeType":"image/png",
             "srcRect":null,"duotone":null,"colorReplaceFrom":null,"alpha":null}
        ]}]});
        assert!(docx_gaps(&plain).is_empty(), "{:?}", docx_gaps(&plain).gaps);
    }

    /// The vector original wins, and it is drawable even when the raster
    /// fallback beside it is an EMF.
    #[test]
    fn an_svg_original_rescues_an_undrawable_fallback() {
        let model = json!({"body": [{"type":"paragraph","runs":[
            {"type":"image","imagePath":"word/media/image1.emf","mimeType":"image/x-emf",
             "svgImagePath":"word/media/image1.svg"}
        ]}]});
        assert!(docx_gaps(&model).is_empty());
    }

    #[test]
    fn footnotes_and_running_heads_are_reported() {
        let model = json!({
            "body": [{"type":"paragraph","runs":[]}],
            "footnotes": [{"id":"1"}, {"id":"2"}],
            "endnotes": [{"id":"3"}],
            "headers": {"default": {"body": [
                {"type":"paragraph","runs":[{"type":"text","text":"Radikal Ungdom"}]}
            ]}, "first": null, "even": null}
        });
        let report = docx_gaps(&model);
        assert!(report.gaps.contains(&Gap::Footnote(3)), "{:?}", report.gaps);
        assert!(report.gaps.contains(&Gap::HeaderFooter));
    }

    /// Reported: a document with no header and no footer said its headers were
    /// missing. The slots are always all three, holding `null` when absent, so
    /// the object is never empty and its size means nothing. These are the exact
    /// shapes the parser produced for real documents from the wiki.
    #[test]
    fn a_document_without_a_header_is_not_told_it_lost_one() {
        let absent = json!({
            "body": [{"type":"paragraph","runs":[{"type":"text","text":"Beretning"}]}],
            "headers": {"default": null, "even": null, "first": null},
            "footers": {"default": null, "even": null, "first": null}
        });
        assert!(
            !docx_gaps(&absent).gaps.contains(&Gap::HeaderFooter),
            "three null slots are three absent headers"
        );

        // Word leaves empty parts behind. An empty paragraph is nothing to miss.
        let leftover = json!({
            "body": [{"type":"paragraph","runs":[{"type":"text","text":"Beretning"}]}],
            "headers": {"default": null, "even": null,
                        "first": {"body": [{"type":"paragraph","runs":[]}]}},
            "footers": {"default": {"body": [
                {"type":"paragraph","runs":[{"type":"text","text":"   "}]}
            ]}, "even": null, "first": null}
        });
        assert!(
            !docx_gaps(&leftover).gaps.contains(&Gap::HeaderFooter),
            "an empty paragraph and a paragraph of spaces are both empty"
        );

        // A page number is a `field` run carrying no text, and a footer that is
        // only a page number is still a footer worth mentioning.
        let page_number = json!({
            "body": [{"type":"paragraph","runs":[{"type":"text","text":"Beretning"}]}],
            "headers": {"default": null, "even": null, "first": null},
            "footers": {"default": {"body": [
                {"type":"paragraph","runs":[{"type":"field","text":null}]}
            ]}, "even": null, "first": null}
        });
        assert!(
            docx_gaps(&page_number).gaps.contains(&Gap::HeaderFooter),
            "a page-number field is content"
        );
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

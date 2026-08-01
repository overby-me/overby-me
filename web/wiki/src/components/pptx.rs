//! A slide deck, rendered by this app.
//!
//! The third of the three, and the one that fits DOM worst. A document flows and
//! a sheet is a grid, but a slide is a fixed canvas: every shape carries an
//! absolute position and size, and moving one relative to another changes what
//! the slide means. There is no reflowing that.
//!
//! So this does not reflow it. Each slide is a box of the deck's own aspect
//! ratio, and every shape sits inside at its own fraction of that box —
//! positions in per cent, type in `cqw` (a hundredth of the box's width). The
//! whole slide then scales with whatever width it is given, on a phone or a
//! projector, and the layout the author made survives intact.
//!
//! What that buys over the embedded viewers is the same as elsewhere: nothing is
//! sent to anybody, and the text is real text — selectable, searchable, and
//! readable by a screen reader in the order the shapes were authored.
//!
//! What it does not do: images, charts, tables inside slides, transitions,
//! speaker notes, or any shape geometry beyond a rectangle. A deck that leans on
//! those will look wrong here, and the other two viewers are one tap away.
//!
//! Parsing is `pptx-parser` (MIT, Yuki Yokotani).

use dioxus::prelude::*;
use serde::Deserialize;

/// English Metric Units per point. OOXML measures everything in EMU: 914400 to
/// the inch, and 72 points to the inch.
const EMU_PER_POINT: f64 = 12700.0;

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Deck {
    #[serde(default)]
    pub slides: Vec<Slide>,
    #[serde(default)]
    pub slide_width: f64,
    #[serde(default)]
    pub slide_height: f64,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Slide {
    #[serde(default)]
    pub elements: Vec<Shape>,
    #[serde(default)]
    pub slide_number: Option<u32>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    #[serde(default)]
    pub text_body: Option<TextBody>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextBody {
    #[serde(default)]
    pub paragraphs: Vec<TextParagraph>,
    #[serde(default)]
    pub default_font_size: Option<f64>,
    /// `t`, `ctr` or `b`: where the text sits in a box taller than itself.
    #[serde(default)]
    pub vertical_anchor: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextParagraph {
    #[serde(default)]
    pub runs: Vec<TextRun>,
    /// Outline depth, for the indent a bulleted list carries.
    #[serde(default)]
    pub lvl: Option<u32>,
    #[serde(default)]
    pub alignment: Option<String>,
    #[serde(default)]
    pub def_font_size: Option<f64>,
    #[serde(default)]
    pub def_color: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub bold: Option<bool>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub underline: Option<bool>,
    #[serde(default)]
    pub strikethrough: Option<bool>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub font_size: Option<f64>,
}

/// A shape's box, as percentages of the slide.
///
/// Per cent rather than pixels so the slide scales: the same deck fills a
/// projector and a phone, and the shapes keep their relationship to each other,
/// which on a slide IS the content.
pub fn shape_box(shape: &Shape, deck: &Deck) -> String {
    let (w, h) = (deck.slide_width.max(1.0), deck.slide_height.max(1.0));
    format!(
        "left:{:.3}%;top:{:.3}%;width:{:.3}%;height:{:.3}%;",
        shape.x / w * 100.0,
        shape.y / h * 100.0,
        shape.width / w * 100.0,
        shape.height / h * 100.0,
    )
}

/// A point size as `cqw` — hundredths of the slide box's width.
///
/// The unit is what makes the deck scale. A 44pt title on a 10-inch slide is
/// 44/720 of its width, and stays that fraction whatever the box is; `pt` here
/// would be 44pt on a phone, which is a title four words wide.
pub fn font_cqw(points: f64, deck: &Deck) -> f64 {
    let slide_points = (deck.slide_width / EMU_PER_POINT).max(1.0);
    points / slide_points * 100.0
}

/// The size a run is actually set in.
///
/// Three places carry it and only one is usually filled: the run itself, else
/// the paragraph's default, else the text body's. A missing size at every level
/// is 18pt, PowerPoint's own default for body text — a slide with no size
/// anywhere still has to render at *something*.
pub fn effective_font_size(
    run: &TextRun,
    paragraph: &TextParagraph,
    body: Option<&TextBody>,
) -> f64 {
    run.font_size
        .or(paragraph.def_font_size)
        .or_else(|| body.and_then(|b| b.default_font_size))
        .unwrap_or(18.0)
}

/// Where text sits in a box taller than the text itself.
pub fn vertical_anchor(body: Option<&TextBody>) -> &'static str {
    match body.and_then(|b| b.vertical_anchor.as_deref()) {
        Some("ctr") => "center",
        Some("b") => "flex-end",
        _ => "flex-start",
    }
}

/// The CSS for one run: colour and the styles that have no element.
pub fn run_css(
    run: &TextRun,
    paragraph: &TextParagraph,
    body: Option<&TextBody>,
    deck: &Deck,
) -> String {
    let mut css = format!(
        "font-size:{:.3}cqw;",
        font_cqw(effective_font_size(run, paragraph, body), deck)
    );
    let colour = run.color.as_deref().or(paragraph.def_color.as_deref());
    if let Some(c) = colour {
        if c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
            css.push_str(&format!("color:#{c};"));
        }
    }
    if run.underline == Some(true) && run.strikethrough == Some(true) {
        css.push_str("text-decoration:underline line-through;");
    } else if run.underline == Some(true) {
        css.push_str("text-decoration:underline;");
    } else if run.strikethrough == Some(true) {
        css.push_str("text-decoration:line-through;");
    }
    css
}

/// The CSS for a paragraph: how it is aligned, and how far its level indents it.
pub fn paragraph_css(paragraph: &TextParagraph, deck: &Deck) -> String {
    let mut css = String::new();
    // `l` is emitted rather than skipped: the file viewer around a slide
    // centres its contents, so a left-aligned run that says nothing inherits
    // centre. See the same note in components::docx.
    match paragraph.alignment.as_deref() {
        Some("ctr") => css.push_str("text-align:center;"),
        Some("r") => css.push_str("text-align:right;"),
        Some("just") => css.push_str("text-align:justify;"),
        Some("l") => css.push_str("text-align:left;"),
        _ => {}
    }
    // Each outline level steps in by a third of an inch, in the same scaling
    // unit as the type so the indent shrinks with the slide.
    if let Some(level) = paragraph.lvl.filter(|l| *l > 0) {
        let indent_pt = level as f64 * 24.0;
        css.push_str(&format!("margin-left:{:.3}cqw;", font_cqw(indent_pt, deck)));
    }
    css
}

/// One deck, as a stack of slides.
#[component]
pub fn DeckView(deck: Deck) -> Element {
    // A deck with no dimensions cannot be laid out; 4:3 is the safest guess and
    // beats dividing by zero.
    let (w, h) = if deck.slide_width > 0.0 && deck.slide_height > 0.0 {
        (deck.slide_width, deck.slide_height)
    } else {
        (9_144_000.0, 6_858_000.0)
    };
    let slides = deck.slides.clone();
    rsx! {
        div { class: "pptx-deck",
            for (i , slide) in slides.into_iter().enumerate() {
                section {
                    key: "s{i}",
                    class: "pptx-slide",
                    style: "aspect-ratio:{w} / {h};",
                    aria_label: "{i + 1}",
                    for (j , shape) in slide.elements.into_iter().enumerate() {
                        if shape.text_body.is_some() {
                            div {
                                key: "e{j}",
                                class: "pptx-shape",
                                style: "{shape_box(&shape, &deck)}justify-content:{vertical_anchor(shape.text_body.as_ref())};",
                                {shape_text(&shape, &deck)}
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A shape's paragraphs.
fn shape_text(shape: &Shape, deck: &Deck) -> Element {
    let body = shape.text_body.clone();
    let paragraphs = body
        .as_ref()
        .map(|b| b.paragraphs.clone())
        .unwrap_or_default();
    let deck = deck.clone();
    rsx! {
        for (i , paragraph) in paragraphs.into_iter().enumerate() {
            p {
                key: "p{i}",
                class: "pptx-para",
                style: "{paragraph_css(&paragraph, &deck)}",
                for (j , run) in paragraph.runs.iter().enumerate() {
                    {
                        let css = run_css(run, &paragraph, body.as_ref(), &deck);
                        let text = run.text.clone();
                        let bold = run.bold == Some(true);
                        let italic = run.italic == Some(true);
                        rsx! {
                            if bold && italic {
                                strong { key: "r{j}", em { style: "{css}", "{text}" } }
                            } else if bold {
                                strong { key: "r{j}", style: "{css}", "{text}" }
                            } else if italic {
                                em { key: "r{j}", style: "{css}", "{text}" }
                            } else {
                                span { key: "r{j}", style: "{css}", "{text}" }
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

    /// A 10 x 7.5 inch slide, which is what PowerPoint calls 4:3.
    fn deck() -> Deck {
        Deck {
            slides: vec![],
            slide_width: 9_144_000.0,
            slide_height: 6_858_000.0,
        }
    }

    #[test]
    fn a_shape_is_placed_as_a_fraction_of_the_slide() {
        let d = deck();
        // Half an inch in from the left edge of a ten-inch slide is 5%.
        let s = Shape {
            x: 457_200.0,
            y: 274_638.0,
            width: 8_229_600.0,
            height: 1_143_000.0,
            ..Default::default()
        };
        let css = shape_box(&s, &d);
        assert!(css.contains("left:5.000%;"), "{css}");
        assert!(css.contains("width:90.000%;"), "{css}");
        // 1143000 EMU of a 6858000-high slide is a sixth.
        assert!(css.contains("height:16.667%;"), "{css}");
    }

    /// A deck with no dimensions must not divide by zero.
    #[test]
    fn a_dimensionless_deck_does_not_divide_by_zero() {
        let d = Deck::default();
        let css = shape_box(
            &Shape {
                x: 100.0,
                width: 100.0,
                ..Default::default()
            },
            &d,
        );
        assert!(!css.contains("NaN") && !css.contains("inf"), "{css}");
        assert!(font_cqw(44.0, &d).is_finite());
    }

    /// The unit that makes a deck scale: 44pt on a 720pt-wide slide is 6.11% of
    /// its width, whatever the box is actually drawn at.
    #[test]
    fn type_is_sized_as_a_fraction_of_the_slide_width() {
        let d = deck();
        assert!(
            (font_cqw(44.0, &d) - 6.111).abs() < 0.01,
            "{}",
            font_cqw(44.0, &d)
        );
        assert!(
            (font_cqw(720.0, &d) - 100.0).abs() < 0.01,
            "a full-width glyph"
        );
    }

    /// Three places carry the size and usually only one is filled.
    #[test]
    fn a_runs_size_falls_back_through_paragraph_then_body() {
        let body = TextBody {
            default_font_size: Some(32.0),
            ..Default::default()
        };
        let para_with = TextParagraph {
            def_font_size: Some(28.0),
            ..Default::default()
        };
        let para_without = TextParagraph::default();
        let run_with = TextRun {
            font_size: Some(18.0),
            ..Default::default()
        };
        let run_without = TextRun::default();

        assert_eq!(
            effective_font_size(&run_with, &para_with, Some(&body)),
            18.0
        );
        assert_eq!(
            effective_font_size(&run_without, &para_with, Some(&body)),
            28.0
        );
        assert_eq!(
            effective_font_size(&run_without, &para_without, Some(&body)),
            32.0
        );
        // Nothing anywhere still has to render at something.
        assert_eq!(
            effective_font_size(&run_without, &para_without, None),
            18.0,
            "PowerPoint's own body default"
        );
    }

    /// The same reported bug, in the deck renderer: `l` used to emit nothing
    /// and inherit the file viewer's centring.
    #[test]
    fn left_alignment_is_stated_in_a_slide_too() {
        let d = deck();
        let left = TextParagraph {
            alignment: Some("l".into()),
            ..Default::default()
        };
        assert!(paragraph_css(&left, &d).contains("text-align:left;"));
        let centred = TextParagraph {
            alignment: Some("ctr".into()),
            ..Default::default()
        };
        assert!(paragraph_css(&centred, &d).contains("text-align:center;"));
    }

    #[test]
    fn text_sits_where_the_box_says() {
        assert_eq!(vertical_anchor(None), "flex-start");
        let ctr = TextBody {
            vertical_anchor: Some("ctr".into()),
            ..Default::default()
        };
        assert_eq!(vertical_anchor(Some(&ctr)), "center");
        let bottom = TextBody {
            vertical_anchor: Some("b".into()),
            ..Default::default()
        };
        assert_eq!(vertical_anchor(Some(&bottom)), "flex-end");
    }

    /// The parser's real wire shape, from a deck made with python-pptx.
    #[test]
    fn the_parsers_json_deserialises() {
        let json = r#"{
          "slideWidth": 9144000, "slideHeight": 6858000,
          "slides": [{
            "index": 0, "slideNumber": 1, "background": null, "partName": "slide1.xml",
            "elements": [{
              "id": "2", "name": "Title 1", "type": "shape", "geometry": "rect",
              "x": 457200, "y": 274638, "width": 8229600, "height": 1143000,
              "rotation": 0.0, "flipH": false, "flipV": false, "fill": null,
              "placeholderType": "ctrTitle",
              "textBody": {
                "autoFit": "none", "defaultFontSize": 44.0, "verticalAnchor": "ctr",
                "paragraphs": [{
                  "alignment": null, "lvl": null, "defFontSize": null, "defColor": null,
                  "runs": [{"text":"Landsmøde 2026","bold":null,"italic":null,
                            "underline":false,"strikethrough":false,"color":null,
                            "fontSize":null,"type":"text","fieldType":null}]
                }]
              }
            }]
          }]
        }"#;
        let deck: Deck = serde_json::from_str(json).expect("the deck model must parse");
        assert_eq!(deck.slides.len(), 1);
        let shape = &deck.slides[0].elements[0];
        assert_eq!(shape.name.as_deref(), Some("Title 1"));
        let body = shape.text_body.as_ref().expect("a text body");
        assert_eq!(vertical_anchor(Some(body)), "center");
        let para = &body.paragraphs[0];
        assert_eq!(para.runs[0].text, "Landsmøde 2026");
        // The size comes from the body, since neither run nor paragraph set one.
        assert_eq!(effective_font_size(&para.runs[0], para, Some(body)), 44.0);
    }
}

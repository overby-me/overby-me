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

use crate::i18n::t;

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

    // --- pictures ---
    /// Zip path of the picture this shape draws, `ppt/media/image1.png`.
    #[serde(default)]
    pub image_path: Option<String>,
    /// The vector original, when PowerPoint kept one beside the raster it
    /// rasterised for compatibility. Preferred: it scales to the projector.
    #[serde(default)]
    pub svg_image_path: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub rotation: Option<f64>,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
    /// The picture itself, filled in by [`attach_images`] after parsing, exactly
    /// as the Word renderer does it.
    #[serde(skip)]
    pub src: Option<std::rc::Rc<str>>,
}

/// Which picture a shape draws, and in what format.
pub fn picture_of(shape: &Shape) -> Option<(&str, &str)> {
    if let Some(svg) = shape.svg_image_path.as_deref().filter(|p| !p.is_empty()) {
        return Some((svg, "image/svg+xml"));
    }
    let path = shape.image_path.as_deref().filter(|p| !p.is_empty())?;
    Some((path, shape.mime_type.as_deref().unwrap_or("")))
}

/// Read every picture the deck draws out of the package, once each.
pub fn collect_images(deck: &Deck, package: &[u8]) -> super::docx::Images {
    let mut wanted: Vec<(String, String)> = Vec::new();
    for slide in &deck.slides {
        for shape in &slide.elements {
            if let Some((path, mime)) = picture_of(shape) {
                if super::docx::is_drawable(mime) && !wanted.iter().any(|(p, _)| p == path) {
                    wanted.push((path.to_string(), mime.to_string()));
                }
            }
        }
    }
    super::docx::read_images(wanted, package)
}

/// The metafile pictures the deck draws, for the backend to render.
pub fn collect_metafiles(deck: &Deck) -> Vec<(String, String)> {
    let mut wanted: Vec<(String, String)> = Vec::new();
    for slide in &deck.slides {
        for shape in &slide.elements {
            if let Some((path, mime)) = picture_of(shape) {
                if super::docx::is_metafile(mime) && !wanted.iter().any(|(p, _)| p == path) {
                    wanted.push((path.to_string(), mime.to_string()));
                }
            }
        }
    }
    wanted
}

/// Give every picture shape the bytes it draws.
pub fn attach_images(deck: &mut Deck, images: &super::docx::Images) {
    for slide in &mut deck.slides {
        for shape in &mut slide.elements {
            if let Some((path, _)) = picture_of(shape) {
                shape.src = images.get(path).cloned();
            }
        }
    }
}

/// How a picture sits in its shape box.
///
/// NOT its width and height: the `<img>` IS the box, placed by [`shape_box`] as
/// a fraction of the slide, and setting a size here would override that and
/// blow the picture up to the whole slide.
///
/// `contain` rather than `cover` because the box is the size the author drew,
/// and cropping to fill it would hide part of what they placed.
pub fn image_style(shape: &Shape) -> String {
    let mut css = String::from("object-fit:contain;");
    let mut transform = String::new();
    if let Some(deg) = shape.rotation.filter(|d| d.abs() > 0.01) {
        transform.push_str(&format!("rotate({deg:.2}deg) "));
    }
    if shape.flip_h || shape.flip_v {
        let x = if shape.flip_h { -1 } else { 1 };
        let y = if shape.flip_v { -1 } else { 1 };
        transform.push_str(&format!("scale({x},{y})"));
    }
    if !transform.trim().is_empty() {
        css.push_str(&format!("transform:{};", transform.trim()));
    }
    css
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
                        if let Some(src) = shape.src.clone() {
                            // A picture sits in its own box, at the place and
                            // size the author put it, like every other shape.
                            img {
                                key: "e{j}",
                                class: "pptx-img",
                                src: "{src}",
                                style: "{shape_box(&shape, &deck)}{image_style(&shape)}",
                                alt: "",
                                loading: "lazy",
                                decoding: "async",
                            }
                        } else if shape.image_path.as_deref().is_some_and(|p| !p.is_empty()) {
                            // A picture the browser cannot decode. Office keeps
                            // pasted figures as EMF, which nothing renders and
                            // which carries no fallback of its own, so there are
                            // no pixels to draw here and never will be.
                            //
                            // Its BOX is drawn regardless, where and how big the
                            // author put it. Skipping the shape entirely left a
                            // silent hole under a caption that promises a figure
                            // ("Figur 7 ..."), which reads as the app losing the
                            // picture rather than as a viewer that cannot show
                            // this one. The banner above the document counts them
                            // too; this says WHICH.
                            div {
                                key: "e{j}",
                                class: "pptx-img-missing",
                                style: "{shape_box(&shape, &deck)}",
                                title: "{t(\"file.imageNotDrawable\")}",
                                role: "img",
                                aria_label: "{t(\"file.imageNotDrawable\")}",
                                span { class: "material-icons", "image_not_supported" }
                            }
                        } else if shape.text_body.is_some() {
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

    /// The numbers are from a real deck in the wiki: a full-bleed background
    /// picture and a small logo, on a 9144000 x 6858000 EMU slide.
    #[test]
    fn a_picture_is_placed_like_any_other_shape() {
        let deck = Deck {
            slide_width: 9_144_000.0,
            slide_height: 6_858_000.0,
            ..Default::default()
        };
        let full_bleed = Shape {
            x: 0.0,
            y: 0.0,
            width: 9_144_000.0,
            height: 5_143_500.0,
            image_path: Some("ppt/media/image1.png".into()),
            mime_type: Some("image/png".into()),
            ..Default::default()
        };
        assert_eq!(
            shape_box(&full_bleed, &deck),
            "left:0.000%;top:0.000%;width:100.000%;height:75.000%;"
        );
        // The box carries the size. The picture style must NOT, or it would
        // override those percentages and blow the picture up to the slide.
        let css = image_style(&full_bleed);
        assert!(!css.contains("width"), "{css}");
        assert!(!css.contains("height"), "{css}");
        assert!(css.contains("object-fit:contain;"), "{css}");

        let logo = Shape {
            x: 540_000.0,
            y: 530_467.0,
            width: 1_235_676.0,
            height: 190_500.0,
            image_path: Some("ppt/media/image2.png".into()),
            mime_type: Some("image/png".into()),
            ..Default::default()
        };
        let box_css = shape_box(&logo, &deck);
        assert!(box_css.contains("left:5.906%"), "{box_css}");
        assert!(box_css.contains("width:13.514%"), "{box_css}");
    }

    #[test]
    fn a_slide_picture_prefers_its_vector_original_and_carries_its_turn() {
        let mut shape = Shape {
            image_path: Some("ppt/media/image1.png".into()),
            mime_type: Some("image/png".into()),
            ..Default::default()
        };
        assert_eq!(
            picture_of(&shape),
            Some(("ppt/media/image1.png", "image/png"))
        );
        shape.svg_image_path = Some("ppt/media/image1.svg".into());
        assert_eq!(
            picture_of(&shape),
            Some(("ppt/media/image1.svg", "image/svg+xml"))
        );

        shape.rotation = Some(90.0);
        shape.flip_v = true;
        let css = image_style(&shape);
        assert!(
            css.contains("transform:rotate(90.00deg) scale(1,-1);"),
            "{css}"
        );

        // A shape that draws no picture is not one.
        assert_eq!(picture_of(&Shape::default()), None);
    }

    /// The shape as the parser writes it for a picture, pasted from a real deck.
    #[test]
    fn a_real_picture_shape_deserialises() {
        let json = r#"{
            "x": 540000, "y": 530467, "width": 1235676, "height": 190500,
            "rotation": 0.0, "flipH": false, "flipV": false,
            "imagePath": "ppt/media/image2.png", "mimeType": "image/png",
            "type": "picture"
        }"#;
        let shape: Shape = serde_json::from_str(json).expect("the parser's own output");
        assert_eq!(shape.image_path.as_deref(), Some("ppt/media/image2.png"));
        assert_eq!(shape.mime_type.as_deref(), Some("image/png"));
        assert_eq!(shape.width, 1_235_676.0);
        assert!(shape.src.is_none(), "the bytes are attached separately");
        assert!(shape.text_body.is_none());
    }

    /// A figure pasted into PowerPoint from Excel is stored as EMF, which no
    /// browser decodes and which carries no fallback of its own. It is not
    /// collected, so the shape keeps a picture path and never gets bytes — the
    /// exact state the slide draws a placeholder box for, rather than skipping
    /// the shape and leaving a hole under the caption that announces it.
    #[test]
    fn an_emf_picture_is_never_collected_but_stays_a_picture() {
        let emf = Shape {
            image_path: Some("ppt/media/image23.emf".into()),
            mime_type: Some("image/x-emf".into()),
            ..Default::default()
        };
        assert!(!super::super::docx::is_drawable("image/x-emf"));
        assert_eq!(
            picture_of(&emf),
            Some(("ppt/media/image23.emf", "image/x-emf"))
        );

        let mut deck = Deck {
            slides: vec![Slide {
                elements: vec![emf],
                ..Default::default()
            }],
            ..Default::default()
        };
        // The package is never opened for it: EMF is filtered out before any
        // reading, so an empty one is enough to show nothing was wanted.
        assert!(
            collect_images(&deck, &[]).is_empty(),
            "nothing to fetch: the browser could not draw it anyway"
        );
        attach_images(&mut deck, &Default::default());
        let shape = &deck.slides[0].elements[0];
        assert!(shape.src.is_none(), "no bytes, so no <img>");
        assert!(
            shape.image_path.as_deref().is_some_and(|p| !p.is_empty()),
            "still a picture, so its box is still drawn"
        );
        assert!(shape.text_body.is_none(), "and it is not a text shape");
    }

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

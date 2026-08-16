//! The pptx → GitHub-flavoured-markdown projection. These `pub(crate)` helpers
//! are the body shared by the `#[wasm_bindgen] pptx_to_markdown`, the native
//! `to_markdown_native`, and `PptxArchive::to_markdown` (all of which stay in
//! `lib.rs`). Extracted verbatim from `lib.rs`.

use crate::types::*;
use ooxml_common::math::nodes_to_text;

pub(crate) fn render_slide_md(slide: &Slide, out: &mut String) {
    use std::fmt::Write as _;
    let title = slide_title_md(slide);
    if let Some(t) = title {
        let _ = writeln!(out, "# {} (slide {})\n", t, slide.slide_number);
    } else {
        let _ = writeln!(out, "# Slide {}\n", slide.slide_number);
    }
    for el in &slide.elements {
        render_element_md(el, out);
    }
    if let Some(notes) = &slide.notes {
        let trimmed = notes.trim();
        if !trimmed.is_empty() {
            let _ = writeln!(out, "## Speaker notes\n\n{}\n", trimmed);
        }
    }
    if !slide.comments.is_empty() {
        let _ = writeln!(out, "## Comments\n");
        for c in &slide.comments {
            let author = c.author.as_deref().unwrap_or("(unknown)");
            let _ = writeln!(out, "> **{}**: {}", author, c.text.trim());
        }
        out.push('\n');
    }
}

pub(crate) fn slide_title_md(slide: &Slide) -> Option<String> {
    for el in &slide.elements {
        if let SlideElement::Shape(s) = el {
            let ph = s.placeholder_type.as_deref().unwrap_or("");
            if ph == "title" || ph == "ctrTitle" {
                let txt = shape_text_plain(s);
                if let Some(t) = txt {
                    if !t.is_empty() {
                        return Some(t);
                    }
                }
            }
        }
    }
    None
}

pub(crate) fn shape_text_plain(s: &ShapeElement) -> Option<String> {
    let tb = s.text_body.as_ref()?;
    let mut buf = String::new();
    for para in &tb.paragraphs {
        for run in &para.runs {
            if let TextRun::Text(t) = run {
                buf.push_str(&t.text);
            }
        }
        buf.push(' ');
    }
    let trimmed = buf.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub(crate) fn render_element_md(el: &SlideElement, out: &mut String) {
    match el {
        SlideElement::Shape(s) => {
            let ph = s.placeholder_type.as_deref().unwrap_or("");
            // The slide-level # heading already used the title placeholder's
            // text — skip it here to avoid duplicating it inside the body.
            if ph == "title" || ph == "ctrTitle" {
                return;
            }
            // Drop auto-generated metadata placeholders (slide number, date,
            // footer, header). Their text is always a single token like "3" or
            // "2026-05-11" that's pure noise for an agent reading the content.
            if matches!(ph, "sldNum" | "dt" | "ftr" | "hdr") {
                return;
            }
            render_shape_md(s, out);
        }
        SlideElement::Table(t) => render_table_md(t, out),
        SlideElement::Chart(c) => render_chart_md(c, out),
        // Pictures / media / connectors carry no readable text; intentionally
        // dropped in the markdown projection. Use `pptx_get_pictures` or the
        // raw JSON path when you need to inspect them.
        SlideElement::Picture(_) | SlideElement::Media(_) => {}
    }
}

pub(crate) fn render_shape_md(s: &ShapeElement, out: &mut String) {
    let Some(tb) = &s.text_body else { return };
    if tb.paragraphs.is_empty() {
        return;
    }
    // Body / subtitle placeholders inherit bullet formatting from the layout's
    // lstStyle (ECMA-376 §19.7.10) — treat `Bullet::Inherit` paragraphs there
    // as bulleted, mirroring what PowerPoint draws. Free text boxes default to
    // plain paragraphs.
    let ph = s.placeholder_type.as_deref().unwrap_or("");
    let inherit_means_bullet = matches!(ph, "body" | "subTitle" | "obj" | "tx" | "ftr" | "hdr");
    for para in &tb.paragraphs {
        render_paragraph_md(para, inherit_means_bullet, out);
    }
    out.push('\n');
}

pub(crate) enum ParaKind {
    Plain,
    Bullet,
    Number,
}

pub(crate) fn paragraph_kind(b: &Bullet, inherit_means_bullet: bool) -> ParaKind {
    match b {
        Bullet::None => ParaKind::Plain,
        Bullet::Char { .. } => ParaKind::Bullet,
        // A picture bullet is still an unordered list item for markdown export.
        Bullet::Blip { .. } => ParaKind::Bullet,
        Bullet::AutoNum { .. } => ParaKind::Number,
        Bullet::Inherit => {
            if inherit_means_bullet {
                ParaKind::Bullet
            } else {
                ParaKind::Plain
            }
        }
    }
}

pub(crate) fn render_paragraph_md(para: &Paragraph, inherit_means_bullet: bool, out: &mut String) {
    use std::fmt::Write as _;
    let text = render_runs_md(&para.runs);
    if text.trim().is_empty() {
        out.push('\n');
        return;
    }
    let indent = "  ".repeat(para.lvl as usize);
    match paragraph_kind(&para.bullet, inherit_means_bullet) {
        ParaKind::Plain => {
            let _ = writeln!(out, "{}{}", indent, text);
        }
        ParaKind::Bullet => {
            let _ = writeln!(out, "{}- {}", indent, text);
        }
        // We deliberately emit `1.` for every numbered paragraph rather than
        // tracking the real counter — every markdown renderer auto-renumbers
        // sequential ordered-list items, so the visual output is correct and
        // we don't need to carry per-list state.
        ParaKind::Number => {
            let _ = writeln!(out, "{}1. {}", indent, text);
        }
    }
}

pub(crate) fn render_runs_md(runs: &[TextRun]) -> String {
    let mut out = String::new();
    for run in runs {
        match run {
            // Intra-paragraph soft break (<a:br/>) → markdown hard line break
            // (two trailing spaces + newline).
            TextRun::Break => out.push_str("  \n"),
            // Equations have no faithful markdown form; emit their flattened text.
            TextRun::Math { nodes, .. } => out.push_str(&nodes_to_text(nodes)),
            TextRun::Text(t) => {
                let raw = &t.text;
                // Empty / whitespace-only runs (separators between formatted
                // spans) shouldn't trigger bold/italic wrappers — `**   **`
                // is awkward and most renderers drop the formatting anyway.
                if raw.chars().all(|c| c.is_whitespace()) {
                    out.push_str(raw);
                    continue;
                }
                // Preserve leading/trailing whitespace OUTSIDE the formatting
                // wrappers so `(bold)" Title "` becomes ` **Title** ` not
                // `**" Title "**`. This is how every markdown renderer treats
                // strong/emphasis spans (they're trimmed of whitespace).
                let leading_len = raw.len() - raw.trim_start().len();
                let trail_start = raw.trim_end().len();
                let leading = &raw[..leading_len];
                let trailing = &raw[trail_start..];
                let trimmed = &raw[leading_len..trail_start];
                let mut s = escape_inline_md(trimmed);
                if let Some(url) = &t.hyperlink {
                    s = format!("[{}]({})", s, url);
                }
                if t.bold == Some(true) {
                    s = format!("**{}**", s);
                }
                if t.italic == Some(true) {
                    s = format!("*{}*", s);
                }
                out.push_str(leading);
                out.push_str(&s);
                out.push_str(trailing);
            }
        }
    }
    out
}

/// Escape the markdown inline metacharacters that would otherwise be parsed as
/// formatting. We deliberately don't escape every potential metachar — pptx
/// body text contains so much punctuation that aggressive escaping makes the
/// output noisier than the structure it's trying to expose. Pipe is handled
/// separately in `render_table_cell_md` since it only matters inside tables.
pub(crate) fn escape_inline_md(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
}

pub(crate) fn render_table_md(t: &TableElement, out: &mut String) {
    use std::fmt::Write as _;
    if t.rows.is_empty() {
        return;
    }
    let cols = t.rows[0].cells.len();
    if cols == 0 {
        return;
    }
    let header_cells: Vec<String> = t.rows[0].cells.iter().map(render_table_cell_md).collect();
    let _ = writeln!(out, "| {} |", header_cells.join(" | "));
    let sep: Vec<&str> = (0..cols).map(|_| "---").collect();
    let _ = writeln!(out, "| {} |", sep.join(" | "));
    for row in t.rows.iter().skip(1) {
        let cells: Vec<String> = row.cells.iter().map(render_table_cell_md).collect();
        let _ = writeln!(out, "| {} |", cells.join(" | "));
    }
    out.push('\n');
}

pub(crate) fn render_table_cell_md(cell: &TableCell) -> String {
    // Continuation cells of a merge carry no content — leave empty so the row
    // alignment stays intact.
    if cell.h_merge || cell.v_merge {
        return String::new();
    }
    let Some(tb) = &cell.text_body else {
        return String::new();
    };
    let mut buf = String::new();
    for (i, para) in tb.paragraphs.iter().enumerate() {
        if i > 0 {
            buf.push_str("<br>");
        }
        for run in &para.runs {
            if let TextRun::Text(t) = run {
                buf.push_str(&t.text);
            }
        }
    }
    buf.trim().replace('|', "\\|")
}

pub(crate) fn render_chart_md(c: &ChartElement, out: &mut String) {
    use std::fmt::Write as _;
    let chart = &c.chart;
    let title = chart.title.as_deref().unwrap_or("(untitled)");
    let _ = writeln!(out, "**Chart ({}): {}**\n", chart.chart_type, title);
    if !chart.categories.is_empty() {
        let _ = writeln!(out, "- Categories: {}", chart.categories.join(", "));
    }
    for s in &chart.series {
        let values: Vec<String> = s
            .values
            .iter()
            .map(|v| match v {
                Some(n) => format!("{n}"),
                None => "—".to_string(),
            })
            .collect();
        let _ = writeln!(out, "- {}: {}", s.name, values.join(", "));
    }
    out.push('\n');
}

/// Project a parsed presentation to GitHub-flavoured markdown. Slides are joined
/// by a `---` rule. Shared by `pptx_to_markdown`, `to_markdown_native`, and
/// `PptxArchive::to_markdown` so every markdown path stays in lock-step.
pub(crate) fn render_presentation_md(pres: &Presentation) -> String {
    let mut out = String::new();
    for (i, slide) in pres.slides.iter().enumerate() {
        if i > 0 {
            out.push_str("\n---\n\n");
        }
        render_slide_md(slide, &mut out);
    }
    out
}

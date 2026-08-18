//! Turn a Windows metafile into SVG, so a pasted figure arrives as shapes and
//! real text instead of a picture of them.
//!
//! Why not the raster the renderer already produces: it draws every string in
//! ONE font. `RenderFontCache` asks fontdb for generic sans once and uses it for
//! the whole file, discarding the LOGFONT each text record carries, so headings,
//! italics and different families all come out as one regular face, filled from
//! glyph outlines with no hinting. That is what makes the PNG look the way it
//! does, and no amount of resolution fixes it.
//!
//! Emitting SVG hands all of that to the browser: it picks the font the record
//! asks for, shapes and hints it, and scales without resampling. The text stays
//! text, so it can be selected, searched and read aloud.
//!
//! **This reads the plain EMF records, not the EMF+ ones.** Office writes
//! dual-mode metafiles: the GDI+ content lives inside `Comment` records, and the
//! plain records beside it are a complete fallback drawing, which is what every
//! non-GDI+ consumer has always rendered. So reading them is the supported path,
//! not a shortcut.
//!
//! It is deliberately partial. Raster operation codes have no SVG equivalent,
//! region clipping is approximated by rectangles, and a record this does not
//! know is skipped rather than guessed at. `metafile.rs` keeps the rasteriser as
//! the fallback for anything this declines.

use std::collections::HashMap;
use std::fmt::Write as _;

use emfsdk::{ColorRef, EmfMetafileRef, EmfRecordData, LogFontW, PointL, PointS, RectL, XForm};

/// Below this, a metafile is drawing nothing worth showing and the caller is
/// better served by the rasteriser.
const MIN_USEFUL_ELEMENTS: usize = 1;

/// A 2x3 affine, in SVG's `matrix(a b c d e f)` order.
///
/// GDI's XForm maps `x' = m11*x + m21*y + dx`, which is the same mapping with
/// the same six numbers, so this is a rename rather than a conversion.
#[derive(Clone, Copy, Debug)]
struct Xf {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Default for Xf {
    fn default() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }
}

impl Xf {
    fn from_gdi(x: &XForm) -> Self {
        Self {
            a: x.m11 as f64,
            b: x.m12 as f64,
            c: x.m21 as f64,
            d: x.m22 as f64,
            e: x.dx as f64,
            f: x.dy as f64,
        }
    }

    /// `self` applied after `other`, which is what MWT_LEFTMULTIPLY means.
    fn times(self, other: Xf) -> Self {
        Self {
            a: other.a * self.a + other.b * self.c,
            b: other.a * self.b + other.b * self.d,
            c: other.c * self.a + other.d * self.c,
            d: other.c * self.b + other.d * self.d,
            e: other.e * self.a + other.f * self.c + self.e,
            f: other.e * self.b + other.f * self.d + self.f,
        }
    }

    fn is_identity(&self) -> bool {
        (self.a - 1.0).abs() < f64::EPSILON
            && self.b.abs() < f64::EPSILON
            && self.c.abs() < f64::EPSILON
            && (self.d - 1.0).abs() < f64::EPSILON
            && self.e.abs() < f64::EPSILON
            && self.f.abs() < f64::EPSILON
    }

    fn attr(&self) -> String {
        format!(
            "matrix({} {} {} {} {} {})",
            n(self.a),
            n(self.b),
            n(self.c),
            n(self.d),
            n(self.e),
            n(self.f)
        )
    }
}

#[derive(Clone, Debug)]
struct Pen {
    colour: Option<String>,
    width: f64,
    dash: Option<&'static str>,
}

impl Default for Pen {
    fn default() -> Self {
        Self {
            colour: Some("#000000".into()),
            width: 1.0,
            dash: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Brush {
    colour: Option<String>,
}

#[derive(Clone, Debug)]
struct Font {
    family: String,
    size: f64,
    weight: i32,
    italic: bool,
    underline: bool,
    /// Tenths of a degree, counter-clockwise, as GDI counts escapement.
    escapement: i32,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: "sans-serif".into(),
            size: 12.0,
            weight: 400,
            italic: false,
            underline: false,
            escapement: 0,
        }
    }
}

#[derive(Clone, Debug)]
struct State {
    xform: Xf,
    pen: Pen,
    brush: Brush,
    font: Font,
    text_colour: String,
    text_align: u32,
    clip: Option<String>,
    pos: (f64, f64),
}

impl Default for State {
    fn default() -> Self {
        Self {
            xform: Xf::default(),
            pen: Pen::default(),
            brush: Brush::default(),
            font: Font::default(),
            text_colour: "#000000".into(),
            text_align: 0,
            clip: None,
            pos: (0.0, 0.0),
        }
    }
}

#[derive(Clone, Debug)]
enum Object {
    Pen(Pen),
    Brush(Brush),
    Font(Font),
}

/// Everything the walk is building.
struct Svg {
    body: String,
    state: State,
    stack: Vec<State>,
    objects: HashMap<u32, Object>,
    /// Set between BeginPath and EndPath, so the drawing records build a path
    /// instead of emitting their own element.
    path: Option<String>,
    elements: usize,
}

/// Trim a float to something short enough to read in the output.
fn n(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn colour(c: ColorRef) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
}

/// XML-escape. Everything reaching the output from the file goes through this:
/// the SVG is served to a browser, and a face name or a caption is attacker
/// data as far as this is concerned.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are not valid XML and a metafile string may
            // carry them as padding.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// The CSS font stack for a Windows face name.
///
/// The name is kept first, so a reader who HAS the font gets it, then the
/// metric-compatible substitute the container ships, then a generic. Metrics
/// matter more than looks here: a metafile positions every run itself, so a face
/// of different widths puts the words in the wrong places.
fn font_stack(face: &str) -> String {
    let generic = match face.to_ascii_lowercase().as_str() {
        f if f.contains("courier") || f.contains("consol") || f.contains("mono") => {
            "\"Liberation Mono\", monospace"
        }
        f if f.contains("times") || f.contains("georgia") || f.contains("garamond") => {
            "\"Liberation Serif\", serif"
        }
        f if f.contains("calibri") => "Carlito, \"Liberation Sans\", sans-serif",
        _ => "\"Liberation Sans\", sans-serif",
    };
    if face.is_empty() {
        generic.to_string()
    } else {
        format!("\"{}\", {generic}", esc(face))
    }
}

/// The font a record asks for, whichever of the three layouts it used to say
/// it. Only the LOGFONT prefix matters here, and all three begin with one.
fn font_of(f: &emfsdk::emf::EmrExtCreateFont) -> Font {
    match f {
        emfsdk::emf::EmrExtCreateFont::LogFont(lf) => font_from(lf),
        emfsdk::emf::EmrExtCreateFont::LogFontPanose(p) => font_from(&p.log_font),
        emfsdk::emf::EmrExtCreateFont::LogFontExDv(d) => font_from(&d.log_font_ex.log_font),
        emfsdk::emf::EmrExtCreateFont::Raw(_) => Font::default(),
    }
}

fn font_from(lf: &LogFontW) -> Font {
    // A negative height is the character height, a positive one the cell
    // height; both are the size to draw at, and the sign is GDI's way of saying
    // which the caller measured.
    let size = (lf.height.unsigned_abs() as f64).max(1.0);
    Font {
        family: font_stack(
            lf.face_name
                .as_str()
                .unwrap_or_default()
                .trim_end_matches('\0'),
        ),
        size,
        weight: if lf.weight <= 0 { 400 } else { lf.weight },
        italic: lf.italic != 0,
        underline: lf.underline != 0,
        escapement: lf.escapement,
    }
}

impl Svg {
    fn new() -> Self {
        Self {
            body: String::new(),
            state: State::default(),
            stack: Vec::new(),
            objects: HashMap::new(),
            path: None,
            elements: 0,
        }
    }

    /// The attributes every drawn element shares: where it sits and what clips
    /// it. Emitted per element rather than as nested `<g>`s, which keeps the
    /// output flat and the state handling honest.
    fn common(&self) -> String {
        let mut out = String::new();
        if !self.state.xform.is_identity() {
            let _ = write!(out, " transform=\"{}\"", self.state.xform.attr());
        }
        if let Some(id) = &self.state.clip {
            let _ = write!(out, " clip-path=\"url(#{id})\"");
        }
        out
    }

    fn paint(&self) -> String {
        let mut out = String::new();
        match &self.state.brush.colour {
            Some(c) => {
                let _ = write!(out, " fill=\"{c}\"");
            }
            None => out.push_str(" fill=\"none\""),
        }
        match &self.state.pen.colour {
            Some(c) => {
                let _ = write!(
                    out,
                    " stroke=\"{c}\" stroke-width=\"{}\"",
                    n(self.state.pen.width.max(1.0))
                );
                if let Some(dash) = self.state.pen.dash {
                    let _ = write!(out, " stroke-dasharray=\"{dash}\"");
                }
            }
            None => out.push_str(" stroke=\"none\""),
        }
        out
    }

    /// Either add to the path being built, or draw it now.
    fn shape(&mut self, d: &str) {
        if let Some(path) = self.path.as_mut() {
            path.push_str(d);
            return;
        }
        let _ = write!(
            self.body,
            "<path d=\"{d}\"{}{}/>",
            self.paint(),
            self.common()
        );
        self.elements += 1;
    }
}

fn pts_l(points: &[PointL]) -> Vec<(f64, f64)> {
    points.iter().map(|p| (p.x as f64, p.y as f64)).collect()
}

fn pts_s(points: &[PointS]) -> Vec<(f64, f64)> {
    points.iter().map(|p| (p.x as f64, p.y as f64)).collect()
}

fn poly_d(points: &[(f64, f64)], close: bool) -> String {
    if points.is_empty() {
        return String::new();
    }
    let mut d = format!("M{} {}", n(points[0].0), n(points[0].1));
    for p in &points[1..] {
        let _ = write!(d, "L{} {}", n(p.0), n(p.1));
    }
    if close {
        d.push('Z');
    }
    d
}

fn bezier_d(points: &[(f64, f64)]) -> String {
    if points.len() < 4 {
        return poly_d(points, false);
    }
    let mut d = format!("M{} {}", n(points[0].0), n(points[0].1));
    for c in points[1..].chunks(3) {
        if c.len() < 3 {
            break;
        }
        let _ = write!(
            d,
            "C{} {} {} {} {} {}",
            n(c[0].0),
            n(c[0].1),
            n(c[1].0),
            n(c[1].1),
            n(c[2].0),
            n(c[2].1)
        );
    }
    d
}

fn rect_d(r: &RectL) -> String {
    format!(
        "M{} {}H{}V{}H{}Z",
        n(r.left as f64),
        n(r.top as f64),
        n(r.right as f64),
        n(r.bottom as f64),
        n(r.left as f64)
    )
}

/// GDI pen styles, as far as SVG can say them.
fn pen_from(style: u32, width: f64, c: ColorRef) -> Pen {
    // The low nibble is the dash style; PS_NULL draws nothing at all.
    let dash = match style & 0x0f {
        1 => Some("4,4"),
        2 => Some("1,3"),
        3 => Some("4,3,1,3"),
        4 => Some("4,3,1,3,1,3"),
        5 => {
            return Pen {
                colour: None,
                width,
                dash: None,
            }
        }
        _ => None,
    };
    Pen {
        colour: Some(colour(c)),
        width: width.max(1.0),
        dash,
    }
}

/// Render `data` as an SVG document, or `None` when there is nothing useful in
/// it and the rasteriser should answer instead.
pub fn to_svg(data: &[u8]) -> Option<String> {
    let metafile = EmfMetafileRef::from_bytes(data).ok()?;
    let mut svg = Svg::new();
    let mut bounds: Option<RectL> = None;

    for record in metafile.records() {
        let Ok(parsed) = EmfRecordData::from_record_ref(record) else {
            continue;
        };
        walk(&mut svg, parsed, &mut bounds);
    }

    if svg.elements < MIN_USEFUL_ELEMENTS {
        return None;
    }
    let b = bounds?;
    let (w, h) = (
        (b.right - b.left).max(1) as f64,
        (b.bottom - b.top).max(1) as f64,
    );
    // A metafile assumes it is drawn onto the page it was cut from.
    let page = format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>",
        n(b.left as f64),
        n(b.top as f64),
        n(w),
        n(h)
    );
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" \
         width=\"{}\" height=\"{}\">{page}{}</svg>",
        n(b.left as f64),
        n(b.top as f64),
        n(w),
        n(h),
        n(w),
        n(h),
        svg.body
    ))
}

fn walk(svg: &mut Svg, record: EmfRecordData<'_>, bounds: &mut Option<RectL>) {
    use EmfRecordData as R;
    match record {
        R::Header(h) => {
            *bounds = Some(if h.bounds.right > h.bounds.left {
                h.bounds
            } else {
                h.frame
            })
        }

        // Transform.
        R::SetWorldTransform(t) => svg.state.xform = Xf::from_gdi(&t.transform),
        R::ModifyWorldTransform(t) => {
            let x = Xf::from_gdi(&t.transform);
            // MWT_IDENTITY(1) resets; LEFT(2)/RIGHT(3) compose; 4 replaces.
            svg.state.xform = match t.mode {
                1 => Xf::default(),
                2 => x.times(svg.state.xform),
                3 => svg.state.xform.times(x),
                _ => x,
            };
        }
        R::SaveDc => svg.stack.push(svg.state.clone()),
        R::RestoreDc(_) => {
            if let Some(prev) = svg.stack.pop() {
                svg.state = prev;
            }
        }

        // Objects.
        R::CreatePen(p) => {
            let pen = pen_from(p.pen_style, p.width.x as f64, p.color);
            svg.objects.insert(p.object_index, Object::Pen(pen));
        }
        R::ExtCreatePen(p) => {
            let width = p.width.max(1) as f64;
            let pen = pen_from(p.pen_style, width, p.color);
            svg.objects.insert(p.object_index, Object::Pen(pen));
        }
        R::CreateBrushIndirect(b) => {
            // Style 1 is BS_NULL: a brush that fills nothing.
            let colour = (b.brush_style != 1).then(|| colour(b.color));
            svg.objects
                .insert(b.object_index, Object::Brush(Brush { colour }));
        }
        R::ExtCreateFontIndirectW(f) => {
            svg.objects
                .insert(f.object_index, Object::Font(font_of(&f.font)));
        }
        R::SelectObject(s) => {
            // The high bit marks a stock object, which has no entry here.
            if let Some(obj) = svg.objects.get(&s.object_index).cloned() {
                match obj {
                    Object::Pen(p) => svg.state.pen = p,
                    Object::Brush(b) => svg.state.brush = b,
                    Object::Font(f) => svg.state.font = f,
                }
            } else if s.object_index & 0x8000_0000 != 0 {
                match s.object_index & 0x7fff_ffff {
                    // WHITE_BRUSH, LTGRAY, GRAY, DKGRAY, BLACK, NULL_BRUSH
                    0 => svg.state.brush.colour = Some("#ffffff".into()),
                    4 => svg.state.brush.colour = Some("#000000".into()),
                    5 => svg.state.brush.colour = None,
                    // WHITE_PEN, BLACK_PEN, NULL_PEN
                    6 => {
                        svg.state.pen = Pen {
                            colour: Some("#ffffff".into()),
                            ..Pen::default()
                        }
                    }
                    7 => svg.state.pen = Pen::default(),
                    8 => svg.state.pen.colour = None,
                    _ => {}
                }
            }
        }
        R::DeleteObject(d) => {
            svg.objects.remove(&d.object_index);
        }

        // Colours and text setup.
        R::SetTextColor(c) => svg.state.text_colour = colour(c.color),
        R::SetTextAlign(a) => svg.state.text_align = a.text_alignment_mode,

        // Paths.
        R::BeginPath => svg.path = Some(String::new()),
        R::EndPath => {}
        R::CloseFigure => {
            if let Some(p) = svg.path.as_mut() {
                p.push('Z');
            }
        }
        R::AbortPath => svg.path = None,
        R::FillPath(_) | R::StrokePath(_) | R::StrokeAndFillPath(_) => {
            if let Some(d) = svg.path.take() {
                if !d.is_empty() {
                    let _ = write!(svg.body, "<path d=\"{d}\"{}{}/>", svg.paint(), svg.common());
                    svg.elements += 1;
                }
            }
        }

        // Clipping, as a rectangle. A real region needs a path this does not
        // reconstruct, and a too-generous clip shows more than it should rather
        // than hiding what it should not.
        // Clipping is deliberately NOT applied. GDI keeps its clip region in
        // DEVICE space, while these elements carry the world transform, and a
        // `clip-path` resolves in the referencing element's own user space. The
        // rect therefore lands somewhere else and eats content: it cut the
        // headings off one of the two real files. Unclipped draws too much,
        // which is the failure worth having.
        R::IntersectClipRect(_) => {}

        // Drawing.
        R::MoveToEx(m) => svg.state.pos = (m.point.x as f64, m.point.y as f64),
        R::LineTo(l) => {
            let from = svg.state.pos;
            let to = (l.point.x as f64, l.point.y as f64);
            svg.shape(&poly_d(&[from, to], false));
            svg.state.pos = to;
        }
        R::Polygon(p) => svg.shape(&poly_d(&pts_l(&p.points), true)),
        R::Polyline(p) => svg.shape(&poly_d(&pts_l(&p.points), false)),
        R::Polygon16(p) => svg.shape(&poly_d(&pts_s(&p.points), true)),
        R::Polyline16(p) => svg.shape(&poly_d(&pts_s(&p.points), false)),
        R::PolylineTo(p) => svg.shape(&poly_d(&pts_l(&p.points), false)),
        R::PolylineTo16(p) => svg.shape(&poly_d(&pts_s(&p.points), false)),
        R::PolyBezier(p) | R::PolyBezierTo(p) => svg.shape(&bezier_d(&pts_l(&p.points))),
        R::PolyBezier16(p) | R::PolyBezierTo16(p) => svg.shape(&bezier_d(&pts_s(&p.points))),
        R::PolyPolygon(p) => {
            let mut d = String::new();
            let mut at = 0usize;
            for count in &p.counts {
                let end = (at + *count as usize).min(p.points.len());
                d.push_str(&poly_d(&pts_l(&p.points[at..end]), true));
                at = end;
            }
            svg.shape(&d);
        }
        R::PolyPolygon16(p) => {
            let mut d = String::new();
            let mut at = 0usize;
            for count in &p.counts {
                let end = (at + *count as usize).min(p.points.len());
                d.push_str(&poly_d(&pts_s(&p.points[at..end]), true));
                at = end;
            }
            svg.shape(&d);
        }
        R::Rectangle(r) => svg.shape(&rect_d(&r.bounds)),
        R::Ellipse(e) => {
            let b = e.bounds;
            let (cx, cy) = (
                (b.left + b.right) as f64 / 2.0,
                (b.top + b.bottom) as f64 / 2.0,
            );
            let (rx, ry) = (
                (b.right - b.left).abs() as f64 / 2.0,
                (b.bottom - b.top).abs() as f64 / 2.0,
            );
            if svg.path.is_some() {
                // Two arcs make an ellipse inside a path.
                svg.shape(&format!(
                    "M{} {}A{} {} 0 1 0 {} {}A{} {} 0 1 0 {} {}Z",
                    n(cx - rx),
                    n(cy),
                    n(rx),
                    n(ry),
                    n(cx + rx),
                    n(cy),
                    n(rx),
                    n(ry),
                    n(cx - rx),
                    n(cy)
                ));
            } else {
                let _ = write!(
                    svg.body,
                    "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"{}{}/>",
                    n(cx),
                    n(cy),
                    n(rx),
                    n(ry),
                    svg.paint(),
                    svg.common()
                );
                svg.elements += 1;
            }
        }

        // A blit carrying no bitmap paints a rectangle: with the current
        // brush for PATCOPY, or flat black/white. That is how this format draws
        // a rule under a heading and a shaded band behind a row, so skipping
        // them lost every one.
        R::BitBlt(b) if b.bitmap.is_none() => {
            let fill = match b.raster_operation {
                0x00f0_0021 => svg.state.brush.colour.clone(),
                0x0000_0042 => Some("#000000".to_string()),
                0x00ff_0062 => Some("#ffffff".to_string()),
                _ => None,
            };
            if let Some(fill) = fill {
                let _ = write!(
                    svg.body,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\"{}/>",
                    n(b.dest.x as f64),
                    n(b.dest.y as f64),
                    n(b.dest_size.cx as f64),
                    n(b.dest_size.cy as f64),
                    svg.common()
                );
                svg.elements += 1;
            }
        }

        // Text.
        R::ExtTextOutW(t) | R::ExtTextOutA(t) => text(svg, &t.text),

        _ => {}
    }
}

fn text(svg: &mut Svg, t: &emfsdk::EmrText) {
    let raw = t.text.as_str().unwrap_or_default().to_string();
    let content = raw.trim_end_matches('\0');
    if content.trim().is_empty() {
        return;
    }
    let f = svg.state.font.clone();
    // TA_CENTER is 6 and TA_RIGHT 2 in the low bits; TA_BASELINE is 24, and
    // without it the reference point is the TOP of the cell.
    let anchor = match svg.state.text_align & 0x06 {
        6 => " text-anchor=\"middle\"",
        2 => " text-anchor=\"end\"",
        _ => "",
    };
    let baseline = if svg.state.text_align & 24 == 24 {
        ""
    } else {
        " dominant-baseline=\"text-before-edge\""
    };
    let mut extra = String::new();
    if f.weight >= 600 {
        let _ = write!(extra, " font-weight=\"{}\"", f.weight);
    }
    if f.italic {
        extra.push_str(" font-style=\"italic\"");
    }
    if f.underline {
        extra.push_str(" text-decoration=\"underline\"");
    }
    // Escapement turns text about its own reference point, in tenths of a
    // degree, counter-clockwise where SVG rotates clockwise.
    let (x, y) = (t.reference.x as f64, t.reference.y as f64);
    if f.escapement != 0 {
        let _ = write!(
            extra,
            " transform=\"rotate({} {} {})\"",
            n(-(f.escapement as f64) / 10.0),
            n(x),
            n(y)
        );
    }
    let _ = write!(
        svg.body,
        "<text x=\"{}\" y=\"{}\" font-family='{}' font-size=\"{}\" fill=\"{}\"\
         {anchor}{baseline}{extra}{}>{}</text>",
        n(x),
        n(y),
        f.family,
        n(f.size),
        svg.state.text_colour,
        svg.common(),
        esc(content)
    );
    svg.elements += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_from_a_document_is_escaped() {
        // A face name or a caption is data from an uploaded file, and this
        // output goes to a browser.
        let out = esc("</text><script>alert(1)</script>");
        assert!(!out.contains('<'), "{out}");
        assert!(!out.contains('>'), "{out}");
        assert_eq!(esc("a & b"), "a &amp; b");
        assert_eq!(esc("\"q\""), "&quot;q&quot;");
    }

    #[test]
    fn a_control_character_is_dropped_rather_than_emitted() {
        // Not valid XML at any escaping, and metafile strings carry them.
        assert_eq!(esc("a\u{1}b"), "ab");
        assert_eq!(esc("keep\ttab"), "keep\ttab");
    }

    #[test]
    fn a_windows_face_keeps_its_name_and_gains_a_substitute() {
        let stack = font_stack("Calibri");
        assert!(stack.starts_with("\"Calibri\""), "{stack}");
        assert!(stack.contains("Carlito"), "{stack}");
        assert!(font_stack("Courier New").contains("monospace"));
        assert!(font_stack("Times New Roman").contains("serif"));
        // No name at all still resolves to something.
        assert!(font_stack("").contains("sans-serif"));
    }

    #[test]
    fn a_null_pen_draws_no_stroke() {
        // PS_NULL is style 5, and a null pen that drew a hairline would put a
        // black outline around every filled shape in the drawing.
        let pen = pen_from(
            5,
            1.0,
            ColorRef {
                red: 1,
                green: 2,
                blue: 3,
                reserved: 0,
            },
        );
        assert!(pen.colour.is_none());
    }

    #[test]
    fn composing_transforms_matches_gdi_left_multiply() {
        // Translate by (10, 0), then scale 2x, left-multiplied: the translation
        // is scaled too, which is what MWT_LEFTMULTIPLY means.
        let translate = Xf {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 10.0,
            f: 0.0,
        };
        let scale = Xf {
            a: 2.0,
            b: 0.0,
            c: 0.0,
            d: 2.0,
            e: 0.0,
            f: 0.0,
        };
        let out = scale.times(translate);
        assert_eq!((out.a, out.d, out.e), (2.0, 2.0, 20.0));
    }
}

#[cfg(test)]
mod fixture {
    /// Renders a real metafile if one is placed beside the crate, for eyeballing
    /// the output. Ignored by default: the files are documents, not fixtures.
    #[test]
    #[ignore]
    fn dump() {
        for name in ["image23", "image24"] {
            let path = format!("/tmp/emf/{name}.emf");
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            match super::to_svg(&bytes) {
                Some(svg) => {
                    std::fs::write(format!("/tmp/emf/{name}.svg"), &svg).unwrap();
                    println!("{name}: {} bytes of svg", svg.len());
                }
                None => println!("{name}: declined"),
            }
        }
    }
}

//! Drawing one character of Unicode, for `unicrud`.
//!
//! The crate carries one bitmap font of Latin glyphs, which is enough for
//! every saver that puts words on the screen and nowhere near enough for one
//! that picks a codepoint anywhere in Unicode. Embedding a font that covered
//! it would be megabytes; the browser already has fonts, so it draws instead.
//!
//! The result is tightly cropped to the ink, because the saver scales the
//! glyph to fill the screen and wants its real proportions rather than a
//! font's line box.
//!
//! A codepoint no font here has is answered with `None`. Every browser draws
//! the missing-glyph box for those, and telling one from a real character has
//! to be done by looking: a character is compared against what the same font
//! draws for a codepoint in a private use area, which is never assigned and
//! so always renders as the box. Anything identical to it is treated as
//! absent, which is upstream's own test, one step earlier: it draws the
//! character and rejects it if the result comes out blank.

use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use xscreensaver::runtime::{XImage, color::rgb};

/// A codepoint in a private use area: unassigned by definition, so whatever
/// the browser draws for it is its missing-glyph box.
const NOTDEF_PROBE: u32 = 0x000F_FFFD;

/// Draw one codepoint, about `size` pixels tall, cropped to its ink.
pub fn render(codepoint: u32, size: i32) -> Option<XImage> {
    let ch = char::from_u32(codepoint)?;
    let notdef = char::from_u32(NOTDEF_PROBE)?;
    let ink = draw(ch, size)?;
    // A character that draws exactly what an unassigned one draws is one the
    // browser has no glyph for.
    if let Some(box_glyph) = draw(notdef, size)
        && box_glyph.0 == ink.0
    {
        return None;
    }
    let (_, image) = ink;
    image
}

/// Draw a character and return a hash of its pixels along with the cropped
/// image, or `None` for the image if nothing was drawn at all.
fn draw(ch: char, size: i32) -> Option<(u64, Option<XImage>)> {
    let window = web_sys::window()?;
    let document = window.document()?;
    let canvas: HtmlCanvasElement = document.create_element("canvas").ok()?.dyn_into().ok()?;
    // Room around the glyph: some characters draw well outside their box.
    let pad = size * 2;
    canvas.set_width(pad as u32);
    canvas.set_height(pad as u32);
    let ctx: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;

    // A stack wide enough to cover most of what gets asked for. The browser
    // walks it until one of them has the character.
    ctx.set_font(&format!(
        "{size}px \"Noto Sans\", \"Noto Sans CJK JP\", \"Noto Sans Symbols 2\", \
         \"Noto Color Emoji\", \"DejaVu Sans\", sans-serif"
    ));
    ctx.set_fill_style_str("#fff");
    ctx.set_text_baseline("middle");
    ctx.set_text_align("center");
    let mut buf = [0u8; 4];
    ctx.fill_text(
        ch.encode_utf8(&mut buf),
        f64::from(pad) / 2.0,
        f64::from(pad) / 2.0,
    )
    .ok()?;

    let data = ctx
        .get_image_data(0.0, 0.0, f64::from(pad), f64::from(pad))
        .ok()?
        .data();

    // The ink's bounding box, and a hash of the alpha channel so two
    // renderings can be compared.
    let (mut x0, mut y0, mut x1, mut y1) = (pad, pad, -1, -1);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for y in 0..pad {
        for x in 0..pad {
            let a = data[((y * pad + x) * 4 + 3) as usize];
            hash ^= u64::from(a);
            hash = hash.wrapping_mul(0x100_0000_01b3);
            if a > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x1 < x0 || y1 < y0 {
        return Some((hash, None)); /* nothing was drawn */
    }

    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut img = XImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let a = data[(((y + y0) * pad + (x + x0)) * 4 + 3) as usize];
            // The coverage goes in all three channels; the saver reads one of
            // them and supplies its own colour.
            img.put_pixel(x, y, rgb(a, a, a));
        }
    }
    Some((hash, Some(img)))
}

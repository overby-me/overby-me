//! Export a document (and any nested content) to an OpenDocument Text `.odt`
//! file, entirely in the browser. An ODT is a ZIP of ODF XML parts; we build a
//! minimal, valid one (mimetype + content.xml + styles.xml + manifest) with a
//! tiny stored-entry ZIP writer so no zip/compression crate is needed in WASM.
//! Mirrors the reference app's DOCX export, but to ODT.

use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

/// CRC-32 (IEEE, the ZIP variant) of `data`.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// One file to place in the archive.
struct ZipEntry {
    name: String,
    data: Vec<u8>,
}

/// Build a ZIP archive from `entries`, all **stored** (uncompressed). That is
/// all an ODT needs, and it keeps the writer dependency-free. The first entry
/// (the mimetype) must be stored and unpadded, which this guarantees.
fn build_zip(entries: &[ZipEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut offsets = Vec::new();

    for entry in entries {
        let crc = crc32(&entry.data);
        let size = entry.data.len() as u32;
        let name = entry.name.as_bytes();
        offsets.push(out.len() as u32);

        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // signature
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes()); // compressed size
        out.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(name);
        out.extend_from_slice(&entry.data);
    }

    for (i, entry) in entries.iter().enumerate() {
        let crc = crc32(&entry.data);
        let size = entry.data.len() as u32;
        let name = entry.name.as_bytes();

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // signature
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        central.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central.extend_from_slice(&0u16.to_le_bytes()); // mod date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra length
        central.extend_from_slice(&0u16.to_le_bytes()); // comment length
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offsets[i].to_le_bytes());
        central.extend_from_slice(name);
    }

    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);

    // End of central directory record.
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk number
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with central dir
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

/// Escape text for XML content.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// One Slate text leaf as escaped ODF, wrapped in spans/link for its marks
/// (bold, italic, underline, code, link). Soft breaks (`\n`) become
/// `<text:line-break/>`. Mirrors the old wiki's inline `format`.
fn leaf_to_odf(leaf: &Value) -> String {
    let raw = leaf.get("text").and_then(|t| t.as_str()).unwrap_or("");
    let mut s = xml_escape(raw).replace('\n', "<text:line-break/>");
    let mark = |key: &str| leaf.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    for (active, style) in [
        (mark("bold"), "Bold"),
        (mark("italic"), "Emphasis"),
        (mark("underline"), "Underline"),
        (mark("strikethrough"), "Strikethrough"),
        (mark("code"), "Code"),
    ] {
        if active {
            s = format!("<text:span text:style-name=\"{style}\">{s}</text:span>");
        }
    }
    if let Some(link) = leaf.get("link").and_then(|l| l.as_str()) {
        if !link.is_empty() {
            s = format!(
                "<text:a xlink:type=\"simple\" xlink:href=\"{}\">{s}</text:a>",
                xml_escape(link)
            );
        }
    }
    s
}

/// The inline content of a block: each child leaf run, concatenated.
fn inline_children(block: &Value) -> String {
    block
        .get("children")
        .and_then(|c| c.as_array())
        .map(|children| children.iter().map(leaf_to_odf).collect())
        .unwrap_or_default()
}

/// The base paragraph kind of a block, used to pick its named ODF style and,
/// when the block carries an `align`, the matching alignment automatic style.
enum Base {
    Paragraph,
    Heading(usize),
    Quote,
    Pre,
}

/// The `fo:text-align` value for a Slate `align`, or `None` for the default
/// (left / unset), which needs no override.
fn align_value(align: Option<&str>) -> Option<&'static str> {
    match align {
        Some("center") => Some("center"),
        Some("right") => Some("end"),
        Some("justify") => Some("justify"),
        _ => None,
    }
}

/// The style key (`p`, `h1`..`h6`, `quote`, `pre`) used to name a base's
/// alignment automatic styles; see [`automatic_styles`].
fn base_key(base: &Base) -> String {
    match base {
        Base::Paragraph => "p".to_string(),
        Base::Heading(l) => format!("h{}", (*l).clamp(1, 6)),
        Base::Quote => "quote".to_string(),
        Base::Pre => "pre".to_string(),
    }
}

/// The named ODF style for a base with no alignment (empty for a plain
/// paragraph, which needs no style attribute).
fn base_style(base: &Base) -> String {
    match base {
        Base::Paragraph => String::new(),
        Base::Heading(l) => format!("Heading_20_{}", (*l).clamp(1, 6)),
        Base::Quote => "Quotations".to_string(),
        Base::Pre => "Preformatted_20_Text".to_string(),
    }
}

/// Render a paragraph-like block (`inner` already XML-escaped) with its base
/// style and, if present, its alignment. Headings emit `<text:h>` with the
/// outline level; everything else emits `<text:p>`.
fn styled_block(base: Base, align: Option<&str>, inner: &str) -> String {
    // With an alignment, use the generated `Al_<base>_<value>` automatic style
    // (which inherits from the base style); otherwise the base style itself.
    let style = match align_value(align) {
        Some(value) => format!("Al_{}_{}", base_key(&base), value),
        None => base_style(&base),
    };
    match base {
        Base::Heading(level) => {
            let l = level.clamp(1, 6);
            let name = if style.is_empty() {
                format!("Heading_20_{l}")
            } else {
                style
            };
            format!(
                "<text:h text:style-name=\"{name}\" text:outline-level=\"{l}\">{inner}</text:h>"
            )
        }
        _ if style.is_empty() => format!("<text:p>{inner}</text:p>"),
        _ => format!("<text:p text:style-name=\"{style}\">{inner}</text:p>"),
    }
}

/// A styled ODF heading with no alignment (document title, node names).
fn heading_el(level: usize, inner: &str) -> String {
    styled_block(Base::Heading(level), None, inner)
}

/// A `<text:list>` with the given list style, one `<text:list-item>` per child.
fn list_to_odf(block: &Value, style: &str) -> String {
    let items: String = block
        .get("children")
        .and_then(|c| c.as_array())
        .map(|children| {
            children
                .iter()
                .map(|item| {
                    format!(
                        "<text:list-item><text:p>{}</text:p></text:list-item>",
                        inline_children(item)
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    format!("<text:list text:style-name=\"{style}\">{items}</text:list>")
}

/// One embedded picture in the ODT package: its in-package path (`Pictures/…`),
/// media type, and bytes.
struct Picture {
    path: String,
    media_type: &'static str,
    data: Vec<u8>,
}

/// Detect an image's format from its magic bytes: returns (file extension, ODF
/// media type, width px, height px). Covers PNG, JPEG and GIF — what the editor
/// and paste flows produce. `None` if unrecognised or dimensions can't be read.
fn image_info(bytes: &[u8]) -> Option<(&'static str, &'static str, u32, u32)> {
    // PNG: 8-byte signature, then IHDR (width/height big-endian at offset 16).
    if bytes.len() >= 24 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some(("png", "image/png", w, h));
    }
    // GIF: "GIF8", then width/height little-endian at offset 6.
    if bytes.len() >= 10 && &bytes[..4] == b"GIF8" {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some(("gif", "image/gif", w, h));
    }
    // JPEG: starts FF D8; scan segments for a Start-Of-Frame marker (C0..CF,
    // excluding the restart/APP markers), whose 5th+ bytes hold height then width.
    if bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            // SOF0..SOF3, SOF5..SOF7, SOF9..SOF11, SOF13..SOF15 carry dimensions.
            let is_sof = matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
            if is_sof {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some(("jpg", "image/jpeg", w, h));
            }
            // Skip this segment (2-byte length after the marker).
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            i += 2 + len;
        }
    }
    None
}

/// Convert pixel dimensions to the ODF frame's `svg:width`/`svg:height` in cm,
/// assuming 96 DPI and capping the width to the page's text area (~16.5cm) so a
/// large image doesn't overflow the margins.
fn frame_size_cm(w: u32, h: u32) -> (f64, f64) {
    const MAX_W: f64 = 16.5;
    let px_to_cm = |px: u32| px as f64 / 96.0 * 2.54;
    let (mut wc, mut hc) = (px_to_cm(w.max(1)), px_to_cm(h.max(1)));
    if wc > MAX_W {
        hc *= MAX_W / wc;
        wc = MAX_W;
    }
    (wc, hc)
}

/// Fetch an image's bytes for embedding. Returns `None` on any error (network,
/// CORS, etc.) so the caller can fall back to emitting the URL as a link.
async fn fetch_image_bytes(url: &str) -> Option<Vec<u8>> {
    let resp = reqwest::Client::new().get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.bytes().await.ok().map(|b| b.to_vec())
}

/// An `image` block, embedded into the ODT when its bytes can be fetched and
/// decoded (recorded in `pics`), else falling back to [`image_to_odf`] (a link).
/// Facebook-style emoji-image blocks are emitted as their character instead.
async fn image_block_to_odf(block: &Value, pics: &RefCell<Vec<Picture>>) -> String {
    let Some(url) = block
        .get("url")
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
    else {
        return String::new();
    };
    if let Some(emoji) = crate::components::content::emoji_from_image_url(url) {
        return format!("<text:p>{}</text:p>", xml_escape(&emoji));
    }
    let Some(bytes) = fetch_image_bytes(url).await else {
        return image_to_odf(block);
    };
    let Some((ext, media_type, w, h)) = image_info(&bytes) else {
        return image_to_odf(block);
    };
    let idx = pics.borrow().len() + 1;
    let path = format!("Pictures/image{idx}.{ext}");
    pics.borrow_mut().push(Picture {
        path: path.clone(),
        media_type,
        data: bytes,
    });
    let (wc, hc) = frame_size_cm(w, h);
    format!(
        "<text:p><draw:frame draw:name=\"img{idx}\" text:anchor-type=\"paragraph\" \
svg:width=\"{wc:.2}cm\" svg:height=\"{hc:.2}cm\" draw:z-index=\"0\">\
<draw:image xlink:href=\"{path}\" xlink:type=\"simple\" xlink:show=\"embed\" \
xlink:actuate=\"onLoad\"/></draw:frame></text:p>"
    )
}

/// A void `image` block: when the bytes can't be embedded, rather than lose the
/// reference we emit the source as a link.
fn image_to_odf(block: &Value) -> String {
    match block
        .get("url")
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
    {
        Some(url) => {
            let escaped = xml_escape(url);
            format!("<text:p><text:a xlink:type=\"simple\" xlink:href=\"{escaped}\">{escaped}</text:a></text:p>")
        }
        None => String::new(),
    }
}

/// Render one Slate block as ODF: headings, bulleted/numbered lists,
/// block-quotes, preformatted blocks, images and paragraphs, preserving inline
/// marks and per-block alignment. Mirrors the old wiki's editor schema.
fn block_to_odf(block: &Value) -> String {
    let ty = block
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("paragraph");
    let align = block.get("align").and_then(|a| a.as_str());
    match ty {
        "heading-one" | "h1" => styled_block(Base::Heading(1), align, &inline_children(block)),
        "heading-two" | "h2" => styled_block(Base::Heading(2), align, &inline_children(block)),
        "heading-three" | "h3" => styled_block(Base::Heading(3), align, &inline_children(block)),
        "heading-four" | "h4" => styled_block(Base::Heading(4), align, &inline_children(block)),
        "heading-five" | "h5" => styled_block(Base::Heading(5), align, &inline_children(block)),
        "heading-six" | "h6" => styled_block(Base::Heading(6), align, &inline_children(block)),
        "bulleted-list" | "ul" => list_to_odf(block, "ListBullet"),
        "numbered-list" | "ol" => list_to_odf(block, "ListNumber"),
        "block-quote" => styled_block(Base::Quote, align, &inline_children(block)),
        "block-pre" | "pre" => styled_block(Base::Pre, align, &inline_children(block)),
        "image" => image_to_odf(block),
        _ => styled_block(Base::Paragraph, align, &inline_children(block)),
    }
}

/// The `<office:automatic-styles>` block: one alignment style per (base kind ×
/// non-default alignment), each inheriting from the base's named style. Kept
/// small and always-present so [`block_to_odf`] can reference them by name
/// without threading a style collector through the recursion.
fn automatic_styles() -> String {
    let bases = [
        ("p", "Standard"),
        ("h1", "Heading_20_1"),
        ("h2", "Heading_20_2"),
        ("h3", "Heading_20_3"),
        ("h4", "Heading_20_4"),
        ("h5", "Heading_20_5"),
        ("h6", "Heading_20_6"),
        ("quote", "Quotations"),
        ("pre", "Preformatted_20_Text"),
    ];
    let mut out = String::from("<office:automatic-styles>");
    for (key, parent) in bases {
        for value in ["center", "end", "justify"] {
            out.push_str(&format!(
                "<style:style style:name=\"Al_{key}_{value}\" style:family=\"paragraph\" \
style:parent-style-name=\"{parent}\">\
<style:paragraph-properties fo:text-align=\"{value}\"/></style:style>"
            ));
        }
    }
    out.push_str("</office:automatic-styles>");
    out
}

/// Wrap a pre-built ODF text `body` in a full `content.xml`, including the
/// alignment automatic styles the body may reference.
fn wrap_content(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<office:document-content \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" \
xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" \
xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
office:version=\"1.2\">\
{}\
<office:body><office:text>{body}</office:text></office:body>\
</office:document-content>",
        automatic_styles()
    )
}

/// The ODF `content.xml` for a single document `title` + its Slate `content`.
/// Test-only: `export_tree` builds the body itself; this exists so the ODT
/// block mapping can be unit-tested.
#[cfg(test)]
fn content_xml(title: &str, content: Option<&Value>) -> String {
    let mut body = heading_el(1, &xml_escape(title));
    if let Some(Value::Array(blocks)) = content {
        for block in blocks {
            body.push_str(&block_to_odf(block));
        }
    }
    wrap_content(&body)
}

/// Named styles referenced by the exported content: `Heading 1`..`Heading 6`
/// (bold, graduated sizes, tied to their outline level), the `Bold` / `Emphasis`
/// / `Underline` / `Strikethrough` / `Code` run styles for inline marks, the
/// indented `Quotations` and monospace `Preformatted Text` paragraphs, and the
/// `ListBullet` / `ListNumber` list styles (so bulleted and numbered lists
/// actually show bullets and numbers). Without these the headings render as
/// plain body text and lists as run-on paragraphs. Mirrors the old wiki, where
/// `html-to-docx` mapped `<h1>`..`<h6>`, `<ul>`/`<ol>` and inline tags to their
/// Word equivalents. Per-block alignment is layered on top via the automatic
/// styles in [`automatic_styles`].
const STYLES_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<office:document-styles \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
office:version=\"1.2\"><office:styles>\
<style:style style:name=\"Standard\" style:family=\"paragraph\" style:class=\"text\"/>\
<style:style style:name=\"Heading\" style:family=\"paragraph\" \
style:parent-style-name=\"Standard\" style:next-style-name=\"Standard\" style:class=\"text\">\
<style:paragraph-properties fo:margin-top=\"0.166in\" fo:margin-bottom=\"0.083in\" \
fo:keep-with-next=\"always\"/><style:text-properties fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"Heading_20_1\" style:display-name=\"Heading 1\" \
style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"1\" \
style:class=\"text\"><style:text-properties fo:font-size=\"22pt\" fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"Heading_20_2\" style:display-name=\"Heading 2\" \
style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"2\" \
style:class=\"text\"><style:text-properties fo:font-size=\"18pt\" fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"Heading_20_3\" style:display-name=\"Heading 3\" \
style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"3\" \
style:class=\"text\"><style:text-properties fo:font-size=\"15pt\" fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"Heading_20_4\" style:display-name=\"Heading 4\" \
style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"4\" \
style:class=\"text\"><style:text-properties fo:font-size=\"13pt\" fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"Heading_20_5\" style:display-name=\"Heading 5\" \
style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"5\" \
style:class=\"text\"><style:text-properties fo:font-size=\"12pt\" fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"Heading_20_6\" style:display-name=\"Heading 6\" \
style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:default-outline-level=\"6\" \
style:class=\"text\"><style:text-properties fo:font-size=\"11pt\" fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"Emphasis\" style:family=\"text\">\
<style:text-properties fo:font-style=\"italic\"/></style:style>\
<style:style style:name=\"Bold\" style:family=\"text\">\
<style:text-properties fo:font-weight=\"bold\"/></style:style>\
<style:style style:name=\"Underline\" style:family=\"text\">\
<style:text-properties style:text-underline-style=\"solid\" \
style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\"/></style:style>\
<style:style style:name=\"Strikethrough\" style:family=\"text\">\
<style:text-properties style:text-line-through-style=\"solid\" \
style:text-line-through-type=\"single\"/></style:style>\
<style:style style:name=\"Code\" style:family=\"text\">\
<style:text-properties fo:font-family=\"Courier New\"/></style:style>\
<style:style style:name=\"Quotations\" style:family=\"paragraph\" \
style:parent-style-name=\"Standard\"><style:paragraph-properties fo:margin-left=\"0.5in\" \
fo:margin-right=\"0.5in\" fo:margin-top=\"0.083in\" fo:margin-bottom=\"0.083in\"/></style:style>\
<style:style style:name=\"Preformatted_20_Text\" style:display-name=\"Preformatted Text\" \
style:family=\"paragraph\" style:parent-style-name=\"Standard\">\
<style:text-properties fo:font-family=\"Courier New\"/></style:style>\
<text:list-style style:name=\"ListBullet\">\
<text:list-level-style-bullet text:level=\"1\" text:bullet-char=\"\u{2022}\">\
<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" \
text:list-tab-stop-position=\"0.5in\" fo:text-indent=\"-0.25in\" fo:margin-left=\"0.5in\"/>\
</style:list-level-properties></text:list-level-style-bullet>\
<text:list-level-style-bullet text:level=\"2\" text:bullet-char=\"\u{25E6}\">\
<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" \
text:list-tab-stop-position=\"1in\" fo:text-indent=\"-0.25in\" fo:margin-left=\"1in\"/>\
</style:list-level-properties></text:list-level-style-bullet>\
<text:list-level-style-bullet text:level=\"3\" text:bullet-char=\"\u{25AA}\">\
<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" \
text:list-tab-stop-position=\"1.5in\" fo:text-indent=\"-0.25in\" fo:margin-left=\"1.5in\"/>\
</style:list-level-properties></text:list-level-style-bullet>\
</text:list-style>\
<text:list-style style:name=\"ListNumber\">\
<text:list-level-style-number text:level=\"1\" style:num-suffix=\".\" style:num-format=\"1\">\
<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" \
text:list-tab-stop-position=\"0.5in\" fo:text-indent=\"-0.25in\" fo:margin-left=\"0.5in\"/>\
</style:list-level-properties></text:list-level-style-number>\
<text:list-level-style-number text:level=\"2\" style:num-suffix=\".\" style:num-format=\"1\">\
<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" \
text:list-tab-stop-position=\"1in\" fo:text-indent=\"-0.25in\" fo:margin-left=\"1in\"/>\
</style:list-level-properties></text:list-level-style-number>\
<text:list-level-style-number text:level=\"3\" style:num-suffix=\".\" style:num-format=\"1\">\
<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" \
text:list-tab-stop-position=\"1.5in\" fo:text-indent=\"-0.25in\" fo:margin-left=\"1.5in\"/>\
</style:list-level-properties></text:list-level-style-number>\
</text:list-style>\
</office:styles></office:document-styles>";

/// The `META-INF/manifest.xml`, listing the parts plus every embedded picture.
fn manifest_xml(pictures: &[Picture]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<manifest:manifest \
xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" \
manifest:version=\"1.2\">\
<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.2\" \
manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\
<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\
<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>",
    );
    for pic in pictures {
        out.push_str(&format!(
            "<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{}\"/>",
            pic.path, pic.media_type
        ));
    }
    out.push_str("</manifest:manifest>");
    out
}

/// Package a full `content.xml` string (and any embedded `pictures`) into a
/// complete `.odt` archive.
fn odt_from_content_xml(content: String, pictures: &[Picture]) -> Vec<u8> {
    let mut entries = vec![
        // The mimetype must be the first entry and stored uncompressed.
        ZipEntry {
            name: "mimetype".to_string(),
            data: b"application/vnd.oasis.opendocument.text".to_vec(),
        },
        ZipEntry {
            name: "content.xml".to_string(),
            data: content.into_bytes(),
        },
        ZipEntry {
            name: "styles.xml".to_string(),
            data: STYLES_XML.as_bytes().to_vec(),
        },
        ZipEntry {
            name: "META-INF/manifest.xml".to_string(),
            data: manifest_xml(pictures).into_bytes(),
        },
    ];
    for pic in pictures {
        entries.push(ZipEntry {
            name: pic.path.clone(),
            data: pic.data.clone(),
        });
    }
    build_zip(&entries)
}

/// Build a complete `.odt` for a single document `title` and its Slate `content`.
/// Test-only: `export_tree` builds multi-doc ODTs directly.
#[cfg(test)]
pub fn build_odt(title: &str, content: Option<&Value>) -> Vec<u8> {
    odt_from_content_xml(content_xml(title, content), &[])
}

/// Trigger a browser download of `bytes` as `filename` with the given `mime`.
/// Uses a base64 data URL (via `btoa`) so no Blob/URL web-sys features are
/// needed; fine for document-sized payloads.
pub fn download_bytes(filename: &str, mime: &str, bytes: &[u8]) {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else {
        return;
    };
    // btoa reads each char's code (must be < 256); map bytes 1:1 to chars.
    let binary: String = bytes.iter().map(|&b| b as char).collect();
    let Ok(b64) = window.btoa(&binary) else {
        return;
    };
    let href = format!("data:{mime};base64,{b64}");

    let Some(document) = window.document() else {
        return;
    };
    let Ok(el) = document.create_element("a") else {
        return;
    };
    let _ = el.set_attribute("href", &href);
    let _ = el.set_attribute("download", filename);
    if let Ok(anchor) = el.dyn_into::<web_sys::HtmlElement>() {
        anchor.click();
    }
}

/// Quote a CSV field per RFC 4180: wrap in double quotes when it contains a
/// comma, quote, or newline, doubling any inner quotes. The one canonical CSV
/// escaper (member roster + poll results both use it).
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Open the browser print dialog for the current page. The print stylesheet
/// (`@media print`) hides the app chrome, so the visible card (a closed poll's
/// results, a document) prints clean.
pub fn print_page() {
    if let Some(window) = web_sys::window() {
        let _ = window.print();
    }
}

/// The letter/number prefix for a heading (policy "A: ", change "1: "), by the
/// node's ordinal among same-type siblings.
fn heading_prefix(mime_id: Option<&str>, ordinal: Option<usize>) -> String {
    use crate::components::loader::index_letter;
    match (mime_id, ordinal) {
        (Some("vote/policy"), Some(i)) => format!("{}: ", index_letter(i)),
        (Some("vote/change"), Some(i)) => format!("{}: ", i + 1),
        _ => String::new(),
    }
}

/// The children to descend into for a recursive export, ordered like the folder
/// view. Structural / content-bearing mimes only (not files, polls, speak…).
fn export_children(
    children: &[crate::model::ChildNodeFields],
) -> Vec<crate::model::ChildNodeFields> {
    let mut out: Vec<_> = children
        .iter()
        .filter(|c| {
            matches!(
                c.mime_id.as_deref(),
                Some(
                    "wiki/folder"
                        | "wiki/document"
                        | "vote/policy"
                        | "vote/change"
                        | "vote/position"
                        | "vote/candidate"
                )
            )
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        a.index.cmp(&b.index).then_with(|| {
            let at = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
            let bt = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
            at.cmp(bt)
        })
    });
    out
}

/// The ODF body for a node and its structural subtree. Boxed: recursive future.
/// Embedded pictures are collected into `pics` (shared across the recursion) for
/// packaging alongside the document. `Rc` so the boxed `'static` future can own it.
fn build_body(
    token: Option<String>,
    node_id: String,
    level: usize,
    prefix: String,
    pics: Rc<RefCell<Vec<Picture>>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = String>>> {
    Box::pin(async move {
        let Ok(Some(node)) = crate::graphql::query_node_by_id(token.as_deref(), &node_id).await
        else {
            return String::new();
        };
        let heading = xml_escape(&format!("{prefix}{}", node.name));
        let mut out = heading_el(level, &heading);

        // "Proposed by": the node's members, unless it is a context (group/event).
        // Italic, matching the old wiki's `<i>` run.
        let is_context = node.mime.as_ref().map(|m| m.context).unwrap_or(false);
        if !is_context {
            let authors: Vec<String> = node
                .members
                .iter()
                .map(|m| m.label())
                .filter(|s| !s.is_empty())
                .collect();
            if !authors.is_empty() {
                out.push_str(&format!(
                    "<text:p><text:span text:style-name=\"Emphasis\">{}: {}</text:span></text:p>",
                    xml_escape(&crate::i18n::t("folder.proposedBy")),
                    xml_escape(&authors.join(", "))
                ));
            }
        }

        // The node's own Slate content. Image blocks are embedded (async fetch);
        // everything else renders synchronously.
        if let Some(Value::Array(blocks)) = node.data.as_ref().and_then(|d| d.0.get("content")) {
            for block in blocks {
                if block.get("type").and_then(|t| t.as_str()) == Some("image") {
                    out.push_str(&image_block_to_odf(block, &pics).await);
                } else {
                    out.push_str(&block_to_odf(block));
                }
            }
        }

        // Recurse into structural children.
        let children = export_children(&node.children);
        let ordinals = crate::components::loader::sibling_ordinals(&children);
        for (child, ordinal) in children.iter().zip(ordinals) {
            let child_prefix = heading_prefix(child.mime_id.as_deref(), ordinal);
            out.push_str(
                &build_body(
                    token.clone(),
                    child.id.0.clone(),
                    level + 1,
                    child_prefix,
                    pics.clone(),
                )
                .await,
            );
        }
        out
    })
}

/// Recursively export a node (document, policy or whole folder) and everything
/// nested under it to a single `.odt`, then start the download.
pub async fn export_tree(token: Option<String>, node_id: String, name: String) {
    let pics = Rc::new(RefCell::new(Vec::new()));
    let body = build_body(token, node_id, 1, String::new(), pics.clone()).await;
    let odt = odt_from_content_xml(wrap_content(&body), &pics.borrow());
    let filename = format!("{}.odt", sanitize_filename(&name));
    download_bytes(&filename, "application/vnd.oasis.opendocument.text", &odt);
}

/// A filesystem-safe version of a node name for the download filename.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "document".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vector() {
        // CRC-32 of "123456789" is 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn image_info_reads_png_gif_jpeg_dimensions() {
        // PNG: 8-byte signature, IHDR length + tag, then width/height big-endian.
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&256u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        assert_eq!(image_info(&png), Some(("png", "image/png", 256, 200)));

        // GIF: "GIF89a" then width/height little-endian.
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&320u16.to_le_bytes());
        gif.extend_from_slice(&300u16.to_le_bytes());
        assert_eq!(image_info(&gif), Some(("gif", "image/gif", 320, 300)));

        // JPEG: FFD8, then an SOF0 segment carrying height then width.
        let jpg = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0x2C, 0x02, 0x58, 0, 0,
        ];
        assert_eq!(image_info(&jpg), Some(("jpg", "image/jpeg", 600, 300)));

        assert_eq!(image_info(b"not an image at all"), None);
    }

    #[test]
    fn frame_size_caps_width_and_keeps_aspect() {
        // A huge image is capped to the page text width, height scaled to match.
        let (w, h) = frame_size_cm(9600, 4800);
        assert!((w - 16.5).abs() < 0.01, "width capped: {w}");
        assert!((h - 8.25).abs() < 0.05, "height keeps 2:1 aspect: {h}");
    }

    #[test]
    fn sanitize_filename_keeps_safe_chars_and_falls_back() {
        assert_eq!(sanitize_filename("My File 2024"), "My_File_2024");
        assert_eq!(sanitize_filename("a/b\\c:d*e?"), "a_b_c_d_e_");
        assert_eq!(sanitize_filename("keep-these_1"), "keep-these_1");
        // Unicode letters count as alphanumeric and are preserved.
        assert_eq!(sanitize_filename("Ærø"), "Ærø");
        // Empty or all-separator names fall back to a safe default.
        assert_eq!(sanitize_filename("   "), "document");
        assert_eq!(sanitize_filename(""), "document");
    }

    #[test]
    fn odt_is_a_zip_with_mimetype_first() {
        let content = serde_json::json!([
            {"type": "paragraph", "children": [{"text": "Hello <world> & \"you\""}]},
            {"type": "heading-two", "children": [{"text": "A section"}]},
        ]);
        let odt = build_odt("My Doc", Some(&content));
        // Local file header signature at the start.
        assert_eq!(&odt[0..4], &[0x50, 0x4b, 0x03, 0x04]);
        // The first entry is the mimetype (its name follows the 30-byte header).
        assert_eq!(&odt[30..38], b"mimetype");
        // End-of-central-directory signature present.
        assert!(odt.windows(4).any(|w| w == [0x50, 0x4b, 0x05, 0x06]));
    }

    #[test]
    fn content_xml_escapes_and_maps_blocks() {
        let content = serde_json::json!([
            {"type": "paragraph", "children": [{"text": "a & b"}, {"text": "\n"}, {"text": "c"}]},
            {"type": "heading-one", "children": [{"text": "Title"}]},
        ]);
        let xml = content_xml("Doc", Some(&content));
        assert!(xml.contains("a &amp; b<text:line-break/>c"));
        assert!(xml.contains(
            "<text:h text:style-name=\"Heading_20_1\" text:outline-level=\"1\">Doc</text:h>"
        ));
        assert!(xml.contains(
            "<text:h text:style-name=\"Heading_20_1\" text:outline-level=\"1\">Title</text:h>"
        ));
        assert!(xml.contains("<text:p>"));
    }

    #[test]
    fn heading_carries_style_and_outline_level() {
        assert_eq!(
            heading_el(2, "Hi"),
            "<text:h text:style-name=\"Heading_20_2\" text:outline-level=\"2\">Hi</text:h>"
        );
        // Levels clamp into the 1..=6 range that styles.xml defines.
        assert!(heading_el(9, "x").contains("Heading_20_6"));
        assert!(heading_el(0, "x").contains("Heading_20_1"));
    }

    #[test]
    fn lists_and_marks_convert() {
        let content = serde_json::json!([
            {"type": "bulleted-list", "children": [
                {"type": "list-item", "children": [{"text": "first"}]},
                {"type": "list-item", "children": [{"text": "second"}]},
            ]},
            {"type": "numbered-list", "children": [
                {"type": "list-item", "children": [{"text": "one"}]},
            ]},
            {"type": "paragraph", "children": [
                {"text": "b", "bold": true},
                {"text": "i", "italic": true},
                {"text": "link", "link": "https://github.com/example/page"},
            ]},
        ]);
        let xml = content_xml("Doc", Some(&content));
        // Bulleted list: one <text:list-item><text:p> per item.
        assert!(xml.contains(
            "<text:list text:style-name=\"ListBullet\">\
<text:list-item><text:p>first</text:p></text:list-item>\
<text:list-item><text:p>second</text:p></text:list-item></text:list>"
        ));
        // Numbered list uses the ListNumber style.
        assert!(xml.contains(
            "<text:list text:style-name=\"ListNumber\">\
<text:list-item><text:p>one</text:p></text:list-item></text:list>"
        ));
        // Inline marks become spans / a link.
        assert!(xml.contains("<text:span text:style-name=\"Bold\">b</text:span>"));
        assert!(xml.contains("<text:span text:style-name=\"Emphasis\">i</text:span>"));
        assert!(xml.contains(
            "<text:a xlink:type=\"simple\" xlink:href=\"https://github.com/example/page\">link</text:a>"
        ));
    }

    #[test]
    fn styles_define_the_styles_they_reference() {
        // Every heading level referenced by `heading_el` must have a matching
        // style definition, or the headers render as plain text.
        for level in 1..=6 {
            assert!(STYLES_XML.contains(&format!("style:name=\"Heading_20_{level}\"")));
            assert!(STYLES_XML.contains(&format!("style:display-name=\"Heading {level}\"")));
        }
        // Run/paragraph/list styles referenced by the block + leaf converters.
        for style in [
            "Emphasis",
            "Bold",
            "Underline",
            "Strikethrough",
            "Code",
            "Quotations",
            "Preformatted_20_Text",
            "ListBullet",
            "ListNumber",
        ] {
            assert!(
                STYLES_XML.contains(&format!("style:name=\"{style}\"")),
                "missing style {style}"
            );
        }
    }

    #[test]
    fn preformatted_strikethrough_and_alignment_convert() {
        let content = serde_json::json!([
            {"type": "block-pre", "children": [{"text": "code()"}]},
            {"type": "paragraph", "children": [{"text": "s", "strikethrough": true}]},
            {"type": "paragraph", "align": "center", "children": [{"text": "mid"}]},
            {"type": "heading-two", "align": "right", "children": [{"text": "Title"}]},
            {"type": "image", "url": "https://github.com/example/pic.png", "children": [{"text": ""}]},
        ]);
        let xml = content_xml("Doc", Some(&content));
        // block-pre uses the monospace preformatted paragraph style.
        assert!(xml.contains("<text:p text:style-name=\"Preformatted_20_Text\">code()</text:p>"));
        // Strikethrough mark.
        assert!(xml.contains("<text:span text:style-name=\"Strikethrough\">s</text:span>"));
        // Centered paragraph and right-aligned heading use alignment auto styles.
        assert!(xml.contains("<text:p text:style-name=\"Al_p_center\">mid</text:p>"));
        assert!(xml.contains(
            "<text:h text:style-name=\"Al_h2_end\" text:outline-level=\"2\">Title</text:h>"
        ));
        // The alignment auto styles are declared and inherit from the base style.
        assert!(xml.contains(
            "<style:style style:name=\"Al_h2_end\" style:family=\"paragraph\" \
style:parent-style-name=\"Heading_20_2\"><style:paragraph-properties \
fo:text-align=\"end\"/></style:style>"
        ));
        // An image is preserved as a link to its source.
        assert!(xml.contains(
            "<text:a xlink:type=\"simple\" \
xlink:href=\"https://github.com/example/pic.png\">\
https://github.com/example/pic.png</text:a>"
        ));
    }
}

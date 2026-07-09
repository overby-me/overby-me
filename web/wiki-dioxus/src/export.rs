//! Export a document (and any nested content) to an OpenDocument Text `.odt`
//! file, entirely in the browser. An ODT is a ZIP of ODF XML parts; we build a
//! minimal, valid one (mimetype + content.xml + styles.xml + manifest) with a
//! tiny stored-entry ZIP writer so no zip/compression crate is needed in WASM.
//! Mirrors the reference app's DOCX export, but to ODT.

use serde_json::Value;

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
    name: &'static str,
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

/// All leaf text of a Slate node, concatenated (soft breaks kept as `\n`).
fn block_text(node: &Value) -> String {
    let mut out = String::new();
    if let Some(t) = node.get("text").and_then(|t| t.as_str()) {
        out.push_str(t);
    }
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            out.push_str(&block_text(child));
        }
    }
    out
}

/// A styled ODF heading: carries both the `Heading N` common style (so it
/// renders as a real header) and the matching outline level (so it shows up in
/// the document structure). `inner` must already be XML-escaped.
fn heading_el(level: usize, inner: &str) -> String {
    let l = level.clamp(1, 6);
    format!(
        "<text:h text:style-name=\"Heading_20_{l}\" text:outline-level=\"{l}\">{inner}</text:h>"
    )
}

/// Render one Slate block as an ODF `<text:h>` / `<text:p>` element, with soft
/// breaks (`\n`) mapped to `<text:line-break/>`.
fn block_to_odf(block: &Value) -> String {
    let text = block_text(block);
    let escaped = xml_escape(&text).replace('\n', "<text:line-break/>");
    let ty = block
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("paragraph");
    let level = match ty {
        "heading-one" | "h1" => Some(1),
        "heading-two" | "h2" => Some(2),
        "heading-three" | "h3" => Some(3),
        "heading-four" | "h4" => Some(4),
        "heading-five" | "h5" => Some(5),
        "heading-six" | "h6" => Some(6),
        _ => None,
    };
    match level {
        Some(l) => heading_el(l, &escaped),
        None => format!("<text:p>{escaped}</text:p>"),
    }
}

/// Wrap a pre-built ODF text `body` in a full `content.xml`.
fn wrap_content(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<office:document-content \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
office:version=\"1.2\">\
<office:body><office:text>{body}</office:text></office:body>\
</office:document-content>"
    )
}

/// The ODF `content.xml` for a single document `title` + its Slate `content`.
fn content_xml(title: &str, content: Option<&Value>) -> String {
    let mut body = heading_el(1, &xml_escape(title));
    if let Some(Value::Array(blocks)) = content {
        for block in blocks {
            body.push_str(&block_to_odf(block));
        }
    }
    wrap_content(&body)
}

/// Named common styles: `Heading 1`..`Heading 6` (bold, graduated sizes, tied to
/// their outline level) plus an italic `Emphasis` run style. Without these, the
/// `<text:h>` elements are structurally headings but render as plain body text,
/// which is why an export previously looked like it had no headers. Mirrors the
/// old wiki, where `html-to-docx` mapped `<h1>`..`<h6>` to Word heading styles.
const STYLES_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<office:document-styles \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
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
</office:styles></office:document-styles>";

const MANIFEST_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<manifest:manifest \
xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" \
manifest:version=\"1.2\">\
<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.2\" \
manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\
<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\
<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>\
</manifest:manifest>";

/// Package a full `content.xml` string into a complete `.odt` archive.
fn odt_from_content_xml(content: String) -> Vec<u8> {
    let entries = [
        // The mimetype must be the first entry and stored uncompressed.
        ZipEntry {
            name: "mimetype",
            data: b"application/vnd.oasis.opendocument.text".to_vec(),
        },
        ZipEntry {
            name: "content.xml",
            data: content.into_bytes(),
        },
        ZipEntry {
            name: "styles.xml",
            data: STYLES_XML.as_bytes().to_vec(),
        },
        ZipEntry {
            name: "META-INF/manifest.xml",
            data: MANIFEST_XML.as_bytes().to_vec(),
        },
    ];
    build_zip(&entries)
}

/// Build a complete `.odt` for a single document `title` and its Slate `content`.
pub fn build_odt(title: &str, content: Option<&Value>) -> Vec<u8> {
    odt_from_content_xml(content_xml(title, content))
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
    children: &[crate::graphql::ChildNodeFields],
) -> Vec<crate::graphql::ChildNodeFields> {
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
fn build_body(
    token: Option<String>,
    node_id: String,
    level: usize,
    prefix: String,
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

        // The node's own Slate content.
        if let Some(Value::Array(blocks)) = node.data.as_ref().and_then(|d| d.0.get("content")) {
            for block in blocks {
                out.push_str(&block_to_odf(block));
            }
        }

        // Recurse into structural children.
        let children = export_children(&node.children);
        let ordinals = crate::components::loader::sibling_ordinals(&children);
        for (child, ordinal) in children.iter().zip(ordinals) {
            let child_prefix = heading_prefix(child.mime_id.as_deref(), ordinal);
            out.push_str(
                &build_body(token.clone(), child.id.0.clone(), level + 1, child_prefix).await,
            );
        }
        out
    })
}

/// Recursively export a node (document, policy or whole folder) and everything
/// nested under it to a single `.odt`, then start the download.
pub async fn export_tree(token: Option<String>, node_id: String, name: String) {
    let body = build_body(token, node_id, 1, String::new()).await;
    let odt = odt_from_content_xml(wrap_content(&body));
    let filename = format!("{}.odt", sanitize_filename(&name));
    download_bytes(&filename, "application/vnd.oasis.opendocument.text", &odt);
}

/// A filesystem-safe version of a node name for the download filename.
fn sanitize_filename(name: &str) -> String {
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
    fn styles_define_the_heading_styles_they_reference() {
        // Every heading level referenced by `heading_el` must have a matching
        // style definition, or the headers render as plain text.
        for level in 1..=6 {
            assert!(STYLES_XML.contains(&format!("style:name=\"Heading_20_{level}\"")));
            assert!(STYLES_XML.contains(&format!("style:display-name=\"Heading {level}\"")));
        }
        assert!(STYLES_XML.contains("style:name=\"Emphasis\""));
    }
}

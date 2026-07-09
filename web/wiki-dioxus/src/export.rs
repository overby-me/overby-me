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
        Some(l) => format!("<text:h text:outline-level=\"{l}\">{escaped}</text:h>"),
        None => format!("<text:p>{escaped}</text:p>"),
    }
}

/// The ODF `content.xml` body for a document `title` + its Slate `content`.
fn content_xml(title: &str, content: Option<&Value>) -> String {
    let mut body = format!(
        "<text:h text:outline-level=\"1\">{}</text:h>",
        xml_escape(title)
    );
    if let Some(Value::Array(blocks)) = content {
        for block in blocks {
            body.push_str(&block_to_odf(block));
        }
    }
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

const STYLES_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<office:document-styles \
xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
office:version=\"1.2\"><office:styles/></office:document-styles>";

const MANIFEST_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<manifest:manifest \
xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" \
manifest:version=\"1.2\">\
<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.2\" \
manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\
<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\
<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>\
</manifest:manifest>";

/// Build a complete `.odt` for a document `title` and its Slate `content`.
pub fn build_odt(title: &str, content: Option<&Value>) -> Vec<u8> {
    let entries = [
        // The mimetype must be the first entry and stored uncompressed.
        ZipEntry {
            name: "mimetype",
            data: b"application/vnd.oasis.opendocument.text".to_vec(),
        },
        ZipEntry {
            name: "content.xml",
            data: content_xml(title, content).into_bytes(),
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

/// Export a document node to `.odt` and start the download.
pub fn export_document(name: &str, data: Option<&Value>) {
    let content = data.and_then(|d| d.get("content"));
    let odt = build_odt(name, content);
    let filename = format!("{}.odt", sanitize_filename(name));
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
        assert!(xml.contains("<text:h text:outline-level=\"1\">Doc</text:h>"));
        assert!(xml.contains("<text:h text:outline-level=\"1\">Title</text:h>"));
        assert!(xml.contains("<text:p>"));
    }
}

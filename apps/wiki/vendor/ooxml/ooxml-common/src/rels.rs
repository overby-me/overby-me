//! OPC relationship (`.rels`) parsing and part-name resolution shared by the
//! docx, pptx and xlsx parsers.
//!
//! Every OOXML package resolves references through the Open Packaging
//! Conventions relationship grammar (ECMA-376 Part 2 §9.3, ISO/IEC 29500-2):
//! a `_rels/<part>.rels` file lists `<Relationship Id Type Target
//! TargetMode?>` entries, and a part references another by relationship id
//! (`r:id` / `r:embed`). The three parsers had three near-identical private
//! copies of "parse the rels map" and "resolve a Target against the source
//! part's directory". Sharing them keeps path resolution byte-identical across
//! formats — notably the `../` normalization that docx's private copy was
//! missing (it concatenated `base_dir + target` verbatim, leaving
//! `word/charts/../media/image.png` unresolved for chart / footnote media).
//!
//! Scope is deliberately "types + parse + pure predicate": this module computes
//! the *zip part name* a relationship points at. It does not read bytes, does
//! not extract media, and does not know about any host schema — each parser
//! keeps its own media pipeline and only borrows the resolution logic.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// ECMA-376 Part 2 §9.3.2 `TargetMode`: whether a relationship's `Target`
/// names a part *inside* the package (`Internal`, the default) or an external
/// resource such as a hyperlink URL (`External`). External targets are opaque
/// URLs and must never be run through part-name resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetMode {
    /// `TargetMode="Internal"` (or omitted). `Target` is a package part name,
    /// relative to the source part's directory (or root-absolute with a leading
    /// `/`); resolve it with [`resolve_target`].
    Internal,
    /// `TargetMode="External"`. `Target` is an absolute URI (hyperlink, linked
    /// image, etc.) that is used verbatim — never resolved as a zip path.
    External,
}

/// A single parsed relationship: its `Target` string exactly as authored, plus
/// its [`TargetMode`]. The target is kept raw (unresolved) so callers can decide
/// whether to resolve it as a part name (Internal) or use it as a URL
/// (External); resolution against a base directory is a separate step
/// ([`resolve_target`]) because the base differs per source part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelTarget {
    /// The `Target` attribute verbatim (e.g. `../media/image1.png`,
    /// `/word/media/image1.png`, or `https://example.com/`).
    pub target: String,
    /// Internal (package part) vs External (opaque URL).
    pub mode: TargetMode,
}

/// Parse a `.rels` XML document into a `rId → `[`RelTarget`] map.
///
/// Reads every `<Relationship>` child of the root `<Relationships>` element
/// that carries both an `Id` and a `Target`, recording its `TargetMode`
/// (`External` when the attribute equals `"External"` case-insensitively, else
/// `Internal` — the spec default). Malformed XML or an empty string yields an
/// empty map. The returned [`BTreeMap`] keeps ids in sorted order so any
/// serialized form is deterministic.
///
/// This does not resolve targets to part names — Targets are stored verbatim;
/// call [`resolve_target`] per source part for Internal entries.
pub fn parse_rels(xml: &str) -> BTreeMap<String, RelTarget> {
    let mut map = BTreeMap::new();
    if xml.is_empty() {
        return map;
    }
    let doc = match crate::depth::parse_guarded(xml) {
        Ok(d) => d,
        Err(_) => return map,
    };
    for rel in doc
        .root_element()
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Relationship")
    {
        let (Some(id), Some(target)) = (rel.attribute("Id"), rel.attribute("Target")) else {
            continue;
        };
        // TargetMode is optional; only "External" (case-insensitive per the OPC
        // schema's xsd:string enumeration usage in the wild) diverts a target
        // away from part-name resolution.
        let mode = match rel.attribute("TargetMode") {
            Some(m) if m.eq_ignore_ascii_case("External") => TargetMode::External,
            _ => TargetMode::Internal,
        };
        map.insert(
            id.to_string(),
            RelTarget {
                target: target.to_string(),
                mode,
            },
        );
    }
    map
}

/// Resolve an OPC relationship `Target` to a normalized zip part name.
///
/// Two cases, both anchored at the package root (ECMA-376 Part 2 §9.3 — part
/// names are root-relative, `/`-separated, with no `.`/`..` segments in their
/// canonical form):
///
/// - **Root-absolute** (`Target` starts with `/`, e.g. openpyxl's
///   `/xl/drawings/drawing1.xml` or `/word/media/image1.png`): resolved from the
///   package root, ignoring `base_dir`. The leading slash is dropped so the
///   result is a bare part name that matches a zip entry.
/// - **Relative** (`../media/image1.png`, `slide1.xml`): resolved against
///   `base_dir` — the *directory* of the source part (e.g. `xl/drawings` for
///   `xl/drawings/_rels/drawing1.xml.rels`). `base_dir` may carry a trailing
///   slash; empty segments are dropped so both `word/` and `xl/worksheets`
///   forms work.
///
/// `..` pops one segment and `.` / empty segments are skipped, yielding a
/// normalized name with no relative components — so `word/charts` +
/// `../media/x.png` becomes `word/media/x.png`, never the unresolved
/// `word/charts/../media/x.png`.
pub fn resolve_target(base_dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        // Root-absolute part name: ignore base_dir entirely.
        Vec::new()
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

impl RelTarget {
    /// Resolve this relationship's target against `base_dir`, honoring
    /// [`TargetMode`]: Internal targets are normalized to a part name via
    /// [`resolve_target`]; External targets (URLs) are returned verbatim.
    pub fn resolve(&self, base_dir: &str) -> String {
        match self.mode {
            TargetMode::Internal => resolve_target(base_dir, &self.target),
            TargetMode::External => self.target.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_id_target_and_mode() {
        let xml = r#"<?xml version="1.0"?>
        <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1" Type="…/image" Target="../media/image1.png"/>
          <Relationship Id="rId2" Type="…/hyperlink" Target="https://example.com/" TargetMode="External"/>
        </Relationships>"#;
        let map = parse_rels(xml);
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("rId1"),
            Some(&RelTarget {
                target: "../media/image1.png".to_string(),
                mode: TargetMode::Internal,
            })
        );
        assert_eq!(
            map.get("rId2"),
            Some(&RelTarget {
                target: "https://example.com/".to_string(),
                mode: TargetMode::External,
            })
        );
    }

    #[test]
    fn parse_defaults_missing_target_mode_to_internal() {
        let xml = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1" Target="slide1.xml"/>
        </Relationships>"#;
        assert_eq!(
            parse_rels(xml).get("rId1").unwrap().mode,
            TargetMode::Internal
        );
    }

    #[test]
    fn parse_empty_or_malformed_is_empty() {
        assert!(parse_rels("").is_empty());
        assert!(parse_rels("<not xml").is_empty());
    }

    #[test]
    fn resolve_relative_target_against_base_dir() {
        // The everyday case: a part references a sibling directory's media.
        assert_eq!(
            resolve_target("ppt/slides", "../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(
            resolve_target("xl/worksheets", "../drawings/drawing1.xml"),
            "xl/drawings/drawing1.xml"
        );
    }

    #[test]
    fn resolve_absolute_leading_slash_ignores_base_dir() {
        // Root-absolute Targets (openpyxl style) resolve from the package root.
        assert_eq!(
            resolve_target("ppt/slides", "/ppt/charts/chart5.xml"),
            "ppt/charts/chart5.xml"
        );
        assert_eq!(
            resolve_target("xl/drawings", "/xl/charts/chart1.xml"),
            "xl/charts/chart1.xml"
        );
        // A trailing-slash base (docx's "word/" convention) is irrelevant for
        // absolute targets and must not leak in.
        assert_eq!(
            resolve_target("word/", "/word/media/image1.png"),
            "word/media/image1.png"
        );
    }

    #[test]
    fn resolve_multi_level_dotdot_normalizes() {
        // Deeply nested relative targets fully normalize — this is exactly the
        // case docx's old `format!("{}{}", base_dir, target)` left unresolved.
        assert_eq!(
            resolve_target("word/charts", "../../media/deep.png"),
            "media/deep.png"
        );
        assert_eq!(
            resolve_target("word/charts", "../media/chart_img.png"),
            "word/media/chart_img.png"
        );
        // Trailing-slash base with a single `..` (docx "word/" convention).
        assert_eq!(
            resolve_target("word/", "../media/footnote.png"),
            "media/footnote.png"
        );
    }

    #[test]
    fn resolve_via_rel_target_honors_external() {
        let internal = RelTarget {
            target: "../media/image1.png".to_string(),
            mode: TargetMode::Internal,
        };
        assert_eq!(internal.resolve("ppt/slides"), "ppt/media/image1.png");
        let external = RelTarget {
            target: "https://example.com/x".to_string(),
            mode: TargetMode::External,
        };
        // External targets pass through untouched regardless of base_dir.
        assert_eq!(external.resolve("ppt/slides"), "https://example.com/x");
    }
}

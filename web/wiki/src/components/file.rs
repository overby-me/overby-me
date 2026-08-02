use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;
use crate::session::use_session;

use super::loader::node_icon_el;

/// Office document mimes (legacy + OpenXML) previewable via the MS Office viewer:
/// Word, Excel and PowerPoint.
fn is_office_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/msword"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.ms-excel"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-powerpoint"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            | ODT
            | ODS
            | ODP
    )
}

/// OpenDocument: what LibreOffice writes.
pub const ODT: &str = "application/vnd.oasis.opendocument.text";
pub const ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";
pub const ODP: &str = "application/vnd.oasis.opendocument.presentation";

/// Whether `mime` is OpenDocument rather than OOXML.
pub fn is_opendocument(mime: &str) -> bool {
    matches!(mime, ODT | ODS | ODP)
}

/// The state of the backend-signed link the Office viewer fetches on. Kept
/// apart from "no URL yet" so a refusal shows the no-preview state instead of a
/// spinner that never stops.
#[derive(Clone, PartialEq)]
enum OfficeLink {
    Pending,
    Ready(String),
    Refused,
}

/// Who renders a Word, Excel or PowerPoint file.
///
/// Neither is us: this app cannot render OpenXML, so a preview means handing the
/// document to somebody who can. Both fetch it from the signed backend link, so
/// the choice is only WHICH third party sees it — worth being able to answer,
/// since the answer used to be "Microsoft, always".
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OfficeViewer {
    Microsoft,
    Google,
    /// Rendered here, by this app, from the file's own bytes. Offered only for
    /// the formats it can actually read (see `renders_natively`).
    Native,
}

impl OfficeViewer {
    fn key(self) -> &'static str {
        match self {
            OfficeViewer::Microsoft => "microsoft",
            OfficeViewer::Google => "google",
            OfficeViewer::Native => "native",
        }
    }

    /// What a reader is offered this viewer as.
    pub fn label_key(self) -> &'static str {
        match self {
            OfficeViewer::Microsoft => "file.viewerMicrosoft",
            OfficeViewer::Google => "file.viewerGoogle",
            OfficeViewer::Native => "file.viewerNative",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "google" => OfficeViewer::Google,
            "microsoft" => OfficeViewer::Microsoft,
            // This app renders the document itself, which is the one option
            // that sends the file to nobody, so it is what an unset or
            // unreadable preference gets. A preference already stored is still
            // read back and still honoured.
            _ => OfficeViewer::Native,
        }
    }
}

/// Where the chosen viewer is asked to render `encoded_src`, which is the signed
/// backend link, URI-component encoded.
///
/// Pure and separate from the component so the two URL shapes can be tested; a
/// wrong parameter name here is a viewer that shows an error page, and neither
/// service tells you which parameter it wanted.
pub fn viewer_embed_url(viewer: OfficeViewer, encoded_src: &str) -> String {
    match viewer {
        OfficeViewer::Microsoft => {
            format!("https://view.officeapps.live.com/op/embed.aspx?src={encoded_src}")
        }
        // `gview` is the free one, and the reason this choice exists. It wants
        // `url`, not `src`, and `embedded=true` or it serves a full page with
        // Google chrome around it.
        OfficeViewer::Google => {
            format!("https://docs.google.com/gview?embedded=true&url={encoded_src}")
        }
        // Native has no embed URL: nothing is embedded. Falling back to
        // Microsoft keeps this total rather than panicking, and it is only
        // reachable for a format the native path declined.
        OfficeViewer::Native => {
            format!("https://view.officeapps.live.com/op/embed.aspx?src={encoded_src}")
        }
    }
}

/// Whether this app can render `mime` itself.
///
/// All three OOXML formats now. A format that is not on this list keeps the
/// embedded viewers, and the native option is not offered for it at all — an
/// option that silently does nothing is worse than no option.
pub fn renders_natively(mime: &str) -> bool {
    matches!(
        mime,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            // OpenDocument text only, so far. An ODF spreadsheet or deck still
            // goes to Google, which reads both families.
            | ODT
    )
}

/// A short, readable name for a file's type.
///
/// The chip used to show the mime SUBTYPE in capitals, which for the OpenXML
/// family is
/// `VND.OPENXMLFORMATS-OFFICEDOCUMENT.WORDPROCESSINGML.DOCUMENT` — forty
/// characters of registry plumbing where the reader wanted the word "Word".
///
/// The names are deliberately not translated: they are product names (Word,
/// Excel) or acronyms (PDF, ZIP), which stay as they are in Danish too. A word
/// that WOULD need translating is a sign the label is trying to say too much.
///
/// Falls back to the file's own extension before it falls back to the mime,
/// because a name a person chose beats a registry string every time.
pub fn type_label(mime: &str, file_name: &str) -> String {
    let known = match mime {
        "application/pdf" => Some("PDF"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        | "application/msword" => Some("Word"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.ms-excel" => Some("Excel"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        | "application/vnd.ms-powerpoint" => Some("PowerPoint"),
        ODT => Some("ODT"),
        ODS => Some("ODS"),
        ODP => Some("ODP"),
        "application/zip" => Some("ZIP"),
        "text/plain" => Some("Text"),
        "text/csv" => Some("CSV"),
        "image/jpeg" => Some("JPEG"),
        "image/png" => Some("PNG"),
        "image/webp" => Some("WebP"),
        "image/heic" | "image/heif" => Some("HEIC"),
        "image/gif" => Some("GIF"),
        "image/svg+xml" => Some("SVG"),
        "audio/midi" | "audio/x-midi" => Some("MIDI"),
        "video/mp4" => Some("MP4"),
        _ => None,
    };
    if let Some(label) = known {
        return label.to_string();
    }

    // The extension the person actually named the file with, when it is short
    // enough to be one.
    if let Some(ext) = file_name.rsplit_once('.').map(|(_, e)| e) {
        if (1..=5).contains(&ext.chars().count()) && ext.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return ext.to_uppercase();
        }
    }

    // Last resort: the mime's subtype, with the registry noise trimmed off.
    // `vnd.oasis.opendocument.graphics` becomes GRAPHICS, not the whole tree,
    // and the `x-` of an unregistered type goes the same way as the `vnd.` of a
    // vendor one — both say where the name was minted, not what the file is.
    let subtype = mime.rsplit('/').next().unwrap_or("");
    let subtype = subtype.strip_prefix("x-").unwrap_or(subtype);
    let trimmed = subtype
        .split('+')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .unwrap_or("");
    trimmed.to_uppercase()
}

/// The viewers worth offering for `mime`, in the order they should appear.
///
/// Microsoft's viewer renders OOXML and the old binary formats; it does NOT
/// render OpenDocument, and offering it for a `.odt` would be offering a button
/// that produces an error page. Google's `gview` reads both families. The
/// native renderer appears where it can do the job.
///
/// Pure, and tested, because an option that cannot work is worse than a missing
/// one — that is the same rule `renders_natively` follows.
pub fn viewers_for(mime: &str) -> Vec<OfficeViewer> {
    let mut out = Vec::new();
    // This app first, where it can do the job: it is the default, and the first
    // entry is the one a reader reaches for. The other two are the fallbacks
    // for a file it renders badly, which is what the gap notice offers them for.
    if renders_natively(mime) {
        out.push(OfficeViewer::Native);
    }
    if !is_opendocument(mime) {
        out.push(OfficeViewer::Microsoft);
    }
    out.push(OfficeViewer::Google);
    out
}

/// The viewer actually used for `mime`.
///
/// A preference is remembered across files, and not every viewer can open every
/// file: this app does not render an OpenDocument SPREADSHEET, and Microsoft's
/// viewer does not render OpenDocument at all. So a choice that cannot open
/// what is in front of it gives way to the first one that can, rather than
/// producing somebody else's error page.
pub fn effective_viewer(chosen: OfficeViewer, mime: &str) -> OfficeViewer {
    let offered = viewers_for(mime);
    match offered.contains(&chosen) {
        true => chosen,
        // Google reads both families, so it is always in the list and this
        // never falls through; the default is there to keep it total.
        false => offered.first().copied().unwrap_or(OfficeViewer::Google),
    }
}

/// Whether `mime` is the spreadsheet the native path renders as a grid rather
/// than as a flowing document.
fn is_spreadsheet(mime: &str) -> bool {
    mime == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
}

/// Whether `mime` is the slide deck the native path lays out as fixed-aspect
/// boxes rather than as flowing content.
fn is_presentation(mime: &str) -> bool {
    mime == "application/vnd.openxmlformats-officedocument.presentationml.presentation"
}

/// The chosen viewer, remembered per device.
///
/// A device preference rather than an account one: it is about which service you
/// are willing to send a document to on the machine in front of you, and storing
/// it on the account would need a schema and a migration to say less.
pub static OFFICE_VIEWER: GlobalSignal<OfficeViewer> = Signal::global(|| {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("wiki_office_viewer").ok().flatten())
        .map(|v| OfficeViewer::from_key(&v))
        .unwrap_or(OfficeViewer::Native)
});

/// Choose a viewer, and remember it.
pub fn set_office_viewer(viewer: OfficeViewer) {
    *OFFICE_VIEWER.write() = viewer;
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("wiki_office_viewer", viewer.key());
    }
}

#[cfg(test)]
mod tests {
    use super::is_office_mime;

    #[test]
    fn office_mimes_are_previewable() {
        for m in [
            "application/msword",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.ms-powerpoint",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ] {
            assert!(
                is_office_mime(m),
                "{m} should preview via the office viewer"
            );
        }
    }

    #[test]
    fn non_office_mimes_are_not() {
        for m in ["application/pdf", "image/png", "text/plain", ""] {
            assert!(!is_office_mime(m));
        }
    }

    /// Both viewers, spelled the way each service actually wants. Microsoft
    /// takes `src`; Google takes `url` and needs `embedded=true` or it returns a
    /// whole Google page rather than the document.
    #[test]
    fn each_viewer_gets_the_parameters_it_wants() {
        use super::{viewer_embed_url, OfficeViewer};
        let src = "https%3A%2F%2Fapi.example%2Foffice%2Ffile%3Ff%3D1%26s%3Dabc";
        let ms = viewer_embed_url(OfficeViewer::Microsoft, src);
        assert!(
            ms.starts_with("https://view.officeapps.live.com/op/embed.aspx?src="),
            "{ms}"
        );
        assert!(ms.ends_with(src), "the signed link travels whole: {ms}");

        let g = viewer_embed_url(OfficeViewer::Google, src);
        assert!(g.starts_with("https://docs.google.com/gview?"), "{g}");
        assert!(g.contains("embedded=true"), "or it is not an embed: {g}");
        assert!(g.ends_with(&format!("url={src}")), "{g}");
        assert!(
            !g.contains("src="),
            "gview ignores src, which would render nothing: {g}"
        );
    }

    /// The chip the report was about. Forty characters of registry plumbing
    /// where the reader wanted one word.
    #[test]
    fn office_files_are_named_not_spelled_out() {
        use super::type_label;
        let cases = [
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "Word",
            ),
            (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "Excel",
            ),
            (
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                "PowerPoint",
            ),
            ("application/msword", "Word"),
            ("application/vnd.ms-excel", "Excel"),
            ("application/pdf", "PDF"),
            ("image/jpeg", "JPEG"),
            ("image/svg+xml", "SVG"),
            (super::ODT, "ODT"),
        ];
        for (mime, want) in cases {
            assert_eq!(type_label(mime, "whatever.bin"), want, "{mime}");
        }
    }

    /// An unknown type falls back to the name the person gave the file before
    /// it falls back to the registry.
    #[test]
    fn an_unknown_type_uses_the_files_own_extension() {
        use super::type_label;
        assert_eq!(type_label("application/x-thing", "notes.md"), "MD");
        assert_eq!(type_label("application/octet-stream", "archive.tar"), "TAR");
        // No usable extension: the subtype, with the registry tree trimmed.
        assert_eq!(
            type_label("application/vnd.oasis.opendocument.graphics", "drawing"),
            "GRAPHICS"
        );
        assert_eq!(
            type_label("application/x-thing", "no-extension-here"),
            "THING"
        );
        // A "." in a name that is not an extension must not become the label.
        assert_eq!(
            type_label("application/x-thing", "Referat af HB-mødet 15. marts"),
            "THING",
            "a date is not a file extension"
        );
    }

    /// Every type actually attached in this wiki, from a census of production
    /// on 2026-08-01. A label nobody's files reach is decoration; these are the
    /// ones people will see.
    #[test]
    fn every_type_in_the_wiki_gets_a_readable_label() {
        use super::type_label;
        let census = [
            ("image/jpeg", 204, "JPEG"),
            ("application/pdf", 200, "PDF"),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                183,
                "Word",
            ),
            ("image/png", 35, "PNG"),
            (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                17,
                "Excel",
            ),
            ("image/webp", 10, "WebP"),
            (
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                3,
                "PowerPoint",
            ),
            ("image/heic", 3, "HEIC"),
            ("application/zip", 2, "ZIP"),
            ("audio/ogg", 1, "OGG"),
            ("audio/midi", 1, "MIDI"),
            ("video/mp4", 1, "MP4"),
        ];
        for (mime, count, want) in census {
            // Named without help from the filename: the chip must be right even
            // for a node whose name carries no extension.
            assert_eq!(
                type_label(mime, "Referat af HB-mødet"),
                want,
                "{mime} ({count} files)"
            );
        }
    }

    /// Whatever comes out is short enough to sit in a chip.
    #[test]
    fn a_label_is_always_short() {
        use super::type_label;
        let monsters = [
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.oasis.opendocument.text",
            "application/vnd.some.very.long.registry.name.indeed",
            "",
        ];
        for mime in monsters {
            let label = type_label(mime, "file.bin");
            assert!(label.chars().count() <= 12, "{mime} -> {label:?}");
        }
    }

    /// Microsoft cannot render OpenDocument, so it is not offered for one.
    /// Offering a button that produces an error page is the same mistake as
    /// offering a native option that renders nothing.
    #[test]
    fn opendocument_is_never_offered_to_microsoft() {
        use super::{viewers_for, OfficeViewer, ODP, ODS, ODT};
        for mime in [ODT, ODS, ODP] {
            let viewers = viewers_for(mime);
            assert!(
                !viewers.contains(&OfficeViewer::Microsoft),
                "{mime} must not offer Microsoft: {viewers:?}"
            );
            assert!(viewers.contains(&OfficeViewer::Google), "gview reads ODF");
        }
        // ODT renders here; an ODF sheet or deck does not, yet.
        assert!(viewers_for(ODT).contains(&OfficeViewer::Native));
        assert!(!viewers_for(ODS).contains(&OfficeViewer::Native));
        assert!(!viewers_for(ODP).contains(&OfficeViewer::Native));
    }

    #[test]
    fn ooxml_is_offered_all_three_with_this_app_first() {
        use super::{viewers_for, OfficeViewer};
        let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        assert_eq!(
            viewers_for(docx),
            vec![
                OfficeViewer::Native,
                OfficeViewer::Microsoft,
                OfficeViewer::Google
            ],
            "this app is the default, so it is the first thing offered"
        );
        // A format nothing here reads keeps the two embedded viewers.
        let legacy = "application/msword";
        assert_eq!(
            viewers_for(legacy),
            vec![OfficeViewer::Microsoft, OfficeViewer::Google]
        );
    }

    /// A preference is remembered across files, and no viewer opens every
    /// format. A choice that cannot open what is in front of it must give way
    /// rather than produce somebody else's error page.
    #[test]
    fn a_viewer_that_cannot_open_the_file_gives_way() {
        use super::{effective_viewer, OfficeViewer, ODS, ODT};
        let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

        // The ordinary case: the choice stands.
        for chosen in [
            OfficeViewer::Native,
            OfficeViewer::Microsoft,
            OfficeViewer::Google,
        ] {
            assert_eq!(effective_viewer(chosen, docx), chosen);
        }

        // An OpenDocument SPREADSHEET: this app does not read one yet, and
        // Microsoft's viewer does not read OpenDocument at all. Google does.
        assert_eq!(
            effective_viewer(OfficeViewer::Native, ODS),
            OfficeViewer::Google
        );
        assert_eq!(
            effective_viewer(OfficeViewer::Microsoft, ODS),
            OfficeViewer::Google
        );
        // An OpenDocument TEXT this app does read, so Microsoft gives way to it.
        assert_eq!(
            effective_viewer(OfficeViewer::Microsoft, ODT),
            OfficeViewer::Native
        );
        // And a legacy .doc, which this app does not read, goes to Microsoft.
        assert_eq!(
            effective_viewer(OfficeViewer::Native, "application/msword"),
            OfficeViewer::Microsoft
        );
    }

    /// Every viewer a reader can be offered must name a label key. That the
    /// key RESOLVES is checked in the i18n suite, which can read the tables
    /// without a renderer; `t` needs one.
    #[test]
    fn every_offered_viewer_has_a_label_key() {
        use super::{viewers_for, ODP, ODS, ODT};
        let mimes = [
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/msword",
            ODT,
            ODS,
            ODP,
        ];
        for mime in mimes {
            let viewers = viewers_for(mime);
            assert!(!viewers.is_empty(), "{mime} must offer something");
            for viewer in viewers {
                assert!(viewer.label_key().starts_with("file.viewer"));
            }
        }
    }

    /// Every stored preference round-trips, so nobody who has chosen a viewer
    /// loses it. Anything else is this app: it renders the document here and
    /// sends the file to nobody, which is the right thing to do by default.
    #[test]
    fn a_stored_preference_survives_and_anything_else_is_this_app() {
        use super::OfficeViewer;
        for viewer in [
            OfficeViewer::Google,
            OfficeViewer::Microsoft,
            OfficeViewer::Native,
        ] {
            assert_eq!(
                OfficeViewer::from_key(viewer.key()),
                viewer,
                "{:?} must round-trip through storage",
                viewer
            );
        }
        for junk in ["", "libreoffice", "GOOGLE", "null"] {
            assert_eq!(
                OfficeViewer::from_key(junk),
                OfficeViewer::Native,
                "{junk:?}"
            );
        }
    }
}

/// What the native renderer will not draw, said plainly.
///
/// A SUGGESTION, never a substitution. The document always renders; this names
/// what is missing from it and puts the viewers that can show it one tap away.
/// When a lot is missing the note is louder, but the reader still decides —
/// quietly swapping somebody's chosen viewer for another is the very thing this
/// is meant to prevent.
#[component]
fn GapNotice(report: super::render_gaps::GapReport, urgent: bool) -> Element {
    let items: Vec<String> = report
        .gaps
        .iter()
        .map(|gap| {
            let label = t(gap.label_key());
            match gap.count() {
                0 => label,
                n => format!("{n} {label}"),
            }
        })
        .collect();
    rsx! {
        div {
            class: if urgent { "file-gap-notice is-urgent" } else { "file-gap-notice" },
            role: "note",
            span { class: "material-icons", if urgent { "report" } else { "info" } }
            div {
                p { class: "body-small",
                    if urgent { "{t(\"file.gapMost\")}" } else { "{t(\"file.gapPartial\")}" }
                    " "
                    "{items.join(\", \")}"
                }
                // The way out, wherever the notice appears: the other viewers
                // can show what this one cannot.
                div { class: "file-gap-actions",
                    button {
                        class: "btn btn-text",
                        onclick: move |_| set_office_viewer(OfficeViewer::Microsoft),
                        "{t(\"file.viewerMicrosoft\")}"
                    }
                    button {
                        class: "btn btn-text",
                        onclick: move |_| set_office_viewer(OfficeViewer::Google),
                        "{t(\"file.viewerGoogle\")}"
                    }
                }
            }
        }
    }
}

/// An OpenDocument text file rendered here.
///
/// No renderer of its own: ODF is converted to the same block model the Word
/// renderer draws (see `components::odf`), so headings, lists, tables and styled
/// runs all work exactly as they do for a `.docx`.
#[component]
fn NativeOdt(file_id: String, name: String) -> Element {
    let token = crate::session::use_session().read().access_token.clone();
    let parsed = crate::use_data_resource!(|(file_id, token)| async move {
        let bytes = crate::backend_api::file_bytes(&file_id, &token.unwrap_or_default()).await?;
        super::odf::parse_odt(&bytes)
    });
    let state = parsed.read().clone();
    match state {
        None => rsx! {
            div { class: "empty-state empty-state-sm",
                div { class: "spinner spinner-sm" }
            }
        },
        Some(Err(e)) => {
            log::info!("native odt render failed: {e}");
            rsx! {
                p { class: "body-medium", "{t(\"file.nativeFailed\")}" }
            }
        }
        Some(Ok(blocks)) => rsx! {
            article { class: "docx-doc", aria_label: "{name}",
                super::docx::DocxBody { blocks }
            }
        },
    }
}

/// A Word file rendered here: fetch the bytes, parse them, show the document.
///
/// The bytes are fetched from storage with the session token in the header, not
/// through a presigned url and not through the signed backend link the embedded
/// viewers use. The backend link exists so a THIRD PARTY can fetch the document,
/// and nothing third-party is involved here; a presigned url would work once and
/// then expire thirty seconds later, which is exactly long enough to look at
/// another viewer and come back to a reader that will not load.
#[component]
fn NativeDocx(file_id: String, name: String) -> Element {
    let token = crate::session::use_session().read().access_token.clone();
    let parsed = crate::use_data_resource!(|(file_id, token)| async move {
        let bytes = crate::backend_api::file_bytes(&file_id, &token.unwrap_or_default()).await?;
        // The parser hands back the document model as JSON; only the parts this
        // renders are deserialised (see components::docx).
        let json = docx_parser::parse_docx_native(&bytes)?;
        let doc: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        // Counted from the RAW model: the render model drops what it cannot
        // draw, so by the time this is a Vec<Block> the evidence is gone.
        let gaps = super::render_gaps::docx_gaps(&doc);
        let mut blocks: Vec<super::docx::Block> =
            serde_json::from_value(doc.get("body").cloned().unwrap_or_default())
                .map_err(|e| e.to_string())?;
        // The model names its pictures by their path inside the package; the
        // bytes are still in the package that was just parsed.
        let images = super::docx::collect_images(&blocks, &bytes);
        super::docx::attach_images(&mut blocks, &images);
        // Needs the whole document: a heading's size means something only
        // against the size of the body text around it.
        super::docx::scale_headings(&mut blocks);
        Ok::<_, String>((blocks, gaps))
    });

    let state = parsed.read().clone();
    match state {
        None => rsx! {
            div { class: "empty-state empty-state-sm",
                div { class: "spinner spinner-sm" }
            }
        },
        Some(Ok((blocks, gaps))) => rsx! {
            if !gaps.is_empty() {
                GapNotice { urgent: gaps.is_major(), report: gaps }
            }
            article { class: "docx-doc", aria_label: "{name}",
                super::docx::DocxBody { blocks }
            }
        },
        // A document this cannot read is not a dead end: say so, and the other
        // two viewers are one tap away in the same sheet.
        Some(Err(e)) => {
            log::info!("native docx render failed: {e}");
            rsx! {
                p { class: "body-medium", "{t(\"file.nativeFailed\")}" }
            }
        }
    }
}

/// A spreadsheet rendered here.
///
/// Two parses, because the parser is built that way: the workbook once for the
/// shared strings and number formats every sheet needs, then each sheet on its
/// own. Only the first sheet is parsed up front — a workbook with twenty sheets
/// should not cost twenty parses to show the one somebody opened.
#[component]
fn NativeXlsx(file_id: String, name: String) -> Element {
    let mut sheet_no = use_signal(|| 0usize);
    let token = crate::session::use_session().read().access_token.clone();
    let parsed = crate::use_data_resource!(|(file_id, token)| async move {
        let bytes = crate::backend_api::file_bytes(&file_id, &token.unwrap_or_default()).await?;
        let wb_json = xlsx_parser::parse_workbook_native(&bytes)?;
        let wb_value: serde_json::Value =
            serde_json::from_str(&wb_json).map_err(|e| e.to_string())?;
        let workbook: super::xlsx::Workbook =
            serde_json::from_value(wb_value.clone()).map_err(|e| e.to_string())?;
        // The sheet list lives under `workbook`; its names are what the tabs say
        // and what `parse_sheet` needs alongside the index.
        let names: Vec<String> = wb_value
            .get("workbook")
            .and_then(|w| w.get("sheets"))
            .and_then(|s| s.as_array())
            .map(|a| {
                a.iter()
                    .map(|s| {
                        s.get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or_default()
                            .to_string()
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok::<_, String>((bytes, workbook, names))
    });

    let state = parsed.read().clone();
    match state {
        None => rsx! {
            div { class: "empty-state empty-state-sm",
                div { class: "spinner spinner-sm" }
            }
        },
        Some(Err(e)) => {
            log::info!("native xlsx render failed: {e}");
            rsx! {
                p { class: "body-medium", "{t(\"file.nativeFailed\")}" }
            }
        }
        Some(Ok((bytes, workbook, names))) => {
            let idx = sheet_no().min(names.len().saturating_sub(1));
            let sheet_json: serde_json::Value = names
                .get(idx)
                .and_then(|n| xlsx_parser::parse_sheet_native(&bytes, idx as u32, n).ok())
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            let gaps = super::render_gaps::xlsx_gaps(&sheet_json);
            let sheet: super::xlsx::Sheet = serde_json::from_value(sheet_json).unwrap_or_default();

            rsx! {
                div { class: "xlsx-doc", aria_label: "{name}",
                    if !gaps.is_empty() {
                        GapNotice { urgent: gaps.is_major(), report: gaps }
                    }
                    // Tabs only when there is a choice to make.
                    if names.len() > 1 {
                        div { class: "xlsx-tabs", role: "tablist",
                            for (i , sheet_name) in names.iter().enumerate() {
                                button {
                                    key: "s{i}",
                                    class: if i == idx { "xlsx-tab is-active" } else { "xlsx-tab" },
                                    role: "tab",
                                    "aria-selected": if i == idx { "true" } else { "false" },
                                    onclick: move |_| sheet_no.set(i),
                                    "{sheet_name}"
                                }
                            }
                        }
                    }
                    super::xlsx::SheetTable { sheet, workbook }
                }
            }
        }
    }
}

/// A slide deck rendered here.
#[component]
fn NativePptx(file_id: String, name: String) -> Element {
    let token = crate::session::use_session().read().access_token.clone();
    let parsed = crate::use_data_resource!(|(file_id, token)| async move {
        let bytes = crate::backend_api::file_bytes(&file_id, &token.unwrap_or_default()).await?;
        let json = pptx_parser::parse_pptx_native(&bytes)?;
        let raw: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let gaps = super::render_gaps::pptx_gaps(&raw);
        let deck: super::pptx::Deck = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        Ok::<_, String>((deck, gaps))
    });
    let state = parsed.read().clone();
    match state {
        None => rsx! {
            div { class: "empty-state empty-state-sm",
                div { class: "spinner spinner-sm" }
            }
        },
        Some(Err(e)) => {
            log::info!("native pptx render failed: {e}");
            rsx! {
                p { class: "body-medium", "{t(\"file.nativeFailed\")}" }
            }
        }
        Some(Ok((deck, gaps))) => rsx! {
            div { class: "pptx-doc", aria_label: "{name}",
                if !gaps.is_empty() {
                    GapNotice { urgent: gaps.is_major(), report: gaps }
                }
                super::pptx::DeckView { deck }
            }
        },
    }
}

#[component]
pub fn FileApp(node: NodeWithChildren) -> Element {
    let name = node.name.as_str();
    let session = use_session();
    let nav = use_navigator();
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    let created = node.created_at.as_ref().map(|t| t.0.clone());
    let node_id = node.id.0.clone();
    // What binning needs: the path stamps the subtree, the actor records who.
    let node_path = node.path.clone();
    let actor = session.read().user.as_ref().map(|u| u.id.clone());
    let context_id = node.context_id.clone().map(|c| c.0);
    // Owners may delete the file (node/context owner); mirrors ContentApp gating.
    let can_manage = node.is_owner.unwrap_or(false) || node.is_context_owner.unwrap_or(false);
    let mut confirm_open = use_signal(|| false);
    // Deleting walks the file's comment subtree a request at a time.
    let mut deleting = use_signal(|| false);

    let data = node.data.map(|d| d.0);

    let file_id = data
        .as_ref()
        .and_then(|d| d.get("fileId"))
        .and_then(|f| f.as_str())
        .unwrap_or("");
    let file_mime = data
        .as_ref()
        .and_then(|d| d.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    // DESIGN (format-aware file card): retint the card accent by format
    // (video/audio -> magenta tertiary, docs -> secondary, else primary) and show
    // a short type chip. Reuses the .card --card-accent system.
    let accent = if file_mime.starts_with("video") || file_mime.starts_with("audio") {
        "accent-tertiary"
    } else if file_mime.contains("pdf")
        || file_mime.contains("word")
        || file_mime.contains("sheet")
        || file_mime.contains("presentation")
        || file_mime.contains("officedocument")
    {
        "accent-secondary"
    } else {
        ""
    };
    let type_label = type_label(file_mime, name);

    // Presigned, not a bare storage URL: this feeds an <iframe>, <video>, <audio>
    // and a download <a>, none of which can send the Authorization header the
    // storage service reads. Empty until it resolves, which the branches below
    // already treat as "no preview yet".
    let file_url = super::loader::use_presigned_url(file_id.to_string()).unwrap_or_default();

    // The Office viewer needs its own link (see the branch that uses it), fetched
    // only for the mimes that go through it.
    let mut office_embed = use_signal(|| OfficeLink::Pending);
    {
        let id = file_id.to_string();
        let wanted = is_office_mime(file_mime);
        let token = session.read().access_token.clone();
        use_effect(use_reactive!(|(id, wanted, token)| {
            office_embed.set(OfficeLink::Pending);
            if !wanted || id.is_empty() {
                return;
            }
            let Some(token) = token.clone() else {
                office_embed.set(OfficeLink::Refused);
                return;
            };
            spawn(async move {
                office_embed.set(
                    match crate::backend_api::office_embed_url(&id, &token).await {
                        Some(url) => OfficeLink::Ready(url),
                        None => OfficeLink::Refused,
                    },
                );
            });
        }));
    }

    rsx! {
        super::widgets::SupportingPaneLayout {
            // Primary pane: the file's identity header above the file itself, so the
            // title / date / tools sit atop the content rather than below it.
            primary: rsx! {
                div { class: "card file-card {accent}",
                    div { class: "card-header",
                        div { class: "avatar", {node_icon_el("wiki/file", data.as_ref())} }
                        div {
                            h3 { class: "title-medium", "{name}" }
                            div { class: "file-meta-chips",
                                if !type_label.is_empty() {
                                    span { class: "file-chip",
                                        // The chip's own format, not a document
                                        // icon beside every label: it read
                                        // "[document] Excel".
                                        {super::loader::icon_el(file_mime)}
                                        "{type_label}"
                                    }
                                }
                                if let Some(iso) = created.as_ref() {
                                    span {
                                        class: "file-chip",
                                        title: "{super::loader::full_datetime(iso)}",
                                        span { class: "material-icons", "schedule" }
                                        "{super::loader::relative_time(iso)}"
                                    }
                                }
                            }
                        }
                        div { class: "flex-grow" }
                        // File actions in the M3 tools sheet.
                        if !file_url.is_empty() || (can_manage && !segments.is_empty()) {
                            super::widgets::ToolSheet {
                                title: t("common.tools"),
                                // Pinned quick group: copy link (the sheet's own
                                // first segment) and downloading the file itself.
                                quick: rsx! {
                                    if !file_url.is_empty() {
                                        a {
                                            href: "{file_url}",
                                            target: "_blank",
                                            download: "{name}",
                                            "referrerpolicy": "no-referrer",
                                            class: "sheet-quick-action",
                                            title: "{t(\"common.download\")}",
                                            aria_label: "{t(\"common.download\")}",
                                            span { class: "material-icons", "download" }
                                        }
                                    }
                                },
                                // Who renders it. Only for the mimes that go
                                // through a third party at all — offering it on
                                // a PDF or an image would be offering a choice
                                // that changes nothing.
                                if is_office_mime(file_mime) {
                                    super::widgets::SheetGroup {
                                        div { class: "sheet-label", "{t(\"file.renderedBy\")}" }
                                        for viewer in viewers_for(file_mime) {
                                            button {
                                                key: "{viewer.key()}",
                                                class: if OFFICE_VIEWER() == viewer { "sheet-action selected" } else { "sheet-action" },
                                                "aria-pressed": if OFFICE_VIEWER() == viewer { "true" } else { "false" },
                                                onclick: move |_| set_office_viewer(viewer),
                                                span { class: "material-icons",
                                                    if OFFICE_VIEWER() == viewer { "radio_button_checked" } else { "radio_button_unchecked" }
                                                }
                                                "{t(viewer.label_key())}"
                                            }
                                        }
                                    }
                                }
                                if can_manage && !segments.is_empty() {
                                    super::widgets::SheetGroup { danger: true,
                                        button {
                                            class: "sheet-action danger",
                                            onclick: move |_| confirm_open.set(true),
                                            span { class: "material-icons", "delete" }
                                            "{t(\"common.delete\")}"
                                        }
                                    }
                                }
                            }
                        }
                        // Delete confirm dialog (owner-only).
                        if can_manage && !segments.is_empty() {
                            super::widgets::Dialog {
                                open: confirm_open(),
                                on_dismiss: move |_| confirm_open.set(false),
                                headline: t("content.confirmDeleteBin"),
                                icon: "delete".to_string(),
                                actions: rsx! {
                                    button {
                                        class: "btn btn-outlined",
                                        onclick: move |_| confirm_open.set(false),
                                        "{t(\"common.cancel\")}"
                                    }
                                    button {
                                        class: "btn btn-primary",
                                        disabled: deleting(),
                                        onclick: {
                                            let node_id = node_id.clone();
                                            let parent = segments[..segments.len() - 1].to_vec();
                                            let node_path = node_path.clone();
                                            let actor = actor.clone();
                                            move |_| {
                                                if deleting() {
                                                    return;
                                                }
                                                let token = session.read().access_token.clone();
                                                let node_id = node_id.clone();
                                                let parent = parent.clone();
                                                let node_path = node_path.clone();
                                                let actor = actor.clone();
                                                // Dialog stays open so the spinner has somewhere
                                                // to be while the statement runs.
                                                deleting.set(true);
                                                spawn(async move {
                                                    // To the bin, like a document or a folder:
                                                    // deleting a file from its own page used to be
                                                    // final while deleting the folder around it was
                                                    // recoverable, which made the promise depend on
                                                    // which page you happened to be looking at.
                                                    // The stored object is untouched either way, so
                                                    // a restored file is a working file.
                                                    match graphql::bin_node(
                                                        token.as_deref(),
                                                        &node_id,
                                                        node_path.as_deref(),
                                                        actor.as_deref(),
                                                    )
                                                    .await
                                                    {
                                                        Ok(_) => {
                                                            crate::session::bump_data_version();
                                                            deleting.set(false);
                                                            confirm_open.set(false);
                                                            nav.push(Route::PathPage { segments: parent, app: None });
                                                        }
                                                        other => {
                                                            log::error!("delete_node failed: {other:?}");
                                                            deleting.set(false);
                                                            crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                        }
                                                    }
                                                });
                                            }
                                        },
                                        if deleting() {
                                            div { class: "spinner spinner-xs" }
                                        }
                                        "{t(\"common.delete\")}"
                                    }
                                },
                                p { class: "body-medium", "{name}" }
                                p { class: "body-medium text-muted", "{t(\"content.deleteRecoverable\")}" }
                            }
                        }
                    }
                    div { class: "file-viewer",
                        if file_url.is_empty() {
                            p { class: "body-medium", "{t(\"common.noContent\")}" }
                        } else if file_mime.starts_with("image/") {
                            super::widgets::ZoomableImage { src: file_url.clone(), alt: name.to_string() }
                        } else if file_mime.starts_with("video/") {
                            video {
                                controls: true,
                                "referrerpolicy": "no-referrer",
                                src: "{file_url}",
                            }
                        } else if file_mime.starts_with("audio/") {
                            audio { controls: true, "referrerpolicy": "no-referrer", src: "{file_url}" }
                        } else if file_mime == "application/pdf" {
                            // DESIGN: frame document previews like the map/graph.
                            div { class: "viewport-frame",
                                iframe { src: "{file_url}", title: "{name}", "referrerpolicy": "no-referrer" }
                            }
                        } else if is_office_mime(file_mime) {
                            // Word/Excel/PowerPoint through Microsoft's hosted
                            // viewer, which fetches the document from ITS servers.
                            // So it gets a backend link, not a storage URL: the
                            // backend checked the caller may read the file and
                            // serves the bytes for a couple of hours. A presigned
                            // storage URL would be dead 30 seconds later.
                            match office_embed() {
                                OfficeLink::Pending => rsx! {
                                    div { class: "empty-state empty-state-sm",
                                        div { class: "spinner spinner-sm" }
                                    }
                                },
                                OfficeLink::Ready(_) if OFFICE_VIEWER() == OfficeViewer::Native
                                    && file_mime == ODT => rsx! {
                                    NativeOdt { file_id: file_id.to_string(), name: name.to_string() }
                                },
                                OfficeLink::Ready(_) if OFFICE_VIEWER() == OfficeViewer::Native
                                    && is_presentation(file_mime) => rsx! {
                                    NativePptx { file_id: file_id.to_string(), name: name.to_string() }
                                },
                                OfficeLink::Ready(_) if OFFICE_VIEWER() == OfficeViewer::Native
                                    && is_spreadsheet(file_mime) => rsx! {
                                    NativeXlsx { file_id: file_id.to_string(), name: name.to_string() }
                                },
                                OfficeLink::Ready(_) if OFFICE_VIEWER() == OfficeViewer::Native
                                    && renders_natively(file_mime) => rsx! {
                                    NativeDocx { file_id: file_id.to_string(), name: name.to_string() }
                                },
                                OfficeLink::Ready(url) => {
                                    let encoded = String::from(&js_sys::encode_uri_component(&url));
                                    // Not the raw preference: reaching here means
                                    // the native path declined this format, and
                                    // Microsoft cannot read OpenDocument either.
                                    let embed = viewer_embed_url(
                                        effective_viewer(OFFICE_VIEWER(), file_mime),
                                        &encoded,
                                    );
                                    rsx! {
                                        div { class: "viewport-frame",
                                            iframe {
                                                // Keyed on the viewer: swapping
                                                // service must reload the frame,
                                                // and a changed `src` alone does
                                                // not reliably do that.
                                                key: "{OFFICE_VIEWER().key()}",
                                                src: "{embed}",
                                                title: "{name}",
                                            }
                                        }
                                    }
                                }
                                OfficeLink::Refused => rsx! {
                                    p { class: "body-medium", "{t(\"common.noPreview\")}" }
                                },
                            }
                        } else {
                            // DESIGN: a rich "no preview" state (format orb +
                            // prominent download) instead of a bare button.
                            div { class: "empty-state empty-state-sm",
                                div { class: "empty-state-orb empty-state-orb-sm",
                                    {node_icon_el("wiki/file", data.as_ref())}
                                }
                                p { class: "empty-state-body", "{t(\"common.noPreview\")}" }
                                a {
                                    href: "{file_url}",
                                    target: "_blank",
                                    "referrerpolicy": "no-referrer",
                                    class: "btn btn-primary",
                                    span { class: "material-icons", "download" }
                                    " {t(\"common.download\")}"
                                }
                            }
                        }
                    }
                }
            },
            // Supporting pane: the discussion.
            supporting: rsx! {
                super::comments::CommentSection {
                    node_id: node_id.clone(),
                    context_id: context_id.clone(),
                }
            },
        }
    }
}

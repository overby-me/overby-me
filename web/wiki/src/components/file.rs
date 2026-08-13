use dioxus::prelude::*;
use wasm_bindgen::JsCast;

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

/// Who renders a PDF.
///
/// A different question from [`OfficeViewer`], which is about which THIRD PARTY
/// sees the document. A PDF already goes to nobody: every browser has a viewer
/// built in and it renders locally. So this is fidelity against readability.
///
/// Readability leads. Most of what this wiki carries a PDF for is read on a
/// phone at a meeting, and there the browser's viewer is a fixed page in a
/// scrolling box: pinching at six-point type, no reflow, and the page's own find
/// blind to it. The native one reflows, works with find, keeps the page marks,
/// and is the one this app can go on improving.
///
/// The browser's stays one tap away and unchanged, for when a reader wants the
/// page exactly as it was laid out, or wants to print it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PdfViewer {
    /// The browser's built-in viewer, in an iframe.
    Browser,
    /// Rendered here, from the file's own bytes, as flowing text. The default.
    Native,
}

impl PdfViewer {
    fn key(self) -> &'static str {
        match self {
            PdfViewer::Browser => "browser",
            PdfViewer::Native => "native",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            PdfViewer::Browser => "file.pdfViewerBrowser",
            PdfViewer::Native => "file.pdfViewerNative",
        }
    }

    /// Anything unrecognised is the default, which is this app's own renderer.
    fn from_key(key: &str) -> Self {
        match key {
            "browser" => PdfViewer::Browser,
            _ => PdfViewer::Native,
        }
    }
}

/// The chosen PDF viewer, remembered per device like the Office one.
pub static PDF_VIEWER: GlobalSignal<PdfViewer> = Signal::global(|| {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("wiki_pdf_viewer").ok().flatten())
        .map(|v| PdfViewer::from_key(&v))
        .unwrap_or(PdfViewer::Native)
});

/// Choose a PDF viewer, and remember it.
pub fn set_pdf_viewer(viewer: PdfViewer) {
    *PDF_VIEWER.write() = viewer;
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("wiki_pdf_viewer", viewer.key());
    }
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

    /// The brands an iPhone writes are recognised, and nothing else is.
    ///
    /// This decides whether a file is decoded here or handed to the browser, and
    /// it is asked of files whose mime said only `application/octet-stream`. A
    /// false positive parks an ordinary picture on a spinner and then shows it as
    /// undrawable, so the shape of the box matters as much as the brand: `ftyp`
    /// at 4, the brand at 8.
    #[test]
    fn only_a_heif_container_is_taken_for_one() {
        use super::looks_like_heif;
        let ftyp = |brand: &[u8]| {
            let mut v = vec![0, 0, 0, 0x18];
            v.extend_from_slice(b"ftyp");
            v.extend_from_slice(brand);
            v.extend_from_slice(&[0; 8]);
            v
        };
        for brand in [b"heic", b"heix", b"mif1", b"msf1"] {
            assert!(looks_like_heif(&ftyp(brand)), "{:?}", brand);
        }
        // AVIF is the same container holding AV1, which every browser draws
        // itself and this decoder cannot read. Taking it would turn a picture
        // that works into one that does not.
        assert!(!looks_like_heif(&ftyp(b"avif")));
        // An MP4 is ISO-BMFF too, and its brand is the only thing telling them
        // apart.
        assert!(!looks_like_heif(&ftyp(b"isom")));
        // Ordinary pictures, which reach this whenever a mime was unhelpful.
        assert!(!looks_like_heif(&[
            0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
        assert!(!looks_like_heif(b"\x89PNG\r\n\x1a\n\0\0\0\0\0"));
        // And a file too short to hold a brand is not read past its end.
        assert!(!looks_like_heif(b"\0\0\0\x18ftyp"));
        assert!(!looks_like_heif(&[]));
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

/// The message shown when this app cannot read a file at all.
///
/// The same shape as [`GapNotice`], because it is the same kind of thing said
/// more firmly: something is wrong with what you are looking at, and here are
/// the two viewers that can show it. It used to be a bare line of body text,
/// which looked like part of the document rather than a note about it.
#[component]
fn FailureNotice() -> Element {
    rsx! {
        div { class: "file-gap-notice is-urgent", role: "note",
            span { class: "material-icons", "report" }
            div {
                p { class: "body-small", "{t(\"file.nativeFailed\")}" }
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
                FailureNotice {}
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
/// A PDF read here and reflowed. The browser's viewer stays the default; this
/// is what the sheet's other option renders.
#[component]
fn NativePdf(file_id: String, name: String) -> Element {
    let token = crate::session::use_session().read().access_token.clone();
    let parsed = crate::use_data_resource!(|(file_id, token)| async move {
        let token = token.unwrap_or_default();
        let bytes = crate::backend_api::file_bytes(&file_id, &token).await?;
        crate::pdf_text::extract(&bytes)
    });

    let state = parsed.read().clone();
    match state {
        None => rsx! {
            div { class: "empty-state empty-state-sm",
                div { class: "spinner spinner-sm" }
            }
        },
        // A scan has no text to reflow, so say that rather than draw nothing.
        Some(Ok(doc)) if !doc.has_text() => rsx! {
            super::pdf::PdfHasNoText {}
        },
        Some(Ok(doc)) => rsx! {
            super::pdf::PdfDocument { doc }
        },
        Some(Err(e)) => {
            crate::errors::log_handled("pdf render failed", &e);
            rsx! {
                super::widgets::ErrorState { title: t("error.couldNotLoad"), small: true }
            }
        }
    }
}

#[component]
fn NativeDocx(file_id: String, name: String) -> Element {
    let token = crate::session::use_session().read().access_token.clone();
    // Kept: the reader below needs to know WHICH document it is holding, and
    // the fetch takes the id itself.
    let which = file_id.clone();
    let parsed = crate::use_data_resource!(|(file_id, token)| async move {
        let token = token.unwrap_or_default();
        let bytes = crate::backend_api::file_bytes(&file_id, &token).await?;
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
        let mut images = super::docx::collect_images(&blocks, &bytes);
        // The figures no browser draws: Word keeps pasted charts as EMF/WMF, so
        // the backend renders those. Anything it cannot render is simply absent,
        // and falls through to the placeholder as before.
        let drawn = super::docx::render_metafiles(
            super::docx::collect_metafiles(&blocks),
            &bytes,
            Some(token.as_str()),
        )
        .await;
        // Same as the deck: what the backend drew is no longer a gap.
        let mut gaps = gaps;
        gaps.drew_pictures(drawn.len());
        images.extend(drawn);
        super::docx::attach_images(&mut blocks, &images);
        // Needs the whole document: a heading's size means something only
        // against the size of the body text around it.
        super::docx::scale_headings(&mut blocks);
        // What the document says its pages are, for working out where they
        // end. `None` where it says nothing usable, and then nothing is marked.
        let page = doc.get("section").and_then(|section| {
            super::docx::PageGeometry::read(
                section,
                doc.get("minorFont").and_then(|f| f.as_str()),
                &blocks,
            )
        });
        Ok::<_, String>((blocks, gaps, page))
    });

    let state = parsed.read().clone();
    match state {
        None => rsx! {
            div { class: "empty-state empty-state-sm",
                div { class: "spinner spinner-sm" }
            }
        },
        Some(Ok((blocks, gaps, page))) => rsx! {
            if !gaps.is_empty() {
                GapNotice { urgent: gaps.is_major(), report: gaps }
            }
            article { class: "docx-doc", aria_label: "{name}",
                match page {
                    // The document states a page size, so where its pages end
                    // can be worked out and a reader can be told "page 7".
                    Some(page) => rsx! {
                        super::docx::PagedDocx { document: which.clone(), blocks, page }
                    },
                    None => rsx! {
                        super::docx::DocxBody { blocks }
                    },
                }
            }
        },
        // A document this cannot read is not a dead end: say so, and the other
        // two viewers are one tap away in the same sheet.
        Some(Err(e)) => {
            log::info!("native docx render failed: {e}");
            rsx! {
                FailureNotice {}
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
        // `parse_xlsx`, NOT `parse_workbook_native`: the latter serialises only
        // the sheet LIST. It looks like the workbook because it is named after
        // it, and it deserialises into `Workbook` without complaint — every
        // field simply defaults. So the shared-string table was always empty,
        // and a cell holding text holds an INDEX into that table, which is how
        // every real spreadsheet came out blank. Reported from a 351-row
        // spreadsheet whose 182 strings all vanished.
        let wb_bytes = xlsx_parser::parse_xlsx(&bytes, None).map_err(|e| format!("{e:?}"))?;
        let wb_value: serde_json::Value =
            serde_json::from_slice(&wb_bytes).map_err(|e| e.to_string())?;
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
                FailureNotice {}
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
        let token = token.unwrap_or_default();
        let bytes = crate::backend_api::file_bytes(&file_id, &token).await?;
        let json = pptx_parser::parse_pptx_native(&bytes)?;
        let raw: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let gaps = super::render_gaps::pptx_gaps(&raw);
        let mut deck: super::pptx::Deck = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        // Same as the Word path: the model names its pictures by their path
        // inside the package, and the bytes are still in the package.
        let mut images = super::pptx::collect_images(&deck, &bytes);
        // Same as the Word path: EMF/WMF figures go to the backend to be drawn.
        let drawn = super::docx::render_metafiles(
            super::pptx::collect_metafiles(&deck),
            &bytes,
            Some(token.as_str()),
        )
        .await;
        // The gap report was taken from the model, which calls a metafile
        // undrawable because no BROWSER draws one. The backend just did, so the
        // banner must stop announcing figures the reader can see.
        let mut gaps = gaps;
        gaps.drew_pictures(drawn.len());
        images.extend(drawn);
        super::pptx::attach_images(&mut deck, &images);
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
                FailureNotice {}
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
    // Still mutable means not yet submitted, which this file's row in the folder
    // listing has always said with its `lock_open` mark. Now the page says it too,
    // and offers the way out of it: a file was the one content view with no submit
    // action, so an uploaded agenda stood marked unsubmitted with no Indsend
    // anywhere on it. Submitting only flips `mutable`, exactly as on a document.
    let is_mutable = node.mutable;
    let can_submit = can_manage && is_mutable;
    let mut confirm_submit = use_signal(|| false);

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
    // Presigned, not a bare storage URL: this feeds an <iframe>, <video>, <audio>
    // and a download <a>, none of which can send the Authorization header the
    // storage service reads. Empty until it resolves, which the branches below
    // already treat as "no preview yet".
    // Re-signed when the viewer changes: a signature outlives opening a file and
    // not coming back to one, so switching away to this app's PDF renderer and
    // back again used to hand the iframe a URL signed minutes earlier, which the
    // storage service answers with "signature already expired".
    let file_url = super::loader::use_presigned_url(
        file_id.to_string(),
        format!("{:?}/{:?}", PDF_VIEWER(), OFFICE_VIEWER()),
    )
    .unwrap_or_default();

    // A HEIC has to be decoded here before anything can draw it.
    let heif = use_heif_preview(file_id.to_string(), file_mime.to_string());

    // Downloading fetches the bytes when the reader asks for them, rather than
    // following a link minted when the page opened.
    //
    // A presigned URL lives about thirty seconds. A reader who opens a file,
    // reads it, and only then reaches for the download button is well past that,
    // and the storage service answers "signature already expired" — reported
    // from the wiki as "the download fails after some time". Nothing can be
    // signed early enough to be safe here, because the press can come at any
    // time; the token, on the other hand, is current at the moment of the press.
    //
    // It buys a real save as well: a cross-origin `download` attribute is
    // ignored, so the old link handed the file to a new tab and let the browser
    // decide. These bytes arrive as a blob of this origin, under the file's own
    // name.
    let mut downloading = use_signal(|| false);
    let download_the_file = {
        let file_id = file_id.to_string();
        let name = name.to_string();
        let mime = file_mime.to_string();
        move |_| {
            if downloading() {
                return;
            }
            let (file_id, name, mime) = (file_id.clone(), name.clone(), mime.clone());
            downloading.set(true);
            spawn(async move {
                let bytes = match crate::session::current_token() {
                    Some(token) => crate::backend_api::file_bytes(&file_id, &token).await,
                    None => Err("not signed in".to_string()),
                };
                match bytes {
                    Ok(bytes) => crate::export::download_bytes(&name, &mime, &bytes),
                    Err(why) => {
                        log::error!("download of {file_id} failed: {why}");
                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                    }
                }
                downloading.set(false);
            });
        }
    };

    // The Office viewer needs its own link (see the branch that uses it), fetched
    // only for the mimes that go through it.
    let mut office_embed = use_signal(|| OfficeLink::Pending);
    {
        let id = file_id.to_string();
        let wanted = is_office_mime(file_mime);
        // Who is reading, not which token says so: on the token this re-fetched
        // the embed link every rotation, and it opens by setting Pending, which
        // takes the document off the screen and puts it back.
        let who = session.read().identity();
        use_effect(use_reactive!(|(id, wanted, who)| {
            let _ = &who;
            office_embed.set(OfficeLink::Pending);
            if !wanted || id.is_empty() {
                return;
            }
            let Some(token) = crate::session::current_token() else {
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
        // ONE column: the file, then the discussion beneath it.
        //
        // This was a supporting-pane split, which stands a comment thread beside
        // the primary content once there is room for both. That is right for a
        // console or a profile, and wrong here: the primary content is a document
        // viewer, and a spreadsheet or a slide wants every pixel of width it can
        // get far more than a comment thread does.
        div { class: "card app-card file-card {accent}",
            div { class: "card-header",
                // The same not-submitted mark the file's row carries in a list, so
                // its own page does not look finished when the listing says it is not.
                super::loader::AvatarBadged { mutable: is_mutable,
                    div { class: "avatar", {node_icon_el("wiki/file", data.as_ref())} }
                }
                div {
                    h3 { class: "title-medium", "{name}" }
                    // No chip for the format. The avatar beside this heading is
                    // already drawn from the file's own type -- picture_as_pdf,
                    // table_chart, slideshow -- so a chip repeating that icon and
                    // naming it said the same thing twice, a centimetre apart.
                    div { class: "file-meta-chips",
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
                            button {
                                class: "sheet-quick-action",
                                onclick: download_the_file.clone(),
                                disabled: downloading(),
                                title: "{t(\"common.download\")}",
                                aria_label: "{t(\"common.download\")}",
                                if downloading() {
                                    div { class: "spinner spinner-xs" }
                                } else {
                                    span { class: "material-icons", "download" }
                                }
                            }
                        },
                        // Who renders it. Offered where there is a real
                        // choice: an Office document goes to a third
                        // party unless this app reads it, and a PDF can
                        // be shown exactly by the browser or reflowed by
                        // this app. Not on an image, where it would be a
                        // choice that changes nothing.
                        // How this app draws it, before who draws it: a
                        // reader who wants the page as printed and a reader who
                        // wants it to reflow are asking about the same renderer.
                        if file_mime == "application/pdf" && PDF_VIEWER() == PdfViewer::Native {
                            super::widgets::SheetGroup {
                                div { class: "sheet-label", "{t(\"file.pdfLayout\")}" }
                                // Default first, as in the renderer group below
                                // (Native before Browser). Reflow is what a reader
                                // arrives in, so listing "As printed" above it put
                                // the option nobody chose at the top and made the
                                // selected row the lower one.
                                for layout in [
                                    super::pdf::PdfLayout::Reflow,
                                    super::pdf::PdfLayout::Page,
                                ] {
                                    button {
                                        key: "{layout.label_key()}",
                                        class: if super::pdf::PDF_LAYOUT() == layout { "sheet-action selected" } else { "sheet-action" },
                                        "aria-pressed": if super::pdf::PDF_LAYOUT() == layout { "true" } else { "false" },
                                        onclick: move |_| super::pdf::set_pdf_layout(layout),
                                        span { class: "material-icons",
                                            if super::pdf::PDF_LAYOUT() == layout { "radio_button_checked" } else { "radio_button_unchecked" }
                                        }
                                        "{t(layout.label_key())}"
                                    }
                                }
                            }
                        }
                        if file_mime == "application/pdf" {
                            super::widgets::SheetGroup {
                                div { class: "sheet-label", "{t(\"file.renderedBy\")}" }
                                for viewer in [PdfViewer::Native, PdfViewer::Browser] {
                                    button {
                                        key: "{viewer.label_key()}",
                                        class: if PDF_VIEWER() == viewer { "sheet-action selected" } else { "sheet-action" },
                                        "aria-pressed": if PDF_VIEWER() == viewer { "true" } else { "false" },
                                        onclick: move |_| set_pdf_viewer(viewer),
                                        span { class: "material-icons",
                                            if PDF_VIEWER() == viewer { "radio_button_checked" } else { "radio_button_unchecked" }
                                        }
                                        "{t(viewer.label_key())}"
                                    }
                                }
                            }
                        }
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
                        // Submit: the file is uploaded already, and this only makes
                        // it final. Gated on the node still being mutable, so the
                        // row disappears once it has been submitted.
                        if can_submit && !segments.is_empty() {
                            super::widgets::SheetGroup { title: t("common.toolsManage"),
                                button {
                                    class: "sheet-action",
                                    onclick: move |_| confirm_submit.set(true),
                                    span { class: "material-icons", "publish" }
                                    "{t(\"content.submit\")}"
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
                // Submit confirm, carrying the same warning ContentApp's does: after
                // this the file can no longer be edited.
                if can_submit && !segments.is_empty() {
                    super::widgets::Dialog {
                        open: confirm_submit(),
                        on_dismiss: move |_| confirm_submit.set(false),
                        headline: t("content.submit"),
                        icon: "publish".to_string(),
                        actions: rsx! {
                            button {
                                class: "btn btn-outlined",
                                onclick: move |_| confirm_submit.set(false),
                                "{t(\"common.cancel\")}"
                            }
                            button {
                                class: "btn btn-primary",
                                onclick: {
                                    let node_id = node_id.clone();
                                    move |_| {
                                        confirm_submit.set(false);
                                        let token = session.read().access_token.clone();
                                        let node_id = node_id.clone();
                                        spawn(async move {
                                            match graphql::update_node(
                                                token.as_deref(),
                                                &node_id,
                                                crate::model::NodesSetInput {
                                                    mutable: Some(false),
                                                    ..Default::default()
                                                },
                                            )
                                            .await
                                            {
                                                Ok(_) => {
                                                    crate::session::bump_data_version();
                                                    crate::snackbar::show_snackbar(&t("content.submit"));
                                                }
                                                Err(e) => {
                                                    crate::errors::log_handled("file submit failed", e);
                                                    crate::snackbar::show_snackbar(&t(
                                                        "error.somethingWentWrong",
                                                    ));
                                                }
                                            }
                                        });
                                    }
                                },
                                "{t(\"content.submit\")}"
                            }
                        },
                        p { class: "body-medium", "{t(\"content.submitWarning\")}" }
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
                // The readers that fetch their OWN bytes come first, because
                // they do not use the url below and must not wait on it. It is
                // presigned, and a signature that has not arrived -- or has
                // failed -- left a document that could be read perfectly well
                // showing "no content". Found by suppressing the signing call
                // and watching the native PDF reader never mount.
                if file_mime == "application/pdf" && PDF_VIEWER() == PdfViewer::Native {
                    NativePdf { file_id: file_id.to_string(), name: name.to_string() }
                } else if is_office_mime(file_mime) && OFFICE_VIEWER() == OfficeViewer::Native {
                    // Same for Word, Excel, PowerPoint and ODF: each reads the
                    // package itself with the session token. The HOSTED viewers
                    // below do need the link, and they still wait for it.
                    if file_mime == ODT {
                        NativeOdt { file_id: file_id.to_string(), name: name.to_string() }
                    } else if is_presentation(file_mime) {
                        NativePptx { file_id: file_id.to_string(), name: name.to_string() }
                    } else if is_spreadsheet(file_mime) {
                        NativeXlsx { file_id: file_id.to_string(), name: name.to_string() }
                    } else {
                        NativeDocx { file_id: file_id.to_string(), name: name.to_string() }
                    }
                } else if file_url.is_empty() {
                    // Still on its way, or it failed. A spinner is the truth
                    // while it is on its way; "no content" said the file was
                    // empty, which it is not.
                    div { class: "empty-state empty-state-sm",
                        div { class: "spinner spinner-sm" }
                    }
                } else if let HeifPreview::Decoding = heif {
                    // Decoding a twelve-megapixel photo is not instant, and this
                    // is the same spinner the file's own arrival shows.
                    div { class: "empty-state empty-state-sm",
                        div { class: "spinner spinner-sm" }
                    }
                } else if let HeifPreview::Ready(src) = &heif {
                    super::widgets::ZoomableImage { src: src.clone(), alt: name.to_string() }
                } else if let HeifPreview::Failed = heif {
                    // Say so rather than showing a broken picture. The download
                    // action still works, and on a phone the file opens.
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "broken_image" }
                        }
                        p { class: "empty-state-body", "{t(\"file.imageNotDrawable\")}" }
                    }
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
                } else if file_mime == "application/pdf" && PDF_VIEWER() == PdfViewer::Native {
                    NativePdf { file_id: file_id.to_string(), name: name.to_string() }
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
                        button {
                            class: "btn btn-primary",
                            onclick: download_the_file.clone(),
                            disabled: downloading(),
                            if downloading() {
                                div { class: "spinner spinner-xs" }
                            } else {
                                span { class: "material-icons", "download" }
                            }
                            " {t(\"common.download\")}"
                        }
                    }
                }
            }
        }
        super::comments::CommentSection {
            node_id: node_id.clone(),
            context_id: context_id.clone(),
        }
    }
}

/// Where a HEIC has got to: not one, being decoded, drawable, or beyond us.
#[derive(Clone, PartialEq)]
pub enum HeifPreview {
    /// Nothing to do. Every ordinary image is this, and so is a HEIC whose bytes
    /// have not arrived yet -- the caller cannot tell them apart, and must not:
    /// deciding "not a HEIC" from a mime alone is what this sniffs to avoid.
    NotHeif,
    Decoding,
    Ready(String),
    /// Decoded and refused. A HEIC is a container, and one holding something
    /// other than an HEVC still (a burst, a depth map, an AV1 image) is a file
    /// this decoder will not read however long it is given.
    Failed,
}

/// Fetch a file and, if it turns out to be a HEIC, decode it for display.
///
/// Keyed on the file, so moving between two images does not leave the first
/// one's picture on the second's page.
///
/// The bytes are fetched WHATEVER the mime says, when the mime is one that could
/// plausibly be a HEIC -- see `is_heif_mime` for why the mime cannot be trusted
/// on its own -- and the sniff decides. An ordinary JPEG never reaches here.
pub fn use_heif_preview(file_id: String, mime: String) -> HeifPreview {
    let mut state = use_signal(|| HeifPreview::NotHeif);
    let maybe = is_heif_mime(&mime) || mime == "application/octet-stream" || mime.is_empty();
    use_effect(use_reactive!(|(file_id, maybe)| {
        state.set(HeifPreview::NotHeif);
        if !maybe || file_id.is_empty() {
            return;
        }
        let Some(token) = crate::session::current_token() else {
            return;
        };
        state.set(HeifPreview::Decoding);
        spawn(async move {
            let url = crate::backend_api::file_url(&file_id);
            let bytes = match reqwest::Client::new()
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => resp.bytes().await.ok(),
                _ => None,
            };
            let Some(bytes) = bytes else {
                state.set(HeifPreview::Failed);
                return;
            };
            // Sniffed, not assumed: an `application/octet-stream` is far more
            // often something ordinary, and that one must go back to being drawn
            // from storage rather than sitting on a spinner.
            if !looks_like_heif(&bytes) {
                state.set(HeifPreview::NotHeif);
                return;
            }
            match heif_to_data_url(&bytes) {
                Some(url) => state.set(HeifPreview::Ready(url)),
                None => state.set(HeifPreview::Failed),
            }
        });
    }));
    state()
}

/// Whether this is a photo off a phone that browsers may refuse to draw.
///
/// Firefox draws no HEIC at all, so an iPhone photo is a broken image there.
/// Matched on the mime AND on the file's own bytes, because the mime is whatever
/// the uploading browser claimed: a HEIC picked from a file dialog often arrives
/// as `application/octet-stream`, and the files that prompted this are already in
/// storage under whatever they were given then.
pub fn is_heif_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/heic" | "image/heif" | "image/heic-sequence" | "image/heif-sequence"
    )
}

/// Whether these bytes open with an ISO-BMFF `ftyp` box naming a HEIF brand.
///
/// The layout is `[4 bytes length][ftyp][major brand]`, so the brand sits at 8.
/// `heic`/`heix` are the HEVC still images an iPhone writes; `mif1`/`msf1` are
/// the generic image brands; `avif` is the same container with AV1 inside, which
/// every current browser draws itself and which this decoder does not read -- so
/// it is deliberately NOT in the list.
pub fn looks_like_heif(bytes: &[u8]) -> bool {
    bytes.len() > 12
        && &bytes[4..8] == b"ftyp"
        && matches!(
            &bytes[8..12],
            b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1"
        )
}

/// Decode a HEIF/HEIC image and hand back something an `<img>` will draw.
///
/// Decoded here rather than converted on upload, because the files that prompted
/// this are already in storage; an upload-time fix would leave them broken.
///
/// The canvas does the encoding. `heif-oxide` gives pixels, and turning pixels
/// back into a picture would otherwise need a PNG or JPEG writer compiled in
/// beside it -- where the browser already has both, and drawing to a canvas to
/// read a data URL back off it costs nothing but the round trip.
///
/// JPEG, not PNG: these are photographs, and a 12-megapixel PNG data URL is tens
/// of megabytes of base64 for a picture nobody will pixel-peep.
pub fn heif_to_data_url(bytes: &[u8]) -> Option<String> {
    let image = heif_oxide::decode_bytes(bytes).ok()?;
    let (width, height) = (image.width, image.height);
    let rgba = image.to_rgba8();
    let document = web_sys::window()?.document()?;
    let canvas: web_sys::HtmlCanvasElement =
        document.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(width);
    canvas.set_height(height);
    let context: web_sys::CanvasRenderingContext2d =
        canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
        wasm_bindgen::Clamped(&rgba),
        width,
        height,
    )
    .ok()?;
    context.put_image_data(&data, 0.0, 0.0).ok()?;
    // Down to something a screen can use, if it came off a modern phone.
    //
    // The portrait that prompted this is 2453x4362: eleven megapixels, which is
    // 43 MB of RGBA before the canvas holds 43 MB more of its own, and then a
    // base64 data URL of the whole thing on top. One is survivable; a list of
    // candidates decoding a dozen at once is not, and a card a few hundred
    // pixels wide had no use for any of it.
    //
    // Scaled by the browser through a second canvas rather than by hand: it
    // filters properly, and the large canvas is dropped right after.
    let longest = width.max(height);
    if longest > HEIF_MAX_EDGE {
        let scale = f64::from(HEIF_MAX_EDGE) / f64::from(longest);
        let w = (f64::from(width) * scale).round().max(1.0);
        let h = (f64::from(height) * scale).round().max(1.0);
        let small: web_sys::HtmlCanvasElement =
            document.create_element("canvas").ok()?.dyn_into().ok()?;
        small.set_width(w as u32);
        small.set_height(h as u32);
        let ctx: web_sys::CanvasRenderingContext2d =
            small.get_context("2d").ok()??.dyn_into().ok()?;
        ctx.draw_image_with_html_canvas_element_and_dw_and_dh(&canvas, 0.0, 0.0, w, h)
            .ok()?;
        return small.to_data_url_with_type("image/jpeg").ok();
    }
    canvas.to_data_url_with_type("image/jpeg").ok()
}

/// The longest edge a decoded HEIC is reduced to.
///
/// Above a retina phone's own screen, so opening the picture still shows more
/// than the page did; far below the eleven megapixels a phone camera writes,
/// which nothing here displays and which costs tens of megabytes to carry.
const HEIF_MAX_EDGE: u32 = 2048;

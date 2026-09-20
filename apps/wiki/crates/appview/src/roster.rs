//! Reading a bulk member-import roster: an .xlsx whose first sheet has
//! `Fornavn`, `Efternavn` and `Email` columns. Carried over from the interim
//! backend (`backend/src/roster.rs`), where it lives so that calamine, zip and
//! inflate ship in a server and not in every phone's bundle.
//!
//! It only reads what the caller sends and hands it back: putting those people
//! on a roster is `inviteMembers`, which checks who is asking.

use crate::session::Caller;
use crate::xrpc::{err, invalid, write_failed};
use axum::Json;
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use calamine::{Data, DataType, Reader, Xlsx};
use serde::Serialize;
use std::io::Cursor;

/// A roster is a few hundred rows. This is room for one with a logo pasted in.
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, PartialEq, Serialize)]
pub struct RosterEntry {
    pub name: String,
    /// Empty when the office has no address for them: still a row, since
    /// dropping it meant that person could never be imported.
    pub email: String,
}

/// One spreadsheet row's worth of member, or nothing if the row names neither a
/// person nor an address.
fn roster_entry(first: &str, last: &str, email: &str) -> Option<RosterEntry> {
    let name = format!("{} {}", first.trim(), last.trim())
        .trim()
        .to_string();
    let email = email.trim().to_lowercase();
    (!name.is_empty() || !email.is_empty()).then_some(RosterEntry { name, email })
}

/// The entries of an .xlsx roster's first sheet, or `None` if it is not an
/// .xlsx at all. The header row names the columns, in any case and any order.
pub fn parse_member_roster(bytes: &[u8]) -> Option<Vec<RosterEntry>> {
    let mut workbook: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).ok()?;
    let range = workbook.worksheet_range_at(0)?.ok()?;
    let mut rows = range.rows();
    let Some(header) = rows.next() else {
        return Some(Vec::new());
    };
    let column = |name: &str| {
        header.iter().position(|cell| {
            cell.as_string()
                .is_some_and(|s| s.trim().eq_ignore_ascii_case(name))
        })
    };
    let (first, last, email) = (column("Fornavn"), column("Efternavn"), column("Email"));
    let cell = |row: &[Data], at: Option<usize>| {
        at.and_then(|i| row.get(i))
            .and_then(|cell| cell.as_string())
            .unwrap_or_default()
    };
    Some(
        rows.filter_map(|row| roster_entry(&cell(row, first), &cell(row, last), &cell(row, email)))
            .collect(),
    )
}

/// `wiki.radikal.parseRoster` (procedure): the raw .xlsx as the body, its
/// rows back as `{ entries: [{ name, email }] }`. Any signed-in caller may.
pub async fn parse_roster(_caller: Caller, body: Body) -> Response {
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BYTES).await else {
        return err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "RosterTooLarge",
            "the file is too large to be a roster",
        );
    };
    // Unzipping and parsing XML is CPU work: off the async threads.
    match tokio::task::spawn_blocking(move || parse_member_roster(&bytes)).await {
        Ok(Some(entries)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "entries": entries })),
        )
            .into_response(),
        Ok(None) => invalid("that is not an .xlsx workbook"),
        Err(e) => write_failed("parseRoster", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{seeded_state, token_for};
    use axum::http::Request;
    use std::io::Write;
    use tower::ServiceExt;

    /// A real, minimal .xlsx: the five parts a workbook cannot do without, with
    /// the cells as inline strings.
    fn workbook(rows: &[&[&str]]) -> Vec<u8> {
        let sheet: String = rows
            .iter()
            .map(|row| {
                let cells: String = row
                    .iter()
                    .map(|text| format!("<c t=\"inlineStr\"><is><t>{text}</t></is></c>"))
                    .collect();
                format!("<row>{cells}</row>")
            })
            .collect();
        let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let package = "http://schemas.openxmlformats.org/package/2006";
        let parts = [
            (
                "[Content_Types].xml",
                format!(
                    "<Types xmlns=\"{package}/content-types\">\
                     <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
                     <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
                     <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
                     <Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
                     </Types>"
                ),
            ),
            (
                "_rels/.rels",
                format!(
                    "<Relationships xmlns=\"{package}/relationships\">\
                     <Relationship Id=\"rId1\" Type=\"{rel}/officeDocument\" Target=\"xl/workbook.xml\"/>\
                     </Relationships>"
                ),
            ),
            (
                "xl/workbook.xml",
                format!(
                    "<workbook xmlns=\"{main}\" xmlns:r=\"{rel}\"><sheets>\
                     <sheet name=\"Medlemmer\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
                ),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                format!(
                    "<Relationships xmlns=\"{package}/relationships\">\
                     <Relationship Id=\"rId1\" Type=\"{rel}/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
                     </Relationships>"
                ),
            ),
            (
                "xl/worksheets/sheet1.xml",
                format!("<worksheet xmlns=\"{main}\"><sheetData>{sheet}</sheetData></worksheet>"),
            ),
        ];
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, xml) in parts {
            zip.start_file(name, stored).expect("part");
            zip.write_all(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{xml}").as_bytes())
                .expect("write");
        }
        zip.finish().expect("zip").into_inner()
    }

    fn entry(name: &str, email: &str) -> RosterEntry {
        RosterEntry {
            name: name.to_string(),
            email: email.to_string(),
        }
    }

    #[test]
    fn a_workbook_is_read_by_its_headers_whatever_their_order() {
        let bytes = workbook(&[
            &["EMAIL", "Lokalforening", "efternavn", "Fornavn"],
            &[" Ada@Example.org ", "Aarhus", "Lovelace", "Ada"],
            &["", "Odense", "Sørensen", "Åse"],
            &["", "", "", ""],
            &["kun@adresse.example", "", "", ""],
        ]);
        assert_eq!(
            parse_member_roster(&bytes).expect("a workbook"),
            vec![
                entry("Ada Lovelace", "ada@example.org"),
                // The reported case: a roster whose Email column is blank. Those
                // rows used to be dropped, so the people could not be imported.
                entry("Åse Sørensen", ""),
                entry("", "kun@adresse.example"),
            ]
        );
    }

    #[test]
    fn what_is_not_a_workbook_is_said_to_be_none() {
        assert!(parse_member_roster(&[]).is_none());
        assert!(parse_member_roster(b"Fornavn;Efternavn;Email\n").is_none());
        assert_eq!(parse_member_roster(&workbook(&[])), Some(Vec::new()));
    }

    #[test]
    fn a_blank_row_is_dropped() {
        assert!(roster_entry("", "", "").is_none());
        assert!(
            roster_entry("  ", " ", "  ").is_none(),
            "whitespace is blank"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_roster_is_parsed_for_whoever_is_signed_in() {
        let state = seeded_state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let send = |token: Option<String>, bytes: Vec<u8>| {
            let app = router(state.clone());
            async move {
                let mut req = Request::builder()
                    .method("POST")
                    .uri("/xrpc/wiki.radikal.parseRoster");
                if let Some(token) = token {
                    req = req.header("authorization", format!("Bearer {token}"));
                }
                let resp = app
                    .oneshot(req.body(Body::from(bytes)).expect("request"))
                    .await
                    .expect("response");
                let status = resp.status();
                let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
                    .await
                    .expect("body");
                let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                (status, json)
            }
        };
        let sheet = workbook(&[
            &["Fornavn", "Efternavn", "Email"],
            &["Bo", "Bro", "bo@bro.example"],
        ]);

        let (status, v) = send(Some(bob.clone()), sheet.clone()).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            v["entries"],
            serde_json::json!([{"name": "Bo Bro", "email": "bo@bro.example"}])
        );
        assert_eq!(send(None, sheet).await.0, StatusCode::UNAUTHORIZED);
        let (status, v) = send(Some(bob), b"not a workbook".to_vec()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    }
}

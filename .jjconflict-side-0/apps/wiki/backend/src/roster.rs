//! Bulk member-import roster parsing, moved off the wasm client so calamine/zip/
//! inflate ship in the backend instead of every phone's bundle. Reads an .xlsx
//! whose first sheet has Fornavn/Efternavn/Email columns and returns (name,email)
//! pairs for rows carrying an email. `POST /roster/parse` with the raw file bytes
//! and an `Authorization: Bearer <nhost jwt>`.

use crate::oauth::Config;
use axum::{body::Body, extract::Request, response::Response};
use calamine::{Data, DataType, Reader, Xlsx};
use http::StatusCode;
use serde::Serialize;
use std::io::Cursor;

#[derive(Serialize)]
pub struct RosterEntry {
    pub name: String,
    pub email: String,
}

/// Parse an .xlsx roster's first sheet into (name, email) entries. Header row
/// names the columns (case-insensitive); rows without an email are skipped.
pub fn parse_member_roster(bytes: &[u8]) -> Vec<RosterEntry> {
    let mut workbook: Xlsx<_> = match Xlsx::new(Cursor::new(bytes)) {
        Ok(wb) => wb,
        Err(_) => return vec![],
    };
    let range = match workbook.worksheet_range_at(0) {
        Some(Ok(r)) => r,
        _ => return vec![],
    };
    let mut rows = range.rows();
    let Some(header) = rows.next() else {
        return vec![];
    };
    let col = |name: &str| -> Option<usize> {
        header.iter().position(|c| {
            c.as_string()
                .map(|s| s.trim().eq_ignore_ascii_case(name))
                .unwrap_or(false)
        })
    };
    let i_first = col("Fornavn");
    let i_last = col("Efternavn");
    let i_email = col("Email");
    let cell = |row: &[Data], idx: Option<usize>| -> String {
        idx.and_then(|i| row.get(i))
            .and_then(|c| c.as_string())
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for row in rows {
        let email = cell(row, i_email).to_lowercase();
        if email.is_empty() {
            continue;
        }
        let name = format!("{} {}", cell(row, i_first), cell(row, i_last))
            .trim()
            .to_string();
        out.push(RosterEntry { name, email });
    }
    out
}

/// Handle `POST /roster/parse`: verify the caller, read the uploaded .xlsx body,
/// return the parsed rows as JSON. Consumes the request body.
pub async fn handle_parse(
    cfg: &Config,
    req: Request<Body>,
    bearer: Option<&str>,
) -> Response<Body> {
    let token = match bearer.filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => {
            return crate::json(
                StatusCode::UNAUTHORIZED,
                "{\"error\":\"missing token\"}".into(),
            )
        }
    };
    if crate::nhost::verify_access_token(&token, &cfg.nhost_jwt_secret).is_err() {
        return crate::json(StatusCode::UNAUTHORIZED, "{\"error\":\"bad token\"}".into());
    }
    // Cap the upload at 8 MiB (a member roster is small).
    let bytes = match axum::body::to_bytes(req.into_body(), 8 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return crate::json(
                StatusCode::BAD_REQUEST,
                "{\"error\":\"body too large\"}".into(),
            )
        }
    };
    let entries = parse_member_roster(&bytes);
    crate::json(
        StatusCode::OK,
        serde_json::to_string(&entries).unwrap_or_else(|_| "[]".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_parse_to_no_entries() {
        assert!(parse_member_roster(&[]).is_empty());
    }
}

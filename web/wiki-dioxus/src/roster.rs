//! Bulk member-import roster parsing (the old wiki's `SheetReader` + `InvitesFab`).
//!
//! Reads an `.xlsx` whose first sheet has a header row with `Fornavn`,
//! `Efternavn` and `Email` columns and returns `(name, email)` pairs for every
//! row that carries an email. Uses [`calamine`] (pure Rust, wasm-friendly).

use std::io::Cursor;

use calamine::{Data, DataType, Reader, Xlsx};

/// One person from an imported roster: their full name and lower-cased email.
#[derive(Debug, Clone, PartialEq)]
pub struct RosterEntry {
    pub name: String,
    pub email: String,
}

/// Parse an `.xlsx` roster's first sheet into `(name, email)` entries. The header
/// row names the columns (`Fornavn` / `Efternavn` / `Email`, case-insensitive);
/// rows without an email are skipped. Returns an empty vec on any parse error.
pub fn parse_member_roster(bytes: Vec<u8>) -> Vec<RosterEntry> {
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
    // Locate the columns by header name (case-insensitive, trimmed).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The Fornavn/Efternavn/Email roster parses: emails are lower-cased, rows
    /// without an email are dropped, and a missing last name still yields a name.
    #[test]
    fn parses_the_roster_fixture() {
        let bytes = include_bytes!("../tests/fixtures/roster.xlsx").to_vec();
        let entries = parse_member_roster(bytes);
        assert_eq!(
            entries,
            vec![
                RosterEntry {
                    name: "Anna Hansen".to_string(),
                    email: "anna.hansen@example.com".to_string(),
                },
                RosterEntry {
                    name: "Bo Jensen".to_string(),
                    email: "bo@example.dk".to_string(),
                },
                RosterEntry {
                    name: "Clara".to_string(),
                    email: "clara@example.org".to_string(),
                },
            ]
        );
    }

    #[test]
    fn garbage_bytes_yield_no_entries() {
        assert!(parse_member_roster(b"not an xlsx".to_vec()).is_empty());
    }
}

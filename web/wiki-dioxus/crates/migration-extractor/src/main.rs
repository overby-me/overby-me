//! Run the extractor over a dumped interim snapshot and emit the fixtures plus
//! the field-gap report. The snapshot is a JSON file `{ "nodes": [...],
//! "members": [...] }` produced by a SEPARATE read-only dump step (the
//! census-style script), so this binary never touches the live DB and no PII
//! is embedded. A live dump is an owner-approved step.
//!
//! Usage: extract <snapshot.json>  (writes extraction.json + report.json)

use migration_extractor::{InterimMember, InterimNode, InterimUser, extract};
use serde::Deserialize;

#[derive(Deserialize)]
struct Snapshot {
    #[serde(default)]
    nodes: Vec<InterimNode>,
    #[serde(default)]
    members: Vec<InterimMember>,
    #[serde(default)]
    users: Vec<InterimUser>,
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: extract <snapshot.json>");
        std::process::exit(2);
    });
    let raw = std::fs::read_to_string(&path).expect("read snapshot");
    let snap: Snapshot = serde_json::from_str(&raw).expect("parse snapshot");
    let ex = extract(&snap.nodes, &snap.members, &snap.users);

    eprintln!(
        "extracted: {} users, {} contexts, {} documents, {} members, {} comments",
        ex.users.len(),
        ex.contexts.len(),
        ex.documents.len(),
        ex.members.len(),
        ex.comments.len()
    );
    eprintln!(
        "field-gap report: {} unmapped source fields, {} unknown mimes, {} unfilled required",
        ex.report.unmapped_source.len(),
        ex.report.unmapped_mimes.len(),
        ex.report.unfilled_required.len()
    );
    std::fs::write(
        "report.json",
        serde_json::to_string_pretty(&ex.report).unwrap(),
    )
    .expect("write report");
    std::fs::write(
        "extraction.json",
        serde_json::to_string_pretty(&ex).unwrap(),
    )
    .expect("write extraction");
}

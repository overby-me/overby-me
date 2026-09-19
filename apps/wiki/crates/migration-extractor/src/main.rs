//! Run the extractor over a dumped interim snapshot and emit the fixtures plus
//! the field-gap report. The snapshot is a JSON file `{ "nodes": [...],
//! "members": [...] }` produced by a SEPARATE read-only dump step (the
//! census-style script), so this binary never touches the live DB and no PII
//! is embedded. A live dump is an owner-approved step.
//!
//! Usage: extract <snapshot.json>  (writes extraction.json + report.json)

use migration_extractor::{Snapshot, extract_snapshot};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: extract <snapshot.json>");
        std::process::exit(2);
    });
    let raw = std::fs::read_to_string(&path).expect("read snapshot");
    let snap: Snapshot = serde_json::from_str(&raw).expect("parse snapshot");
    let ex = extract_snapshot(&snap);

    eprintln!(
        "extracted: {} users ({} to be recognized by address), {} contexts ({} public), \
         {} documents, {} members, {} comments, {} reactions, {} polls, {} canvases, {} feedback",
        ex.users.len(),
        ex.accounts.len(),
        ex.contexts.len(),
        ex.contexts
            .iter()
            .filter(|c| c.visibility == wiki_domain_types::Visibility::Public)
            .count(),
        ex.documents.len(),
        ex.members.len(),
        ex.comments.len(),
        ex.reactions.len(),
        ex.polls.len(),
        ex.canvases.len(),
        ex.feedback.len()
    );
    eprintln!(
        "field-gap report: {} unmapped source fields, {} unknown mimes, {} unfilled required",
        ex.report.unmapped_source.len(),
        ex.report.unmapped_mimes.len(),
        ex.report.unfilled_required.len()
    );
    for (what, count) in &ex.report.left_behind {
        eprintln!("left behind on purpose: {count} x {what}");
    }
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

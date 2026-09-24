//! `appview verify <extraction.json> [<files dir>]`: the cutover's go/no-go
//! gates (`docs/cutover-runbook.md`), asked of the datastore a load has filled.
//!
//! Run after `import` and `import-files`, with the service still stopped. On
//! the host nobody can look into the service's private state directory by
//! hand, so a gate that is not a command is a gate nobody checks.

use crate::AppState;
use crate::import::ImportError;
use migration_extractor::Extraction;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tokio::io::AsyncReadExt;
use turso::{Connection, Value};

/// One gate: green unless `red` names something.
#[derive(Debug)]
pub struct Gate {
    pub name: &'static str,
    /// What it counted, for the record of a green gate as much as a red one.
    pub found: String,
    pub red: Vec<String>,
}

/// How many of a gate's findings are named before the rest are only counted:
/// the journal is read by a person.
const NAMED: usize = 20;

impl Gate {
    fn new(name: &'static str) -> Self {
        Gate {
            name,
            found: String::new(),
            red: Vec::new(),
        }
    }

    fn fail(&mut self, what: String) {
        self.red.push(what);
    }

    /// The lines to print: the verdict, then what is red.
    pub fn lines(&self) -> Vec<String> {
        let verdict = if self.red.is_empty() {
            "green"
        } else {
            "RED  "
        };
        let mut lines = vec![format!("{verdict}  {}: {}", self.name, self.found)];
        lines.extend(self.red.iter().take(NAMED).map(|r| format!("         {r}")));
        if self.red.len() > NAMED {
            lines.push(format!("         and {} more", self.red.len() - NAMED));
        }
        lines
    }
}

pub async fn verify(
    state: &AppState,
    ex: &Extraction,
    files: Option<&Path>,
) -> Result<Vec<Gate>, ImportError> {
    let conn = state.db.acquire().await?;
    Ok(vec![
        everything_arrived(&conn, ex).await?,
        legacy_ids(&conn, ex).await?,
        field_gaps(ex),
        people(ex),
        waiting_seats(&conn).await?,
        paths(&conn).await?,
        authorship(&conn, ex).await?,
        the_files(state, &conn, ex, files).await?,
    ])
}

async fn column(conn: &Connection, sql: &str) -> Result<Vec<String>, turso::Error> {
    let mut rows = conn.query(sql, ()).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(row.get::<String>(0)?);
    }
    Ok(out)
}

async fn count(conn: &Connection, sql: &str) -> Result<i64, turso::Error> {
    let mut rows = conn.query(sql, ()).await?;
    match rows.next().await? {
        Some(row) => row.get::<i64>(0),
        None => Ok(0),
    }
}

/// Every row of the extraction is in its table, by id. A row the datastore has
/// besides (its own home, a configured site owner) is counted and not red.
async fn everything_arrived(conn: &Connection, ex: &Extraction) -> Result<Gate, ImportError> {
    let mut gate = Gate::new("everything arrived");
    let ids = |it: &mut dyn Iterator<Item = &String>| it.cloned().collect::<BTreeSet<String>>();
    let tables: [(&str, &str, BTreeSet<String>); 10] = [
        ("user", "did", ids(&mut ex.users.iter().map(|u| &u.did))),
        ("context", "id", ids(&mut ex.contexts.iter().map(|c| &c.id))),
        (
            "document",
            "id",
            ids(&mut ex.documents.iter().map(|d| &d.id)),
        ),
        ("member", "id", ids(&mut ex.members.iter().map(|m| &m.id))),
        ("comment", "id", ids(&mut ex.comments.iter().map(|c| &c.id))),
        (
            "reaction",
            "id",
            ids(&mut ex.reactions.iter().map(|r| &r.id)),
        ),
        ("poll", "id", ids(&mut ex.polls.iter().map(|p| &p.id))),
        ("canvas", "id", ids(&mut ex.canvases.iter().map(|c| &c.id))),
        (
            "feedback",
            "id",
            ids(&mut ex.feedback.iter().map(|f| &f.id)),
        ),
        (
            "legacy_account",
            "id",
            ids(&mut ex.accounts.iter().map(|a| &a.id)),
        ),
    ];
    let mut found = Vec::new();
    for (table, key, wanted) in tables {
        let here: BTreeSet<String> = column(conn, &format!("SELECT {key} FROM {table}"))
            .await?
            .into_iter()
            .collect();
        for missing in wanted.difference(&here) {
            gate.fail(format!("{table} {missing} is not in the datastore"));
        }
        let besides = here.difference(&wanted).count();
        found.push(match besides {
            0 => format!("{} {table}", wanted.len()),
            n => format!("{} {table} (and {n} of the datastore's own)", wanted.len()),
        });
    }
    gate.found = found.join(", ");
    Ok(gate)
}

/// A second load adds nothing only because every row is known by the id it had.
async fn legacy_ids(conn: &Connection, ex: &Extraction) -> Result<Gate, ImportError> {
    let mut gate = Gate::new("known by the id they had");
    let expected: [(&str, usize); 6] = [
        (
            "user",
            ex.users.iter().filter(|u| u.legacy_id.is_some()).count(),
        ),
        (
            "context",
            ex.contexts.iter().filter(|c| c.legacy_id.is_some()).count(),
        ),
        (
            "document",
            ex.documents
                .iter()
                .filter(|d| d.legacy_id.is_some())
                .count(),
        ),
        (
            "member",
            ex.members.iter().filter(|m| m.legacy_id.is_some()).count(),
        ),
        (
            "comment",
            ex.comments.iter().filter(|c| c.legacy_id.is_some()).count(),
        ),
        (
            "reaction",
            ex.reactions
                .iter()
                .filter(|r| r.legacy_id.is_some())
                .count(),
        ),
    ];
    let mut total = 0;
    for (table, wanted) in expected {
        let sql = format!("SELECT count(*) FROM {table} WHERE legacy_id IS NOT NULL");
        let here = usize::try_from(count(conn, &sql).await?).unwrap_or(0);
        if here != wanted {
            gate.fail(format!(
                "{table}: {here} rows carry a legacy id, of {wanted}"
            ));
        }
        total += here;
    }
    gate.found = format!("{total} rows");
    Ok(gate)
}

fn field_gaps(ex: &Extraction) -> Gate {
    let mut gate = Gate::new("nothing without a home");
    let report = &ex.report;
    for (key, entry) in &report.unmapped_source {
        gate.fail(format!("{} x {key}: {}", entry.count, entry.note));
    }
    for (mime, n) in &report.unmapped_mimes {
        gate.fail(format!("{n} x a node of kind {mime}, which nothing maps"));
    }
    for (field, n) in &report.unfilled_required {
        gate.fail(format!("{n} x {field} had nothing to fill it"));
    }
    let left: u64 = report.left_behind.values().sum();
    let reshaped: u64 = report.reshaped.values().map(|e| e.count).sum();
    gate.found = format!(
        "{left} things left behind on purpose and {reshaped} carried in another shape, \
         each kind of which is for a person to read in the report"
    );
    gate
}

/// Whoever is not recognized by address needs a claim link for each seat. Most
/// of them is a decision to take before the flip, so it stops the flip.
fn people(ex: &Extraction) -> Gate {
    let mut gate = Gate::new("people can get back in");
    let (all, by_address) = (ex.users.len(), ex.accounts.len());
    gate.found = format!("{by_address} of {all} accounts are recognized by their address");
    if by_address * 2 < all {
        gate.fail(format!(
            "{} accounts would each need a claim link for every seat",
            all - by_address
        ));
    }
    gate
}

/// One address waits for one seat in a context, or an invitation is two rows
/// and the person who takes one leaves the other behind them.
async fn waiting_seats(conn: &Connection) -> Result<Gate, ImportError> {
    let mut gate = Gate::new("one waiting seat to an address");
    let mut rows = conn
        .query(
            "SELECT context_id, email FROM member WHERE user_did IS NULL AND email IS NOT NULL",
            (),
        )
        .await?;
    let mut seen = BTreeSet::new();
    let mut waiting = 0;
    while let Some(row) = rows.next().await? {
        waiting += 1;
        let context: String = row.get(0)?;
        if !seen.insert((context.clone(), row.get::<String>(1)?)) {
            gate.fail(format!("an address waits twice in context {context}"));
        }
    }
    let addresses: BTreeSet<&String> = seen.iter().map(|(_, email)| email).collect();
    gate.found = format!("{waiting} waiting seats, of {} addresses", addresses.len());
    Ok(gate)
}

struct Placed {
    id: String,
    parent: Option<String>,
    slug: String,
    path: String,
    live: bool,
    home: bool,
}

async fn placed(conn: &Connection, table: &str, home: &str) -> Result<Vec<Placed>, turso::Error> {
    let sql = format!("SELECT id, parent_id, slug, path, deleted_at IS NULL, {home} FROM {table}");
    let mut rows = conn.query(&sql, ()).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(Placed {
            id: row.get(0)?,
            parent: match row.get_value(1)? {
                Value::Text(parent) => Some(parent),
                _ => None,
            },
            slug: row.get(2)?,
            path: row.get(3)?,
            live: row.get::<i64>(4)? == 1,
            home: row.get::<i64>(5)? == 1,
        });
    }
    Ok(out)
}

/// A node is reached by its path, so a live one has a path nobody else has,
/// and it is its parent's path and its own slug: the tree and the URLs agree.
async fn paths(conn: &Connection) -> Result<Gate, ImportError> {
    let mut gate = Gate::new("every node is where its URL says");
    let mut nodes = placed(conn, "context", "kind = 'home'").await?;
    nodes.extend(placed(conn, "document", "0").await?);
    let path_of: BTreeMap<&str, &str> = nodes
        .iter()
        .map(|n| (n.id.as_str(), n.path.as_str()))
        .collect();
    let mut held: BTreeMap<&str, &str> = BTreeMap::new();
    for n in nodes.iter().filter(|n| n.live) {
        if n.path.is_empty() && !n.home {
            gate.fail(format!("{} has no path", n.id));
        }
        if let Some(other) = held.insert(n.path.as_str(), n.id.as_str()) {
            gate.fail(format!("{} and {other} are both at one path", n.id));
        }
        let Some(above) = n.parent.as_deref().and_then(|p| path_of.get(p)) else {
            continue;
        };
        let expected = match *above {
            "" => n.slug.clone(),
            above => format!("{above}/{}", n.slug),
        };
        if n.path != expected {
            gate.fail(format!("{} is not under its parent's path", n.id));
        }
    }
    gate.found = format!("{} live paths", held.len());
    Ok(gate)
}

async fn authorship(conn: &Connection, ex: &Extraction) -> Result<Gate, ImportError> {
    let mut gate = Gate::new("authorship preserved");
    let mut rows = conn
        .query(
            "SELECT document_id, count(*) FROM document_author GROUP BY document_id",
            (),
        )
        .await?;
    let mut here: BTreeMap<String, i64> = BTreeMap::new();
    while let Some(row) = rows.next().await? {
        here.insert(row.get(0)?, row.get(1)?);
    }
    let mut authors = 0;
    for d in ex.documents.iter().filter(|d| !d.authors.is_empty()) {
        authors += d.authors.len();
        let kept = here.get(&d.id).copied().unwrap_or(0);
        if usize::try_from(kept).unwrap_or(0) != d.authors.len() {
            gate.fail(format!(
                "document {} has {kept} authors, of {}",
                d.id,
                d.authors.len()
            ));
        }
    }
    gate.found = format!("{authors} authors, by account, by name or as a group");
    Ok(gate)
}

async fn sha256_of(path: &Path) -> std::io::Result<(String, u64)> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut size = 0;
    loop {
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
        size += read as u64;
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((digest, size))
}

/// Every file something points at is in the store, whole: the size storage
/// reported, and bytes that hash to what the row says. A file the interim's own
/// storage no longer had was lost before the move, and is counted, not red.
async fn the_files(
    state: &AppState,
    conn: &Connection,
    ex: &Extraction,
    files: Option<&Path>,
) -> Result<Gate, ImportError> {
    let mut gate = Gate::new("every file came across");
    let listed: Option<BTreeMap<String, Option<u64>>> = match files {
        Some(dir) => match tokio::fs::read(dir.join("manifest.json")).await {
            Ok(raw) => Some(
                serde_json::from_slice::<Vec<crate::import::Listed>>(&raw)?
                    .into_iter()
                    .map(|l| (l.id, l.size))
                    .collect(),
            ),
            Err(e) => {
                gate.fail(format!("no manifest.json among the files: {e}"));
                None
            }
        },
        None => None,
    };
    let mut rows = conn
        .query("SELECT id FROM context WHERE kind = 'home'", ())
        .await?;
    let home: Option<String> = match rows.next().await? {
        Some(row) => Some(row.get(0)?),
        None => None,
    };
    drop(rows);

    let wanted: BTreeSet<String> = crate::import::wanted(ex, home.as_deref())
        .into_iter()
        .map(|w| w.id)
        .collect();
    let (mut whole, mut bytes, mut lost_before) = (0, 0u64, 0);
    for id in &wanted {
        let said = listed.as_ref().map(|l| l.get(id));
        if said == Some(None) {
            lost_before += 1;
            continue;
        }
        let Some(blob) = crate::blob::meta(state, id).await? else {
            gate.fail(format!("file {id} is pointed at and not in the store"));
            continue;
        };
        if let Some(Some(Some(size))) = said
            && i64::try_from(*size).ok() != Some(blob.size)
        {
            gate.fail(format!("file {id} is {} bytes, of {size}", blob.size));
            continue;
        }
        match sha256_of(&crate::blob::path_of(&state.config, &blob.sha256)).await {
            Ok((digest, size)) if digest == blob.sha256 => {
                whole += 1;
                bytes += size;
            }
            Ok(_) => gate.fail(format!("file {id} does not hash to what its row says")),
            Err(e) => gate.fail(format!("file {id} cannot be read from the store: {e}")),
        }
    }
    gate.found = format!(
        "{whole} of {} files pointed at are in the store whole ({bytes} bytes); \
         {lost_before} the interim's storage had already lost",
        wanted.len()
    );
    Ok(gate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::tests::{fresh, interim};
    use crate::import::{import, import_files};
    use serde_json::json;

    /// A loaded wiki with two files: one whole, one the interim had lost.
    async fn loaded() -> (AppState, Extraction, std::path::PathBuf) {
        let mut ex = interim();
        let motion = ex.documents.iter_mut().find(|d| d.id == "mo").expect("mo");
        motion.data = Some(json!({"fileId": "f-agenda", "image": "f-lost"}));
        motion.authors = vec![wiki_domain_types::Author::FreeText {
            display: "Carl".into(),
        }];
        let mut state = fresh().await;
        let dir = std::env::temp_dir().join(format!("verify-{}", crate::util::random_token(8)));
        state.config.blob_dir = dir.join("blobs").to_string_lossy().into_owned();
        let files = dir.join("files");
        std::fs::create_dir_all(&files).expect("dir");
        std::fs::write(files.join("f-agenda"), b"%PDF the agenda").expect("file");
        let manifest = json!([{"id": "f-agenda", "mimeType": "application/pdf", "size": 15}]);
        std::fs::write(files.join("manifest.json"), manifest.to_string()).expect("manifest");
        import(&state.db, &ex).await.expect("import");
        import_files(&state, &ex, &files).await.expect("files");
        (state, ex, files)
    }

    fn red(gates: &[Gate]) -> Vec<&'static str> {
        gates
            .iter()
            .filter(|g| !g.red.is_empty())
            .map(|g| g.name)
            .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_wiki_that_came_across_whole_passes_every_gate() {
        let (state, ex, files) = loaded().await;
        let gates = verify(&state, &ex, Some(&files)).await.expect("verify");
        assert!(red(&gates).is_empty(), "{gates:#?}");
        let of_files = gates.last().expect("the files gate");
        assert!(
            of_files.found.starts_with("1 of 2 files") && of_files.found.contains("1 the interim"),
            "a file storage never listed was lost before the move: {}",
            of_files.found
        );
    }

    /// Each gate goes red for the loss it is there to catch, and only that one.
    #[tokio::test(flavor = "current_thread")]
    async fn each_gate_catches_what_it_is_for() {
        let (state, ex, files) = loaded().await;
        let conn = state.db.acquire().await.expect("conn");
        let after = |sql: &'static str| {
            let (state, ex, files, conn) = (&state, &ex, &files, &conn);
            async move {
                conn.execute(sql, ()).await.expect(sql);
                let gates = verify(state, ex, Some(files)).await.expect("verify");
                red(&gates).join(", ")
            }
        };

        assert_eq!(
            after("DELETE FROM document_author WHERE document_id = 'mo'").await,
            "authorship preserved"
        );
        conn.execute("DELETE FROM document_author", ())
            .await
            .expect("authors");
        let mut bare = ex;
        bare.documents.iter_mut().for_each(|d| d.authors.clear());
        let ex = bare;
        let after = |sql: &'static str| {
            let (state, ex, files, conn) = (&state, &ex, &files, &conn);
            async move {
                conn.execute(sql, ()).await.expect(sql);
                let gates = verify(state, ex, Some(files)).await.expect("verify");
                red(&gates).join(", ")
            }
        };
        assert_eq!(
            after("UPDATE document SET path = 'elsewhere' WHERE id = 'mo'").await,
            "every node is where its URL says"
        );
        conn.execute(
            "UPDATE document SET path = 'hb/forslag_1' WHERE id = 'mo'",
            (),
        )
        .await
        .expect("back");
        assert_eq!(
            after("DELETE FROM member WHERE id = 'm-carl'").await,
            "everything arrived, known by the id they had"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_file_that_rotted_in_the_store_is_caught() {
        let (state, ex, files) = loaded().await;
        let blob = crate::blob::meta(&state, "f-agenda")
            .await
            .expect("meta")
            .expect("the blob");
        let kept = crate::blob::path_of(&state.config, &blob.sha256);
        std::fs::write(&kept, b"%PDF the agendA").expect("rot");
        let gates = verify(&state, &ex, Some(&files)).await.expect("verify");
        assert_eq!(red(&gates), ["every file came across"], "{gates:#?}");

        std::fs::remove_file(&kept).expect("gone");
        let gates = verify(&state, &ex, None).await.expect("verify");
        let of_files = gates.last().expect("the files gate");
        assert_eq!(
            of_files.red.len(),
            2,
            "with no manifest, the lost one too: {of_files:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_the_extractor_could_not_place_stops_the_flip() {
        let (state, mut ex, files) = loaded().await;
        ex.report.unmapped_mimes.insert("map/map".into(), 1);
        ex.accounts.clear();
        let gates = verify(&state, &ex, Some(&files)).await.expect("verify");
        assert_eq!(
            red(&gates),
            ["nothing without a home", "people can get back in"],
            "what the datastore has besides is counted and not red: {gates:#?}"
        );
    }
}

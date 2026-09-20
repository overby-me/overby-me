//! `appview import <extraction.json>`: the load step of the cutover.
//!
//! The migration loader fills the entity tables. What a poll came to, what was
//! painted on a canvas and what people reported live in tables this crate owns,
//! so those are loaded here. All of it is one transaction: a load that fails
//! leaves the datastore as it found it, and loading the same extraction again
//! adds nothing.
//!
//! Run it with the service stopped, into a datastore nobody has signed in to:
//! loading again over one that has been lived in would bring back whatever was
//! deleted there since. The search index is rebuilt at every start.

use crate::{Db, DbError};
use migration_extractor::Extraction;
use migration_loader::{LoadError, LoadStats};
use turso::{Connection, Value};
use wiki_domain_types::{Canvas, ContextKind, Feedback, Poll};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub entities: LoadStats,
    /// Accounts their person can take over by signing in (`crate::legacy`).
    pub accounts: usize,
    pub polls: usize,
    pub canvases: usize,
    pub cells: usize,
    pub feedback: usize,
}

#[derive(Debug)]
pub enum ImportError {
    Read(std::io::Error),
    Parse(serde_json::Error),
    Load(LoadError),
    Db(DbError),
    /// Somebody has signed in to this datastore.
    LivedIn,
    /// The datastore has a home that things have been put in.
    HomeTaken,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Read(e) => write!(f, "cannot read the extraction: {e}"),
            ImportError::Parse(e) => write!(f, "the extraction does not parse: {e}"),
            ImportError::Load(e) => write!(f, "{e}"),
            ImportError::Db(e) => write!(f, "{e}"),
            ImportError::LivedIn => write!(
                f,
                "this datastore has been signed in to, and loading over it would bring back \
                 what was deleted there since; load into a fresh one"
            ),
            ImportError::HomeTaken => write!(
                f,
                "this datastore already has a home with things in it, and a wiki has one home; \
                 load into a fresh one"
            ),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<LoadError> for ImportError {
    fn from(e: LoadError) -> Self {
        ImportError::Load(e)
    }
}
impl From<DbError> for ImportError {
    fn from(e: DbError) -> Self {
        ImportError::Db(e)
    }
}
impl From<turso::Error> for ImportError {
    fn from(e: turso::Error) -> Self {
        ImportError::Db(DbError::Turso(e))
    }
}
impl From<serde_json::Error> for ImportError {
    fn from(e: serde_json::Error) -> Self {
        ImportError::Parse(e)
    }
}

/// Load the extraction written to `path` by `extract`.
pub async fn import_file(db: &Db, path: &str) -> Result<ImportStats, ImportError> {
    let raw = tokio::fs::read(path).await.map_err(ImportError::Read)?;
    import(db, &serde_json::from_slice(&raw)?).await
}

pub async fn import(db: &Db, ex: &Extraction) -> Result<ImportStats, ImportError> {
    let conn = db.acquire().await?;
    let _turn = db.write_turn().await;
    conn.execute("BEGIN IMMEDIATE", ()).await?;
    match load_all(&conn, ex).await {
        Ok(stats) => {
            conn.execute("COMMIT", ()).await?;
            Ok(stats)
        }
        Err(e) => {
            // Best effort: the error worth reporting is the one that got us here.
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(e)
        }
    }
}

async fn load_all(conn: &Connection, ex: &Extraction) -> Result<ImportStats, ImportError> {
    let mut sessions = conn.query("SELECT 1 FROM session LIMIT 1", ()).await?;
    if sessions.next().await?.is_some() {
        return Err(ImportError::LivedIn);
    }
    drop(sessions);
    if ex.contexts.iter().any(|c| c.kind == ContextKind::Home) {
        make_way_for_the_home(conn).await?;
    }
    let mut stats = ImportStats {
        entities: migration_loader::load(conn, ex).await?,
        ..ImportStats::default()
    };
    for account in &ex.accounts {
        stats.accounts += conn
            .execute(
                "INSERT INTO legacy_account (id, email) VALUES (?1, ?2) ON CONFLICT (id) DO NOTHING",
                [account.id.as_str(), account.email.as_str()],
            )
            .await? as usize;
    }
    for poll in &ex.polls {
        if load_poll(conn, poll).await? {
            stats.polls += 1;
        }
    }
    for canvas in &ex.canvases {
        if let Some(cells) = load_canvas(conn, canvas).await? {
            stats.canvases += 1;
            stats.cells += cells;
        }
    }
    for report in &ex.feedback {
        if load_feedback(conn, report).await? {
            stats.feedback += 1;
        }
    }
    Ok(stats)
}

/// What came of copying the interim's files across.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileStats {
    pub copied: usize,
    /// Here already, from an earlier run.
    pub already: usize,
    /// Pointed at and not copied: `(file id, why)`.
    pub failed: Vec<(String, String)>,
    /// In the directory with nothing pointing at it. Not copied.
    pub unreferenced: Vec<String>,
}

/// A file something in the extraction points at, and whose it is to read.
pub(crate) struct Wanted {
    pub(crate) id: String,
    context_id: String,
    owner: Option<String>,
    name: Option<String>,
    mime: Option<String>,
}

/// What the interim's storage said of a file (its `files` rows, as dumped).
#[derive(Debug, serde::Deserialize)]
pub(crate) struct Listed {
    pub(crate) id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "mimeType", default)]
    mime_type: Option<String>,
    #[serde(default)]
    pub(crate) size: Option<u64>,
}

pub(crate) fn wanted(ex: &Extraction, home: Option<&str>) -> Vec<Wanted> {
    let text = |data: &Option<serde_json::Value>, key: &str| {
        data.as_ref()
            .and_then(|d| d.get(key))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let mut wanted = Vec::new();
    for d in &ex.documents {
        if let Some(id) = text(&d.data, "fileId") {
            wanted.push(Wanted {
                id,
                context_id: d.context_id.clone(),
                owner: d.place.owner_did.clone(),
                name: Some(d.title.clone()),
                mime: text(&d.data, "type"),
            });
        }
        if let Some(id) = text(&d.data, "image") {
            wanted.push(Wanted {
                id,
                context_id: d.context_id.clone(),
                owner: d.place.owner_did.clone(),
                name: None,
                mime: None,
            });
        }
    }
    for c in &ex.contexts {
        if let Some(id) = text(&c.data, "image") {
            wanted.push(Wanted {
                id,
                context_id: c.id.clone(),
                owner: c.place.owner_did.clone(),
                name: None,
                mime: None,
            });
        }
    }
    for k in &ex.comments {
        if let Some(id) = &k.image {
            wanted.push(Wanted {
                id: id.clone(),
                context_id: k.context_id.clone(),
                owner: k.author.did().map(str::to_string),
                name: None,
                mime: None,
            });
        }
    }
    // A report belongs to no group. Its screenshot is for whoever runs the site.
    for report in &ex.feedback {
        if let (Some(id), Some(home)) = (&report.image, home) {
            wanted.push(Wanted {
                id: id.clone(),
                context_id: home.to_string(),
                owner: report.owner_did.clone(),
                name: None,
                mime: None,
            });
        }
    }
    wanted
}

/// Copy the interim's files, downloaded into `dir` under their ids, into the
/// blob store under those same ids, so that nothing that points at one needs
/// rewriting. Each is filed under the context of what points at it, which is
/// who gets to read it. A file nothing points at is listed and left. Run after
/// [`import`], and as often as it takes: what is here already is passed over.
pub async fn import_files(
    state: &crate::AppState,
    ex: &Extraction,
    dir: &std::path::Path,
) -> Result<FileStats, ImportError> {
    let listed: Vec<Listed> = match tokio::fs::read(dir.join("manifest.json")).await {
        Ok(raw) => serde_json::from_slice(&raw)?,
        Err(_) => Vec::new(),
    };
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query("SELECT id FROM context WHERE kind = 'home'", ())
        .await?;
    let home: Option<String> = match rows.next().await? {
        Some(row) => Some(row.get(0)?),
        None => None,
    };
    drop(rows);

    let mut stats = FileStats::default();
    let mut seen = std::collections::BTreeSet::new();
    for file in wanted(ex, home.as_deref()) {
        // A file id is a path segment here, and the dump is not ours.
        let plain = file
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !seen.insert(file.id.clone()) || !plain {
            continue;
        }
        if exists(&conn, "blob", &file.id).await? {
            stats.already += 1;
            continue;
        }
        let said = listed.iter().find(|l| l.id == file.id);
        let source = dir.join(&file.id);
        let size = tokio::fs::metadata(&source).await.ok().map(|m| m.len());
        let refusal = match (size, said.and_then(|l| l.size)) {
            (None, _) => Some("not among the downloaded files".to_string()),
            (Some(read), Some(said)) if read != said => {
                Some(format!("{read} bytes were downloaded, of {said}"))
            }
            _ => None,
        };
        if let Some(why) = refusal {
            stats.failed.push((file.id, why));
            continue;
        }
        let blob = crate::blob::BlobMeta {
            id: file.id.clone(),
            context_id: file.context_id,
            owner_did: match known(&conn, file.owner.as_deref()).await? {
                Value::Text(did) => Some(did),
                _ => None,
            },
            sha256: String::new(),
            size: 0,
            mime: file
                .mime
                .or_else(|| said.and_then(|l| l.mime_type.clone()))
                .unwrap_or_default(),
            name: file.name.or_else(|| said.and_then(|l| l.name.clone())),
        };
        match crate::blob::file_a_copy(state, &source, blob).await {
            Ok(_) => stats.copied += 1,
            Err(e) => stats.failed.push((file.id, e.to_string())),
        }
    }
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != "manifest.json" && !seen.contains(&name) {
                stats.unreferenced.push(name);
            }
        }
    }
    stats.unreferenced.sort();
    Ok(stats)
}

/// The service gives a datastore a home the first time it starts. Started once
/// before the load, that empty home is in the way of the one being loaded, and
/// goes. One that has been used is not this command's to remove.
async fn make_way_for_the_home(conn: &Connection) -> Result<(), ImportError> {
    let mut rows = conn
        .query(
            // A loaded home has a `legacy_id`, and is this same load run again.
            "SELECT id, EXISTS (SELECT 1 FROM context k WHERE k.parent_id = h.id) \
                     OR EXISTS (SELECT 1 FROM document d WHERE d.context_id = h.id) \
                     OR EXISTS (SELECT 1 FROM blob b WHERE b.context_id = h.id) \
             FROM context h WHERE h.kind = 'home' AND h.legacy_id IS NULL",
            (),
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(());
    };
    let (id, used): (String, bool) = (row.get(0)?, row.get::<i64>(1)? != 0);
    drop(rows);
    if used {
        return Err(ImportError::HomeTaken);
    }
    for sql in [
        "DELETE FROM member WHERE context_id = ?1",
        "DELETE FROM search_index WHERE node_id = ?1",
        "DELETE FROM context WHERE id = ?1",
    ] {
        conn.execute(sql, [id.as_str()]).await?;
    }
    Ok(())
}

async fn exists(conn: &Connection, table: &str, id: &str) -> Result<bool, turso::Error> {
    let mut rows = conn
        .query(&format!("SELECT 1 FROM {table} WHERE id = ?1"), [id])
        .await?;
    Ok(rows.next().await?.is_some())
}

fn opt(s: Option<&str>) -> Value {
    s.map_or(Value::Null, |s| Value::Text(s.to_string()))
}

fn int(n: impl TryInto<i64>) -> Value {
    Value::Integer(n.try_into().unwrap_or(i64::MAX))
}

/// A DID only if the account came across, since the columns that name one hold
/// a foreign key to it.
async fn known(conn: &Connection, did: Option<&str>) -> Result<Value, turso::Error> {
    let Some(did) = did else {
        return Ok(Value::Null);
    };
    let mut rows = conn
        .query("SELECT 1 FROM user WHERE did = ?1", [did])
        .await?;
    Ok(rows.next().await?.map_or(Value::Null, |_| opt(Some(did))))
}

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

/// Closed, with its result and no roster, board or issuer key: a poll the
/// interim ran can be read here and never voted in.
async fn load_poll(conn: &Connection, poll: &Poll) -> Result<bool, ImportError> {
    if exists(conn, "poll", &poll.id).await? {
        return Ok(false);
    }
    conn.execute(
        &format!(
            "INSERT INTO poll (id, context_id, question, options, min_choices, max_choices, blank,
                               open, secret, hide_tally, counts, ballots, created_at, closed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10, ?11,
                     coalesce(?12, {NOW}), coalesce(?13, ?12, {NOW}))"
        ),
        vec![
            Value::Text(poll.id.clone()),
            Value::Text(poll.context_id.clone()),
            Value::Text(poll.question.clone()),
            Value::Text(serde_json::to_string(&poll.options)?),
            int(poll.min),
            int(poll.max),
            int(poll.blank),
            int(poll.secret),
            int(poll.hide_tally),
            Value::Text(serde_json::to_string(&poll.counts)?),
            int(poll.ballots),
            opt(poll.created_at.as_deref()),
            opt(poll.closed_at.as_deref()),
        ],
    )
    .await?;
    Ok(true)
}

/// Returns how many cells came with it, or `None` for a canvas already here.
async fn load_canvas(conn: &Connection, canvas: &Canvas) -> Result<Option<usize>, ImportError> {
    if exists(conn, "canvas", &canvas.id).await? {
        return Ok(None);
    }
    conn.execute(
        "INSERT INTO canvas (id, width, height, cooldown, open) VALUES (?1, ?2, ?3, ?4, ?5)",
        vec![
            Value::Text(canvas.id.clone()),
            int(canvas.width),
            int(canvas.height),
            int(canvas.cooldown),
            int(canvas.open),
        ],
    )
    .await?;
    let mut cells = 0;
    for cell in &canvas.cells {
        if cell.x >= canvas.width || cell.y >= canvas.height {
            continue;
        }
        cells += conn
            .execute(
                &format!(
                    "INSERT INTO canvas_cell (canvas_id, x, y, colour, painter, painted_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, coalesce(?6, {NOW}))
                     ON CONFLICT (canvas_id, x, y) DO NOTHING"
                ),
                vec![
                    Value::Text(canvas.id.clone()),
                    int(cell.x),
                    int(cell.y),
                    int(cell.colour),
                    known(conn, cell.painter_did.as_deref()).await?,
                    opt(cell.painted_at.as_deref()),
                ],
            )
            .await? as usize;
    }
    Ok(Some(cells))
}

async fn load_feedback(conn: &Connection, report: &Feedback) -> Result<bool, ImportError> {
    if exists(conn, "feedback", &report.id).await? {
        return Ok(false);
    }
    conn.execute(
        &format!(
            "INSERT INTO feedback (id, kind, message, path, app_version, build, user_agent, image,
                                   digest, seen, owner_did, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                     coalesce(?12, {NOW}), coalesce(?13, ?12, {NOW}))"
        ),
        vec![
            Value::Text(report.id.clone()),
            Value::Text(report.kind.clone()),
            Value::Text(report.message.clone()),
            Value::Text(report.path.clone()),
            Value::Text(report.app_version.clone()),
            Value::Text(report.commit.clone()),
            Value::Text(report.user_agent.clone()),
            opt(report.image.as_deref()),
            opt(report.digest.as_deref()),
            int(report.seen),
            known(conn, report.owner_did.as_deref()).await?,
            opt(report.created_at.as_deref()),
            opt(report.updated_at.as_deref()),
        ],
    )
    .await?;
    for reporter in &report.reporters {
        conn.execute(
            "INSERT INTO feedback_reporter (feedback_id, reporter) VALUES (?1, ?2)
             ON CONFLICT (feedback_id, reporter) DO NOTHING",
            [report.id.as_str(), reporter.as_str()],
        )
        .await?;
    }
    Ok(true)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::profile::{account_from, apply};
    use crate::xrpc::tests::{get_as, token_for};
    use crate::{AppState, Config, router};
    use axum::http::StatusCode;
    use migration_extractor::{Snapshot, extract_snapshot};
    use serde_json::json;

    const ALICE: &str = "did:plc:alice";

    /// A small wiki as the interim dumps it, through the real extractor.
    pub(crate) fn interim() -> Extraction {
        let at = "2026-01-05T10:00:00+00:00";
        let node = |id: &str, mime: &str, key: &str, parent: &str, more: serde_json::Value| {
            let mut row = json!({
                "id": id, "name": id, "key": key, "mimeId": mime, "parentId": parent,
                "contextId": "hb", "ownerId": "u-alice", "createdAt": at, "updatedAt": at
            });
            for (key, value) in more.as_object().expect("an object") {
                row[key] = value.clone();
            }
            row
        };
        let ballot = |id: &str, chosen: u64| {
            node(
                id,
                "vote/vote",
                id,
                "p1",
                json!({"ownerId": null, "data": [chosen]}),
            )
        };
        let snapshot: Snapshot = serde_json::from_value(json!({
            "users": [
                {"id": "u-alice", "displayName": "Alice", "email": "Alice@X.dk", "emailVerified": true},
                {"id": "u-bob", "displayName": "Bob", "email": "bob@x.dk", "emailVerified": false},
            ],
            "members": [
                {"id": "m-alice", "nodeId": "u-alice", "parentId": "hb", "owner": true,
                 "active": true, "accepted": true, "email": "alice@x.dk", "claimToken": "spent"},
                {"id": "m-carl", "parentId": "hb", "name": "Carl", "email": "carl@x.dk",
                 "active": true, "claimToken": "tok-carl"},
                {"id": "m-site", "nodeId": "u-alice", "parentId": "home", "owner": true,
                 "active": true, "accepted": true},
            ],
            "permissions": [],
            "nodes": [
                {"id": "home", "key": "", "mimeId": "wiki/home", "name": "Radikal Ungdom",
                 "contextId": "home",
                 "data": {"content": [{"children": [{"text": "Velkommen"}]}]}},
                node("hb", "wiki/group", "hb", "home", json!({"name": "Hovedbestyrelsen"})),
                node("mo", "vote/policy", "forslag_1", "hb", json!({"name": "Forslag 1"})),
                node("p1", "vote/poll", "afstemning", "mo", json!({
                    "name": "Forslag 1", "mutable": false,
                    "data": {"options": ["for", "imod", "blank"], "minVote": 1, "maxVote": 1}
                })),
                ballot("v1", 0), ballot("v2", 0), ballot("v3", 1),
                node("cv", "canvas/canvas", "tavlen", "hb", json!({
                    "name": "Tavlen", "data": {"w": 8, "h": 8, "cooldown": 30}
                })),
                node("x1", "canvas/pixel", "p_3_4", "cv", json!({"data": {"c": 7}})),
                node("x2", "canvas/pixel", "p_0_0", "cv", json!({"ownerId": "u-gone", "data": {"c": 2}})),
                node("x3", "canvas/pixel", "p_9_9", "cv", json!({"data": {"c": 1}})),
                node("k1", "vote/comment", "k1", "mo", json!({"data": {"text": "Godt forslag"}})),
                node("k2", "vote/comment", "k2", "mo", json!({
                    "data": {"text": "Fortrudt"}, "deleted_at": at, "deleted_root": "k2"
                })),
                node("r1", "vote/reaction", "r1", "k1", json!({"name": "🎉", "data": {"emoji": "🎉"}})),
                node("fb", "wiki/feedback", "fb", "home", json!({
                    "contextId": null, "ownerId": "u-gone",
                    "data": {"kind": "crash", "message": "panicked", "crashDigest": "00ff",
                             "seen": 3, "reporters": ["u-alice", "anonymous"]}
                })),
            ],
        }))
        .expect("a snapshot");
        extract_snapshot(&snapshot)
    }

    pub(crate) async fn fresh() -> AppState {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        AppState::new(db, Config::default())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_wiki_comes_across_and_its_people_find_it_as_they_left_it() {
        let ex = interim();
        assert!(
            ex.report.unmapped_source.is_empty() && ex.report.unmapped_mimes.is_empty(),
            "{:?}",
            ex.report
        );
        let state = fresh().await;
        let stats = import(&state.db, &ex).await.expect("import");
        assert_eq!(
            stats,
            ImportStats {
                entities: LoadStats {
                    users: 2,
                    contexts: 2,
                    documents: 3,
                    document_authors: 0,
                    members: 3,
                    comments: 2,
                    reactions: 1,
                },
                accounts: 1,
                polls: 1,
                canvases: 1,
                cells: 2,
                feedback: 1,
            },
            "the cell off the board is not painted"
        );
        assert_eq!(
            import(&state.db, &ex).await.expect("again"),
            ImportStats::default(),
            "loading it twice adds nothing"
        );

        // Alice signs in with the address her old account was registered under.
        let alice = token_for(&state, ALICE).await;
        let session =
            json!({"handle": "alice.example", "email": "alice@x.dk", "emailConfirmed": true});
        let account = account_from(ALICE, "https://bsky.social", &session, None);
        assert_eq!(apply(&state, ALICE, &account).await.expect("apply"), 2);

        let read = |uri: &'static str| get_as(router(state.clone()), uri, &alice);
        let (status, v) = read("/xrpc/wiki.radikal.getNode?path=hb/forslag_1").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["viewer"]["is_context_owner"], true, "{v}");

        // She runs the site, as she did: its welcome is hers to read and change,
        // and the reports are hers to see.
        let (status, v) = read("/xrpc/wiki.radikal.getNode?path=").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["node"]["name"], "Radikal Ungdom");
        assert_eq!(v["node"]["content"][0]["children"][0]["text"], "Velkommen");
        assert_eq!(v["children"][0]["path"], "hb", "{v}");
        let (status, v) = read("/xrpc/wiki.radikal.listFeedback").await;
        assert_eq!(status, StatusCode::OK, "{v}");

        let (status, v) = read("/xrpc/wiki.radikal.getPoll?id=p1").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            (&v["open"], &v["question"]),
            (&json!(false), &json!("Forslag 1"))
        );
        assert_eq!(v["counts"], json!([2, 1, 0]), "{v}");
        assert_eq!(
            (&v["ballots"], &v["blank"]),
            (&json!(3), &json!(true)),
            "{v}"
        );

        let (status, v) = read("/xrpc/wiki.radikal.getCanvas?id=cv").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["cells"].as_array().expect("cells").len(), 2, "{v}");
        assert_eq!(
            v["painters"],
            json!([ALICE]),
            "who is gone painted as nobody: {v}"
        );

        let (status, v) = read("/xrpc/wiki.radikal.getComments?on=mo").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["comments"][0]["text"], "Godt forslag", "{v}");
        assert_eq!(
            v["comments"].as_array().expect("comments").len(),
            1,
            "what she had deleted came back: {v}"
        );
        let (_, v) = read("/xrpc/wiki.radikal.listDeleted?context=hb").await;
        assert_eq!(
            v["deleted"][0]["title"], "Fortrudt",
            "it is in the bin, as it was: {v}"
        );
        let (_, v) = read("/xrpc/wiki.radikal.getReactions?subject=k1").await;
        assert_eq!(v["reactions"][0]["emoji"], "🎉", "{v}");

        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query("SELECT seen, owner_did FROM feedback WHERE id = 'fb'", ())
            .await
            .expect("q");
        let row = rows.next().await.expect("next").expect("the report");
        assert_eq!(row.get::<i64>(0).expect("seen"), 3);
        assert!(matches!(row.get_value(1).expect("owner"), Value::Null));

        // Lived in now, so a load over it would undo what people do from here on.
        assert!(matches!(
            import(&state.db, &ex).await,
            Err(ImportError::LivedIn)
        ));
    }

    /// The service gives a datastore a home the first time it starts, and an
    /// operator may well start it once before loading.
    #[tokio::test(flavor = "current_thread")]
    async fn a_home_made_at_start_makes_way_for_the_one_loaded() {
        let state = fresh().await;
        crate::context::ensure_home(&state.db, &state.config)
            .await
            .expect("home");
        import(&state.db, &interim()).await.expect("import");
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query("SELECT name FROM context WHERE kind = 'home'", ())
            .await
            .expect("q");
        let name: String = rows
            .next()
            .await
            .expect("next")
            .expect("a home")
            .get(0)
            .expect("name");
        assert_eq!(name, "Radikal Ungdom");
        assert!(
            rows.next().await.expect("next").is_none(),
            "a wiki has one home"
        );

        // One that has been used is not quietly replaced.
        let used = fresh().await;
        crate::context::ensure_home(&used.db, &used.config)
            .await
            .expect("home");
        let conn = used.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO context (id, kind, name, slug, path, parent_id) \
             VALUES ('g', 'group', 'Ny gruppe', 'ny', 'ny', 'home')",
            (),
        )
        .await
        .expect("a group");
        assert!(matches!(
            import(&used.db, &interim()).await,
            Err(ImportError::HomeTaken)
        ));
    }

    /// Every file keeps its id, so nothing that points at one is rewritten, and
    /// is read by whoever reads what points at it.
    #[tokio::test(flavor = "current_thread")]
    async fn the_files_come_across_under_the_ids_they_had() {
        let mut ex = interim();
        let agenda = ex.documents.iter_mut().find(|d| d.id == "mo").expect("mo");
        agenda.data =
            Some(json!({"fileId": "f-agenda", "type": "application/pdf", "image": "f-cover"}));
        ex.contexts[0].data = Some(json!({"image": "f-front"}));
        ex.comments[0].image = Some("f-gone".into());

        let mut state = fresh().await;
        let dir =
            std::env::temp_dir().join(format!("interim-files-{}", crate::util::random_token(8)));
        state.config.blob_dir = dir.join("blobs").to_string_lossy().into_owned();
        let files = dir.join("files");
        std::fs::create_dir_all(&files).expect("dir");
        for (name, bytes) in [
            ("f-agenda", &b"%PDF the agenda"[..]),
            ("f-cover", b"a cover"),
            ("f-front", b"cut short"),
            ("f-stray", b"nothing points here"),
        ] {
            std::fs::write(files.join(name), bytes).expect("file");
        }
        let manifest = json!([
            {"id": "f-cover", "name": "forside.png", "mimeType": "image/png", "size": 7},
            {"id": "f-front", "name": "front.jpg", "mimeType": "image/jpeg", "size": 4096},
        ]);
        std::fs::write(files.join("manifest.json"), manifest.to_string()).expect("manifest");

        import(&state.db, &ex).await.expect("import");
        let stats = import_files(&state, &ex, &files).await.expect("files");
        assert_eq!((stats.copied, stats.already), (2, 0), "{stats:?}");
        let failed: Vec<&str> = stats.failed.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            failed,
            ["f-front", "f-gone"],
            "one cut short in the download, one never downloaded: {stats:?}"
        );
        assert_eq!(stats.unreferenced, ["f-stray"]);
        let again = import_files(&state, &ex, &files).await.expect("again");
        assert_eq!((again.copied, again.already), (0, 2));

        let alice = token_for(&state, ALICE).await;
        let session = json!({"email": "alice@x.dk", "emailConfirmed": true});
        let account = account_from(ALICE, "https://bsky.social", &session, None);
        apply(&state, ALICE, &account).await.expect("apply");
        let stranger = token_for(&state, "did:plc:mallory").await;
        let (status, headers, bytes) =
            crate::blob::tests::fetch(&state, "/blob/f-agenda", Some(&alice), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, b"%PDF the agenda");
        assert_eq!(headers["content-type"], "application/pdf");
        let (status, headers, _) =
            crate::blob::tests::fetch(&state, "/blob/f-cover", Some(&alice), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers["content-type"], "image/png",
            "what the interim said it was"
        );
        let (status, _, _) =
            crate::blob::tests::fetch(&state, "/blob/f-agenda", Some(&stranger), None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a file is read through its context"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What the NixOS VM test loads (`nixos-test.nix`). Loaded here too, so that
    /// a change to what an extraction looks like fails in seconds and not only
    /// where a VM has to boot to find out. Made by `extract` from a made-up
    /// snapshot: run that again when this fails.
    #[tokio::test(flavor = "current_thread")]
    async fn the_extraction_the_vm_test_loads_still_loads() {
        let ex: Extraction = serde_json::from_str(include_str!("../fixtures/extraction.json"))
            .expect("the fixture parses");
        let mut state = fresh().await;
        let blobs =
            std::env::temp_dir().join(format!("fixture-blobs-{}", crate::util::random_token(8)));
        state.config.blob_dir = blobs.to_string_lossy().into_owned();
        let stats = import(&state.db, &ex).await.expect("import");
        assert_eq!(
            (
                stats.entities.contexts,
                stats.entities.documents,
                stats.accounts
            ),
            (2, 2, 1)
        );
        let files = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/files"));
        let copied = import_files(&state, &ex, files).await.expect("files");
        assert_eq!((copied.copied, copied.failed.len()), (1, 0), "{copied:?}");
        // The unit runs the gates after the load, and a red one fails it.
        let gates = crate::verify::verify(&state, &ex, Some(files))
            .await
            .expect("verify");
        assert!(gates.iter().all(|gate| gate.red.is_empty()), "{gates:#?}");
        let _ = std::fs::remove_dir_all(&blobs);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_load_that_fails_leaves_nothing_behind() {
        let mut ex = interim();
        ex.canvases[0].id = "no-such-document".into();
        let state = fresh().await;
        assert!(import(&state.db, &ex).await.is_err());
        let conn = state.db.acquire().await.expect("conn");
        for table in [
            "user",
            "context",
            "document",
            "member",
            "poll",
            "legacy_account",
        ] {
            let mut rows = conn
                .query(&format!("SELECT count(*) FROM {table}"), ())
                .await
                .expect("q");
            let row = rows.next().await.expect("next").expect("a count");
            assert_eq!(row.get::<i64>(0).expect("count"), 0, "{table}");
        }
    }
}

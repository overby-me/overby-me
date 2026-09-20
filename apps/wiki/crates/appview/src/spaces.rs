//! The wiki on atproto spaces (`docs/atproto-spaces-redesign.md`), first stage:
//! what is written to the datastore is ALSO written as records, into the
//! organization's own repo in a space per context, and the organization's PDS
//! asks this AppView who may read and write each space.
//!
//! Off unless configured (`APPVIEW_SPACES_*`), which is how it stays until
//! spaces are released. The datastore remains the source of truth here: a
//! record follows its row, never the other way, and [`Spaces::mirror_context`]
//! compares rather than remembers, so a write that failed is made by the next
//! pass. That is what makes it safe to run beside everything else.

use crate::{AppState, Config, Store};
use atproto_spaces::client::{Host, is_managed_by};
use atproto_spaces::credential::Credential;
use atproto_spaces::directory::Directory;
use atproto_spaces::sync::{self, Pulled};
use atproto_spaces::{Error as SpaceError, jwt, service_auth};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::time::Instant;
use turso::Value as Sql;
use wiki_records::{
    Addresses, COMMENT, CONTEXT_SPACE, Comment, ContextProfile, Hanging, NODE, Node, PROFILE,
    REACTION, Reaction, SpaceUri,
};

pub const SPACES_DDL: &str = r#"
-- The space a context has at the organization's PDS.
CREATE TABLE IF NOT EXISTS space (
  context_id TEXT PRIMARY KEY,
  uri        TEXT NOT NULL,
  made_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
-- What was last written for a row: the version a reference to it pins, and
-- what it said, to tell a row that changed from one that did not.
CREATE TABLE IF NOT EXISTS space_record (
  context_id TEXT NOT NULL,
  collection TEXT NOT NULL,
  rkey       TEXT NOT NULL,
  cid        TEXT NOT NULL,
  said       TEXT NOT NULL,
  written_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (context_id, collection, rkey)
);
"#;

const CHECK_USER_ACCESS: &str = "com.atproto.simplespace.checkUserAccess";
const NOTIFY_WRITE: &str = "com.atproto.space.notifyWrite";
const NOTIFY_SPACE_DELETED: &str = "com.atproto.space.notifySpaceDeleted";

/// A PDS session is good for a couple of hours. Asked for again well inside that.
const SESSION_SECS: u64 = 45 * 60;

/// A context is mirrored once it has been quiet this long: a bin, a move or a
/// purge announces one id and changes a subtree, so the unit is the context.
const QUIET: Duration = Duration::from_secs(2);
/// However busy a context stays, its records are never further behind.
const AT_MOST: Duration = Duration::from_secs(30);
const SWEEP: Duration = Duration::from_secs(15 * 60);

type Failure = Box<dyn std::error::Error + Send + Sync>;

/// What mirroring came to, in records.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Swept {
    pub written: usize,
    pub deleted: usize,
    pub same: usize,
    /// What they point at has no record yet. Tried again by the next pass.
    pub waiting: usize,
    /// Rows the PDS will not take a record of as they are: too large, or data
    /// it cannot hold. Not tried again until the row changes.
    pub refused: usize,
    /// What could not be done this time, and the next pass tries again.
    pub failed: usize,
}

impl std::ops::AddAssign for Swept {
    fn add_assign(&mut self, other: Swept) {
        self.written += other.written;
        self.deleted += other.deleted;
        self.same += other.same;
        self.waiting += other.waiting;
        self.refused += other.refused;
        self.failed += other.failed;
    }
}

pub struct Spaces {
    pub(crate) host: Host,
    directory: Directory,
    identifier: String,
    password: crate::config::Secret,
    /// This AppView as a space names it: `did#fragment`.
    pub service: String,
    /// How long a context is left to go quiet before it is mirrored.
    pub quiet: Duration,
    session: tokio::sync::Mutex<Option<(String, String, u64)>>,
}

impl Spaces {
    /// All four settings or none: half of them is a space nobody can read.
    pub fn from_config(config: &Config) -> Result<Option<Spaces>, String> {
        let set = [
            !config.spaces_pds.is_empty(),
            !config.spaces_identifier.is_empty(),
            !config.spaces_password.is_empty(),
            !config.spaces_service.is_empty(),
        ];
        if set.iter().all(|s| !s) {
            return Ok(None);
        }
        if !set.iter().all(|s| *s) {
            return Err(
                "APPVIEW_SPACES_PDS, _IDENTIFIER, _PASSWORD and _SERVICE go together".into(),
            );
        }
        if !config.spaces_service.contains('#') {
            return Err("APPVIEW_SPACES_SERVICE is a DID and a fragment, did:..#name".into());
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let plc = match config.plc_url.as_str() {
            "" => "https://plc.directory",
            own => own,
        };
        Ok(Some(Spaces {
            host: Host::new(http.clone(), &config.spaces_pds),
            directory: Directory::new(http, plc),
            identifier: config.spaces_identifier.clone(),
            password: config.spaces_password.clone(),
            service: config.spaces_service.clone(),
            quiet: QUIET,
            session: Default::default(),
        }))
    }

    /// The organization's DID and a session of its own at its PDS.
    pub(crate) async fn session(&self) -> Result<(String, String), SpaceError> {
        let mut held = self.session.lock().await;
        if let Some((did, token, since)) = held.as_ref()
            && jwt::now() < since + SESSION_SECS
        {
            return Ok((did.clone(), token.clone()));
        }
        let (did, token) = self
            .host
            .create_session(&self.identifier, self.password.expose())
            .await?;
        *held = Some((did.clone(), token.clone(), jwt::now()));
        Ok((did, token))
    }

    /// A PDS says a session has run out as a request it could not read, and
    /// the one held here is then asked for again.
    pub(crate) async fn forget_session_if_spent(&self, error: &SpaceError) {
        let spent = matches!(error.xrpc_name(), Some("ExpiredToken" | "InvalidToken"))
            || matches!(error, SpaceError::Xrpc { status: 401, .. });
        if spent {
            *self.session.lock().await = None;
        }
    }

    /// The organization's DID, which every space of the wiki is under.
    pub async fn organization(&self) -> Result<String, SpaceError> {
        Ok(self.session().await?.0)
    }

    fn space_of(organization: &str, context_id: &str) -> SpaceUri {
        SpaceUri {
            authority: organization.to_string(),
            space_type: CONTEXT_SPACE.to_string(),
            skey: context_id.to_string(),
        }
    }

    pub(crate) async fn ensure_space(
        &self,
        state: &AppState,
        context_id: &str,
    ) -> Result<SpaceUri, Failure> {
        let (organization, session) = self.session().await?;
        let space = Self::space_of(&organization, context_id);
        if !ids(
            state,
            "SELECT 1 FROM space WHERE context_id = ?1",
            [context_id],
        )
        .await?
        .is_empty()
        {
            return Ok(space);
        }
        match self
            .host
            .create_managed_space(&session, CONTEXT_SPACE, context_id, &self.service)
            .await
        {
            Ok(_) => {}
            // Made by an earlier run that did not get to write it down.
            Err(e) if e.xrpc_name() == Some("SpaceAlreadyExists") => {}
            Err(e) => return Err(e.into()),
        }
        state
            .db
            .acquire()
            .await?
            .execute(
                "INSERT OR REPLACE INTO space (context_id, uri) VALUES (?1, ?2)",
                [context_id, &space.to_string()],
            )
            .await?;
        Ok(space)
    }

    /// Make the records of one context say what its rows say.
    pub async fn mirror_context(
        &self,
        state: &AppState,
        context_id: &str,
    ) -> Result<Swept, Failure> {
        let store = Store::new(state.db.clone());
        let Some(context) = store.row_context(context_id).await? else {
            return self.forget_context(state, context_id).await;
        };
        let organization = self.organization().await?;
        let mut pass = Pass {
            spaces: self,
            state,
            context_id,
            held: Held::of(state, &organization, context_id).await?,
            rows: BTreeSet::new(),
            swept: Swept::default(),
        };

        let hanging = hanging_of(&store, &context).await?;
        let profile = ContextProfile::of(&context, &hanging, &pass.held);
        pass.put(PROFILE, "self", Some(serde_json::to_value(profile)?))
            .await;

        // Parents before what hangs under them, and a comment before its replies.
        let documents = "SELECT id FROM document WHERE context_id = ?1 ORDER BY length(path), path";
        for id in ids(state, documents, [context_id]).await? {
            let record = match store.row_document(&id).await? {
                Some(doc) => Some(serde_json::to_value(Node::of(&doc, &pass.held))?),
                None => continue,
            };
            pass.put(NODE, &id, record).await;
        }
        let comments = "SELECT id FROM comment WHERE context_id = ?1 ORDER BY created_at, id";
        for id in ids(state, comments, [context_id]).await? {
            let record = match store.row_comment(&id).await? {
                Some(row) => Comment::of(&row, &pass.held).map(serde_json::to_value),
                None => continue,
            };
            pass.put(COMMENT, &id, record.transpose()?).await;
        }
        let reactions = "SELECT r.id FROM reaction r WHERE \
             EXISTS (SELECT 1 FROM document d WHERE d.id = r.subject_uri AND d.context_id = ?1) \
             OR EXISTS (SELECT 1 FROM comment c WHERE c.id = r.subject_uri AND c.context_id = ?1) \
             ORDER BY r.created_at, r.id";
        for id in ids(state, reactions, [context_id]).await? {
            let record = match store.row_reaction(&id).await? {
                Some(row) => Reaction::of(&row, context_id, &pass.held).map(serde_json::to_value),
                None => continue,
            };
            pass.put(REACTION, &id, record.transpose()?).await;
        }

        pass.delete_what_has_no_row().await?;
        Ok(pass.swept)
    }

    /// A context purged from the datastore: its space goes, and with it what
    /// the organization held there.
    async fn forget_context(&self, state: &AppState, context_id: &str) -> Result<Swept, Failure> {
        let spaces = ids(
            state,
            "SELECT uri FROM space WHERE context_id = ?1",
            [context_id],
        )
        .await?;
        let Some(uri) = spaces.first() else {
            return Ok(Swept::default());
        };
        let (_, session) = self.session().await?;
        match self.host.delete_space(&session, uri).await {
            Ok(()) => {}
            Err(e) if e.xrpc_name() == Some("SpaceNotFound") => {}
            Err(e) => return Err(e.into()),
        }
        let deleted = forget_rows(state, context_id).await?;
        Ok(Swept {
            deleted,
            ..Swept::default()
        })
    }

    /// A space is the organization's to delete or open at any console, and a
    /// write into one that is gone succeeds all the same. So it is asked about:
    /// one that is gone is forgotten, for the pass to make again, and one that
    /// someone else has been given the say over is taken back.
    async fn hold_space(&self, state: &AppState, context_id: &str) -> Result<(), Failure> {
        let known = ids(
            state,
            "SELECT uri FROM space WHERE context_id = ?1",
            [context_id],
        )
        .await?;
        let Some(uri) = known.first() else {
            return Ok(());
        };
        let (_, session) = self.session().await?;
        match self.host.space_setup(&session, uri).await? {
            None => {
                tracing::warn!("spaces: {uri} is gone from the PDS, and is made again");
                forget_rows(state, context_id).await?;
            }
            Some(setup) if !is_managed_by(&setup, &self.service) => {
                tracing::warn!("spaces: {uri} was not under this AppView, and is again");
                self.host.manage_space(&session, uri, &self.service).await?;
            }
            Some(_) => {}
        }
        Ok(())
    }

    /// Every context against its records, and every space against its context:
    /// what a start does, and what catches up on whatever a failed write or a
    /// listener that fell behind left undone.
    pub async fn mirror_everything(&self, state: &AppState) -> Result<Swept, Failure> {
        let mut swept = Swept::default();
        let every = "SELECT id FROM context ORDER BY length(path), path";
        let gone = "SELECT context_id FROM space s \
             WHERE NOT EXISTS (SELECT 1 FROM context c WHERE c.id = s.context_id)";
        for sql in [every, gone] {
            for context_id in ids(state, sql, ()).await? {
                if let Err(e) = self.hold_space(state, &context_id).await {
                    tracing::warn!("spaces: could not ask about the space of {context_id}: {e}");
                }
                match self.mirror_context(state, &context_id).await {
                    Ok(done) => swept += done,
                    Err(e) => {
                        swept.failed += 1;
                        tracing::warn!("spaces: could not mirror {context_id}: {e}");
                    }
                }
            }
        }
        Ok(swept)
    }

    /// [`Self::check_context`] of every context, each difference under the
    /// context it is in.
    pub async fn check_everything(&self, state: &AppState) -> Result<Vec<String>, Failure> {
        let mut wrong = Vec::new();
        let every = "SELECT id FROM context ORDER BY length(path), path";
        for context_id in ids(state, every, ()).await? {
            match self.check_context(state, &context_id).await {
                Ok(found) => wrong.extend(found.iter().map(|w| format!("{context_id}: {w}"))),
                Err(e) => wrong.push(format!("{context_id}: could not be read back: {e}")),
            }
        }
        Ok(wrong)
    }

    /// Read a context's space as any syncer would, through a credential, and
    /// hold the organization's repo in it against what the index says was
    /// written: the same records, at the same versions, under a commit that
    /// verifies. What differs, in words. Empty is the answer wanted.
    pub async fn check_context(
        &self,
        state: &AppState,
        context_id: &str,
    ) -> Result<Vec<String>, Failure> {
        let organization = self.organization().await?;
        let space = Self::space_of(&organization, context_id).to_string();
        let key = self.directory.resolve(&organization).await?.signing_key;
        let (_, session) = self.session().await?;
        let credential = Credential::obtain(&self.host, &session, &self.host, &space, None).await?;
        // From nothing, so the whole repo is listed and held to its commit.
        let mut copy = sync::Copy::default();
        let pulled = sync::pull(
            &self.host,
            credential.auth(),
            &space,
            &organization,
            &key,
            &mut copy,
        );
        // A poll's board is in the same repo, and is its publisher's to account
        // for and a member's mirror's to check (`crate::board`).
        let mirrored = [PROFILE, NODE, COMMENT, REACTION];
        let there: BTreeMap<(String, String), String> = match pulled.await? {
            Pulled::Everything(records) => records
                .into_iter()
                .filter(|r| mirrored.contains(&r.collection.as_str()))
                .map(|r| ((r.collection, r.rkey), r.cid))
                .collect(),
            Pulled::Nothing | Pulled::Changes(_) => BTreeMap::new(),
        };
        let expected = Held::of(state, &organization, context_id).await?.cids;
        let mut wrong = Vec::new();
        for (at, cid) in &expected {
            match there.get(at) {
                Some(found) if found == cid => {}
                Some(found) => wrong.push(format!(
                    "{}/{}: the repo has {found}, the index {cid}",
                    at.0, at.1
                )),
                None => wrong.push(format!("{}/{}: in the index, not in the repo", at.0, at.1)),
            }
        }
        for at in there.keys().filter(|at| !expected.contains_key(*at)) {
            wrong.push(format!("{}/{}: in the repo, not in the index", at.0, at.1));
        }
        Ok(wrong)
    }
}

/// The first column of what a query finds.
async fn ids(
    state: &AppState,
    sql: &str,
    params: impl turso::IntoParams,
) -> Result<Vec<String>, Failure> {
    let conn = state.db.acquire().await?;
    let mut rows = conn.query(sql, params).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(match row.get_value(0)? {
            Sql::Text(text) => text,
            other => format!("{other:?}"),
        });
    }
    Ok(out)
}

/// Forget a space that is gone, and with it that its polls' boards went out: a
/// space made again gets them again. Answers how many records it had.
async fn forget_rows(state: &AppState, context_id: &str) -> Result<usize, Failure> {
    let conn = state.db.acquire().await?;
    let records = "DELETE FROM space_record WHERE context_id = ?1";
    let records = conn.execute(records, [context_id]).await?;
    for table in ["space_board_published", "space_board_publication", "space"] {
        let of = match table {
            "space" => "context_id = ?1",
            _ => "poll_id IN (SELECT id FROM poll WHERE context_id = ?1)",
        };
        conn.execute(&format!("DELETE FROM {table} WHERE {of}"), [context_id])
            .await?;
    }
    Ok(records as usize)
}

/// What a record says, less the versions it pins: a comment is not rewritten
/// because the page under it was edited.
fn said(record: &Value) -> String {
    let mut unpinned = record.clone();
    for reference in ["subject", "parent"] {
        if let Some(pinned) = unpinned.get_mut(reference).and_then(Value::as_object_mut) {
            pinned.remove("cid");
        }
    }
    Sha256::digest(unpinned.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Pass<'a> {
    spaces: &'a Spaces,
    state: &'a AppState,
    context_id: &'a str,
    held: Held,
    /// Every row met, with a record or still waiting for one.
    rows: BTreeSet<(String, String)>,
    swept: Swept,
}

enum Wrote {
    Written,
    Same,
    Refused,
}

impl Pass<'_> {
    /// A failure is counted and the pass goes on: one record that cannot be
    /// written must not keep the rest of a context from being.
    async fn put(&mut self, collection: &str, rkey: &str, record: Option<Value>) {
        self.rows.insert((collection.to_string(), rkey.to_string()));
        let Some(record) = record else {
            self.swept.waiting += 1;
            return;
        };
        match self.write(collection, rkey, record).await {
            Ok(Wrote::Written) => self.swept.written += 1,
            Ok(Wrote::Same) => self.swept.same += 1,
            Ok(Wrote::Refused) => self.swept.refused += 1,
            Err(e) => {
                self.swept.failed += 1;
                tracing::warn!("spaces: could not write {collection}/{rkey}: {e}");
            }
        }
    }

    async fn write(
        &mut self,
        collection: &str,
        rkey: &str,
        mut record: Value,
    ) -> Result<Wrote, Failure> {
        record["$type"] = Value::String(collection.to_string());
        let said = said(&record);
        let at = (collection.to_string(), rkey.to_string());
        if self.held.said.get(&at) == Some(&said) {
            return Ok(match self.held.cids.contains_key(&at) {
                true => Wrote::Same,
                false => Wrote::Refused,
            });
        }
        let space = self
            .spaces
            .ensure_space(self.state, self.context_id)
            .await?;
        let (organization, session) = self.spaces.session().await?;
        let space = space.to_string();
        let put =
            self.spaces
                .host
                .put_record(&session, &space, &organization, collection, rkey, &record);
        let cid = match put.await {
            Ok(written) => Some(written.cid),
            // The record itself is what the PDS will not take (too large, or
            // data it cannot hold), and asking again changes nothing: it is
            // remembered as refused until the row says something else.
            Err(e) if e.is_about_the_record() => {
                let bytes = record.to_string().len();
                tracing::warn!("spaces: {collection}/{rkey} ({bytes} bytes) was refused: {e}");
                None
            }
            Err(e) => {
                self.spaces.forget_session_if_spent(&e).await;
                return Err(e.into());
            }
        };
        self.state
            .db
            .acquire()
            .await?
            .execute(
                "INSERT OR REPLACE INTO space_record (context_id, collection, rkey, cid, said) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                [
                    self.context_id,
                    collection,
                    rkey,
                    cid.as_deref().unwrap_or_default(),
                    said.as_str(),
                ],
            )
            .await?;
        self.held.said.insert(at.clone(), said);
        Ok(match cid {
            Some(cid) => {
                self.held.cids.insert(at, cid);
                Wrote::Written
            }
            None => {
                // Whatever version was there before is what the PDS still has.
                self.held.cids.remove(&at);
                Wrote::Refused
            }
        })
    }

    /// A row purged, or moved to another context, leaves a record behind here.
    async fn delete_what_has_no_row(&mut self) -> Result<(), Failure> {
        let left: Vec<_> = self
            .held
            .said
            .keys()
            .filter(|at| !self.rows.contains(*at))
            .cloned()
            .collect();
        if left.is_empty() {
            return Ok(());
        }
        let (organization, session) = self.spaces.session().await?;
        let space = Spaces::space_of(&organization, self.context_id).to_string();
        for (collection, rkey) in left {
            let gone = self
                .spaces
                .host
                .delete_record(&session, &space, &organization, &collection, &rkey)
                .await;
            if let Err(e) = gone {
                self.swept.failed += 1;
                tracing::warn!("spaces: could not delete {collection}/{rkey}: {e}");
                self.spaces.forget_session_if_spent(&e).await;
                continue;
            }
            self.state
                .db
                .acquire()
                .await?
                .execute(
                    "DELETE FROM space_record \
                     WHERE context_id = ?1 AND collection = ?2 AND rkey = ?3",
                    [self.context_id, collection.as_str(), rkey.as_str()],
                )
                .await?;
            self.swept.deleted += 1;
        }
        Ok(())
    }
}

/// Where things are held in the first stage, everything with the organization,
/// and what is known of one context's records.
struct Held {
    organization: String,
    cids: BTreeMap<(String, String), String>,
    said: BTreeMap<(String, String), String>,
}

impl Held {
    async fn of(state: &AppState, organization: &str, context_id: &str) -> Result<Held, Failure> {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT collection, rkey, cid, said FROM space_record WHERE context_id = ?1",
                [context_id],
            )
            .await?;
        let mut held = Held {
            organization: organization.to_string(),
            cids: BTreeMap::new(),
            said: BTreeMap::new(),
        };
        while let Some(row) = rows.next().await? {
            let at = (row.get::<String>(0)?, row.get::<String>(1)?);
            // No CID is a version the PDS refused: nothing a reference can pin.
            let cid: String = row.get(2)?;
            if !cid.is_empty() {
                held.cids.insert(at.clone(), cid);
            }
            held.said.insert(at, row.get(3)?);
        }
        Ok(held)
    }
}

impl Addresses for Held {
    fn space(&self, context_id: &str) -> SpaceUri {
        Spaces::space_of(&self.organization, context_id)
    }
    fn holder(&self, _id: &str) -> String {
        self.organization.clone()
    }
    fn cid(&self, collection: &str, id: &str) -> Option<String> {
        self.cids
            .get(&(collection.to_string(), id.to_string()))
            .cloned()
    }
}

/// A context's row names what it hangs in by one id, a context's or a folder's.
/// Its record names the space, and the folder if there is one.
async fn hanging_of(store: &Store, ctx: &wiki_domain_types::Context) -> Result<Hanging, Failure> {
    let Some(parent) = ctx.place.parent_id.as_deref() else {
        return Ok(Hanging::default());
    };
    if store.row_context(parent).await?.is_some() {
        return Ok(Hanging {
            context_id: Some(parent.to_string()),
            folder_id: None,
        });
    }
    Ok(match store.row_document(parent).await? {
        Some(folder) => Hanging {
            context_id: Some(folder.context_id),
            folder_id: Some(parent.to_string()),
        },
        None => Hanging::default(),
    })
}

/// Follow what the write paths announce. A listener that falls behind loses
/// announcements, not rows: the next sweep finds what it missed.
pub async fn run(state: AppState) {
    let Some(spaces) = state.spaces.clone() else {
        return;
    };
    let mut changes = state.changes.subscribe();
    match spaces.mirror_everything(&state).await {
        Ok(swept) => tracing::info!("spaces: mirrored at start: {swept:?}"),
        Err(e) => tracing::error!("spaces: could not mirror at start: {e}"),
    }
    let mut sweep = tokio::time::interval_at(Instant::now() + SWEEP, SWEEP);
    // Each context with a change not yet mirrored, and when its first one came.
    let mut stale: BTreeMap<String, Instant> = BTreeMap::new();
    let mut last = Instant::now();
    loop {
        let due = stale
            .values()
            .min()
            .map(|first| (*first + AT_MOST).min(last + spaces.quiet));
        tokio::select! {
            change = changes.recv() => match change {
                Ok(crate::live::Change { topic: crate::live::Topic::Context(id), .. }) => {
                    last = Instant::now();
                    stale.entry(id).or_insert(last);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    stale.clear();
                    if let Err(e) = spaces.mirror_everything(&state).await {
                        tracing::warn!("spaces: could not catch up: {e}");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            _ = tokio::time::sleep_until(due.unwrap_or(last)), if due.is_some() => {
                for context_id in std::mem::take(&mut stale).into_keys() {
                    if let Err(e) = spaces.mirror_context(&state, &context_id).await {
                        tracing::warn!("spaces: could not mirror {context_id}: {e}");
                    }
                }
            }
            _ = sweep.tick() => {
                if let Err(e) = spaces.mirror_everything(&state).await {
                    tracing::warn!("spaces: the sweep failed: {e}");
                }
            }
        }
    }
}

// -- What the organization's PDS asks and tells. --

fn refused(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({"error": error}))).into_response()
}

/// The caller of a method of ours, if it is the organization's own PDS speaking
/// for the organization: nobody else has anything to ask or tell.
async fn the_organization(
    spaces: &Spaces,
    headers: &HeaderMap,
    method: &str,
) -> Result<String, Response> {
    let authorization = headers.get("authorization").and_then(|h| h.to_str().ok());
    let caller =
        service_auth::caller(&spaces.directory, authorization, &spaces.service, method).await;
    match (caller, spaces.organization().await) {
        (Ok(caller), Ok(organization)) if caller == organization => Ok(organization),
        (Ok(_), Ok(_)) => Err(refused(StatusCode::FORBIDDEN, "Forbidden")),
        (Err(_), _) => Err(refused(StatusCode::UNAUTHORIZED, "AuthRequired")),
        (_, Err(_)) => Err(refused(StatusCode::BAD_GATEWAY, "UpstreamFailure")),
    }
}

/// `/.well-known/did.json`: where a PDS finds this AppView from the name the
/// spaces know it by, when that name is a `did:web` of this host. No key: the
/// AppView is called under the caller's token and signs nothing as itself.
pub async fn did_document(State(state): State<AppState>) -> Response {
    let named = state
        .spaces
        .as_ref()
        .and_then(|s| s.service.split_once('#'));
    match named {
        Some((did, fragment)) if did.starts_with("did:web:") => Json(json!({
            "@context": ["https://www.w3.org/ns/did/v1"],
            "id": did,
            "service": [{
                "id": format!("#{fragment}"),
                "type": "WikiAppView",
                "serviceEndpoint": state.config.public_url,
            }],
        }))
        .into_response(),
        _ => refused(StatusCode::NOT_FOUND, "NotFound"),
    }
}

/// The context a space of ours is of. `None` for anyone else's space.
fn context_of(space: Option<&str>, organization: &str) -> Option<String> {
    let space: SpaceUri = space?.parse().ok()?;
    (space.authority == organization && space.space_type == CONTEXT_SPACE).then_some(space.skey)
}

/// `com.atproto.simplespace.checkUserAccess`: may this user read, or write,
/// this space? Answered from the roster, as a space has no list of its own:
/// members read and write, and anyone reads a context that is open.
pub async fn check_user_access(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<BTreeMap<String, String>>,
) -> Response {
    let Some(spaces) = state.spaces.clone() else {
        return refused(StatusCode::NOT_IMPLEMENTED, "MethodNotImplemented");
    };
    let organization = match the_organization(&spaces, &headers, CHECK_USER_ACCESS).await {
        Ok(organization) => organization,
        Err(no) => return no,
    };
    let context_id = context_of(q.get("space").map(String::as_str), &organization);
    let (Some(context_id), Some(user)) = (context_id, q.get("user")) else {
        return refused(StatusCode::BAD_REQUEST, "InvalidRequest");
    };
    // Open to everyone is to read, and not what is in the bin.
    let open = match q.get("access").map(String::as_str) {
        Some("read") => {
            let public = "SELECT 1 FROM context \
                 WHERE id = ?1 AND visibility = 'public' AND deleted_at IS NULL";
            ids(&state, public, [context_id.as_str()]).await
        }
        Some("write") => Ok(Vec::new()),
        _ => return refused(StatusCode::BAD_REQUEST, "InvalidRequest"),
    };
    // The organization is who the AppView reads its own spaces back as.
    if *user == organization {
        return Json(json!({"authorized": true})).into_response();
    }
    let member = crate::authz::Authz::new(state.db.clone())
        .is_member(&context_id, user)
        .await;
    match (member, open) {
        (Ok(member), Ok(open)) => {
            Json(json!({"authorized": member || !open.is_empty()})).into_response()
        }
        _ => refused(StatusCode::INTERNAL_SERVER_ERROR, "InternalServerError"),
    }
}

/// `com.atproto.space.notifyWrite`: a repo of one of the wiki's spaces moved.
/// In the first stage every record is the organization's and written from
/// here, so there is nothing to pull: no other repo is indexed, whatever a
/// member writes into their own.
pub async fn notify_write(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(spaces) = state.spaces.clone() else {
        return refused(StatusCode::NOT_IMPLEMENTED, "MethodNotImplemented");
    };
    match the_organization(&spaces, &headers, NOTIFY_WRITE).await {
        Ok(_) => {
            tracing::debug!(
                "spaces: {} is at {} in {}",
                body["repo"].as_str().unwrap_or("?"),
                body["rev"].as_str().unwrap_or("?"),
                body["space"].as_str().unwrap_or("?")
            );
            Json(json!({})).into_response()
        }
        Err(no) => no,
    }
}

/// `com.atproto.space.notifySpaceDeleted`: a space of ours is gone from the
/// PDS. If its context is not, it was deleted behind our back: what is known of
/// its records is forgotten, and the next pass makes the space again.
pub async fn notify_space_deleted(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(spaces) = state.spaces.clone() else {
        return refused(StatusCode::NOT_IMPLEMENTED, "MethodNotImplemented");
    };
    let organization = match the_organization(&spaces, &headers, NOTIFY_SPACE_DELETED).await {
        Ok(organization) => organization,
        Err(no) => return no,
    };
    let Some(context_id) = context_of(body["space"].as_str(), &organization) else {
        return refused(StatusCode::BAD_REQUEST, "InvalidRequest");
    };
    match forget_rows(&state, &context_id).await {
        Ok(_) => {
            tokio::spawn(async move {
                if let Err(e) = spaces.mirror_context(&state, &context_id).await {
                    tracing::warn!("spaces: could not make {context_id} again: {e}");
                }
            });
            Json(json!({})).into_response()
        }
        Err(_) => refused(StatusCode::INTERNAL_SERVER_ERROR, "InternalServerError"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::xrpc::tests::{post, seeded_state};
    use axum::body::Body;
    use axum::http::Request;
    use fake_pds::{DID, FakePds, STRANGER};
    use std::sync::Arc;
    use tower::ServiceExt;
    use wiki_domain_types::Visibility;
    use wiki_records::Kept;

    const SERVICE: &str = "did:web:wiki.test#wiki_appview";

    fn space(context_id: &str) -> String {
        Spaces::space_of(DID, context_id).to_string()
    }

    /// The seeded wiki, set up to mirror into a PDS of its own.
    async fn mirroring() -> (AppState, FakePds, Arc<Spaces>) {
        let pds = FakePds::start("app-password").await;
        let mut state = seeded_state().await;
        state.config.spaces_pds = pds.url.clone();
        state.config.spaces_identifier = "wiki.test".into();
        state.config.spaces_password = crate::config::Secret::new("app-password");
        state.config.spaces_service = SERVICE.into();
        state.config.plc_url = pds.plc_url();
        state.config.public_url = "https://wiki.test".into();
        let spaces = Spaces::from_config(&state.config)
            .expect("a whole configuration")
            .expect("spaces");
        let spaces = Arc::new(spaces);
        state.spaces = Some(spaces.clone());
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO reaction (id, subject_uri, reactor_did, emoji) \
             VALUES ('r1', 'd1', 'did:plc:bob', '👍')",
            (),
        )
        .await
        .expect("a reaction");
        (state, pds, spaces)
    }

    async fn run_sql(state: &AppState, sql: &str) {
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(sql).await.expect(sql);
    }

    fn keys(pds: &FakePds, context_id: &str) -> Vec<String> {
        let mut keys: Vec<String> = pds
            .space_records(&space(context_id))
            .iter()
            .map(|r| {
                format!(
                    "{}/{}",
                    r.collection.trim_start_matches("wiki.radikal."),
                    r.rkey
                )
            })
            .collect();
        keys.sort();
        keys
    }

    #[tokio::test]
    async fn a_wiki_is_mirrored_once_and_a_second_pass_writes_nothing() {
        let (state, pds, spaces) = mirroring().await;
        let first = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!(
            first,
            Swept {
                written: 10,
                ..Swept::default()
            }
        );
        assert_eq!(
            pds.spaces(),
            [space("c1"), space("c10"), space("c2"), space("c9")]
        );
        assert_eq!(
            keys(&pds, "c1"),
            [
                "comment/k1",
                "contextProfile/self",
                "node/d1",
                "node/d2",
                "reaction/r1"
            ]
        );
        assert_eq!(
            keys(&pds, "c9"),
            ["comment/ks", "contextProfile/self", "node/s1"]
        );
        // The reaction to a post is given to nothing a space holds.
        assert_eq!(keys(&pds, "c2"), ["contextProfile/self"]);

        let second = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!(
            second,
            Swept {
                same: 10,
                ..Swept::default()
            }
        );
    }

    #[tokio::test]
    async fn what_is_written_is_the_row_and_reads_back_as_it() {
        let (state, pds, spaces) = mirroring().await;
        spaces.mirror_context(&state, "c1").await.expect("a pass");
        let records = pds.space_records(&space("c1"));
        let of = |collection: &str, rkey: &str| {
            let found = records
                .iter()
                .find(|r| r.collection == collection && r.rkey == rkey);
            found.expect("the record").clone()
        };
        let store = Store::new(state.db.clone());
        let at = Spaces::space_of(DID, "c1");

        let node: Node = serde_json::from_value(of(NODE, "d1").value).expect("a node");
        let row = node.row(&at.record(DID, NODE, "d1"), "group-one", Kept::default());
        assert_eq!(row, store.row_document("d1").await.expect("a row"));

        // A comment pins the version of the page it was made on, and a reply
        // would pin the comment's.
        let comment = of(COMMENT, "k1");
        assert_eq!(comment.value["subject"]["cid"], of(NODE, "d1").cid());
        let comment: Comment = serde_json::from_value(comment.value).expect("a comment");
        let row = comment.row(&at.record(DID, COMMENT, "k1"), None);
        assert_eq!(row, store.row_comment("k1").await.expect("a row"));

        let reaction: Reaction =
            serde_json::from_value(of(REACTION, "r1").value).expect("a reaction");
        let row = reaction.row(&at.record(DID, REACTION, "r1"));
        assert_eq!(row, store.row_reaction("r1").await.expect("a row"));

        // Who a context is open to is the AppView's answer and not the
        // record's: rebuilt from the record alone, an open context is closed.
        let profile: ContextProfile =
            serde_json::from_value(of(PROFILE, "self").value).expect("a profile");
        let c1 = store.row_context("c1").await.expect("a row").expect("c1");
        let alone = profile.row(&at, "", Kept::default()).expect("a row");
        assert_eq!(
            (c1.visibility, alone.visibility),
            (Visibility::Public, Visibility::Private)
        );
        let kept = Kept {
            visibility: c1.visibility,
            published_uri: c1.published_uri.clone(),
        };
        assert_eq!(profile.row(&at, "", kept), Some(c1));
    }

    #[tokio::test]
    async fn an_edit_a_purge_and_a_move_are_followed() {
        let (state, pds, spaces) = mirroring().await;
        spaces.mirror_everything(&state).await.expect("a pass");

        // The page is rewritten. What pins it is not: it was made on the
        // version it pins.
        run_sql(
            &state,
            "UPDATE document SET title = 'Motion, amended' WHERE id = 'd1'",
        )
        .await;
        let edited = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!((edited.written, edited.same), (1, 4));
        let d1 = pds.space_records(&space("c1"));
        let d1 = d1.iter().find(|r| r.rkey == "d1").expect("d1");
        assert_eq!(d1.value["title"], "Motion, amended");

        run_sql(&state, "DELETE FROM comment WHERE id = 'k1'").await;
        let purged = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!((purged.deleted, purged.written), (1, 0));
        assert!(!keys(&pds, "c1").contains(&"comment/k1".to_string()));

        // Across contexts a record leaves one space and arrives in the other.
        run_sql(
            &state,
            "UPDATE document SET context_id = 'c9', parent_id = 'c9', \
               path = 'closed/child_doc' WHERE id = 'd2'",
        )
        .await;
        let left = spaces.mirror_context(&state, "c1").await.expect("a pass");
        let arrived = spaces.mirror_context(&state, "c9").await.expect("a pass");
        assert_eq!((left.deleted, arrived.written), (1, 1));
        assert_eq!(
            keys(&pds, "c1"),
            ["contextProfile/self", "node/d1", "reaction/r1"]
        );
        assert!(keys(&pds, "c9").contains(&"node/d2".to_string()));
    }

    #[tokio::test]
    async fn a_purged_context_takes_its_space_with_it() {
        let (state, pds, spaces) = mirroring().await;
        spaces.mirror_everything(&state).await.expect("a pass");
        run_sql(
            &state,
            "DELETE FROM member WHERE context_id = 'c10'; DELETE FROM context WHERE id = 'c10'",
        )
        .await;
        let swept = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!((swept.deleted, swept.written, swept.failed), (1, 0, 0));
        assert_eq!(pds.spaces(), [space("c1"), space("c2"), space("c9")]);
        // And is not looked for again.
        let again = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!(again.deleted, 0);
    }

    #[tokio::test]
    async fn a_space_deleted_at_another_console_is_made_again() {
        let (state, pds, spaces) = mirroring().await;
        spaces.mirror_everything(&state).await.expect("a pass");
        // A PDS takes a write into a space that is gone, so a pass over the one
        // context writes into nothing and is none the wiser.
        pds.tamper_spaces(|all| {
            all.remove(&space("c9"));
        });
        run_sql(
            &state,
            "UPDATE document SET title = 'Minutes' WHERE id = 's1'",
        )
        .await;
        let unaware = spaces.mirror_context(&state, "c9").await.expect("a pass");
        assert_eq!((unaware.written, unaware.failed), (1, 0));
        assert_eq!(keys(&pds, "c9"), Vec::<String>::new());
        assert_eq!(pds.orphans().len(), 1);

        // The sweep asks, and makes it again with everything in it.
        let swept = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!((swept.written, swept.failed), (3, 0));
        assert_eq!(
            keys(&pds, "c9"),
            ["comment/ks", "contextProfile/self", "node/s1"]
        );
        let s1 = pds.space_records(&space("c9"));
        let s1 = s1.iter().find(|r| r.rkey == "s1").expect("s1");
        assert_eq!(s1.value["title"], "Minutes");
    }

    #[tokio::test]
    async fn told_that_a_space_was_deleted_it_is_made_again_at_once() {
        let (state, pds, spaces) = mirroring().await;
        spaces.mirror_everything(&state).await.expect("a pass");
        pds.tamper_spaces(|all| {
            all.remove(&space("c9"));
        });
        let token = pds.service_token(DID, SERVICE, NOTIFY_SPACE_DELETED);
        let (status, _) = post(
            crate::router(state.clone()),
            "/xrpc/com.atproto.space.notifySpaceDeleted",
            Some(&token),
            json!({"space": space("c9")}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        for _ in 0..200 {
            if keys(&pds, "c9").len() == 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(
            keys(&pds, "c9"),
            ["comment/ks", "contextProfile/self", "node/s1"]
        );
    }

    #[tokio::test]
    async fn a_space_someone_opened_to_everyone_is_taken_back() {
        let (state, pds, spaces) = mirroring().await;
        spaces.mirror_everything(&state).await.expect("a pass");
        let setup = |pds: &FakePds| {
            let mut found = None;
            pds.tamper_spaces(|all| found = all.get(&space("c9")).map(|s| s.setup.clone()));
            found.expect("the space")
        };
        assert!(is_managed_by(&setup(&pds), SERVICE));
        pds.tamper_spaces(|all| {
            let c9 = all.get_mut(&space("c9")).expect("the space");
            c9.setup["readPolicy"] = json!({"$type": "com.atproto.simplespace.defs#publicPolicy"});
        });
        assert!(!is_managed_by(&setup(&pds), SERVICE));
        spaces.mirror_everything(&state).await.expect("a pass");
        assert!(is_managed_by(&setup(&pds), SERVICE));
    }

    #[tokio::test]
    async fn what_a_pds_cannot_hold_is_carried_and_what_it_will_not_take_is_not_asked_twice() {
        let (state, pds, spaces) = mirroring().await;
        spaces.mirror_everything(&state).await.expect("a pass");
        let puts = |pds: &FakePds| {
            let calls = pds.calls();
            calls
                .iter()
                .filter(|(method, _)| method == "space.putRecord")
                .count()
        };

        // A fraction, which atproto data has none of and the PDS refuses as is.
        run_sql(
            &state,
            r#"UPDATE document SET data = '{"threshold":0.5}' WHERE id = 'd1'"#,
        )
        .await;
        let carried = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!((carried.written, carried.refused), (1, 0));

        // A page past what a PDS takes in one request.
        let conn = state.db.acquire().await.expect("conn");
        let long = json!({"text": "x".repeat(1_100_000)}).to_string();
        conn.execute("UPDATE document SET content = ?1 WHERE id = 'd2'", [long])
            .await
            .expect("a long page");
        let before = puts(&pds);
        let refused = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!(
            (refused.refused, refused.failed, puts(&pds) - before),
            (1, 0, 1)
        );
        // Counted for as long as it is so, and not sent again.
        let again = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!((again.refused, puts(&pds) - before), (1, 1));
        // Until the row is something else.
        run_sql(&state, "UPDATE document SET content = NULL WHERE id = 'd2'").await;
        let shorter = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!((shorter.written, shorter.refused), (1, 0));
    }

    #[tokio::test]
    async fn the_listener_mirrors_a_context_once_it_goes_quiet() {
        let (mut state, pds, spaces) = mirroring().await;
        let mut hasty = Spaces::from_config(&state.config)
            .expect("whole")
            .expect("spaces");
        hasty.quiet = Duration::from_millis(50);
        drop(spaces);
        state.spaces = Some(Arc::new(hasty));
        tokio::spawn(run(state.clone()));
        let title = |pds: &FakePds| {
            let records = pds.space_records(&space("c1"));
            let d1 = records.iter().find(|r| r.rkey == "d1");
            d1.map(|r| r.value["title"].clone())
        };
        let until = async |want: &str| {
            for _ in 0..400 {
                if title(&pds) == Some(json!(want)) {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            false
        };
        assert!(until("Motion").await, "mirrored at start");
        run_sql(
            &state,
            "UPDATE document SET title = 'Moved' WHERE id = 'd1'",
        )
        .await;
        state.publish(crate::live::Topic::Context("c1".into()), "node", "d1");
        assert!(until("Moved").await, "mirrored after the announcement");
    }

    async fn ask(state: &AppState, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::builder().uri(uri);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let answer = crate::router(state.clone())
            .oneshot(request.body(Body::empty()).expect("a request"))
            .await
            .expect("an answer");
        let status = answer.status();
        let bytes = axum::body::to_bytes(answer.into_body(), 64 * 1024)
            .await
            .expect("a body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn check(context_id: &str, user: &str, access: &str) -> String {
        let space = space(context_id).replace(':', "%3A").replace('/', "%2F");
        format!("/xrpc/{CHECK_USER_ACCESS}?space={space}&user={user}&access={access}")
    }

    #[tokio::test]
    async fn the_pds_is_answered_from_the_roster() {
        let (state, pds, _) = mirroring().await;
        let token = || pds.service_token(DID, SERVICE, CHECK_USER_ACCESS);
        let allowed = async |context_id: &str, user: &str, access: &str| {
            let (status, said) =
                ask(&state, &check(context_id, user, access), Some(&token())).await;
            assert_eq!(status, StatusCode::OK, "{said}");
            said["authorized"].as_bool().expect("an answer")
        };
        // A member reads and writes. Reading never asked for voting rights,
        // which is all an inactive seat lacks.
        assert!(allowed("c9", "did:plc:bob", "read").await);
        assert!(allowed("c9", "did:plc:bob", "write").await);
        assert!(allowed("c9", "did:plc:ivan", "read").await);
        // Membership does not inherit, either way.
        assert!(!allowed("c9", "did:plc:zoe", "read").await);
        assert!(!allowed("c10", "did:plc:bob", "read").await);
        // Anyone reads what is open, and only members write there.
        assert!(allowed("c1", "did:plc:zoe", "read").await);
        assert!(!allowed("c1", "did:plc:zoe", "write").await);
        // Not once it is in the bin, which is nobody's to read but a member's.
        run_sql(
            &state,
            "UPDATE context SET deleted_at = '2026-09-20T12:00:00.000Z' WHERE id = 'c1'",
        )
        .await;
        assert!(!allowed("c1", "did:plc:zoe", "read").await);
    }

    #[tokio::test]
    async fn nobody_but_the_organizations_pds_is_answered() {
        let (state, pds, _) = mirroring().await;
        let uri = check("c9", "did:plc:bob", "read");
        let status = async |token: Option<String>| ask(&state, &uri, token.as_deref()).await.0;
        assert_eq!(status(None).await, StatusCode::UNAUTHORIZED);
        let elsewhere =
            pds.service_token(DID, "did:web:other.test#wiki_appview", CHECK_USER_ACCESS);
        assert_eq!(status(Some(elsewhere)).await, StatusCode::UNAUTHORIZED);
        let another_method = pds.service_token(DID, SERVICE, NOTIFY_WRITE);
        assert_eq!(status(Some(another_method)).await, StatusCode::UNAUTHORIZED);
        let unknown = pds.service_token(
            "did:plc:nobody0000000000000000000",
            SERVICE,
            CHECK_USER_ACCESS,
        );
        assert_eq!(status(Some(unknown)).await, StatusCode::UNAUTHORIZED);
        // A token that verifies, from someone the spaces are not under.
        let stranger = pds.service_token(STRANGER, SERVICE, CHECK_USER_ACCESS);
        assert_eq!(status(Some(stranger)).await, StatusCode::FORBIDDEN);

        // Nor about a space that is not the wiki's.
        let token = pds.service_token(DID, SERVICE, CHECK_USER_ACCESS);
        let theirs = format!(
            "/xrpc/{CHECK_USER_ACCESS}?space=at%3A%2F%2F{STRANGER}%2Fspace%2F{CONTEXT_SPACE}%2Fc9\
             &user=did:plc:bob&access=read"
        );
        assert_eq!(
            ask(&state, &theirs, Some(&token)).await.0,
            StatusCode::BAD_REQUEST
        );
        let sideways = check("c9", "did:plc:bob", "admin");
        assert_eq!(
            ask(&state, &sideways, Some(&token)).await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn a_did_web_name_resolves_here_and_nothing_is_served_without_spaces() {
        let (state, _, _) = mirroring().await;
        let (status, document) = ask(&state, "/.well-known/did.json", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(document["id"], "did:web:wiki.test");
        assert_eq!(document["service"][0]["id"], "#wiki_appview");
        assert_eq!(
            document["service"][0]["serviceEndpoint"],
            "https://wiki.test"
        );

        let plain = seeded_state().await;
        assert_eq!(
            ask(&plain, "/.well-known/did.json", None).await.0,
            StatusCode::NOT_FOUND
        );
        let uri = check("c9", "did:plc:bob", "read");
        assert_eq!(ask(&plain, &uri, None).await.0, StatusCode::NOT_IMPLEMENTED);
    }

    pub(crate) const ALPHA_PASSWORD: &str = "a-password-for-a-made-up-account";

    /// A made-up account on the alpha's PDS: its DID, and a session.
    pub(crate) async fn account(http: &reqwest::Client, pds: &str, name: &str) -> (String, String) {
        let body = json!({
            "handle": format!("{name}.test"), "email": format!("{name}@wiki.test"),
            "password": ALPHA_PASSWORD,
        });
        let asked = http
            .post(format!("{pds}/xrpc/com.atproto.server.createAccount"))
            .json(&body);
        let said: Value = asked
            .send()
            .await
            .expect("the PDS")
            .json()
            .await
            .expect("an account");
        (
            said["did"].as_str().expect("a did").to_string(),
            said["accessJwt"].as_str().expect("a session").to_string(),
        )
    }

    /// The alpha's PDS and the directory beside it, as `scripts/test-spaces.nu`
    /// runs them, with a new organization on it.
    pub(crate) struct Alpha {
        pub http: reqwest::Client,
        pub pds_url: String,
        pub plc_url: String,
        pub pds: Host,
        /// What makes this run's handles its own.
        pub run: String,
        pub org: String,
        pub org_session: String,
    }

    impl Alpha {
        pub(crate) async fn with_an_organization() -> Alpha {
            let pds_url = std::env::var("SPACES_ALPHA_PDS").expect("SPACES_ALPHA_PDS");
            let plc_url = std::env::var("SPACES_ALPHA_PLC").expect("SPACES_ALPHA_PLC");
            let http = reqwest::Client::new();
            let run = jwt::nonce()[..8].to_string();
            let (org, org_session) = account(&http, &pds_url, &format!("wiki{run}")).await;
            Alpha {
                pds: Host::new(http.clone(), &pds_url),
                http,
                pds_url,
                plc_url,
                run,
                org,
                org_session,
            }
        }

        /// Mirror `state` into the organization's spaces, and serve it over
        /// HTTP under a DID of its own in the directory, as the spaces name it:
        /// the PDS then asks THIS AppView who gets in.
        pub(crate) async fn serve(&self, mut state: AppState) -> (AppState, Arc<Spaces>) {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("a port");
            let endpoint = format!("http://{}", listener.local_addr().expect("an address"));
            let app_did = format!("did:plc:{:a<24}", format!("app{}", self.run));
            let op = json!({
                "type": "plc_operation", "prev": null, "sig": "unchecked",
                "rotationKeys": [], "alsoKnownAs": [], "verificationMethods": {},
                "services": {"wiki_appview": {"type": "WikiAppView", "endpoint": endpoint}},
            });
            let registered = self
                .http
                .post(format!("{}/{app_did}", self.plc_url))
                .json(&op);
            let registered = registered.send().await.expect("the directory");
            assert!(registered.status().is_success());

            state.config.spaces_pds = self.pds_url.clone();
            state.config.spaces_identifier = format!("wiki{}.test", self.run);
            state.config.spaces_password = crate::config::Secret::new(ALPHA_PASSWORD);
            state.config.spaces_service = format!("{app_did}#wiki_appview");
            state.config.plc_url = self.plc_url.clone();
            let spaces = Spaces::from_config(&state.config)
                .expect("whole")
                .expect("spaces");
            let spaces = Arc::new(spaces);
            state.spaces = Some(spaces.clone());
            let router = crate::router(state.clone());
            tokio::spawn(async move { axum::serve(listener, router).await });
            (state, spaces)
        }
    }

    /// The same wiki, mirrored into the alpha's PDS, which asks this AppView
    /// over HTTP who may read: the whole of stage one against the real thing.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a spaces PDS: run scripts/test-spaces.nu"]
    async fn the_wiki_in_a_real_pds() {
        let alpha = Alpha::with_an_organization().await;
        let (http, pds_url, run) = (&alpha.http, &alpha.pds_url, &alpha.run);
        let (org, org_session) = (alpha.org.clone(), alpha.org_session.clone());
        let (alice, alice_session) = account(http, pds_url, &format!("alice{run}")).await;
        let (_, bob_session) = account(http, pds_url, &format!("bob{run}")).await;
        let (state, spaces) = alpha.serve(seeded_state().await).await;
        run_sql(
            &state,
            &format!(
                "INSERT INTO user (did) VALUES ('{alice}'); \
                 INSERT INTO member (id, user_did, context_id, role, active) \
                   VALUES ('m-real-alice', '{alice}', 'c9', 'member', 1); \
                 INSERT INTO reaction (id, subject_uri, reactor_did, emoji) \
                   VALUES ('r1', 'd1', '{alice}', '👍'); \
                 UPDATE document SET data = '{{\"threshold\":0.5}}', \
                   content = '[{{\"type\":\"paragraph\",\"children\":[{{\"text\":\"Vi foreslår\"}}]}}]' \
                   WHERE id = 'd1'"
            ),
        )
        .await;

        let first = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!(
            first,
            Swept {
                written: 10,
                ..Swept::default()
            }
        );
        // Read back as any syncer would: the same records, the same versions,
        // under a commit the organization signed.
        let wrong = spaces.check_everything(&state).await.expect("a check");
        assert_eq!(wrong, Vec::<String>::new());

        // A member gets in through her own PDS session, and reads the page as
        // it was written. Who the roster does not name does not.
        let pds = &alpha.pds;
        let c9 = Spaces::space_of(&org, "c9").to_string();
        let hers = Credential::obtain(pds, &alice_session, pds, &c9, None)
            .await
            .expect("a credential for a member");
        let read = pds.records(hers.auth(), &c9, &org).await.expect("records");
        let s1 = read.iter().find(|r| r.rkey == "s1").expect("s1");
        assert_eq!(s1.value["title"], "Secret Minutes");
        let his = Credential::obtain(pds, &bob_session, pds, &c9, None).await;
        assert_eq!(
            his.err().as_ref().and_then(|e| e.xrpc_name()),
            Some("UserNotAuthorized")
        );
        // What is open is anyone's to read, with its fraction back in place.
        let c1 = Spaces::space_of(&org, "c1").to_string();
        let anyones = Credential::obtain(pds, &bob_session, pds, &c1, None)
            .await
            .expect("a credential for anyone");
        let read = pds
            .records(anyones.auth(), &c1, &org)
            .await
            .expect("records");
        let d1 = read.iter().find(|r| r.rkey == "d1").expect("d1");
        let node: Node = serde_json::from_value(d1.value.clone()).expect("a node");
        let at = Spaces::space_of(&org, "c1").record(&org, NODE, "d1");
        let row = node.row(&at, "group-one", Kept::default()).expect("a row");
        assert_eq!(row.data, Some(json!({"threshold": 0.5})));

        // An edit and a purge, and the repo is still the index.
        run_sql(
            &state,
            "UPDATE document SET title = 'Minutes' WHERE id = 's1'; \
             DELETE FROM comment WHERE id = 'ks'",
        )
        .await;
        let edited = spaces.mirror_context(&state, "c9").await.expect("a pass");
        assert_eq!((edited.written, edited.deleted, edited.failed), (1, 1, 0));
        let wrong = spaces.check_context(&state, "c9").await.expect("a check");
        assert_eq!(wrong, Vec::<String>::new());

        // The space deleted at another console, and written into meanwhile.
        pds.delete_space(&org_session, &c9).await.expect("a delete");
        run_sql(
            &state,
            "UPDATE document SET title = 'Referat' WHERE id = 's1'",
        )
        .await;
        spaces.mirror_context(&state, "c9").await.expect("a pass");
        let swept = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!((swept.written, swept.failed), (2, 0), "{swept:?}");
        let wrong = spaces.check_context(&state, "c9").await.expect("a check");
        assert_eq!(wrong, Vec::<String>::new());

        // A context purged takes its space along.
        run_sql(
            &state,
            "DELETE FROM member WHERE context_id = 'c10'; DELETE FROM context WHERE id = 'c10'",
        )
        .await;
        spaces.mirror_everything(&state).await.expect("a pass");
        let c10 = Spaces::space_of(&org, "c10").to_string();
        assert_eq!(
            pds.space_setup(&org_session, &c10)
                .await
                .expect("an answer"),
            None
        );
    }

    #[test]
    fn half_a_configuration_is_refused() {
        let mut config = Config::default();
        assert!(
            Spaces::from_config(&config)
                .expect("none is fine")
                .is_none()
        );
        config.spaces_pds = "https://pds.test".into();
        assert!(Spaces::from_config(&config).is_err());
        config.spaces_identifier = "wiki.test".into();
        config.spaces_password = crate::config::Secret::new("pw");
        config.spaces_service = "did:web:wiki.test".into();
        assert!(
            Spaces::from_config(&config).is_err(),
            "a service with no fragment"
        );
        config.spaces_service = SERVICE.into();
        assert!(Spaces::from_config(&config).expect("whole").is_some());
    }
}

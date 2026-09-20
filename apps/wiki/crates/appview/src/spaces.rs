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
use atproto_spaces::attestation::ClientKey;
use atproto_spaces::client::{Host, Record, is_set_up};
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
    Addresses, COMMENT, CONTEXT_SPACE, Comment, ContextProfile, FILE, File, FileRow, Found,
    Hanging, Kept, NODE, Node, PROFILE, REACTION, Reaction, SpaceUri,
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

/// What a record may come to before its body goes out as a blob. A PDS took
/// 900 KB in one request and refused 1 MB, and a request is more than its record.
const RECORD_ROOM: usize = 800_000;

/// Where the key this application attests with is published.
pub const JWKS_PATH: &str = "/jwks.json";

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
    blob_limit: u64,
    /// The key this application attests with, published at `/jwks.json`.
    key: ClientKey,
    /// What this application attests as: its OAuth `client_id`. `None` for one
    /// that is not deployed and has none a PDS could look up.
    pub client_id: Option<String>,
    /// The applications the wiki's spaces admit, this one first. Empty for any.
    allowed: Vec<String>,
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
        let client_id = (!config.public_url.is_empty()).then(|| {
            format!(
                "{}{}",
                config.public_url,
                crate::oauth::CLIENT_METADATA_PATH
            )
        });
        let allowed = client_id
            .iter()
            .chain(&config.spaces_allowed_clients)
            .cloned()
            .collect();
        Ok(Some(Spaces {
            key: client_key(config),
            client_id,
            allowed,
            host: Host::new(http.clone(), &config.spaces_pds),
            directory: Directory::new(http, plc),
            identifier: config.spaces_identifier.clone(),
            password: config.spaces_password.clone(),
            service: config.spaces_service.clone(),
            quiet: QUIET,
            blob_limit: config.spaces_blob_limit,
            session: Default::default(),
        }))
    }

    /// Attest as `client_id`, which the spaces then admit first of all. What a
    /// deployment gets from its public URL; a rehearsal against a PDS on the
    /// same machine has to say, since that PDS looks the `client_id` up.
    pub fn attest_as(&mut self, client_id: String) {
        self.allowed
            .retain(|known| Some(known) != self.client_id.as_ref());
        self.allowed.insert(0, client_id.clone());
        self.client_id = Some(client_id);
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
            .create_managed_space(
                &session,
                CONTEXT_SPACE,
                context_id,
                &self.service,
                &self.allowed,
            )
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

        // The files something mirrored names. A report's picture is in the same
        // store and is its reader's alone (`crate::feedback`).
        let files = "SELECT b.id FROM blob b WHERE b.context_id = ?1 AND ( \
             EXISTS (SELECT 1 FROM document d WHERE json_extract(d.data, '$.fileId') = b.id \
                     OR json_extract(d.data, '$.image') = b.id) \
             OR EXISTS (SELECT 1 FROM context c WHERE json_extract(c.data, '$.image') = b.id) \
             OR EXISTS (SELECT 1 FROM comment k WHERE k.image = b.id)) \
             ORDER BY b.created_at, b.id";
        for id in ids(state, files, [context_id]).await? {
            pass.put_file(&id).await;
        }

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
            Some(setup) if !is_set_up(&setup, &self.service, &self.allowed) => {
                tracing::warn!(
                    "spaces: {uri} was not set up as this AppView makes it, and is again"
                );
                let (service, allowed) = (&self.service, &self.allowed);
                self.host
                    .manage_space(&session, uri, service, allowed)
                    .await?;
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

    /// Read every context's space back, as [`Self::check_context`] does, and
    /// then ask the question the redesign rests on: is the index rebuildable
    /// from the records ALONE? Each difference, under the context it is in.
    ///
    /// With `bytes`, every file is fetched back too and held to its hash, which
    /// is a download of everything the wiki keeps.
    pub async fn check_everything(
        &self,
        state: &AppState,
        bytes: bool,
    ) -> Result<Vec<String>, Failure> {
        let organization = self.organization().await?;
        let mut wrong = Vec::new();
        let mut found = BTreeMap::new();
        let every = "SELECT id FROM context ORDER BY length(path), path";
        for context_id in ids(state, every, ()).await? {
            match self.pulled(&organization, &context_id).await {
                Ok((records, credential)) => {
                    let expected = Held::of(state, &organization, &context_id).await?.cids;
                    let mut differ = versions_differ(&expected, &records);
                    let files = records.iter().filter(|r| bytes && r.collection == FILE);
                    for file in files {
                        let space = Self::space_of(&organization, &context_id).to_string();
                        let cid = file.value["blob"]["ref"]["$link"]
                            .as_str()
                            .unwrap_or_default();
                        let auth = credential.auth();
                        let said = match self.host.blob(auth, &space, &organization, cid).await {
                            Ok(fetched) => hex(&Sha256::digest(&fetched)),
                            Err(e) => format!("nothing ({e})"),
                        };
                        if file.value["sha256"] != said.as_str() {
                            differ.push(format!(
                                "{FILE}/{}: its bytes came back as {said}",
                                file.rkey
                            ));
                        }
                    }
                    // A body that went out as a file is part of the page it is of.
                    let mut records = records;
                    for record in &mut records {
                        let Some(cid) = wiki_records::body_blob(&record.value).map(str::to_string)
                        else {
                            continue;
                        };
                        let space = Self::space_of(&organization, &context_id).to_string();
                        let auth = credential.auth();
                        let body = self.host.blob(auth, &space, &organization, &cid).await;
                        if !body.is_ok_and(|bytes| wiki_records::body_in(&mut record.value, &bytes))
                        {
                            differ.push(format!(
                                "{}/{}: its body did not come back",
                                record.collection, record.rkey
                            ));
                        }
                    }
                    wrong.extend(differ.iter().map(|w| format!("{context_id}: {w}")));
                    found.insert(context_id, records);
                }
                Err(e) => wrong.push(format!("{context_id}: could not be read back: {e}")),
            }
        }
        wrong.extend(rebuilt_differs(state, &organization, &found).await?);
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
        let (records, _) = self.pulled(&organization, context_id).await?;
        let expected = Held::of(state, &organization, context_id).await?.cids;
        Ok(versions_differ(&expected, &records))
    }

    /// The organization's records in one context's space, held to its commit,
    /// and the credential they were read with.
    async fn pulled(
        &self,
        organization: &str,
        context_id: &str,
    ) -> Result<(Vec<Record>, Credential), Failure> {
        let space = Self::space_of(organization, context_id).to_string();
        let key = self.directory.resolve(organization).await?.signing_key;
        let (_, session) = self.session().await?;
        // A space that admits by list asks its own authority's application too.
        let attestation = self
            .client_id
            .as_deref()
            .map(|client_id| self.key.attest(client_id, organization));
        let (host, attestation) = (&self.host, attestation.as_deref());
        let credential = Credential::obtain(host, &session, host, &space, attestation).await?;
        // From nothing, so the whole repo is listed and held to its commit.
        let mut copy = sync::Copy::default();
        let pull = sync::pull(
            &self.host,
            credential.auth(),
            &space,
            organization,
            &key,
            &mut copy,
        );
        let records = match pull.await? {
            Pulled::Everything(records) => records,
            Pulled::Nothing | Pulled::Changes(_) => Vec::new(),
        };
        Ok((records, credential))
    }
}

/// A poll's board is in the same repo, and is its publisher's to account for
/// and a member's mirror's to check (`crate::board`).
const MIRRORED: [&str; 5] = [PROFILE, NODE, COMMENT, REACTION, FILE];

fn versions_differ(
    expected: &BTreeMap<(String, String), String>,
    records: &[Record],
) -> Vec<String> {
    let there: BTreeMap<(String, String), &str> = records
        .iter()
        .filter(|r| MIRRORED.contains(&r.collection.as_str()))
        .map(|r| ((r.collection.clone(), r.rkey.clone()), r.cid.as_str()))
        .collect();
    let mut wrong = Vec::new();
    for (at, cid) in expected {
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
    wrong
}

/// Rebuild every row from the records in `found` (by context) and nothing
/// else, but for what no record carries ([`Kept`]), and hold each to the row
/// that is there. What a rebuild would get wrong, in words.
pub(crate) async fn rebuilt_differs(
    state: &AppState,
    organization: &str,
    found: &BTreeMap<String, Vec<Record>>,
) -> Result<Vec<String>, Failure> {
    let store = Store::new(state.db.clone());
    let mut tree = Found::default();
    for (context_id, records) in found {
        for record in records {
            let value = record.value.clone();
            match record.collection.as_str() {
                PROFILE => {
                    if let Ok(profile) = serde_json::from_value(value) {
                        tree.profiles.insert(context_id.clone(), profile);
                    }
                }
                NODE => {
                    if let Ok(node) = serde_json::from_value(value) {
                        tree.nodes
                            .insert(record.rkey.clone(), (context_id.clone(), node));
                    }
                }
                _ => {}
            }
        }
    }

    let mut wrong = Vec::new();
    for (context_id, records) in found {
        let space = Spaces::space_of(organization, context_id);
        for record in records
            .iter()
            .filter(|r| MIRRORED.contains(&r.collection.as_str()))
        {
            let at = space.record(organization, &record.collection, &record.rkey);
            let value = record.value.clone();
            // `None` for a record that does not read as what its collection says.
            let sides: Option<(Option<Value>, Option<Value>)> = match record.collection.as_str() {
                PROFILE => {
                    let there = store.row_context(context_id).await?;
                    let kept = there.as_ref().map(|row| Kept {
                        visibility: row.visibility,
                        published_uri: row.published_uri.clone(),
                    });
                    let above = tree.context_parent_path(context_id);
                    serde_json::from_value::<ContextProfile>(value)
                        .ok()
                        .map(|profile| {
                            let rebuilt = above.and_then(|above| {
                                profile.row(&space, &above, kept.unwrap_or_default())
                            });
                            (json_of(&rebuilt), json_of(&there))
                        })
                }
                NODE => {
                    let there = store.row_document(&record.rkey).await?;
                    let kept = there.as_ref().map(|row| Kept {
                        visibility: row.visibility,
                        published_uri: row.published_uri.clone(),
                    });
                    let above = tree.node_parent_path(&record.rkey);
                    serde_json::from_value::<Node>(value).ok().map(|node| {
                        let rebuilt =
                            above.and_then(|above| node.row(&at, &above, kept.unwrap_or_default()));
                        (json_of(&rebuilt), json_of(&there))
                    })
                }
                FILE => {
                    let there = file_row(state, &record.rkey).await?;
                    serde_json::from_value::<File>(value)
                        .ok()
                        .map(|file| (json_of(&file.row(&at)), json_of(&there)))
                }
                COMMENT => {
                    let there = store.row_comment(&record.rkey).await?;
                    serde_json::from_value::<Comment>(value)
                        .ok()
                        .map(|comment| (json_of(&comment.row(&at)), json_of(&there)))
                }
                _ => {
                    let there = store.row_reaction(&record.rkey).await?;
                    serde_json::from_value::<Reaction>(value)
                        .ok()
                        .map(|reaction| (json_of(&reaction.row(&at)), json_of(&there)))
                }
            };
            let name = format!("{context_id}: {}/{}", record.collection, record.rkey);
            match sides {
                None => wrong.push(format!("{name}: does not read as what it is filed as")),
                Some((None, _)) => wrong.push(format!("{name}: no row can be rebuilt from it")),
                Some((Some(_), None)) => wrong.push(format!("{name}: there is no such row")),
                Some((Some(rebuilt), Some(there))) => {
                    let differ = fields_that_differ(&rebuilt, &there);
                    if !differ.is_empty() {
                        wrong.push(format!(
                            "{name}: rebuilt differently in {}",
                            differ.join(", ")
                        ));
                    }
                }
            }
        }
    }
    // In one order, whatever order a host lists a repo in.
    wrong.sort();
    Ok(wrong)
}

/// A row as JSON, less what is filled in when a row is READ and is no part of
/// it: the name and path of a group credited as an author.
fn json_of<T: serde::Serialize>(row: &Option<T>) -> Option<Value> {
    let mut json = serde_json::to_value(row.as_ref()?).ok()?;
    let credited = json.get_mut("authors").and_then(Value::as_array_mut);
    for author in credited
        .into_iter()
        .flatten()
        .filter_map(Value::as_object_mut)
    {
        author.remove("name");
        author.remove("path");
    }
    Some(json)
}

/// The fields two rows differ in, by name, a level into `place`.
fn fields_that_differ(rebuilt: &Value, there: &Value) -> Vec<String> {
    let (Some(rebuilt), Some(there)) = (rebuilt.as_object(), there.as_object()) else {
        return vec!["everything".to_string()];
    };
    let names: BTreeSet<&String> = rebuilt.keys().chain(there.keys()).collect();
    let mut differ = Vec::new();
    for name in names {
        match (rebuilt.get(name), there.get(name)) {
            (a, b) if a == b => {}
            (Some(a), Some(b)) if name == "place" => {
                let within = fields_that_differ(a, b);
                differ.extend(within.iter().map(|field| format!("place.{field}")));
            }
            _ => differ.push(name.clone()),
        }
    }
    differ
}

/// The key this application attests with. Derived, as the custody key is, so
/// that it survives a restart and a restore and is in no copy of the database.
/// One in 2^128 derivations is no valid scalar, so a counter is bound in.
fn client_key(config: &Config) -> ClientKey {
    (0u8..=255)
        .find_map(|attempt| {
            let mut seed = [0u8; 32];
            hkdf::Hkdf::<Sha256>::new(Some(&[attempt]), config.secret.as_bytes())
                .expand(b"wiki-appview spaces client key v1", &mut seed)
                .ok()?;
            ClientKey::from_seed(&seed)
        })
        .expect("one of 256 derivations is a valid P-256 scalar")
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
    hex(&Sha256::digest(unpinned.to_string().as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
        if let Some(settled) = self.settled(collection, rkey, &said) {
            return Ok(settled);
        }
        // A page too long to be one record: its body goes as a file, which the
        // record then names. After `said`, so that it goes once.
        if let Some(body) = wiki_records::body_out(&mut record, RECORD_ROOM) {
            if body.len() as u64 > self.spaces.blob_limit {
                let bytes = body.len();
                tracing::warn!(
                    "spaces: {collection}/{rkey} has a body of {bytes} bytes, past what the PDS takes"
                );
                return self.remember(collection, rkey, None, said).await;
            }
            let (_, session) = self.spaces.session().await?;
            let host = &self.spaces.host;
            let blob = host
                .upload_blob(&session, body, wiki_records::BODY_MIME)
                .await?;
            wiki_records::body_at(&mut record, blob);
        }
        self.send(collection, rkey, &record, said).await
    }

    /// What became of this version of a row already, if anything did.
    fn settled(&self, collection: &str, rkey: &str, said: &str) -> Option<Wrote> {
        let at = (collection.to_string(), rkey.to_string());
        let same = self.held.said.get(&at).map(String::as_str) == Some(said);
        same.then(|| match self.held.cids.contains_key(&at) {
            true => Wrote::Same,
            false => Wrote::Refused,
        })
    }

    async fn send(
        &mut self,
        collection: &str,
        rkey: &str,
        record: &Value,
        said: String,
    ) -> Result<Wrote, Failure> {
        let space = self
            .spaces
            .ensure_space(self.state, self.context_id)
            .await?;
        let (organization, session) = self.spaces.session().await?;
        let space = space.to_string();
        let put =
            self.spaces
                .host
                .put_record(&session, &space, &organization, collection, rkey, record);
        match put.await {
            Ok(written) => {
                self.remember(collection, rkey, Some(written.cid), said)
                    .await
            }
            // The record itself is what the PDS will not take (too large, or
            // data it cannot hold), and asking again changes nothing: it is
            // remembered as refused until the row says something else.
            Err(e) if e.is_about_the_record() => {
                let bytes = record.to_string().len();
                tracing::warn!("spaces: {collection}/{rkey} ({bytes} bytes) was refused: {e}");
                self.remember(collection, rkey, None, said).await
            }
            Err(e) => {
                self.spaces.forget_session_if_spent(&e).await;
                Err(e.into())
            }
        }
    }

    /// No CID is a version the PDS refused.
    async fn remember(
        &mut self,
        collection: &str,
        rkey: &str,
        cid: Option<String>,
        said: String,
    ) -> Result<Wrote, Failure> {
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
        let at = (collection.to_string(), rkey.to_string());
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

    /// A file: its bytes to the PDS as a blob, and the record that names it.
    /// The bytes go once, for what the row says tells a file that changed.
    async fn put_file(&mut self, id: &str) {
        self.rows.insert((FILE.to_string(), id.to_string()));
        match self.write_file(id).await {
            Ok(Some(Wrote::Written)) => self.swept.written += 1,
            Ok(Some(Wrote::Same)) => self.swept.same += 1,
            Ok(Some(Wrote::Refused)) => self.swept.refused += 1,
            Ok(None) => {}
            Err(e) => {
                self.swept.failed += 1;
                tracing::warn!("spaces: could not write the file {id}: {e}");
            }
        }
    }

    async fn write_file(&mut self, id: &str) -> Result<Option<Wrote>, Failure> {
        let Some(row) = file_row(self.state, id).await? else {
            return Ok(None);
        };
        let said = said(&serde_json::to_value(&row)?);
        if let Some(settled) = self.settled(FILE, id, &said) {
            return Ok(Some(settled));
        }
        if row.size.max(0) as u64 > self.spaces.blob_limit {
            tracing::warn!(
                "spaces: the file {id} ({} bytes) is past what the PDS takes",
                row.size
            );
            return self.remember(FILE, id, None, said).await.map(Some);
        }
        let bytes = tokio::fs::read(crate::blob::path_of(&self.state.config, &row.sha256)).await?;
        let (_, session) = self.spaces.session().await?;
        let blob = match self
            .spaces
            .host
            .upload_blob(&session, bytes, &row.mime)
            .await
        {
            Ok(blob) => blob,
            Err(e) if e.is_about_the_record() => {
                tracing::warn!(
                    "spaces: the file {id} ({} bytes) was refused: {e}",
                    row.size
                );
                return self.remember(FILE, id, None, said).await.map(Some);
            }
            Err(e) => {
                self.spaces.forget_session_if_spent(&e).await;
                return Err(e.into());
            }
        };
        let mut record = serde_json::to_value(File::of(&row, blob))?;
        record["$type"] = Value::String(FILE.to_string());
        self.send(FILE, id, &record, said).await.map(Some)
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

/// What is kept of a file beside its bytes (`crate::blob`).
async fn file_row(state: &AppState, id: &str) -> Result<Option<FileRow>, Failure> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query(
            "SELECT id, context_id, owner_did, sha256, size, mime, name, created_at \
             FROM blob WHERE id = ?1",
            [id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let text = |i: usize| match row.get_value(i) {
        Ok(Sql::Text(text)) => Some(text),
        _ => None,
    };
    Ok(Some(FileRow {
        id: row.get(0)?,
        context_id: row.get(1)?,
        owner_did: text(2),
        sha256: row.get(3)?,
        size: row.get(4)?,
        mime: row.get(5)?,
        name: text(6),
        created_at: row.get(7)?,
    }))
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

/// `/jwks.json`: the key this application attests with, where its client
/// metadata says its keys are (`jwks_uri`).
pub async fn jwks(State(state): State<AppState>) -> Response {
    match &state.spaces {
        Some(spaces) => Json(spaces.key.jwks()).into_response(),
        None => refused(StatusCode::NOT_FOUND, "NotFound"),
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
        let row = comment.row(&at.record(DID, COMMENT, "k1"));
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

    /// What the fake holds of every space, as a read-back would have found it.
    fn held_by(pds: &FakePds, contexts: &[&str]) -> BTreeMap<String, Vec<Record>> {
        let of = |context_id: &&str| {
            let records = pds.space_records(&space(context_id));
            let found = records.into_iter().map(|r| {
                let cid = r.cid();
                let mut value = r.value;
                let body = wiki_records::body_blob(&value).and_then(|cid| pds.blob(cid));
                if let Some(body) = body {
                    assert!(wiki_records::body_in(&mut value, &body));
                }
                Record {
                    cid,
                    collection: r.collection,
                    rkey: r.rkey,
                    value,
                }
            });
            (context_id.to_string(), found.collect())
        };
        contexts.iter().map(of).collect()
    }

    #[tokio::test]
    async fn a_varied_wiki_is_rebuilt_from_its_records_alone() {
        let (state, pds, spaces) = mirroring().await;
        // A group in a folder of another, every kind of author, a locked page, a
        // draft, a binned subtree, a thread with a picture and a tombstone, a
        // reactor whose account did not come across, fractions, stamps and ids
        // from the interim.
        run_sql(
            &state,
            r#"INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, idx,
                 mutable, attachable, owner_did, content, data, legacy_id, created_at, updated_at)
               VALUES ('f1', 'c9', 'c9', 'folder', 'Møder', 'moeder', 'closed/moeder', 3,
                 0, 0, 'did:plc:alice', NULL, '{"icon":"event"}', 'old-f1',
                 '2024-03-01T09:00:00.000Z', '2025-01-02T10:30:00.000Z');
               INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, idx,
                 mutable, content, data, visibility, published_uri)
               VALUES ('p1', 'c9', 'f1', 'policy', 'Kontingent', 'kontingent',
                 'closed/moeder/kontingent', 1, 1,
                 '[{"type":"image","width":0.75,"children":[{"text":""}]}]',
                 '{"threshold":66.7,"fileId":"file-1"}', 'public', 'at-uri-of-a-resolution');
               INSERT INTO document_author (document_id, author_did, author_text, author_context, ord)
               VALUES ('p1', 'did:plc:bob', NULL, NULL, 0), ('p1', NULL, 'Aarhus', NULL, 1),
                      ('p1', NULL, NULL, 'c1', 2);
               INSERT INTO context (id, kind, name, slug, path, parent_id, idx, content)
               VALUES ('c11', 'event', 'Landsmøde', 'landsmoede', 'closed/moeder/landsmoede',
                 'f1', 2, '[{"children":[{"text":"Velkommen"}]}]');
               INSERT INTO document (id, context_id, parent_id, kind, title, slug, path)
               VALUES ('a1', 'c11', 'c11', 'document', 'Dagsorden', 'dagsorden',
                 'closed/moeder/landsmoede/dagsorden');
               INSERT INTO document (id, context_id, parent_id, kind, title, slug, path,
                 deleted_at, deleted_root)
               VALUES ('b1', 'c9', 'c9', 'folder', 'Gammelt', 'gammelt', 'closed/gammelt',
                 '2026-09-01T08:00:00.000Z', 'b1'),
                      ('b2', 'c9', 'b1', 'document', 'Noter', 'noter', 'closed/gammelt/noter',
                 '2026-09-01T08:00:00.000Z', 'b1');
               INSERT INTO comment (id, on_id, root_id, context_id, author_did, author_text, text,
                 image, tombstone, created_at, deleted_at, deleted_root, legacy_id)
               VALUES ('k2', 'p1', 'p1', 'c9', 'did:plc:bob', NULL, 'Enig', 'file-9', 0,
                 '2026-09-02T10:00:00.000Z', NULL, NULL, 'old-k2'),
                      ('k3', 'k2', 'p1', 'c9', NULL, '', '', NULL, 1,
                 '2026-09-02T10:05:00.000Z', NULL, NULL, NULL),
                      ('k4', 'k3', 'p1', 'c9', NULL, 'En gæst', 'Også mig', NULL, 0,
                 '2026-09-02T10:06:00.000Z', '2026-09-03T08:00:00.000Z', 'k4', NULL);
               INSERT INTO reaction (id, subject_uri, reactor_did, emoji, legacy_id)
               VALUES ('r2', 'k2', NULL, '❤️', 'old-r2'), ('r3', 'p1', 'did:plc:alice', '👍', NULL)"#,
        )
        .await;
        let swept = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!(
            (swept.waiting, swept.refused, swept.failed),
            (0, 0, 0),
            "{swept:?}"
        );

        let found = held_by(&pds, &["c1", "c2", "c9", "c10", "c11"]);
        let wrong = rebuilt_differs(&state, DID, &found)
            .await
            .expect("a rebuild");
        assert_eq!(wrong, Vec::<String>::new());

        // And it notices: a record that says something else than its row, one
        // whose parent is nowhere, and one that is not what it is filed as.
        let mut found = found;
        let c9 = found.get_mut("c9").expect("c9");
        for record in c9.iter_mut() {
            match record.rkey.as_str() {
                "p1" => record.value["title"] = json!("Kontingent 2027"),
                "k2" => record.value = json!({"text": 7}),
                "b2" => record.value["parent"] = json!(space("c9") + "/x/wiki.radikal.node/gone"),
                _ => {}
            }
        }
        let wrong = rebuilt_differs(&state, DID, &found)
            .await
            .expect("a rebuild");
        assert_eq!(
            wrong,
            [
                "c9: wiki.radikal.comment/k2: does not read as what it is filed as",
                "c9: wiki.radikal.node/b2: no row can be rebuilt from it",
                "c9: wiki.radikal.node/p1: rebuilt differently in title",
            ]
        );
    }

    /// Keep `bytes` as the file `id` of c9, as an upload would have.
    async fn a_file(state: &AppState, id: &str, mime: &str, bytes: &[u8]) {
        let source = std::env::temp_dir().join(format!("a-file-{}", crate::util::random_token(8)));
        tokio::fs::write(&source, bytes).await.expect("a source");
        let blob = crate::blob::BlobMeta {
            id: id.to_string(),
            context_id: "c9".to_string(),
            owner_did: Some("did:plc:alice".to_string()),
            sha256: String::new(),
            size: 0,
            mime: mime.to_string(),
            name: Some(format!("{id}.bin")),
        };
        crate::blob::file_a_copy(state, &source, blob)
            .await
            .expect("a file");
        let _ = tokio::fs::remove_file(source).await;
    }

    #[tokio::test]
    async fn a_file_goes_to_the_pds_once_and_one_too_large_stays_home() {
        let (mut state, pds, spaces) = mirroring().await;
        state.config.blob_dir = std::env::temp_dir()
            .join(format!("appview-blobs-{}", crate::util::random_token(8)))
            .to_string_lossy()
            .into_owned();
        a_file(
            &state,
            "file-1",
            "application/pdf",
            b"%PDF a made-up agenda",
        )
        .await;
        a_file(&state, "file-2", "image/png", b"a picture, more or less").await;
        a_file(&state, "file-3", "image/jpeg", &vec![7u8; 6 * 1024 * 1024]).await;
        a_file(&state, "file-4", "image/png", b"what a report showed").await;
        // A file node, a picture in a thread, a cover past what a PDS takes, and
        // a report's picture, which is nobody's in the group to see.
        run_sql(
            &state,
            r#"INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, data)
               VALUES ('fd1', 'c9', 'c9', 'file', 'Dagsorden', 'dagsorden', 'closed/dagsorden',
                 '{"fileId":"file-1"}');
               UPDATE comment SET image = 'file-2' WHERE id = 'ks';
               UPDATE context SET data = '{"image":"file-3"}' WHERE id = 'c9'"#,
        )
        .await;
        let uploads = |pds: &FakePds| {
            let calls = pds.calls();
            calls
                .iter()
                .filter(|(method, _)| method == "uploadBlob")
                .count()
        };

        let first = spaces.mirror_context(&state, "c9").await.expect("a pass");
        assert_eq!(
            (first.refused, first.failed, uploads(&pds)),
            (1, 0, 2),
            "{first:?}"
        );
        let files: Vec<String> = keys(&pds, "c9")
            .into_iter()
            .filter(|key| key.starts_with("file/"))
            .collect();
        assert_eq!(files, ["file/file-1", "file/file-2"]);
        let records = pds.space_records(&space("c9"));
        let agenda = records
            .iter()
            .find(|r| r.rkey == "file-1")
            .expect("the file");
        assert_eq!(agenda.value["name"], "file-1.bin");
        assert_eq!(agenda.value["blob"]["mimeType"], "application/pdf");
        let cid = agenda.value["blob"]["ref"]["$link"]
            .as_str()
            .expect("a cid");
        assert_eq!(
            pds.blob(cid).as_deref(),
            Some(&b"%PDF a made-up agenda"[..])
        );

        // Nothing is sent twice, and what stayed home is said every time.
        let again = spaces.mirror_context(&state, "c9").await.expect("a pass");
        assert_eq!((again.written, again.refused, uploads(&pds)), (0, 1, 2));

        // The rows are what the records say, down to what the blob says.
        let found = held_by(&pds, &["c9"]);
        let wrong = rebuilt_differs(&state, DID, &found)
            .await
            .expect("a rebuild");
        assert_eq!(wrong, Vec::<String>::new());

        // A file nothing names any more is a file the space lets go of.
        run_sql(&state, "UPDATE comment SET image = NULL WHERE id = 'ks'").await;
        let let_go = spaces.mirror_context(&state, "c9").await.expect("a pass");
        assert_eq!(let_go.deleted, 1);
        assert!(!keys(&pds, "c9").contains(&"file/file-2".to_string()));
        let _ = tokio::fs::remove_dir_all(&state.config.blob_dir).await;
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
    async fn a_space_someone_opened_is_taken_back() {
        let (state, pds, spaces) = mirroring().await;
        spaces.mirror_everything(&state).await.expect("a pass");
        let setup = |pds: &FakePds| {
            let mut found = None;
            pds.tamper_spaces(|all| found = all.get(&space("c9")).map(|s| s.setup.clone()));
            found.expect("the space")
        };
        // Made for this application and nobody else's, since it is deployed and
        // so has a name to attest as.
        let only_us = ["https://wiki.test/client-metadata.json".to_string()];
        assert!(is_set_up(&setup(&pds), SERVICE, &only_us));

        // To every reader, and then to every application.
        for (policy, opened) in [
            ("readPolicy", "com.atproto.simplespace.defs#publicPolicy"),
            ("appAccess", "com.atproto.simplespace.defs#open"),
        ] {
            pds.tamper_spaces(|all| {
                let c9 = all.get_mut(&space("c9")).expect("the space");
                c9.setup[policy] = json!({"$type": opened});
            });
            assert!(!is_set_up(&setup(&pds), SERVICE, &only_us));
            spaces.mirror_everything(&state).await.expect("a pass");
            assert!(is_set_up(&setup(&pds), SERVICE, &only_us), "{policy}");
        }
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

        // A page past what a PDS takes in one request: its body goes as a file,
        // and comes back as the page's.
        let conn = state.db.acquire().await.expect("conn");
        let long = json!({"text": "x".repeat(1_100_000)}).to_string();
        conn.execute("UPDATE document SET content = ?1 WHERE id = 'd2'", [long])
            .await
            .expect("a long page");
        let as_a_file = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!((as_a_file.written, as_a_file.refused), (1, 0));
        let d2 = pds.space_records(&space("c1"));
        let d2 = d2.iter().find(|r| r.rkey == "d2").expect("d2");
        assert!(d2.value.get("content").is_none() && d2.value["contentBlob"]["size"].is_u64());
        let found = held_by(&pds, &["c1"]);
        let wrong = rebuilt_differs(&state, DID, &found)
            .await
            .expect("a rebuild");
        assert_eq!(wrong, Vec::<String>::new());

        // And one past what it takes as a file stays home.
        let longer = json!({"text": "x".repeat(6 * 1024 * 1024)}).to_string();
        conn.execute("UPDATE document SET content = ?1 WHERE id = 'd2'", [longer])
            .await
            .expect("a longer page");
        let before = puts(&pds);
        let refused = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!(
            (refused.refused, refused.failed, puts(&pds) - before),
            (1, 0, 0)
        );
        // Counted for as long as it is so, and not sent.
        let again = spaces.mirror_context(&state, "c1").await.expect("a pass");
        assert_eq!((again.refused, puts(&pds) - before), (1, 0));
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

        // The key it attests with, by the name an attestation gives it.
        let (status, keys) = ask(&state, JWKS_PATH, None).await;
        assert_eq!(status, StatusCode::OK);
        let spaces = state.spaces.clone().expect("spaces");
        assert_eq!(keys["keys"][0]["kid"], spaces.key.kid().as_str());
        assert!(keys["keys"][0].get("d").is_none());

        let plain = seeded_state().await;
        assert_eq!(ask(&plain, JWKS_PATH, None).await.0, StatusCode::NOT_FOUND);
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

    /// The key of the board mirror the alpha tests let into their spaces.
    pub(crate) fn mirror_key() -> ClientKey {
        ClientKey::from_seed(&[9u8; 32]).expect("a scalar")
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

            // Client metadata a PDS on this machine will fetch and take: by
            // name and not by address, with a redirect by address and not by
            // name. This application's, and a board mirror's (`mirror_key`).
            let at = endpoint.replace("127.0.0.1", "localhost");
            let metadata = |name: &str, keys: Value| {
                let mut doc = json!({
                    "client_id": format!("{at}/{name}"),
                    "redirect_uris": [format!("{endpoint}/callback")],
                    "grant_types": ["authorization_code"], "response_types": ["code"],
                    "scope": "atproto", "token_endpoint_auth_method": "none",
                    "application_type": "web", "dpop_bound_access_tokens": true,
                });
                let named = if keys.is_string() { "jwks_uri" } else { "jwks" };
                doc[named] = keys;
                doc
            };
            let ours = metadata("test-client.json", json!(format!("{at}{JWKS_PATH}")));
            let mirrors = metadata("mirror-client.json", mirror_key().jwks());
            let client_ids = [ours["client_id"].clone(), mirrors["client_id"].clone()];
            let documents = axum::Router::new()
                .route(
                    "/test-client.json",
                    axum::routing::get(|| async { Json(ours) }),
                )
                .route(
                    "/mirror-client.json",
                    axum::routing::get(|| async { Json(mirrors) }),
                );

            state.config.spaces_pds = self.pds_url.clone();
            state.config.spaces_identifier = format!("wiki{}.test", self.run);
            state.config.spaces_password = crate::config::Secret::new(ALPHA_PASSWORD);
            state.config.spaces_service = format!("{app_did}#wiki_appview");
            state.config.plc_url = self.plc_url.clone();
            let [ours, mirrors] = client_ids.map(|id| id.as_str().expect("an id").to_string());
            state.config.spaces_allowed_clients = vec![mirrors];
            let mut spaces = Spaces::from_config(&state.config)
                .expect("whole")
                .expect("spaces");
            spaces.attest_as(ours);
            let spaces = Arc::new(spaces);
            state.spaces = Some(spaces.clone());
            let router = crate::router(state.clone()).merge(documents);
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
        let mut state = seeded_state().await;
        state.config.blob_dir = std::env::temp_dir()
            .join(format!("appview-blobs-{}", crate::util::random_token(8)))
            .to_string_lossy()
            .into_owned();
        let (state, spaces) = alpha.serve(state).await;
        a_file(
            &state,
            "file-1",
            "application/pdf",
            b"%PDF a made-up agenda",
        )
        .await;
        run_sql(
            &state,
            &format!(
                "INSERT INTO user (did) VALUES ('{alice}'); \
                 INSERT INTO member (id, user_did, context_id, role, active) \
                   VALUES ('m-real-alice', '{alice}', 'c9', 'member', 1); \
                 INSERT INTO reaction (id, subject_uri, reactor_did, emoji) \
                   VALUES ('r1', 'd1', '{alice}', '👍'); \
                 INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, data) \
                   VALUES ('fd1', 'c9', 'c9', 'file', 'Dagsorden', 'dagsorden', \
                           'closed/dagsorden', '{{\"fileId\":\"file-1\"}}'); \
                 UPDATE document SET data = '{{\"threshold\":0.5}}', \
                   content = '[{{\"type\":\"paragraph\",\"children\":[{{\"text\":\"Vi foreslår\"}}]}}]' \
                   WHERE id = 'd1'"
            ),
        )
        .await;
        // A page past what a PDS takes as one record, whose body goes as a file.
        let conn = state.db.acquire().await.expect("conn");
        let long = json!([{"children": [{"text": "x".repeat(1_100_000)}]}]).to_string();
        conn.execute("UPDATE document SET content = ?1 WHERE id = 'd2'", [long])
            .await
            .expect("a long page");

        let first = spaces.mirror_everything(&state).await.expect("a pass");
        assert_eq!(
            first,
            Swept {
                written: 12,
                ..Swept::default()
            }
        );
        // Read back as any syncer would: the same records, the same versions,
        // under a commit the organization signed, every row rebuilt from its
        // record alone, and every file's bytes as they were.
        let wrong = spaces
            .check_everything(&state, true)
            .await
            .expect("a check");
        assert_eq!(wrong, Vec::<String>::new());

        // A member gets in through her own PDS session, and reads the page as
        // it was written. Who the roster does not name does not.
        let pds = &alpha.pds;
        let c9 = Spaces::space_of(&org, "c9").to_string();
        // Through an application the space names, which a member's own choice
        // of tool is not: the mirror's key stands in for one that is.
        let mirror = format!(
            "{}/mirror-client.json",
            spaces
                .client_id
                .as_deref()
                .expect("a client")
                .trim_end_matches("/test-client.json")
        );
        let attest = || mirror_key().attest(&mirror, &org);
        let unnamed = Credential::obtain(pds, &alice_session, pds, &c9, None).await;
        assert_eq!(
            unnamed.err().as_ref().and_then(|e| e.xrpc_name()),
            Some("AppNotAuthorized")
        );
        let hers = Credential::obtain(pds, &alice_session, pds, &c9, Some(&attest()))
            .await
            .expect("a credential for a member");
        let read = pds.records(hers.auth(), &c9, &org).await.expect("records");
        let s1 = read.iter().find(|r| r.rkey == "s1").expect("s1");
        assert_eq!(s1.value["title"], "Secret Minutes");
        let agenda = read.iter().find(|r| r.rkey == "file-1").expect("the file");
        let cid = agenda.value["blob"]["ref"]["$link"]
            .as_str()
            .expect("a cid");
        let bytes = pds
            .blob(hers.auth(), &c9, &org, cid)
            .await
            .expect("the bytes");
        assert_eq!(bytes, b"%PDF a made-up agenda");
        let his = Credential::obtain(pds, &bob_session, pds, &c9, Some(&attest())).await;
        assert_eq!(
            his.err().as_ref().and_then(|e| e.xrpc_name()),
            Some("UserNotAuthorized")
        );
        // What is open is anyone's to read, with its fraction back in place.
        let c1 = Spaces::space_of(&org, "c1").to_string();
        let anyones = Credential::obtain(pds, &bob_session, pds, &c1, Some(&attest()))
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
        assert_eq!((swept.written, swept.failed), (4, 0), "{swept:?}");
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

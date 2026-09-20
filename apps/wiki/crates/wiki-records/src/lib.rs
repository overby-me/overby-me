//! A context's space, as records (`lexicons/wiki/radikal/{contextProfile,node,
//! comment,reaction}.json`), and the way between them and the rows the AppView
//! keeps (`wiki-domain-types`).
//!
//! The mapping loses nothing in either direction (`tests/roundtrip.rs`): the
//! index has to be rebuildable from the records alone, and a row written
//! through a record has to come back as it went. What a record does not carry
//! is derived on the way back: a path from the parents' slugs, and which
//! context a node is in from the space it was found in.

mod uri;

pub use uri::{RecordUri, SpaceUri};

use serde::{Deserialize, Serialize};
use wiki_domain_types as rows;

/// The type of every context's space.
pub const CONTEXT_SPACE: &str = "wiki.radikal.context";
pub const PROFILE: &str = "wiki.radikal.contextProfile";
pub const NODE: &str = "wiki.radikal.node";
pub const COMMENT: &str = "wiki.radikal.comment";
pub const REACTION: &str = "wiki.radikal.reaction";

/// The one format a body is kept in today: the editor's own document.
pub const SLATE: &str = "slate/1";

/// `wiki.radikal.spaceDefs#author`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// The space of a group or event that is credited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// `wiki.radikal.spaceDefs#binned`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binned {
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

/// `com.atproto.repo.strongRef`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrongRef {
    pub uri: String,
    pub cid: String,
}

/// `wiki.radikal.contextProfile`, at the key `self` of the organization's repo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextProfile {
    pub kind: String,
    pub name: String,
    pub slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_by: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binned: Option<Binned>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
}

/// `wiki.radikal.node`, keyed by the node's id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub kind: String,
    pub title: String,
    pub slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_by: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binned: Option<Binned>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
}

/// `wiki.radikal.comment`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub subject: StrongRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<StrongRef>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tombstone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binned: Option<Binned>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    pub created_at: String,
}

/// `wiki.radikal.reaction`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    pub subject: StrongRef,
    pub emoji: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    pub created_at: String,
}

/// What a row has that no record carries, because it is the AppView's word and
/// not content: who a context or a page is open to (what the managing app
/// answers), and where a page was published on the open network. A rebuild
/// keeps them from the row it replaces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Kept {
    pub visibility: rows::Visibility,
    pub published_uri: Option<String>,
}

/// What the mapping has to be told, because a record says it by address and a
/// row by id: where things are held, and the version a reference pins.
pub trait Addresses {
    /// The space of the context with this id.
    fn space(&self, context_id: &str) -> SpaceUri;
    /// The repository that holds the node, comment or reaction with this id. In
    /// the first stage always the organization's.
    fn holder(&self, id: &str) -> String;
    /// The CID of the record last written for this id, which a strong reference
    /// to it pins. `None` for one that has no record yet.
    fn cid(&self, collection: &str, id: &str) -> Option<String>;
}

/// The stamp a record must have where a row may have none (one carried over
/// with no dates): the epoch, which no real row has and which sorts first.
const NO_DATE: &str = "1970-01-01T00:00:00.000Z";

const NUMBER: &str = "wiki.radikal.spaceDefs#number";
/// As far as every JSON reader keeps an integer exact, and as far as a PDS
/// takes one.
const EXACT: u64 = (1 << 53) - 1;

/// An editor's document or a node's settings, as a record may hold it. atproto
/// data has no fractions and a PDS refuses a record with one, so a number it
/// would not take goes as `wiki.radikal.spaceDefs#number`, in its own digits.
fn storable(json: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match json {
        Value::Number(n) => {
            let fits = n.as_u64().is_some_and(|u| u <= EXACT)
                || n.as_i64().is_some_and(|i| i.unsigned_abs() <= EXACT);
            match fits {
                true => json.clone(),
                false => serde_json::json!({"$type": NUMBER, "value": n.to_string()}),
            }
        }
        Value::Array(items) => Value::Array(items.iter().map(storable).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), storable(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// [`storable`], undone.
fn restored(json: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match json {
        Value::Array(items) => Value::Array(items.iter().map(restored).collect()),
        Value::Object(fields) => {
            let number = (fields.get("$type").and_then(Value::as_str) == Some(NUMBER))
                .then(|| fields.get("value").and_then(Value::as_str))
                .flatten()
                .and_then(|digits| serde_json::from_str::<serde_json::Number>(digits).ok());
            match number {
                Some(number) => Value::Number(number),
                None => Value::Object(
                    fields
                        .iter()
                        .map(|(name, value)| (name.clone(), restored(value)))
                        .collect(),
                ),
            }
        }
        other => other.clone(),
    }
}

fn kind_name<T: Serialize>(kind: &T) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

fn author_of(author: &rows::Author, at: &dyn Addresses) -> Author {
    let blank = Author {
        kind: String::new(),
        did: None,
        display: None,
        context: None,
    };
    match author {
        rows::Author::User { did } => Author {
            kind: "user".into(),
            did: Some(did.clone()),
            ..blank
        },
        rows::Author::FreeText { display } => Author {
            kind: "free_text".into(),
            display: Some(display.clone()),
            ..blank
        },
        rows::Author::Context { context_id, .. } => Author {
            kind: "context".into(),
            context: Some(at.space(context_id).to_string()),
            ..blank
        },
    }
}

fn author_back(author: &Author) -> rows::Author {
    match (author.kind.as_str(), &author.did, &author.context) {
        ("user", Some(did), _) => rows::Author::User { did: did.clone() },
        ("context", _, Some(space)) => rows::Author::Context {
            context_id: space
                .parse::<SpaceUri>()
                .map(|s| s.skey)
                .unwrap_or_default(),
            name: None,
            path: None,
        },
        _ => rows::Author::FreeText {
            display: author.display.clone().unwrap_or_default(),
        },
    }
}

fn binned_of(place_deleted_at: &Option<String>, root: Option<String>) -> Option<Binned> {
    place_deleted_at.as_ref().map(|at| Binned {
        at: at.clone(),
        root,
    })
}

impl Node {
    /// The record for a document's row.
    pub fn of(doc: &rows::Document, at: &dyn Addresses) -> Node {
        let space = at.space(&doc.context_id);
        let node_uri = |id: &str| space.record(&at.holder(id), NODE, id).to_string();
        let place = &doc.place;
        Node {
            kind: kind_name(&doc.kind),
            title: doc.title.clone(),
            slug: place.slug.clone(),
            // Directly under its context is the absence of a parent.
            parent: place
                .parent_id
                .as_deref()
                .filter(|parent| *parent != doc.context_id)
                .map(node_uri),
            index: Some(place.idx),
            draft: Some(doc.mutable),
            locked: Some(!place.attachable),
            content_format: doc.content.as_ref().map(|_| SLATE.to_string()),
            content: doc.content.as_ref().map(storable),
            image: None,
            file: None,
            data: doc.data.as_ref().map(storable),
            authors: doc.authors.iter().map(|a| author_of(a, at)).collect(),
            made_by: place.owner_did.clone(),
            created_at: place.created_at.clone().unwrap_or_else(|| NO_DATE.into()),
            updated_at: place.updated_at.clone(),
            binned: binned_of(
                &place.deleted_at,
                place.deleted_root.as_deref().map(node_uri),
            ),
            legacy_id: doc.legacy_id.clone(),
        }
    }

    /// The row for a record found at `uri`, under the path its parents give it.
    pub fn row(&self, uri: &RecordUri, parent_path: &str, kept: Kept) -> Option<rows::Document> {
        let kind: rows::DocumentKind =
            serde_json::from_value(serde_json::Value::String(self.kind.clone())).ok()?;
        let context_id = uri.space.skey.clone();
        let rkey_of = |at_uri: &str| at_uri.parse::<RecordUri>().ok().map(|u| u.rkey);
        Some(rows::Document {
            id: uri.rkey.clone(),
            kind,
            title: self.title.clone(),
            place: rows::Place {
                slug: self.slug.clone(),
                path: join(parent_path, &self.slug),
                parent_id: Some(match &self.parent {
                    Some(parent) => rkey_of(parent)?,
                    None => context_id.clone(),
                }),
                idx: self.index.unwrap_or(0),
                attachable: !self.locked.unwrap_or(false),
                owner_did: self.made_by.clone(),
                created_at: Some(self.created_at.clone()).filter(|at| at != NO_DATE),
                updated_at: self.updated_at.clone(),
                deleted_at: self.binned.as_ref().map(|b| b.at.clone()),
                deleted_root: self
                    .binned
                    .as_ref()
                    .and_then(|b| b.root.as_deref())
                    .and_then(rkey_of),
            },
            context_id,
            mutable: self.draft.unwrap_or(false),
            content: self.content.as_ref().map(restored),
            data: self.data.as_ref().map(restored),
            authors: self.authors.iter().map(author_back).collect(),
            visibility: kept.visibility,
            published_uri: kept.published_uri,
            legacy_id: self.legacy_id.clone(),
        })
    }
}

/// Where a context hangs, which its row says by one id and its record by two
/// addresses: a group can sit directly in a context or in one of its folders.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hanging {
    /// The id of the context it is in. `None` for the home.
    pub context_id: Option<String>,
    /// The folder of that context it sits in, if it sits in one.
    pub folder_id: Option<String>,
}

impl ContextProfile {
    pub fn of(ctx: &rows::Context, hanging: &Hanging, at: &dyn Addresses) -> ContextProfile {
        let place = &ctx.place;
        let parent_space = hanging.context_id.as_deref().map(|id| at.space(id));
        ContextProfile {
            kind: kind_name(&ctx.kind),
            name: ctx.name.clone(),
            slug: place.slug.clone(),
            parent_node: match (&parent_space, &hanging.folder_id) {
                (Some(space), Some(folder)) => {
                    Some(space.record(&at.holder(folder), NODE, folder).to_string())
                }
                _ => None,
            },
            parent_space: parent_space.map(|s| s.to_string()),
            index: Some(place.idx),
            locked: Some(!place.attachable),
            content_format: ctx.content.as_ref().map(|_| SLATE.to_string()),
            content: ctx.content.as_ref().map(storable),
            image: None,
            data: ctx.data.as_ref().map(storable),
            made_by: place.owner_did.clone(),
            created_at: place.created_at.clone().unwrap_or_else(|| NO_DATE.into()),
            updated_at: place.updated_at.clone(),
            binned: binned_of(
                &place.deleted_at,
                place
                    .deleted_root
                    .as_deref()
                    .map(|root| at.space(root).to_string()),
            ),
            legacy_id: ctx.legacy_id.clone(),
        }
    }

    /// The row for the profile of `space`, under the path its parents give it.
    pub fn row(&self, space: &SpaceUri, parent_path: &str, kept: Kept) -> Option<rows::Context> {
        let kind: rows::ContextKind =
            serde_json::from_value(serde_json::Value::String(self.kind.clone())).ok()?;
        let parent_id = match (&self.parent_node, &self.parent_space) {
            (Some(node), _) => Some(node.parse::<RecordUri>().ok()?.rkey),
            (None, Some(parent)) => Some(parent.parse::<SpaceUri>().ok()?.skey),
            (None, None) => None,
        };
        Some(rows::Context {
            id: space.skey.clone(),
            kind,
            name: self.name.clone(),
            place: rows::Place {
                slug: self.slug.clone(),
                path: join(parent_path, &self.slug),
                parent_id,
                idx: self.index.unwrap_or(0),
                attachable: !self.locked.unwrap_or(false),
                owner_did: self.made_by.clone(),
                created_at: Some(self.created_at.clone()).filter(|at| at != NO_DATE),
                updated_at: self.updated_at.clone(),
                deleted_at: self.binned.as_ref().map(|b| b.at.clone()),
                deleted_root: self
                    .binned
                    .as_ref()
                    .and_then(|b| b.root.as_deref())
                    .and_then(|root| root.parse::<SpaceUri>().ok())
                    .map(|s| s.skey),
            },
            content: self.content.as_ref().map(restored),
            data: self.data.as_ref().map(restored),
            visibility: kept.visibility,
            published_uri: kept.published_uri,
            legacy_id: self.legacy_id.clone(),
        })
    }
}

impl Comment {
    /// The record for a comment's row, or `None` while what it hangs on has no
    /// record to point at: a strong reference pins a version.
    pub fn of(row: &rows::Comment, at: &dyn Addresses) -> Option<Comment> {
        let space = at.space(&row.context_id);
        let pin = |collection: &str, id: &str| {
            Some(StrongRef {
                uri: space.record(&at.holder(id), collection, id).to_string(),
                cid: at.cid(collection, id)?,
            })
        };
        let root = if row.root_id.is_empty() {
            &row.on_id
        } else {
            &row.root_id
        };
        Some(Comment {
            subject: pin(NODE, root)?,
            parent: match row.on_id == *root {
                true => None,
                false => Some(pin(COMMENT, &row.on_id)?),
            },
            text: row.text.clone(),
            image: None,
            author: Some(author_of(&row.author, at)),
            tombstone: Some(row.tombstone).filter(|t| *t),
            binned: binned_of(
                &row.deleted_at,
                row.deleted_root
                    .as_deref()
                    .map(|id| space.record(&at.holder(id), COMMENT, id).to_string()),
            ),
            legacy_id: row.legacy_id.clone(),
            created_at: row.created_at.clone().unwrap_or_else(|| NO_DATE.into()),
        })
    }

    /// The row for a record found at `uri`. `image` is the row's own file id,
    /// which the caller knows from the blob and the record does not.
    pub fn row(&self, uri: &RecordUri, image: Option<String>) -> Option<rows::Comment> {
        let root = self.subject.uri.parse::<RecordUri>().ok()?.rkey;
        let on = match &self.parent {
            Some(parent) => parent.uri.parse::<RecordUri>().ok()?.rkey,
            None => root.clone(),
        };
        Some(rows::Comment {
            id: uri.rkey.clone(),
            on_id: on,
            root_id: root,
            context_id: uri.space.skey.clone(),
            // In its author's own repository the repository says who.
            author: match &self.author {
                Some(author) => author_back(author),
                None => rows::Author::User {
                    did: uri.author.clone(),
                },
            },
            text: self.text.clone(),
            image,
            tombstone: self.tombstone.unwrap_or(false),
            created_at: Some(self.created_at.clone()).filter(|at| at != NO_DATE),
            deleted_at: self.binned.as_ref().map(|b| b.at.clone()),
            deleted_root: self
                .binned
                .as_ref()
                .and_then(|b| b.root.as_deref())
                .and_then(|root| root.parse::<RecordUri>().ok())
                .map(|u| u.rkey),
            legacy_id: self.legacy_id.clone(),
        })
    }
}

impl Reaction {
    /// The record for a reaction's row, or `None` while what it is given to has
    /// no record to point at. A reaction is given to a node or to a comment,
    /// and the row does not say which: whichever has a record is it.
    pub fn of(row: &rows::Reaction, context_id: &str, at: &dyn Addresses) -> Option<Reaction> {
        let space = at.space(context_id);
        let id = &row.subject_uri;
        let (collection, cid) = [NODE, COMMENT]
            .into_iter()
            .find_map(|collection| Some((collection, at.cid(collection, id)?)))?;
        let blank = Author {
            kind: "user".into(),
            did: None,
            display: None,
            context: None,
        };
        Some(Reaction {
            subject: StrongRef {
                uri: space.record(&at.holder(id), collection, id).to_string(),
                cid,
            },
            emoji: row.emoji.clone(),
            author: row.reactor_did.clone().map(|did| Author {
                did: Some(did),
                ..blank
            }),
            legacy_id: row.legacy_id.clone(),
            created_at: row.created_at.clone().unwrap_or_else(|| NO_DATE.into()),
        })
    }

    pub fn row(&self, uri: &RecordUri) -> Option<rows::Reaction> {
        // Held by the organization with nobody named: someone whose account
        // did not come across. In anyone else's repository, the repository's.
        let held_by_the_organization = uri.author == uri.space.authority;
        Some(rows::Reaction {
            id: uri.rkey.clone(),
            subject_uri: self.subject.uri.parse::<RecordUri>().ok()?.rkey,
            reactor_did: match (&self.author, held_by_the_organization) {
                (Some(author), _) => author.did.clone(),
                (None, true) => None,
                (None, false) => Some(uri.author.clone()),
            },
            emoji: self.emoji.clone(),
            created_at: Some(self.created_at.clone()).filter(|at| at != NO_DATE),
            legacy_id: self.legacy_id.clone(),
        })
    }
}

fn join(parent_path: &str, slug: &str) -> String {
    match (parent_path.is_empty(), slug.is_empty()) {
        (_, true) => parent_path.to_string(),
        (true, false) => slug.to_string(),
        (false, false) => format!("{parent_path}/{slug}"),
    }
}

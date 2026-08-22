//! Server-side listing: `sh.tangled.repo.listRepos`, `listIssues`,
//! `listPulls` and `getRepo`.
//!
//! These replace scanning a PDS with `com.atproto.repo.listRecords`. Scanning
//! only ever sees records on one PDS, stops at whatever limit is asked for,
//! and fails whole if a single record does not deserialise. The appview
//! answers with an index: it paginates, it carries derived fields a record
//! does not have (an issue's state and comment count), and it is the same
//! view the website shows.
//!
//! Reading is public, so none of this needs a token.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::client::TangledClient;

/// Tangled's own appview. `tg` defaults to a third-party one
/// (bobbin.klbr.net), which is a reminder that this is configurable
/// infrastructure rather than a fixed address.
pub const DEFAULT_APPVIEW: &str = "https://api.tangled.org";

pub const LIST_REPOS: &str = "sh.tangled.repo.listRepos";
pub const LIST_ISSUES: &str = "sh.tangled.repo.listIssues";
pub const LIST_PULLS: &str = "sh.tangled.repo.listPulls";
pub const GET_REPO: &str = "sh.tangled.repo.getRepo";

/// The appview base, honouring an override.
pub fn appview_base() -> String {
    std::env::var("TANGLED_APPVIEW_BASE").unwrap_or_else(|_| DEFAULT_APPVIEW.to_string())
}

/// One row of a listing: the record, plus whatever the index derived.
#[derive(Debug, Clone, Deserialize)]
pub struct ListItem<T> {
    pub uri: String,
    #[serde(default)]
    pub cid: Option<String>,
    pub value: T,
    /// Present on issues and pulls. A pull reports `merged` here as well as
    /// `open` and `closed`.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(rename = "commentCount", default)]
    pub comment_count: i64,
    #[serde(rename = "stateUpdatedAt", default)]
    pub state_updated_at: Option<String>,
}

impl<T> ListItem<T> {
    /// The record key, which for a repo is its name.
    pub fn rkey(&self) -> &str {
        self.uri.rsplit('/').next().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    items: Vec<ListItem<T>>,
    #[serde(default)]
    cursor: Option<String>,
}

/// A repo as the appview reports it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoValue {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub knot: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub spindle: Option<String>,
    #[serde(rename = "repoDid", default)]
    pub repo_did: Option<String>,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IssueValue {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PullValue {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
}

impl TangledClient {
    /// Every page of a listing, following cursors. `limit` is the page size;
    /// `max` caps the total so a huge account cannot hang a terminal.
    async fn paged<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        base_params: &[(&str, String)],
        max: usize,
    ) -> Result<Vec<ListItem<T>>> {
        let mut out: Vec<ListItem<T>> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params: Vec<(&str, String)> = base_params.to_vec();
            params.push(("limit", "100".to_string()));
            if let Some(c) = &cursor {
                params.push(("cursor", c.clone()));
            }
            let page: Page<T> = self.get_json(method, &params, None).await?;
            let empty = page.items.is_empty();
            out.extend(page.items);
            if out.len() >= max || empty {
                out.truncate(max);
                return Ok(out);
            }
            match page.cursor {
                // A cursor that does not move would loop forever.
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => return Ok(out),
            }
        }
    }

    /// Repos owned by an account. `subject` is the owner's DID.
    pub async fn list_repos_indexed(
        &self,
        owner_did: &str,
        max: usize,
    ) -> Result<Vec<ListItem<RepoValue>>> {
        self.paged(LIST_REPOS, &[("subject", owner_did.to_string())], max)
            .await
    }

    /// One repo, addressed by its at-uri (not by its DID: the endpoint
    /// rejects a bare DID here, which is the opposite of the listings below).
    pub async fn get_repo_indexed(&self, at_uri: &str) -> Result<ListItem<RepoValue>> {
        self.get_json(GET_REPO, &[("repo", at_uri.to_string())], None)
            .await
    }

    /// Issues in a repo. `subject` must be the repo's own DID, bare.
    pub async fn list_issues_indexed(
        &self,
        repo_did: &str,
        state: Option<&str>,
        max: usize,
    ) -> Result<Vec<ListItem<IssueValue>>> {
        let mut params = vec![("subject", repo_did.to_string())];
        if let Some(s) = state {
            params.push(("state", s.to_string()));
        }
        self.paged(LIST_ISSUES, &params, max).await
    }

    /// Pulls in a repo. `subject` must be the repo's own DID, bare.
    pub async fn list_pulls_indexed(
        &self,
        repo_did: &str,
        status: Option<&str>,
        max: usize,
    ) -> Result<Vec<ListItem<PullValue>>> {
        let mut params = vec![("subject", repo_did.to_string())];
        if let Some(s) = status {
            params.push(("status", s.to_string()));
        }
        self.paged(LIST_PULLS, &params, max).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rkey_is_the_last_uri_segment() {
        let item: ListItem<RepoValue> = serde_json::from_str(
            r#"{"uri":"at://did-plc-abc/sh.tangled.repo/oxidized-awk","value":{"name":"oxidized-awk"}}"#,
        )
        .unwrap();
        assert_eq!(item.rkey(), "oxidized-awk");
    }

    #[test]
    fn a_record_without_a_name_still_decodes() {
        // `cider` predates the name field; scanning used to fail the whole
        // response on exactly this record.
        let item: ListItem<RepoValue> = serde_json::from_str(
            r#"{"uri":"at://did-plc-abc/sh.tangled.repo/cider","value":{"knot":"knot1.tangled.sh"}}"#,
        )
        .unwrap();
        assert!(item.value.name.is_none());
        assert_eq!(item.rkey(), "cider");
    }

    #[test]
    fn a_pull_reports_merged_as_a_state() {
        let item: ListItem<PullValue> = serde_json::from_str(
            r#"{"uri":"at://x/y/z","value":{"title":"t"},"state":"merged","commentCount":3}"#,
        )
        .unwrap();
        assert_eq!(item.state.as_deref(), Some("merged"));
        assert_eq!(item.comment_count, 3);
    }
}

/// Full-text search across indexed records: `sh.tangled.search.query`.
pub const SEARCH_QUERY: &str = "sh.tangled.search.query";

/// One search hit. `value` stays raw because a hit can be any record type,
/// and the nsid says which.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchHit {
    pub uri: String,
    #[serde(default)]
    pub cid: Option<String>,
    #[serde(default)]
    pub nsid: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResult {
    #[serde(default = "Vec::new")]
    pub hits: Vec<SearchHit>,
    #[serde(default)]
    pub cursor: Option<String>,
}

impl SearchHit {
    /// A one-line label for a hit, whatever kind of record it is.
    pub fn title(&self) -> String {
        for key in ["name", "title", "description"] {
            if let Some(v) = self.value.get(key).and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    return v.chars().take(72).collect();
                }
            }
        }
        // Nothing human-readable: the record key is at least an identifier.
        self.uri.rsplit('/').next().unwrap_or("").to_string()
    }

    /// The record type without the `sh.tangled.` prefix, for display.
    pub fn kind(&self) -> &str {
        self.nsid.strip_prefix("sh.tangled.").unwrap_or(&self.nsid)
    }
}

impl TangledClient {
    pub async fn search(&self, query: &str, limit: usize) -> Result<SearchResult> {
        let params = [("q", query.to_string()), ("limit", limit.to_string())];
        self.get_json(SEARCH_QUERY, &params, None).await
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn labels_a_hit_by_whatever_field_it_has() {
        let repo: SearchHit = serde_json::from_str(
            r#"{"uri":"x/y/z","nsid":"sh.tangled.repo","value":{"name":"oxidized-awk"}}"#,
        )
        .unwrap();
        assert_eq!(repo.title(), "oxidized-awk");
        assert_eq!(repo.kind(), "repo");

        let issue: SearchHit = serde_json::from_str(
            r#"{"uri":"x/y/z","nsid":"sh.tangled.repo.issue","value":{"title":"a bug"}}"#,
        )
        .unwrap();
        assert_eq!(issue.title(), "a bug");
    }

    #[test]
    fn falls_back_to_the_record_key() {
        let hit: SearchHit =
            serde_json::from_str(r#"{"uri":"at://x/coll/3abc","nsid":"other","value":{}}"#)
                .unwrap();
        assert_eq!(hit.title(), "3abc");
    }
}

/// How many accounts have starred something: `sh.tangled.feed.countStars`.
pub const COUNT_STARS: &str = "sh.tangled.feed.countStars";

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StarCount {
    #[serde(default)]
    pub count: i64,
    #[serde(rename = "distinctAuthors", default)]
    pub distinct_authors: i64,
}

impl TangledClient {
    /// Star count for a repo, keyed by the repo's own DID.
    pub async fn count_stars(&self, repo_did: &str) -> Result<StarCount> {
        self.get_json(COUNT_STARS, &[("subject", repo_did.to_string())], None)
            .await
    }
}

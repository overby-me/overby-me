use std::io::Write;

use anyhow::{anyhow, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tangled_config::session::Session;

use crate::oauth::PersistedOAuthSession;

/// Gzip-compress a byte slice.
fn gzip_bytes(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

#[derive(Clone, Debug)]
pub struct TangledClient {
    base_url: String,
    oauth: Option<PersistedOAuthSession>,
}

const REPO_CREATE: &str = "sh.tangled.repo.create";
const FEED_STAR: &str = "sh.tangled.feed.star";
const FEED_STAR_REPO: &str = "sh.tangled.feed.star#repo";
/// Pull status. Renamed from `...pull.state`, and it gained `merged`.
pub const PULL_STATUS: &str = "sh.tangled.repo.pull.status";
pub const PULL_STATUS_OPEN: &str = "sh.tangled.repo.pull.status.open";
pub const PULL_STATUS_CLOSED: &str = "sh.tangled.repo.pull.status.closed";
pub const PULL_STATUS_MERGED: &str = "sh.tangled.repo.pull.status.merged";

impl Default for TangledClient {
    fn default() -> Self {
        Self::new("https://tngl.sh")
    }
}

impl TangledClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            oauth: None,
        }
    }

    pub fn with_oauth(mut self, oauth: PersistedOAuthSession) -> Self {
        self.oauth = Some(oauth);
        self
    }

    /// Create a new client with a different base URL but the same OAuth context.
    pub(crate) fn derive(&self, base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            oauth: self.oauth.clone(),
        }
    }

    fn xrpc_url(&self, method: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        // Add https:// if no protocol is present
        let base_with_protocol = if base.starts_with("http://") || base.starts_with("https://") {
            base.to_string()
        } else {
            format!("https://{}", base)
        };
        format!("{}/xrpc/{}", base_with_protocol, method)
    }

    /// Use OAuth DPoP auth only when no explicit bearer token is provided.
    /// When a bearer token is given (e.g. service auth tokens), use it as-is.
    /// Treats empty bearer strings as absent.
    fn should_use_oauth(&self, bearer: Option<&str>) -> Option<&PersistedOAuthSession> {
        if bearer.is_some_and(|b| !b.is_empty()) {
            return None;
        }
        self.oauth.as_ref()
    }

    async fn post_json<TReq: Serialize, TRes: DeserializeOwned>(
        &self,
        method: &str,
        req: &TReq,
        bearer: Option<&str>,
    ) -> Result<TRes> {
        let url = self.xrpc_url(method);
        if let Some(oauth) = self.should_use_oauth(bearer) {
            let json_body = serde_json::to_vec(req)?;
            let body = crate::oauth::oauth_post(oauth, &url, &json_body).await?;
            return serde_json::from_slice(&body).map_err(|e| {
                let snippet: String = String::from_utf8_lossy(&body).chars().take(300).collect();
                anyhow!(
                    "error decoding response from {}: {}\nBody: {}",
                    url,
                    e,
                    snippet
                )
            });
        }
        let client = reqwest::Client::new();
        let mut reqb = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        if let Some(token) = bearer {
            reqb = reqb.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token));
        }
        let res = reqb.json(req).send().await?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("{}: {}", status, body));
        }
        Ok(res.json::<TRes>().await?)
    }

    pub(crate) async fn post<TReq: Serialize>(
        &self,
        method: &str,
        req: &TReq,
        bearer: Option<&str>,
    ) -> Result<()> {
        let url = self.xrpc_url(method);
        if let Some(oauth) = self.should_use_oauth(bearer) {
            let json_body = serde_json::to_vec(req)?;
            crate::oauth::oauth_post(oauth, &url, &json_body).await?;
            return Ok(());
        }
        let client = reqwest::Client::new();
        let mut reqb = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        if let Some(token) = bearer {
            reqb = reqb.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token));
        }
        let res = reqb.json(req).send().await?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("{}: {}", status, body));
        }
        Ok(())
    }

    /// Upload a blob to the PDS via com.atproto.repo.uploadBlob.
    /// Returns the blob JSON value (with $type, ref, mimeType, size).
    pub async fn upload_blob(
        &self,
        data: &[u8],
        mime_type: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<serde_json::Value> {
        let pds_client = self.derive(pds_base);
        let url = pds_client.xrpc_url("com.atproto.repo.uploadBlob");

        if let Some(oauth) = pds_client.should_use_oauth(Some(access_jwt)) {
            let body = crate::oauth::oauth_post_raw(oauth, &url, data, mime_type).await?;
            let res: serde_json::Value = serde_json::from_slice(&body)?;
            return Ok(res["blob"].clone());
        }

        let client = reqwest::Client::new();
        let res = client
            .post(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", access_jwt),
            )
            .header(reqwest::header::CONTENT_TYPE, mime_type)
            .body(data.to_vec())
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("{}: {}", status, body));
        }
        let res: serde_json::Value = res.json().await?;
        Ok(res["blob"].clone())
    }

    pub async fn get_json<TRes: DeserializeOwned>(
        &self,
        method: &str,
        params: &[(&str, String)],
        bearer: Option<&str>,
    ) -> Result<TRes> {
        let url = self.xrpc_url(method);
        if let Some(oauth) = self.should_use_oauth(bearer) {
            // Build full URL with query params
            let mut full_url = reqwest::Url::parse(&url)?;
            for (k, v) in params {
                full_url.query_pairs_mut().append_pair(k, v);
            }
            let body = crate::oauth::oauth_get(oauth, full_url.as_str()).await?;
            return serde_json::from_slice(&body).map_err(|e| {
                let snippet: String = String::from_utf8_lossy(&body).chars().take(300).collect();
                anyhow!(
                    "error decoding response from {}: {}\nBody (first 300 chars): {}",
                    url,
                    e,
                    snippet
                )
            });
        }
        let client = reqwest::Client::new();
        let mut reqb = client
            .get(&url)
            .query(&params)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(token) = bearer {
            reqb = reqb.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token));
        }
        let res = reqb.send().await?;
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("GET {} -> {}: {}", url, status, body));
        }
        serde_json::from_str::<TRes>(&body).map_err(|e| {
            let snippet = body.chars().take(300).collect::<String>();
            anyhow!(
                "error decoding response from {}: {}\nBody (first 300 chars): {}",
                url,
                e,
                snippet
            )
        })
    }

    pub async fn post_json_pub<TReq: Serialize, TRes: DeserializeOwned>(
        &self,
        method: &str,
        req: &TReq,
        bearer: Option<&str>,
    ) -> Result<TRes> {
        self.post_json(method, req, bearer).await
    }

    pub async fn login_with_password(
        &self,
        handle: &str,
        password: &str,
        _pds: &str,
    ) -> Result<Session> {
        #[derive(Serialize)]
        struct Req<'a> {
            #[serde(rename = "identifier")]
            identifier: &'a str,
            #[serde(rename = "password")]
            password: &'a str,
        }
        #[derive(Deserialize)]
        struct Res {
            #[serde(rename = "accessJwt")]
            access_jwt: String,
            #[serde(rename = "refreshJwt")]
            refresh_jwt: String,
            did: String,
            handle: String,
        }
        let body = Req {
            identifier: handle,
            password,
        };
        let res: Res = self
            .post_json("com.atproto.server.createSession", &body, None)
            .await?;
        Ok(Session {
            access_jwt: res.access_jwt,
            refresh_jwt: res.refresh_jwt,
            did: res.did,
            handle: res.handle,
            ..Default::default()
        })
    }

    pub async fn refresh_session(&self, refresh_jwt: &str) -> Result<Session> {
        #[derive(Deserialize)]
        struct Res {
            #[serde(rename = "accessJwt")]
            access_jwt: String,
            #[serde(rename = "refreshJwt")]
            refresh_jwt: String,
            did: String,
            handle: String,
        }
        let url = self.xrpc_url("com.atproto.server.refreshSession");
        let client = reqwest::Client::new();
        let res = client
            .post(url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", refresh_jwt),
            )
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("{}: {}", status, body));
        }
        let res_data: Res = res.json().await?;
        Ok(Session {
            access_jwt: res_data.access_jwt,
            refresh_jwt: res_data.refresh_jwt,
            did: res_data.did,
            handle: res_data.handle,
            ..Default::default()
        })
    }

    pub async fn list_repos(
        &self,
        user: Option<&str>,
        knot: Option<&str>,
        starred: bool,
        bearer: Option<&str>,
    ) -> Result<Vec<Repository>> {
        // NOTE: Repo listing is done via the user's PDS using com.atproto.repo.listRecords
        // for the collection "sh.tangled.repo". This does not go through the Tangled API base.
        // Here, `self.base_url` must be the PDS base (e.g., https://bsky.social).
        // Resolve handle to DID if needed
        let did = match user {
            Some(u) if u.starts_with("did:") => u.to_string(),
            Some(handle) => {
                #[derive(Deserialize)]
                struct Res {
                    did: String,
                }
                let params = [("handle", handle.to_string())];
                let res: Res = self
                    .get_json("com.atproto.identity.resolveHandle", &params, bearer)
                    .await?;
                res.did
            }
            None => {
                return Err(anyhow!(
                    "missing user for list_repos; provide handle or DID"
                ));
            }
        };

        #[derive(Deserialize)]
        struct RecordItem {
            uri: String,
            value: Repository,
        }
        #[derive(Deserialize)]
        struct ListRes {
            #[serde(default)]
            records: Vec<RecordItem>,
        }

        let params = vec![
            ("repo", did),
            ("collection", "sh.tangled.repo".to_string()),
            ("limit", "100".to_string()),
        ];

        let res: ListRes = self
            .get_json("com.atproto.repo.listRecords", &params, bearer)
            .await?;
        let mut repos: Vec<Repository> = res
            .records
            .into_iter()
            .map(|r| {
                let mut val = r.value;
                if val.rkey.is_none() {
                    if let Some(k) = Self::uri_rkey(&r.uri) {
                        val.rkey = Some(k);
                    }
                }
                if val.did.is_none() {
                    if let Some(d) = Self::uri_did(&r.uri) {
                        val.did = Some(d);
                    }
                }
                if val.name.is_empty() {
                    if let Some(k) = val.rkey.clone() {
                        val.name = k;
                    }
                }
                val
            })
            .collect();
        // Apply optional filters client-side
        if let Some(k) = knot {
            repos.retain(|r| r.knot.as_deref().unwrap_or("") == k);
        }
        if starred {
            // TODO: implement starred filtering when API is available. For now, no-op.
        }
        Ok(repos)
    }

    /// Register a repo on its knot, then record it on the PDS.
    pub async fn create_repo(&self, opts: CreateRepoOptions<'_>) -> Result<()> {
        let pds_client = self.derive(opts.pds_base);
        // The record key identifies the repo: sh.tangled.repo declares
        // `key: "any"` and `name` is only "Cosmetic name of the repo", so the
        // appview addresses it as <handle>/<rkey>. A PDS-assigned TID yields a
        // repo that pushes over SSH and 404s on the web.
        let rkey = opts.name;

        // 1) Service auth for the knot. `lxm` binds the token to the method:
        //    without it the knot answers `method binding mismatch`.
        #[derive(Deserialize)]
        struct GetSARes {
            token: String,
        }
        let params = [
            ("aud", format!("did:web:{}", opts.knot)),
            ("exp", (chrono::Utc::now().timestamp() + 60).to_string()),
            ("lxm", REPO_CREATE.to_string()),
        ];
        let sa: GetSARes = pds_client
            .get_json(
                "com.atproto.server.getServiceAuth",
                &params,
                Some(opts.access_jwt),
            )
            .await?;

        // 2) Create it on the knot first: it mints the repo's own DID, which
        //    the record must carry, and a failure here leaves nothing behind
        //    rather than a record for a repo that does not exist.
        #[derive(Serialize)]
        struct CreateRepoReq<'a> {
            rkey: &'a str,
            name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "defaultBranch")]
            default_branch: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            source: Option<&'a str>,
        }
        #[derive(Deserialize)]
        struct CreateRepoRes {
            #[serde(rename = "repoDid")]
            repo_did: Option<String>,
        }
        let req = CreateRepoReq {
            rkey,
            name: opts.name,
            default_branch: opts.default_branch,
            source: opts.source,
        };
        let knot_client = self.derive(format!("https://{}", opts.knot));
        let knot_res: CreateRepoRes = knot_client
            .post_json(REPO_CREATE, &req, Some(&sa.token))
            .await?;
        let repo_did = knot_res
            .repo_did
            .filter(|d| !d.is_empty())
            .ok_or_else(|| anyhow!("knot did not return a repoDid"))?;

        // 3) Record it on the PDS, at the same rkey. `knot` and `createdAt`
        //    are the only required fields; the appview resolves the repo
        //    itself through `repoDid`.
        #[derive(Serialize)]
        struct Record<'a> {
            #[serde(rename = "$type")]
            lexicon_type: &'a str,
            knot: &'a str,
            #[serde(rename = "createdAt")]
            created_at: String,
            #[serde(rename = "repoDid")]
            repo_did: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            source: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct PutRecordReq<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
            validate: bool,
            record: Record<'a>,
        }
        let put_req = PutRecordReq {
            repo: opts.did,
            collection: "sh.tangled.repo",
            rkey,
            validate: false,
            record: Record {
                lexicon_type: "sh.tangled.repo",
                knot: opts.knot,
                created_at: chrono::Utc::now().to_rfc3339(),
                repo_did: &repo_did,
                name: Some(opts.name),
                description: opts.description,
                source: opts.source_at,
            },
        };
        let _: serde_json::Value = pds_client
            .post_json(
                "com.atproto.repo.putRecord",
                &put_req,
                Some(opts.access_jwt),
            )
            .await?;
        Ok(())
    }

    /// Look up one repo by owner and name.
    ///
    /// This asks the appview, not a PDS. Scanning a PDS only works for repos
    /// whose records live on the PDS this client happens to point at, so
    /// looking up somebody else's repo failed with "Could not find repo".
    pub async fn get_repo_info(
        &self,
        owner: &str,
        name: &str,
        bearer: Option<&str>,
    ) -> Result<RepoRecord> {
        let did = self.resolve_handle(owner, bearer).await?;
        let appview = self.derive(crate::appview::appview_base());
        let repos = appview.list_repos_indexed(&did, 1000).await?;
        let found = repos
            .into_iter()
            // The record key is the repo's identity; `name` is cosmetic and
            // absent on older records, so match either.
            .find(|r| r.rkey() == name || r.value.name.as_deref() == Some(name))
            .ok_or_else(|| anyhow!("no repo {owner}/{name}"))?;
        Ok(RepoRecord {
            rkey: found.rkey().to_string(),
            did,
            name: found
                .value
                .name
                .clone()
                .unwrap_or_else(|| found.rkey().to_string()),
            knot: found.value.knot.clone().unwrap_or_default(),
            description: found.value.description.clone(),
            spindle: found.value.spindle.clone(),
            repo_did: found.value.repo_did.clone(),
        })
    }

    /// Delete a repo: the PDS record first, then the knot.
    ///
    /// The order is the knot's choice, not ours. It refuses to delete while
    /// the record still exists — "sh.tangled.repo record still exists on the
    /// owner's PDS. Remove it there first or force the delete." — and `force`
    /// is admin-only, answering everyone else with "only knot admin may force
    /// a delete past the PDS record check".
    ///
    /// The cost of that order is that a knot failure after the record is gone
    /// leaves a repo nothing can address, since every knot call is keyed on a
    /// repoDid only the record carries. Read the info before deleting
    /// anything, so at least the identifiers are in hand.
    pub async fn delete_repo(
        &self,
        did: &str,
        name: &str,
        force: bool,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<()> {
        let pds_client = self.derive(pds_base);
        let info = pds_client
            .get_repo_info(did, name, Some(access_jwt))
            .await?;
        let repo_did = info
            .repo_did
            .as_deref()
            .ok_or_else(|| anyhow!("{name} has no repoDid; it cannot be deleted from the knot"))?;

        // 1) the record, which the knot checks for
        #[derive(Serialize)]
        struct DeleteRecordReq<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
        }
        pds_client
            .post(
                "com.atproto.repo.deleteRecord",
                &DeleteRecordReq {
                    repo: did,
                    collection: "sh.tangled.repo",
                    rkey: &info.rkey,
                },
                Some(access_jwt),
            )
            .await?;

        // 2) the knot. Its DeleteInput is { repo: RepoDid, force: bool }: the
        // repo's own DID, not the owner DID plus name plus rkey. The older
        // shape fails with "missing field `repo`", and tg still sends it, so
        // tg is not a reliable oracle here; the knot's source is.
        #[derive(Serialize)]
        struct DeleteReq<'a> {
            repo: &'a str,
            force: bool,
        }
        let sa = self
            .knot_push_token(
                pds_base,
                access_jwt,
                &info.knot,
                "sh.tangled.repo.delete",
                240,
            )
            .await?;
        self.derive(format!("https://{}", info.knot))
            .post(
                "sh.tangled.repo.delete",
                &DeleteReq {
                    repo: repo_did,
                    force,
                },
                Some(&sa),
            )
            .await?;
        Ok(())
    }

    pub async fn update_repo_knot(
        &self,
        did: &str,
        rkey: &str,
        new_knot: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<()> {
        let pds_client = self.derive(pds_base);
        #[derive(Deserialize, Serialize, Clone)]
        struct Rec {
            // Absent on older records, where the record key is the name.
            // Optional both ways, so a record without one does not gain an
            // empty `name` when it is written back.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            name: Option<String>,
            knot: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Deserialize)]
        struct GetRes {
            value: Rec,
        }
        let params = [
            ("repo", did.to_string()),
            ("collection", "sh.tangled.repo".to_string()),
            ("rkey", rkey.to_string()),
        ];
        let got: GetRes = pds_client
            .get_json("com.atproto.repo.getRecord", &params, Some(access_jwt))
            .await?;
        let mut rec = got.value;
        rec.knot = new_knot.to_string();
        #[derive(Serialize)]
        struct PutReq<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
            validate: bool,
            record: Rec,
        }
        let req = PutReq {
            repo: did,
            collection: "sh.tangled.repo",
            rkey,
            validate: false,
            record: rec,
        };
        let _: serde_json::Value = pds_client
            .post_json("com.atproto.repo.putRecord", &req, Some(access_jwt))
            .await?;
        Ok(())
    }

    pub async fn edit_repo(
        &self,
        did: &str,
        rkey: &str,
        description: Option<&str>,
        private: Option<bool>,
        bearer: Option<&str>,
    ) -> Result<()> {
        #[derive(Deserialize, Serialize, Clone)]
        struct Rec {
            // Absent on older records; see update_repo_knot.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            name: Option<String>,
            knot: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            spindle: Option<String>,
            #[serde(default)]
            private: bool,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Deserialize)]
        struct GetRes {
            value: Rec,
        }
        let params = [
            ("repo", did.to_string()),
            ("collection", "sh.tangled.repo".to_string()),
            ("rkey", rkey.to_string()),
        ];
        let got: GetRes = self
            .get_json("com.atproto.repo.getRecord", &params, bearer)
            .await?;
        let mut rec = got.value;
        if let Some(desc) = description {
            rec.description = Some(desc.to_string());
        }
        if let Some(priv_flag) = private {
            rec.private = priv_flag;
        }
        #[derive(Serialize)]
        struct PutReq<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
            validate: bool,
            record: Rec,
        }
        let req = PutReq {
            repo: did,
            collection: "sh.tangled.repo",
            rkey,
            validate: false,
            record: rec,
        };
        let _: serde_json::Value = self
            .post_json("com.atproto.repo.putRecord", &req, bearer)
            .await?;
        Ok(())
    }

    pub async fn get_default_branch(
        &self,
        knot_host: &str,
        did: &str,
        name: &str,
    ) -> Result<DefaultBranch> {
        #[derive(Deserialize)]
        struct Res {
            name: String,
            hash: String,
            #[serde(rename = "shortHash")]
            short_hash: Option<String>,
            when: String,
            message: Option<String>,
        }
        let knot_client = self.derive(knot_host);
        let repo_param = format!("{}/{}", did, name);
        let params = [("repo", repo_param)];
        let res: Res = knot_client
            .get_json("sh.tangled.repo.getDefaultBranch", &params, None)
            .await?;
        Ok(DefaultBranch {
            name: res.name,
            hash: res.hash,
            short_hash: res.short_hash,
            when: res.when,
            message: res.message,
        })
    }

    pub async fn get_languages(&self, knot_host: &str, did: &str, name: &str) -> Result<Languages> {
        let knot_client = self.derive(knot_host);
        let repo_param = format!("{}/{}", did, name);
        let params = [("repo", repo_param)];
        let res: serde_json::Value = knot_client
            .get_json("sh.tangled.repo.languages", &params, None)
            .await?;
        let langs = res
            .get("languages")
            .cloned()
            .unwrap_or(serde_json::json!([]));
        let languages: Vec<Language> = serde_json::from_value(langs)?;
        let total_size = res.get("totalSize").and_then(|v| v.as_u64());
        let total_files = res.get("totalFiles").and_then(|v| v.as_u64());
        Ok(Languages {
            languages,
            total_size,
            total_files,
        })
    }

    pub async fn star_repo(
        &self,
        pds_base: &str,
        access_jwt: &str,
        repo_did: &str,
        user_did: &str,
    ) -> Result<String> {
        // A star's subject is an object, not an at-uri: it names the repo's
        // own DID and tags itself with the #repo variant. Written flat, the
        // record is stored but never counted.
        #[derive(Serialize)]
        struct Subject<'a> {
            #[serde(rename = "$type")]
            lexicon_type: &'a str,
            did: &'a str,
        }
        #[derive(Serialize)]
        struct Rec<'a> {
            #[serde(rename = "$type")]
            lexicon_type: &'a str,
            subject: Subject<'a>,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            validate: bool,
            record: Rec<'a>,
        }
        #[derive(Deserialize)]
        struct Res {
            uri: String,
        }
        let now = chrono::Utc::now().to_rfc3339();
        let rec = Rec {
            lexicon_type: FEED_STAR,
            subject: Subject {
                lexicon_type: FEED_STAR_REPO,
                did: repo_did,
            },
            created_at: now,
        };
        let req = Req {
            repo: user_did,
            collection: FEED_STAR,
            validate: false,
            record: rec,
        };
        let pds_client = self.derive(pds_base);
        let res: Res = pds_client
            .post_json("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        let rkey = Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in star uri"))?;
        Ok(rkey)
    }

    pub async fn unstar_repo(
        &self,
        pds_base: &str,
        access_jwt: &str,
        repo_did: &str,
        legacy_at: &str,
        user_did: &str,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Item {
            uri: String,
            value: StarRecord,
        }
        #[derive(Deserialize)]
        struct ListRes {
            #[serde(default)]
            records: Vec<Item>,
        }
        let pds_client = self.derive(pds_base);
        let params = vec![
            ("repo", user_did.to_string()),
            ("collection", FEED_STAR.to_string()),
            ("limit", "100".to_string()),
        ];
        let res: ListRes = pds_client
            .get_json("com.atproto.repo.listRecords", &params, Some(access_jwt))
            .await?;
        let mut rkey = None;
        for item in res.records {
            let matches = item.value.subject.did() == Some(repo_did)
                || item.value.subject.legacy_uri() == Some(legacy_at);
            if matches {
                rkey = Self::uri_rkey(&item.uri);
                if rkey.is_some() {
                    break;
                }
            }
        }
        let rkey = rkey.ok_or_else(|| anyhow!("star record not found"))?;
        #[derive(Serialize)]
        struct Del<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
        }
        let del = Del {
            repo: user_did,
            collection: FEED_STAR,
            rkey: &rkey,
        };
        let _: serde_json::Value = pds_client
            .post_json("com.atproto.repo.deleteRecord", &del, Some(access_jwt))
            .await?;
        Ok(())
    }

    pub(crate) fn uri_rkey(uri: &str) -> Option<String> {
        uri.rsplit('/').next().map(|s| s.to_string())
    }
    fn uri_did(uri: &str) -> Option<String> {
        let parts: Vec<&str> = uri.split('/').collect();
        if parts.len() >= 3 {
            Some(parts[2].to_string())
        } else {
            None
        }
    }

    // ========== Issues ==========
    pub async fn list_issues(
        &self,
        author_did: &str,
        repo_at_uri: Option<&str>,
        bearer: Option<&str>,
    ) -> Result<Vec<IssueRecord>> {
        #[derive(Deserialize)]
        struct Item {
            uri: String,
            value: Issue,
        }
        #[derive(Deserialize)]
        struct ListRes {
            #[serde(default)]
            records: Vec<Item>,
        }
        let params = vec![
            ("repo", author_did.to_string()),
            ("collection", "sh.tangled.repo.issue".to_string()),
            ("limit", "100".to_string()),
        ];
        let res: ListRes = self
            .get_json("com.atproto.repo.listRecords", &params, bearer)
            .await?;
        let mut out = vec![];
        for it in res.records {
            if let Some(filter_repo) = repo_at_uri {
                if it.value.repo.as_str() != filter_repo {
                    continue;
                }
            }
            let rkey = Self::uri_rkey(&it.uri).unwrap_or_default();
            out.push(IssueRecord {
                author_did: author_did.to_string(),
                rkey,
                issue: it.value,
            });
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_issue(
        &self,
        author_did: &str,
        repo_did: &str,
        repo_rkey: &str,
        title: &str,
        body: Option<&str>,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Rec<'a> {
            repo: &'a str,
            title: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            body: Option<&'a str>,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            validate: bool,
            record: Rec<'a>,
        }
        #[derive(Deserialize)]
        struct Res {
            uri: String,
        }
        let issue_repo_at = format!("at://{}/sh.tangled.repo/{}", repo_did, repo_rkey);
        let now = chrono::Utc::now().to_rfc3339();
        let rec = Rec {
            repo: &issue_repo_at,
            title,
            body,
            created_at: now,
        };
        let req = Req {
            repo: author_did,
            collection: "sh.tangled.repo.issue",
            validate: false,
            record: rec,
        };
        let pds_client = self.derive(pds_base);
        let res: Res = pds_client
            .post_json("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in issue uri"))
    }

    pub async fn comment_issue(
        &self,
        author_did: &str,
        issue_at: &str,
        body: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Rec<'a> {
            issue: &'a str,
            body: &'a str,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            validate: bool,
            record: Rec<'a>,
        }
        #[derive(Deserialize)]
        struct Res {
            uri: String,
        }
        let now = chrono::Utc::now().to_rfc3339();
        let rec = Rec {
            issue: issue_at,
            body,
            created_at: now,
        };
        let req = Req {
            repo: author_did,
            collection: "sh.tangled.repo.issue.comment",
            validate: false,
            record: rec,
        };
        let pds_client = self.derive(pds_base);
        let res: Res = pds_client
            .post_json("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in issue comment uri"))
    }

    pub async fn get_issue_record(
        &self,
        author_did: &str,
        rkey: &str,
        bearer: Option<&str>,
    ) -> Result<Issue> {
        #[derive(Deserialize)]
        struct GetRes {
            value: Issue,
        }
        let params = [
            ("repo", author_did.to_string()),
            ("collection", "sh.tangled.repo.issue".to_string()),
            ("rkey", rkey.to_string()),
        ];
        let res: GetRes = self
            .get_json("com.atproto.repo.getRecord", &params, bearer)
            .await?;
        Ok(res.value)
    }

    pub async fn put_issue_record(
        &self,
        author_did: &str,
        rkey: &str,
        record: &Issue,
        bearer: Option<&str>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct PutReq<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
            validate: bool,
            record: &'a Issue,
        }
        let req = PutReq {
            repo: author_did,
            collection: "sh.tangled.repo.issue",
            rkey,
            validate: false,
            record,
        };
        let _: serde_json::Value = self
            .post_json("com.atproto.repo.putRecord", &req, bearer)
            .await?;
        Ok(())
    }

    pub async fn set_issue_state(
        &self,
        author_did: &str,
        issue_at: &str,
        state_nsid: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Rec<'a> {
            // `createdAt` is required by sh.tangled.repo.issue.state; a
            // record without it is written but never read.
            #[serde(rename = "$type")]
            lexicon_type: &'a str,
            issue: &'a str,
            state: &'a str,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            validate: bool,
            record: Rec<'a>,
        }
        #[derive(Deserialize)]
        struct Res {
            uri: String,
        }
        let rec = Rec {
            lexicon_type: "sh.tangled.repo.issue.state",
            issue: issue_at,
            state: state_nsid,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let req = Req {
            repo: author_did,
            collection: "sh.tangled.repo.issue.state",
            validate: false,
            record: rec,
        };
        let pds_client = self.derive(pds_base);
        let res: Res = pds_client
            .post_json("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in issue state uri"))
    }

    pub async fn delete_issue(
        &self,
        author_did: &str,
        rkey: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
        }
        let req = Req {
            repo: author_did,
            collection: "sh.tangled.repo.issue",
            rkey,
        };
        let pds_client = self.derive(pds_base);
        let _: serde_json::Value = pds_client
            .post_json("com.atproto.repo.deleteRecord", &req, Some(access_jwt))
            .await?;
        Ok(())
    }

    pub async fn get_pull_record(
        &self,
        author_did: &str,
        rkey: &str,
        bearer: Option<&str>,
    ) -> Result<Pull> {
        #[derive(Deserialize)]
        struct GetRes {
            value: Pull,
        }
        let params = [
            ("repo", author_did.to_string()),
            ("collection", "sh.tangled.repo.pull".to_string()),
            ("rkey", rkey.to_string()),
        ];
        let res: GetRes = self
            .get_json("com.atproto.repo.getRecord", &params, bearer)
            .await?;
        Ok(res.value)
    }

    // ========== Pull Requests ==========
    pub async fn list_pulls(
        &self,
        author_did: &str,
        target_repo_at_uri: Option<&str>,
        bearer: Option<&str>,
    ) -> Result<Vec<PullRecord>> {
        #[derive(Deserialize)]
        struct Item {
            uri: String,
            value: Pull,
        }
        #[derive(Deserialize)]
        struct ListRes {
            #[serde(default)]
            records: Vec<Item>,
        }
        let params = vec![
            ("repo", author_did.to_string()),
            ("collection", "sh.tangled.repo.pull".to_string()),
            ("limit", "100".to_string()),
        ];
        let res: ListRes = self
            .get_json("com.atproto.repo.listRecords", &params, bearer)
            .await?;
        let mut out = vec![];
        for it in res.records {
            if let Some(target) = target_repo_at_uri {
                if it.value.target.repo.as_str() != target {
                    continue;
                }
            }
            let rkey = Self::uri_rkey(&it.uri).unwrap_or_default();
            out.push(PullRecord {
                author_did: author_did.to_string(),
                rkey,
                pull: it.value,
            });
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_pull(
        &self,
        author_did: &str,
        repo_did: &str,
        repo_rkey: &str,
        target_branch: &str,
        patch: &str,
        title: &str,
        body: Option<&str>,
        source_branch: &str,
        source_sha: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct Res {
            uri: String,
        }

        let repo_at = format!("at://{}/sh.tangled.repo/{}", repo_did, repo_rkey);
        let now = chrono::Utc::now().to_rfc3339();

        // Gzip the patch and upload as a blob, matching the tangled server's
        // convention (application/gzip patchBlob).
        let gz_data = gzip_bytes(patch.as_bytes())?;
        let blob_ref = self
            .upload_blob(&gz_data, "application/gzip", pds_base, access_jwt)
            .await?;

        let record = serde_json::json!({
            "target": { "repo": repo_at, "branch": target_branch },
            "source": { "branch": source_branch, "sha": source_sha },
            "title": title,
            "body": body,
            "patchBlob": blob_ref,
            "createdAt": now,
        });

        let req = serde_json::json!({
            "repo": author_did,
            "collection": "sh.tangled.repo.pull",
            "validate": false,
            "record": record,
        });

        let pds_client = self.derive(pds_base);
        let res: Res = pds_client
            .post_json("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in pull uri"))
    }

    /// Create a PR via the appview web form endpoint.
    /// This is the correct way to create PRs — the appview generates the
    /// format-patch from branches, inserts the DB record, and creates the
    /// AT Protocol record.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_pull_via_appview(
        &self,
        owner: &str,
        repo_name: &str,
        target_branch: &str,
        source_branch: &str,
        title: &str,
        body: &str,
        _pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        let appview_base = std::env::var("TANGLED_APPVIEW_BASE")
            .unwrap_or_else(|_| "https://tangled.org".to_string());
        let url = format!("{}/{}/{}/pulls/new", appview_base, owner, repo_name);

        let form_params = [
            ("targetBranch", target_branch),
            ("sourceBranch", source_branch),
            ("title", title),
            ("body", body),
        ];

        if let Some(oauth) = self.should_use_oauth(Some(access_jwt)) {
            let form_body = serde_urlencoded::to_string(form_params)
                .map_err(|e| anyhow!("failed to encode form: {}", e))?;
            let resp_body = crate::oauth::oauth_post_raw(
                oauth,
                &url,
                form_body.as_bytes(),
                "application/x-www-form-urlencoded",
            )
            .await?;
            let resp_text = String::from_utf8_lossy(&resp_body);
            // Look for the PR URL in the response
            if let Some(pr_path) = resp_text
                .lines()
                .find(|l| l.contains("/pulls/"))
                .and_then(|l| l.split('"').find(|s| s.contains("/pulls/")))
            {
                return Ok(format!("{}{}", appview_base, pr_path));
            }
            return Ok(format!("{}/{}/{}/pulls", appview_base, owner, repo_name));
        }

        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let resp = http_client
            .post(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", access_jwt),
            )
            .form(&form_params)
            .send()
            .await?;

        let status = resp.status();

        // The appview redirects (302 or HX-Location) to the new PR on success
        if let Some(location) = resp
            .headers()
            .get("HX-Location")
            .or_else(|| resp.headers().get("Location"))
        {
            let loc = location.to_str().unwrap_or("");
            if loc.starts_with("http") {
                return Ok(loc.to_string());
            }
            return Ok(format!("{}{}", appview_base, loc));
        }

        if !status.is_success() && !status.is_redirection() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "failed to create PR via appview: {} {}",
                status,
                body
            ));
        }

        Ok(format!("{}/{}/{}/pulls", appview_base, owner, repo_name))
    }

    // ========== Spindle: Secrets Management ==========
    pub async fn list_repo_secrets(
        &self,
        pds_base: &str,
        access_jwt: &str,
        repo_at: &str,
    ) -> Result<Vec<Secret>> {
        let sa = self
            .service_auth_token(
                self.base_host(),
                pds_base,
                access_jwt,
                "sh.tangled.repo.listSecrets",
            )
            .await?;
        #[derive(Deserialize)]
        struct Res {
            secrets: Vec<Secret>,
        }
        let params = [("repo", repo_at.to_string())];
        let res: Res = self
            .get_json("sh.tangled.repo.listSecrets", &params, Some(&sa))
            .await?;
        Ok(res.secrets)
    }

    pub async fn add_repo_secret(
        &self,
        pds_base: &str,
        access_jwt: &str,
        repo_at: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let sa = self
            .service_auth_token(
                self.base_host(),
                pds_base,
                access_jwt,
                "sh.tangled.repo.addSecret",
            )
            .await?;
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            key: &'a str,
            value: &'a str,
        }
        let body = Req {
            repo: repo_at,
            key,
            value,
        };
        self.post("sh.tangled.repo.addSecret", &body, Some(&sa))
            .await
    }

    pub async fn remove_repo_secret(
        &self,
        pds_base: &str,
        access_jwt: &str,
        repo_at: &str,
        key: &str,
    ) -> Result<()> {
        let sa = self
            .service_auth_token(
                self.base_host(),
                pds_base,
                access_jwt,
                "sh.tangled.repo.removeSecret",
            )
            .await?;
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            key: &'a str,
        }
        let body = Req { repo: repo_at, key };
        self.post("sh.tangled.repo.removeSecret", &body, Some(&sa))
            .await
    }

    fn base_host(&self) -> &str {
        let base = self.base_url.trim_end_matches('/');
        base.strip_prefix("https://")
            .or_else(|| base.strip_prefix("http://"))
            .unwrap_or(base)
    }

    /// The base URL with a scheme, defaulting to https when the configured
    /// host is a bare name.
    pub(crate) fn base_url_with_scheme(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.starts_with("http://") || base.starts_with("https://") {
            base.to_string()
        } else {
            format!("https://{base}")
        }
    }

    /// Repoint a repo's default branch (the bare repo's HEAD) on its knot.
    pub async fn set_default_branch(
        &self,
        knot_host: &str,
        repo_did: &str,
        branch: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<()> {
        const SET_DEFAULT_BRANCH: &str = "sh.tangled.repo.setDefaultBranch";
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            #[serde(rename = "defaultBranch")]
            default_branch: &'a str,
        }
        let sa = self
            .knot_push_token(pds_base, access_jwt, knot_host, SET_DEFAULT_BRANCH, 240)
            .await?;
        let req = Req {
            repo: repo_did,
            default_branch: branch,
        };
        self.derive(format!("https://{knot_host}"))
            .post(SET_DEFAULT_BRANCH, &req, Some(&sa))
            .await
    }

    /// Fetch a blob and, if it is gzipped, decompress it.
    ///
    /// A pull's patch is stored as a gzipped blob rather than inline, so
    /// reading the record alone yields no diff at all.
    pub async fn get_patch_blob(&self, pds_base: &str, did: &str, cid: &str) -> Result<String> {
        let url = format!(
            "{}/xrpc/com.atproto.sync.getBlob?did={did}&cid={cid}",
            pds_base.trim_end_matches('/')
        );
        let res = reqwest::Client::new().get(&url).send().await?;
        let status = res.status();
        let bytes = res.bytes().await?;
        if !status.is_success() {
            return Err(anyhow!(
                "{status}: {}",
                String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ));
        }
        // gzip starts 1f 8b; anything else is already plain text.
        if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            use std::io::Read;
            let mut out = String::new();
            flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut out)?;
            return Ok(out);
        }
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    /// Resolve a handle to a DID.
    pub async fn resolve_handle(&self, handle: &str, bearer: Option<&str>) -> Result<String> {
        if handle.starts_with("did:") {
            return Ok(handle.to_string());
        }
        #[derive(Deserialize)]
        struct Res {
            did: String,
        }
        let params = [("handle", handle.to_string())];
        let res: Res = self
            .get_json("com.atproto.identity.resolveHandle", &params, bearer)
            .await?;
        Ok(res.did)
    }

    /// A service-auth token for a knot, bound to `lxm`. A knot caps the
    /// lifetime it will accept, so the caller chooses it.
    pub async fn knot_push_token(
        &self,
        pds_base: &str,
        access_jwt: &str,
        knot_host: &str,
        lxm: &str,
        lifetime_secs: i64,
    ) -> Result<String> {
        let audience = format!("did:web:{knot_host}");
        #[derive(Deserialize)]
        struct GetSARes {
            token: String,
        }
        let pds = self.derive(pds_base);
        let params = [
            ("aud", audience),
            (
                "exp",
                (chrono::Utc::now().timestamp() + lifetime_secs).to_string(),
            ),
            ("lxm", lxm.to_string()),
        ];
        let sa: GetSARes = pds
            .get_json(
                "com.atproto.server.getServiceAuth",
                &params,
                Some(access_jwt),
            )
            .await?;
        Ok(sa.token)
    }

    /// Service auth for the spindle this client points at, bound to `lxm`.
    pub(crate) async fn spindle_auth(
        &self,
        pds_base: &str,
        access_jwt: &str,
        lxm: &str,
    ) -> Result<String> {
        self.service_auth_token(self.base_host(), pds_base, access_jwt, lxm)
            .await
    }

    /// Mint an inter-service auth token for `target_host`.
    ///
    /// `lxm` is the lexicon method the token will be spent on. It is not
    /// optional in practice: a knot checks the token's `lxm` claim against the
    /// method being called and rejects a mismatch with
    /// `method binding mismatch: token bound to None, expected <method>`.
    async fn service_auth_token(
        &self,
        target_host: &str,
        pds_base: &str,
        access_jwt: &str,
        lxm: &str,
    ) -> Result<String> {
        let audience = format!("did:web:{}", target_host);
        #[derive(Deserialize)]
        struct GetSARes {
            token: String,
        }
        let pds = self.derive(pds_base);
        let params = [
            ("aud", audience),
            ("exp", (chrono::Utc::now().timestamp() + 60).to_string()),
            ("lxm", lxm.to_string()),
        ];
        let sa: GetSARes = pds
            .get_json(
                "com.atproto.server.getServiceAuth",
                &params,
                Some(access_jwt),
            )
            .await?;
        Ok(sa.token)
    }

    pub async fn comment_pull(
        &self,
        author_did: &str,
        pull_at: &str,
        body: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Rec<'a> {
            pull: &'a str,
            body: &'a str,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            validate: bool,
            record: Rec<'a>,
        }
        #[derive(Deserialize)]
        struct Res {
            uri: String,
        }
        let now = chrono::Utc::now().to_rfc3339();
        let rec = Rec {
            pull: pull_at,
            body,
            created_at: now,
        };
        let req = Req {
            repo: author_did,
            collection: "sh.tangled.repo.pull.comment",
            validate: false,
            record: rec,
        };
        let pds_client = self.derive(pds_base);
        let res: Res = pds_client
            .post_json("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in pull comment uri"))
    }

    /// Record a pull request's status.
    ///
    /// The collection is `sh.tangled.repo.pull.status`, not `...pull.state`:
    /// pulls were renamed and gained a third value, `merged`. The lexicon
    /// requires `createdAt`, so a record without one is written but never
    /// read.
    pub async fn set_pull_status(
        &self,
        author_did: &str,
        pull_at: &str,
        status_nsid: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Rec<'a> {
            #[serde(rename = "$type")]
            lexicon_type: &'a str,
            pull: &'a str,
            status: &'a str,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            validate: bool,
            record: Rec<'a>,
        }
        #[derive(Deserialize)]
        struct Res {
            uri: String,
        }
        let rec = Rec {
            lexicon_type: PULL_STATUS,
            pull: pull_at,
            status: status_nsid,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let req = Req {
            repo: author_did,
            collection: PULL_STATUS,
            validate: false,
            record: rec,
        };
        let pds_client = self.derive(pds_base);
        let res: Res = pds_client
            .post_json("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in pull status uri"))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn merge_pull(
        &self,
        pull_did: &str,
        pull_rkey: &str,
        repo_did: &str,
        repo_name: &str,
        knot: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<()> {
        // Fetch the pull request to get patch and target branch
        let pds_client = self.derive(pds_base);
        let pull = pds_client
            .get_pull_record(pull_did, pull_rkey, Some(access_jwt))
            .await?;

        // Get service auth token for the knot
        let sa = self
            .service_auth_token(knot, pds_base, access_jwt, "sh.tangled.repo.merge")
            .await?;

        #[derive(Serialize)]
        struct MergeReq<'a> {
            did: &'a str,
            name: &'a str,
            patch: &'a str,
            branch: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "commitMessage")]
            commit_message: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            #[serde(rename = "commitBody")]
            commit_body: Option<&'a str>,
        }

        let commit_body = if pull.body.is_empty() {
            None
        } else {
            Some(pull.body.as_str())
        };

        let req = MergeReq {
            did: repo_did,
            name: repo_name,
            patch: pull.patch.as_deref().unwrap_or(""),
            branch: &pull.target.branch,
            commit_message: Some(&pull.title),
            commit_body,
        };

        let knot_client = self.derive(format!("https://{}", knot));
        knot_client
            .post("sh.tangled.repo.merge", &req, Some(&sa))
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn merge_check(
        &self,
        repo_did: &str,
        repo_name: &str,
        branch: &str,
        patch: &str,
        knot: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<MergeCheckResponse> {
        let sa = self
            .service_auth_token(knot, pds_base, access_jwt, "sh.tangled.repo.mergeCheck")
            .await?;

        let req = MergeCheckRequest {
            did: repo_did.to_string(),
            name: repo_name.to_string(),
            branch: branch.to_string(),
            patch: patch.to_string(),
        };

        let knot_client = self.derive(format!("https://{}", knot));
        knot_client
            .post_json("sh.tangled.repo.mergeCheck", &req, Some(&sa))
            .await
    }

    pub async fn update_repo_spindle(
        &self,
        did: &str,
        rkey: &str,
        new_spindle: Option<&str>,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<()> {
        let pds_client = self.derive(pds_base);
        #[derive(Deserialize, Serialize, Clone)]
        struct Rec {
            // Absent on older records; see update_repo_knot.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            name: Option<String>,
            knot: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            spindle: Option<String>,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Deserialize)]
        struct GetRes {
            value: Rec,
        }
        let params = [
            ("repo", did.to_string()),
            ("collection", "sh.tangled.repo".to_string()),
            ("rkey", rkey.to_string()),
        ];
        let got: GetRes = pds_client
            .get_json("com.atproto.repo.getRecord", &params, Some(access_jwt))
            .await?;
        let mut rec = got.value;
        rec.spindle = new_spindle.map(|s| s.to_string());
        #[derive(Serialize)]
        struct PutReq<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
            validate: bool,
            record: Rec,
        }
        let req = PutReq {
            repo: did,
            collection: "sh.tangled.repo",
            rkey,
            validate: false,
            record: rec,
        };
        let _: serde_json::Value = pds_client
            .post_json("com.atproto.repo.putRecord", &req, Some(access_jwt))
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Repository {
    pub did: Option<String>,
    pub rkey: Option<String>,
    // Older sh.tangled.repo records carry no `name`: the record key is the
    // name. Default to empty here and backfill from the record's rkey, so one
    // legacy record does not fail the whole listing.
    #[serde(default)]
    pub name: String,
    pub knot: Option<String>,
    pub description: Option<String>,
    pub spindle: Option<String>,
    #[serde(rename = "repoDid", default)]
    pub repo_did: Option<String>,
    #[serde(default)]
    pub private: bool,
}

// Issue record value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub repo: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct IssueRecord {
    pub author_did: String,
    pub rkey: String,
    pub issue: Issue,
}

// Pull record value (subset)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullTarget {
    pub repo: String,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullSource {
    pub sha: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pull {
    pub target: PullTarget,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Older records inlined the patch here. Current ones do not: see
    /// `patch_blob`.
    #[serde(default)]
    pub patch: Option<String>,
    /// The patch as a gzipped blob. A pull carries it directly, or inside
    /// the last of its `rounds` when it has been revised.
    #[serde(rename = "patchBlob", default)]
    pub patch_blob: Option<BlobRef>,
    #[serde(default)]
    pub rounds: Vec<PullRound>,
    #[serde(default)]
    pub source: Option<PullSource>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    // Stack support fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_change_id: Option<String>,
}

/// A blob reference: the CID lives at `ref.$link`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobRef {
    #[serde(rename = "ref", default)]
    pub reference: BlobLink,
    #[serde(rename = "mimeType", default)]
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlobLink {
    #[serde(rename = "$link", default)]
    pub link: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRound {
    #[serde(rename = "patchBlob", default)]
    pub patch_blob: Option<BlobRef>,
}

impl Pull {
    /// The CID of the patch to show: the newest round's, else the pull's own.
    pub fn patch_cid(&self) -> Option<&str> {
        self.rounds
            .iter()
            .rev()
            .find_map(|r| r.patch_blob.as_ref())
            .or(self.patch_blob.as_ref())
            .map(|b| b.reference.link.as_str())
            .filter(|c| !c.is_empty())
    }
}

#[derive(Debug, Clone)]
pub struct PullRecord {
    pub author_did: String,
    pub rkey: String,
    pub pull: Pull,
}

// Merge check types for stacked diff conflict detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeCheckRequest {
    pub did: String,
    pub name: String,
    pub branch: String,
    pub patch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeCheckResponse {
    pub is_conflicted: bool,
    #[serde(default)]
    pub conflicts: Vec<ConflictInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictInfo {
    pub filename: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct RepoRecord {
    pub did: String,
    pub name: String,
    pub rkey: String,
    pub knot: String,
    pub description: Option<String>,
    pub spindle: Option<String>,
    /// The repo's own DID, minted by the knot. Pipelines are keyed by it.
    pub repo_did: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultBranch {
    pub name: String,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_hash: Option<String>,
    pub when: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    pub name: String,
    pub size: u64,
    pub percentage: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Languages {
    pub languages: Vec<Language>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_files: Option<u64>,
}

/// A star's subject, in either shape it can be found in.
///
/// Current records nest an object naming the repo's own DID. Records written
/// by older clients hold a bare at-uri string instead, and a PDS that has both
/// must still deserialise, so this accepts either.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct StarSubject(pub serde_json::Value);

impl StarSubject {
    /// The repo DID, for the current shape.
    pub fn did(&self) -> Option<&str> {
        self.0.get("did").and_then(|v| v.as_str())
    }

    /// The at-uri, for the legacy shape.
    pub fn legacy_uri(&self) -> Option<&str> {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarRecord {
    #[serde(default)]
    pub subject: StarSubject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secret {
    pub repo: String,
    pub key: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "createdBy")]
    pub created_by: String,
}

#[derive(Debug, Clone)]
pub struct CreateRepoOptions<'a> {
    pub did: &'a str,
    pub name: &'a str,
    pub knot: &'a str,
    pub description: Option<&'a str>,
    pub default_branch: Option<&'a str>,
    pub source: Option<&'a str>,
    /// AT URI of the source repo record (for forks), stored in the PDS record.
    pub source_at: Option<&'a str>,
    pub pds_base: &'a str,
    pub access_jwt: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerMetadata {
    pub kind: String,
    pub repo: TriggerRepo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerRepo {
    pub knot: String,
    pub did: String,
    pub repo: String,
    #[serde(rename = "defaultBranch")]
    pub default_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub engine: String,
}

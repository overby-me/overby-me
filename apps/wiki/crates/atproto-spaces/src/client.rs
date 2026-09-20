//! The calls, against one host: a user's PDS for what a session may do, a
//! space's host for the space as a whole, a repo's host for one repo in it.
//! A PDS is usually all three.

use crate::dpop::DpopKey;
use crate::{Error, SignedCommit};
use serde_json::{Value, json};

/// Who is asking.
#[derive(Clone, Copy)]
pub enum Auth<'a> {
    /// A session of the user's at their own PDS: what writes, and what asks
    /// for a delegation token.
    Bearer(&'a str),
    /// A space credential with the key it is bound to: what reads a space.
    Space(&'a str, &'a DpopKey),
}

#[derive(Clone)]
pub struct Host {
    http: reqwest::Client,
    base: String,
}

/// A record as a write answers with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub uri: String,
    pub cid: String,
}

/// One entry of a repo's operation log. `cid` is absent for a delete and
/// `prev` for a create; `value` only for the state a path is in now.
#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    pub rev: String,
    pub collection: String,
    pub rkey: String,
    pub cid: Option<String>,
    pub prev: Option<String>,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub collection: String,
    pub rkey: String,
    pub cid: String,
    pub value: Value,
}

/// One repo of the writer set: who, and what their repo has come to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    pub did: String,
    pub rev: String,
}

fn managed_by(app: &str) -> Value {
    json!({"$type": "com.atproto.simplespace.defs#managingAppPolicy", "managingApp": app})
}

/// Whether `setup` (of [`Host::space_setup`]) leaves both who reads and who
/// writes to `app` and to nobody else.
pub fn is_managed_by(setup: &Value, app: &str) -> bool {
    [&setup["readPolicy"], &setup["writePolicy"]]
        .iter()
        .all(|policy| policy["$type"] == managed_by(app)["$type"] && policy["managingApp"] == app)
}

fn text(value: &Value, name: &str) -> Result<String, Error> {
    value[name]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::Malformed(format!("an answer with no `{name}`")))
}

impl Host {
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Host {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    pub fn url(&self) -> &str {
        &self.base
    }

    async fn send(
        &self,
        method: reqwest::Method,
        nsid: &str,
        params: &[(&str, &str)],
        body: Option<&Value>,
        auth: Auth<'_>,
    ) -> Result<reqwest::Response, Error> {
        let url = format!("{}/xrpc/{nsid}", self.base);
        let mut request = self.http.request(method.clone(), &url).query(params);
        request = match auth {
            Auth::Bearer(token) => request.bearer_auth(token),
            Auth::Space(credential, key) => request
                .header("authorization", format!("DPoP {credential}"))
                .header("dpop", key.proof(method.as_str(), &url, Some(credential))),
        };
        if let Some(body) = body {
            request = request.json(body);
        }
        let answer = request.send().await?;
        if answer.status().is_success() {
            return Ok(answer);
        }
        let status = answer.status().as_u16();
        let said: Value = answer.json().await.unwrap_or(Value::Null);
        Err(Error::Xrpc {
            status,
            error: said["error"].as_str().unwrap_or("Unknown").to_string(),
            message: said["message"].as_str().unwrap_or_default().to_string(),
        })
    }

    async fn query(
        &self,
        nsid: &str,
        params: &[(&str, &str)],
        auth: Auth<'_>,
    ) -> Result<Value, Error> {
        let answer = self
            .send(reqwest::Method::GET, nsid, params, None, auth)
            .await?;
        Ok(answer.json().await?)
    }

    async fn procedure(&self, nsid: &str, body: &Value, auth: Auth<'_>) -> Result<Value, Error> {
        let answer = self
            .send(reqwest::Method::POST, nsid, &[], Some(body), auth)
            .await?;
        // Some procedures answer with no body at all.
        Ok(answer.json().await.unwrap_or(Value::Null))
    }

    // -- An account's own PDS. --

    /// A password session: the organization's own account, by an app password.
    /// Answers `(did, access token)`.
    pub async fn create_session(
        &self,
        identifier: &str,
        password: &str,
    ) -> Result<(String, String), Error> {
        let body = json!({"identifier": identifier, "password": password});
        let url = format!("{}/xrpc/com.atproto.server.createSession", self.base);
        let answer = self.http.post(url).json(&body).send().await?;
        if !answer.status().is_success() {
            return Err(Error::Xrpc {
                status: answer.status().as_u16(),
                error: "AuthenticationRequired".into(),
                message: "the PDS refused the session".into(),
            });
        }
        let said: Value = answer.json().await?;
        Ok((text(&said, "did")?, text(&said, "accessJwt")?))
    }

    /// Upload a blob for a record to name. Answers the blob reference as the
    /// record is to carry it.
    pub async fn upload_blob(
        &self,
        session: &str,
        bytes: Vec<u8>,
        mime: &str,
    ) -> Result<Value, Error> {
        let url = format!("{}/xrpc/com.atproto.repo.uploadBlob", self.base);
        let answer = self
            .http
            .post(url)
            .bearer_auth(session)
            .header("content-type", mime)
            .body(bytes)
            .send()
            .await?;
        if !answer.status().is_success() {
            return Err(Error::Xrpc {
                status: answer.status().as_u16(),
                error: "BlobRefused".into(),
                message: answer.text().await.unwrap_or_default(),
            });
        }
        let said: Value = answer.json().await?;
        Ok(said["blob"].clone())
    }

    pub async fn put_record(
        &self,
        session: &str,
        space: &str,
        repo: &str,
        collection: &str,
        rkey: &str,
        record: &Value,
    ) -> Result<Written, Error> {
        let body = json!({"space": space, "repo": repo, "collection": collection, "rkey": rkey, "record": record});
        let said = self
            .procedure("com.atproto.space.putRecord", &body, Auth::Bearer(session))
            .await?;
        Ok(Written {
            uri: text(&said, "uri")?,
            cid: text(&said, "cid")?,
        })
    }

    pub async fn delete_record(
        &self,
        session: &str,
        space: &str,
        repo: &str,
        collection: &str,
        rkey: &str,
    ) -> Result<(), Error> {
        let body = json!({"space": space, "repo": repo, "collection": collection, "rkey": rkey});
        self.procedure(
            "com.atproto.space.deleteRecord",
            &body,
            Auth::Bearer(session),
        )
        .await
        .map(|_| ())
    }

    /// A delegation token: that this application acts for this user, addressed
    /// to the space's authority and good for a minute.
    pub async fn delegation_token(&self, session: &str, space: &str) -> Result<String, Error> {
        let said = self
            .query(
                "com.atproto.space.getDelegationToken",
                &[("space", space)],
                Auth::Bearer(session),
            )
            .await?;
        text(&said, "token")
    }

    // -- Spaces of the simplest kind, on the account's PDS. --

    /// Make a space whose readers and writers a managing app decides.
    pub async fn create_managed_space(
        &self,
        session: &str,
        space_type: &str,
        skey: &str,
        managing_app: &str,
    ) -> Result<String, Error> {
        let policy = managed_by(managing_app);
        let body = json!({
            "type": space_type, "skey": skey,
            "readPolicy": policy, "writePolicy": policy,
            "appAccess": {"$type": "com.atproto.simplespace.defs#open"},
        });
        let said = self
            .procedure(
                "com.atproto.simplespace.createSpace",
                &body,
                Auth::Bearer(session),
            )
            .await?;
        text(&said, "uri")
    }

    /// How a space is set up, or `None` for one that is not there. Writing
    /// into a space that is gone succeeds, so this is the only way to know.
    pub async fn space_setup(&self, session: &str, space: &str) -> Result<Option<Value>, Error> {
        let asked = self
            .query(
                "com.atproto.simplespace.getSpace",
                &[("space", space)],
                Auth::Bearer(session),
            )
            .await;
        match asked {
            Ok(setup) => Ok(Some(setup)),
            Err(e) if e.xrpc_name() == Some("SpaceNotFound") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Put a space under a managing app, whoever decided its access before.
    pub async fn manage_space(
        &self,
        session: &str,
        space: &str,
        managing_app: &str,
    ) -> Result<(), Error> {
        let policy = managed_by(managing_app);
        let body = json!({"space": space, "readPolicy": policy, "writePolicy": policy});
        self.procedure(
            "com.atproto.simplespace.updateSpace",
            &body,
            Auth::Bearer(session),
        )
        .await
        .map(|_| ())
    }

    pub async fn delete_space(&self, session: &str, space: &str) -> Result<(), Error> {
        self.procedure(
            "com.atproto.simplespace.deleteSpace",
            &json!({"space": space}),
            Auth::Bearer(session),
        )
        .await
        .map(|_| ())
    }

    // -- A space's host. --

    /// Exchange a delegation token for a credential bound to `key`.
    pub async fn space_credential(
        &self,
        delegation: &str,
        space: &str,
        key: &DpopKey,
        attestation: Option<&str>,
    ) -> Result<String, Error> {
        let url = format!("{}/xrpc/com.atproto.space.getSpaceCredential", self.base);
        let mut body = json!({"space": space});
        if let Some(attestation) = attestation {
            body["clientAttestation"] = Value::String(attestation.to_string());
        }
        // The delegation token is a grant spent once, not an access token: the
        // proof is not bound to it.
        let answer = self
            .http
            .post(&url)
            .bearer_auth(delegation)
            .header("dpop", key.proof("POST", &url, None))
            .json(&body)
            .send()
            .await?;
        let status = answer.status();
        let said: Value = answer.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(Error::Xrpc {
                status: status.as_u16(),
                error: said["error"].as_str().unwrap_or("Unknown").to_string(),
                message: said["message"].as_str().unwrap_or_default().to_string(),
            });
        }
        text(&said, "credential")
    }

    /// The writer set, all of it.
    pub async fn writers(&self, auth: Auth<'_>, space: &str) -> Result<Vec<Writer>, Error> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = vec![("space", space), ("limit", "500")];
            if let Some(cursor) = &cursor {
                params.push(("cursor", cursor));
            }
            let said = self
                .query("com.atproto.space.listRepos", &params, auth)
                .await?;
            for repo in said["repos"].as_array().into_iter().flatten() {
                out.push(Writer {
                    did: text(repo, "did")?,
                    rev: text(repo, "rev")?,
                });
            }
            cursor = said["cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                return Ok(out);
            }
        }
    }

    /// Ask to be told when any repo of the space advances. Answers when the
    /// registration runs out.
    pub async fn register_notify(
        &self,
        auth: Auth<'_>,
        space: &str,
        service: &str,
    ) -> Result<String, Error> {
        let said = self
            .procedure(
                "com.atproto.space.registerNotify",
                &json!({"space": space, "service": service}),
                auth,
            )
            .await?;
        text(&said, "expiresAt")
    }

    // -- A repo's host. --

    /// The log after `since`, all of it, with the commit it ends at when the
    /// host had one to give.
    pub async fn ops_since(
        &self,
        auth: Auth<'_>,
        space: &str,
        repo: &str,
        since: Option<&str>,
    ) -> Result<(Vec<Op>, Option<SignedCommit>), Error> {
        let mut ops = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = vec![("space", space), ("repo", repo), ("limit", "500")];
            if let Some(since) = since {
                params.push(("since", since));
            }
            if let Some(cursor) = &cursor {
                params.push(("cursor", cursor));
            }
            let said = self
                .query("com.atproto.space.listRepoOps", &params, auth)
                .await?;
            for op in said["ops"].as_array().into_iter().flatten() {
                ops.push(Op {
                    rev: text(op, "rev")?,
                    collection: text(op, "collection")?,
                    rkey: text(op, "rkey")?,
                    cid: op["cid"].as_str().map(str::to_string),
                    prev: op["prev"].as_str().map(str::to_string),
                    value: op.get("value").filter(|v| !v.is_null()).cloned(),
                });
            }
            cursor = said["cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                return Ok((ops, SignedCommit::from_json(&said["commit"])));
            }
        }
    }

    /// Everything a repo holds now, with the values.
    pub async fn records(
        &self,
        auth: Auth<'_>,
        space: &str,
        repo: &str,
    ) -> Result<Vec<Record>, Error> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = vec![("space", space), ("repo", repo), ("limit", "500")];
            if let Some(cursor) = &cursor {
                params.push(("cursor", cursor));
            }
            let said = self
                .query("com.atproto.space.listRecords", &params, auth)
                .await?;
            for record in said["records"].as_array().into_iter().flatten() {
                out.push(Record {
                    collection: text(record, "collection")?,
                    rkey: text(record, "rkey")?,
                    cid: text(record, "cid")?,
                    value: record["value"].clone(),
                });
            }
            cursor = said["cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                return Ok(out);
            }
        }
    }

    pub async fn latest_commit(
        &self,
        auth: Auth<'_>,
        space: &str,
        repo: &str,
    ) -> Result<SignedCommit, Error> {
        let said = self
            .query(
                "com.atproto.space.getLatestCommit",
                &[("space", space), ("repo", repo)],
                auth,
            )
            .await?;
        SignedCommit::from_json(&said["commit"])
            .ok_or_else(|| Error::Malformed("a commit that is none".into()))
    }

    pub async fn blob(
        &self,
        auth: Auth<'_>,
        space: &str,
        repo: &str,
        cid: &str,
    ) -> Result<Vec<u8>, Error> {
        let answer = self
            .send(
                reqwest::Method::GET,
                "com.atproto.space.getBlob",
                &[("space", space), ("repo", repo), ("cid", cid)],
                None,
                auth,
            )
            .await?;
        Ok(answer.bytes().await?.to_vec())
    }
}

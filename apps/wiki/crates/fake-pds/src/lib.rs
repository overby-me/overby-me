//! A PDS for tests: one account, its records in memory, and the calls the
//! board's publisher and its mirror make of a real one
//! (`com.atproto.server.createSession`, `com.atproto.repo.createRecord`,
//! `putRecord`, `applyWrites`, `listRecords`, `deleteRecord`). It checks the
//! password and the bearer, since a publisher that forgot either would pass
//! against anything laxer and fail against the real thing.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

pub const DID: &str = "did:plc:boardaccount0000000000000";
const ACCESS: &str = "fake-access-jwt";

/// One record: its collection, its key, and what it says.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub collection: String,
    pub rkey: String,
    pub value: Value,
}

impl Record {
    pub fn uri(&self) -> String {
        format!("at://{DID}/{}/{}", self.collection, self.rkey)
    }

    /// Not a real CID: a digest of the value, which is all a test needs of one
    /// (the same record has the same, a rewritten one another).
    pub fn cid(&self) -> String {
        let digest = Sha256::digest(self.value.to_string().as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        format!("bafyfake{hex}")
    }
}

#[derive(Clone)]
struct Repo {
    password: String,
    records: Arc<Mutex<Vec<Record>>>,
    /// Every write call as it arrived, for a test to read the batching off.
    calls: Arc<Mutex<Vec<(String, usize)>>>,
}

pub struct FakePds {
    pub url: String,
    records: Arc<Mutex<Vec<Record>>>,
    calls: Arc<Mutex<Vec<(String, usize)>>>,
}

impl FakePds {
    pub async fn start(password: &str) -> FakePds {
        let repo = Repo {
            password: password.to_string(),
            records: Arc::default(),
            calls: Arc::default(),
        };
        let (records, calls) = (repo.records.clone(), repo.calls.clone());
        let app = Router::new()
            .route(
                "/xrpc/com.atproto.server.createSession",
                post(create_session),
            )
            .route("/xrpc/com.atproto.repo.createRecord", post(create_record))
            .route("/xrpc/com.atproto.repo.putRecord", post(put_record))
            .route("/xrpc/com.atproto.repo.applyWrites", post(apply_writes))
            .route("/xrpc/com.atproto.repo.deleteRecord", post(delete_record))
            .route("/xrpc/com.atproto.repo.listRecords", get(list_records))
            .with_state(repo);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let url = format!("http://{}", listener.local_addr().expect("an address"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        FakePds {
            url,
            records,
            calls,
        }
    }

    pub fn records(&self, collection: &str) -> Vec<Record> {
        let all = self.records.lock().expect("records");
        all.iter()
            .filter(|r| r.collection == collection)
            .cloned()
            .collect()
    }

    /// `(method, how many records it wrote)`, in the order the calls came.
    pub fn calls(&self) -> Vec<(String, usize)> {
        self.calls.lock().expect("calls").clone()
    }

    /// What a custodian gone bad would do behind a mirror's back.
    pub fn tamper(&self, change: impl FnOnce(&mut Vec<Record>)) {
        change(&mut self.records.lock().expect("records"));
    }
}

fn refused(status: StatusCode, error: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": error, "message": error })))
}

fn authorized(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {ACCESS}"))
}

async fn create_session(
    State(repo): State<Repo>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if body["password"].as_str() != Some(repo.password.as_str()) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    let answer =
        json!({ "did": DID, "handle": "board.test", "accessJwt": ACCESS, "refreshJwt": "r" });
    (StatusCode::OK, Json(answer))
}

fn write(repo: &Repo, collection: &str, rkey: &str, value: &Value) -> Record {
    let record = Record {
        collection: collection.to_string(),
        rkey: rkey.to_string(),
        value: value.clone(),
    };
    let mut all = repo.records.lock().expect("records");
    all.retain(|r| !(r.collection == record.collection && r.rkey == record.rkey));
    all.push(record.clone());
    record
}

async fn create_record(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    put(&repo, &headers, &body, "createRecord")
}

async fn put_record(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    put(&repo, &headers, &body, "putRecord")
}

fn put(repo: &Repo, headers: &HeaderMap, body: &Value, method: &str) -> (StatusCode, Json<Value>) {
    if !authorized(headers) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    let (Some(collection), Some(rkey)) = (body["collection"].as_str(), body["rkey"].as_str())
    else {
        return refused(StatusCode::BAD_REQUEST, "InvalidRequest");
    };
    if body["repo"].as_str() != Some(DID) {
        return refused(StatusCode::BAD_REQUEST, "InvalidRequest");
    }
    let record = write(repo, collection, rkey, &body["record"]);
    repo.calls
        .lock()
        .expect("calls")
        .push((method.to_string(), 1));
    (
        StatusCode::OK,
        Json(json!({ "uri": record.uri(), "cid": record.cid() })),
    )
}

async fn apply_writes(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    let writes = body["writes"].as_array().cloned().unwrap_or_default();
    let mut results = Vec::new();
    for op in &writes {
        let (Some(collection), Some(rkey)) = (op["collection"].as_str(), op["rkey"].as_str())
        else {
            return refused(StatusCode::BAD_REQUEST, "InvalidRequest");
        };
        let record = write(&repo, collection, rkey, &op["value"]);
        results.push(json!({ "uri": record.uri(), "cid": record.cid() }));
    }
    repo.calls
        .lock()
        .expect("calls")
        .push(("applyWrites".to_string(), writes.len()));
    (StatusCode::OK, Json(json!({ "results": results })))
}

async fn delete_record(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    let (collection, rkey) = (body["collection"].as_str(), body["rkey"].as_str());
    repo.records
        .lock()
        .expect("records")
        .retain(|r| !(Some(r.collection.as_str()) == collection && Some(r.rkey.as_str()) == rkey));
    (StatusCode::OK, Json(json!({})))
}

#[derive(serde::Deserialize)]
struct ListParams {
    collection: String,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// Public, as a real PDS's is: a mirror needs no account.
async fn list_records(State(repo): State<Repo>, Query(p): Query<ListParams>) -> Json<Value> {
    let all = repo.records.lock().expect("records");
    let mut of: Vec<&Record> = all
        .iter()
        .filter(|r| r.collection == p.collection)
        .collect();
    of.sort_by(|a, b| a.rkey.cmp(&b.rkey));
    let after = p.cursor.unwrap_or_default();
    let page: Vec<&Record> = of
        .into_iter()
        .filter(|r| r.rkey > after)
        .take(p.limit.unwrap_or(50))
        .collect();
    let cursor = page.last().map(|r| r.rkey.clone());
    let records: Vec<Value> = page
        .iter()
        .map(|r| json!({ "uri": r.uri(), "cid": r.cid(), "value": r.value }))
        .collect();
    Json(json!({ "records": records, "cursor": cursor }))
}

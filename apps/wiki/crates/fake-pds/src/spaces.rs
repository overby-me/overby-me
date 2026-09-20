//! The space calls the AppView makes of the organization's PDS
//! (`com.atproto.simplespace.createSpace`, `getSpace`, `updateSpace`,
//! `deleteSpace`, `com.atproto.space.putRecord`, `deleteRecord`), answered as
//! the alpha was found to answer them, the account's DID document
//! as a directory serves it, and the service token a PDS calls a managing app
//! under. What reads a space (credentials, the log, commits) is held to the
//! real alpha PDS instead (`just test-spaces`): a fake of that proves nothing.

use crate::{DID, Record, Repo, authorized, refused};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// A space of the account: how it is set up, and the account's records in it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Space {
    /// `readPolicy`, `writePolicy` and `appAccess`, as it was made or updated.
    pub setup: Value,
    pub records: Vec<Record>,
}

/// Every space of the account, by its URI.
pub(crate) type Spaces = Arc<Mutex<BTreeMap<String, Space>>>;

pub(crate) fn routes() -> Router<Repo> {
    Router::new()
        .route(
            "/xrpc/com.atproto.simplespace.createSpace",
            post(create_space),
        )
        .route(
            "/xrpc/com.atproto.simplespace.deleteSpace",
            post(delete_space),
        )
        .route("/xrpc/com.atproto.simplespace.getSpace", get(get_space))
        .route(
            "/xrpc/com.atproto.simplespace.updateSpace",
            post(update_space),
        )
        .route(
            "/xrpc/com.atproto.repo.uploadBlob",
            // Past the limit below, so that it is this that refuses and not axum.
            post(upload_blob).layer(axum::extract::DefaultBodyLimit::max(2 * BLOB_LIMIT)),
        )
        .route("/xrpc/com.atproto.space.putRecord", post(put_record))
        .route("/xrpc/com.atproto.space.createRecord", post(create_record))
        .route("/xrpc/com.atproto.space.applyWrites", post(apply_writes))
        .route("/xrpc/com.atproto.space.deleteRecord", post(delete_record))
        .route("/plc/{did}", get(did_document))
}

/// `PDS_BLOB_UPLOAD_LIMIT` as a PDS ships.
const BLOB_LIMIT: usize = 5 * 1024 * 1024;

/// The bytes of a file, for a record to name. Not a real CID either.
async fn upload_blob(
    State(repo): State<Repo>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    use sha2::{Digest, Sha256};
    if !authorized(&headers) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    repo.calls
        .lock()
        .expect("calls")
        .push(("uploadBlob".to_string(), bytes.len()));
    if bytes.len() > BLOB_LIMIT {
        return refused(StatusCode::PAYLOAD_TOO_LARGE, "PayloadTooLargeError");
    }
    let mime = headers.get("content-type").and_then(|v| v.to_str().ok());
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let cid = format!("bafkfake{digest}");
    let blob = json!({
        "$type": "blob", "ref": {"$link": cid},
        "mimeType": mime.unwrap_or("application/octet-stream"), "size": bytes.len(),
    });
    repo.blobs
        .lock()
        .expect("blobs")
        .insert(cid, bytes.to_vec());
    (StatusCode::OK, Json(json!({ "blob": blob })))
}

async fn create_space(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    let (Some(space_type), Some(skey)) = (body["type"].as_str(), body["skey"].as_str()) else {
        return refused(StatusCode::BAD_REQUEST, "InvalidRequest");
    };
    // The one policy the wiki asks for, and who it names.
    let app = body["readPolicy"]["managingApp"].as_str();
    if app.is_none() || app != body["writePolicy"]["managingApp"].as_str() {
        return refused(StatusCode::BAD_REQUEST, "UnsupportedPolicy");
    }
    let uri = format!("at://{DID}/space/{space_type}/{skey}");
    let mut spaces = repo.spaces.lock().expect("spaces");
    if spaces.contains_key(&uri) {
        return refused(StatusCode::BAD_REQUEST, "SpaceAlreadyExists");
    }
    let setup = json!({
        "readPolicy": body["readPolicy"], "writePolicy": body["writePolicy"],
        "appAccess": body["appAccess"],
    });
    let made = Space {
        setup,
        records: Vec::new(),
    };
    spaces.insert(uri.clone(), made);
    (StatusCode::OK, Json(json!({ "uri": uri })))
}

async fn get_space(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Query(asked): Query<BTreeMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    let uri = asked.get("space").cloned().unwrap_or_default();
    match repo.spaces.lock().expect("spaces").get(&uri) {
        Some(space) => {
            let mut setup = space.setup.clone();
            setup["uri"] = Value::String(uri);
            (StatusCode::OK, Json(setup))
        }
        None => refused(StatusCode::BAD_REQUEST, "SpaceNotFound"),
    }
}

async fn update_space(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    let mut spaces = repo.spaces.lock().expect("spaces");
    let Some(space) = spaces.get_mut(body["space"].as_str().unwrap_or_default()) else {
        return refused(StatusCode::BAD_REQUEST, "SpaceNotFound");
    };
    // Omitted fields are left as they were.
    for policy in ["readPolicy", "writePolicy", "appAccess"] {
        if !body[policy].is_null() {
            space.setup[policy] = body[policy].clone();
        }
    }
    (StatusCode::OK, Json(json!({})))
}

async fn delete_space(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !authorized(&headers) {
        return refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired");
    }
    // The real one answers the same for a space that is already gone.
    let space = body["space"].as_str().unwrap_or_default();
    repo.spaces.lock().expect("spaces").remove(space);
    (StatusCode::OK, Json(json!({})))
}

async fn put_record(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let one = [(
        body["collection"].clone(),
        body["rkey"].clone(),
        body["record"].clone(),
    )];
    match write(&repo, &headers, &body, "space.putRecord", &one, false) {
        Ok(mut written) => (StatusCode::OK, Json(written.remove(0))),
        Err(refused) => refused,
    }
}

async fn create_record(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let one = [(
        body["collection"].clone(),
        body["rkey"].clone(),
        body["record"].clone(),
    )];
    match write(&repo, &headers, &body, "space.createRecord", &one, true) {
        Ok(mut written) => (StatusCode::OK, Json(written.remove(0))),
        Err(refused) => refused,
    }
}

/// Creates only, which is all the board's publisher asks for.
async fn apply_writes(
    State(repo): State<Repo>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let writes = body["writes"].as_array().cloned().unwrap_or_default();
    let creates = "com.atproto.space.applyWrites#create";
    if writes.iter().any(|w| w["$type"] != creates) {
        return refused(StatusCode::BAD_REQUEST, "InvalidRequest");
    }
    let all: Vec<_> = writes
        .iter()
        .map(|w| {
            (
                w["collection"].clone(),
                w["rkey"].clone(),
                w["value"].clone(),
            )
        })
        .collect();
    match write(&repo, &headers, &body, "space.applyWrites", &all, true) {
        Ok(results) => (StatusCode::OK, Json(json!({ "results": results }))),
        Err(refused) => refused,
    }
}

/// One commit: every record is taken or none is. `(collection, rkey, record)`.
fn write(
    repo: &Repo,
    headers: &HeaderMap,
    body: &Value,
    method: &str,
    records: &[(Value, Value, Value)],
    fresh: bool,
) -> Result<Vec<Value>, (StatusCode, Json<Value>)> {
    if !authorized(headers) {
        return Err(refused(StatusCode::UNAUTHORIZED, "AuthenticationRequired"));
    }
    repo.calls
        .lock()
        .expect("calls")
        .push((method.to_string(), records.len()));
    let Some(space) = body["space"].as_str() else {
        return Err(refused(StatusCode::BAD_REQUEST, "InvalidRequest"));
    };
    if body["repo"].as_str() != Some(DID) {
        return Err(refused(StatusCode::BAD_REQUEST, "InvalidRequest"));
    }
    // As the real one: a write into a space that is gone is taken, into a repo
    // nobody can be given a credential for. Kept apart, so a test can tell.
    let mut spaces = repo.spaces.lock().expect("spaces");
    let mut orphans = repo.orphans.lock().expect("orphans");
    let held = match spaces.get_mut(space) {
        Some(space) => &mut space.records,
        None => &mut *orphans,
    };
    let mut taken = Vec::new();
    for (collection, rkey, value) in records {
        let (Some(collection), Some(rkey)) = (collection.as_str(), rkey.as_str()) else {
            return Err(refused(StatusCode::BAD_REQUEST, "InvalidRequest"));
        };
        if value["$type"].as_str() != Some(collection) {
            return Err(refused(StatusCode::BAD_REQUEST, "InvalidRequest"));
        }
        if let Some(refused) = unstorable(value) {
            return Err(refused);
        }
        let there = |r: &Record| r.collection == collection && r.rkey == rkey;
        if fresh && (held.iter().any(there) || taken.iter().any(there)) {
            return Err(refused(StatusCode::BAD_REQUEST, "RecordAlreadyExists"));
        }
        taken.push(Record {
            collection: collection.to_string(),
            rkey: rkey.to_string(),
            value: value.clone(),
        });
    }
    let mut results = Vec::new();
    for record in taken {
        held.retain(|r| !(r.collection == record.collection && r.rkey == record.rkey));
        let uri = format!("{space}/{DID}/{}/{}", record.collection, record.rkey);
        results.push(json!({ "uri": uri, "cid": record.cid() }));
        held.push(record);
    }
    Ok(results)
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
    let mut spaces = repo.spaces.lock().expect("spaces");
    // Nothing to delete is no error, of the real one either.
    if let Some(space) = spaces.get_mut(body["space"].as_str().unwrap_or_default()) {
        space.records.retain(|r| {
            !(Some(r.collection.as_str()) == collection && Some(r.rkey.as_str()) == rkey)
        });
    }
    repo.calls
        .lock()
        .expect("calls")
        .push(("space.deleteRecord".to_string(), 1));
    (StatusCode::OK, Json(json!({})))
}

/// What the real one refuses of a record before it looks at anything else: a
/// number atproto data cannot hold (found against the alpha, 0.5.32), and a
/// request past its size limit.
fn unstorable(record: &Value) -> Option<(StatusCode, Json<Value>)> {
    fn refuses(json: &Value) -> bool {
        match json {
            Value::Number(n) => {
                let whole = n.as_i64().or(n.as_u64().map(|u| u as i64));
                whole.is_none_or(|i| i.unsigned_abs() >= 1 << 53)
            }
            Value::Array(items) => items.iter().any(refuses),
            Value::Object(fields) => fields.values().any(refuses),
            _ => false,
        }
    }
    if record.to_string().len() > 1_000_000 {
        return Some(refused(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PayloadTooLargeError",
        ));
    }
    refuses(record).then(|| refused(StatusCode::BAD_REQUEST, "InvalidRequest"))
}

/// The account's DID document, under `/plc` as a directory would serve it.
async fn did_document(
    State(repo): State<Repo>,
    Path(did): Path<String>,
) -> (StatusCode, Json<Value>) {
    if did != DID && did != crate::STRANGER {
        return refused(StatusCode::NOT_FOUND, "NotFound");
    }
    let key = atproto_spaces::keys::p256_multikey(repo.key.verifying_key());
    let document = json!({
        "id": did,
        "verificationMethod": [{
            "id": format!("{did}#atproto"),
            "type": "Multikey",
            "controller": did,
            "publicKeyMultibase": key,
        }],
        "service": [],
    });
    (StatusCode::OK, Json(document))
}

/// The token a PDS calls a service under for its account: signed by the
/// account's `#atproto` key, for one service and one method, for a minute.
pub(crate) fn service_token(
    key: &p256::ecdsa::SigningKey,
    issuer: &str,
    audience: &str,
    method: &str,
) -> String {
    let now = atproto_spaces::jwt::now();
    atproto_spaces::jwt::sign_es256(
        &json!({"typ": "JWT", "alg": "ES256"}),
        &json!({
            "iss": issuer, "aud": audience, "lxm": method,
            "iat": now, "exp": now + 60, "jti": atproto_spaces::jwt::nonce(),
        }),
        key,
    )
}

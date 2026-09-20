//! A `did:plc` directory that keeps the last operation it was sent for each DID
//! and serves the document that operation describes. It verifies no signature
//! and no DID: it is for a PDS and an AppView on one machine to agree on who a
//! made-up account is, which the public directory is no place for.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;

type Kept = Arc<RwLock<BTreeMap<String, Value>>>;

pub fn router() -> Router {
    Router::new()
        .route(
            "/_health",
            get(|| async { Json(json!({"version": "fake"})) }),
        )
        .route("/{did}", get(document).post(keep))
        .route("/{did}/data", get(data))
        .route("/{did}/log/last", get(last))
        .with_state(Kept::default())
}

async fn keep(
    State(kept): State<Kept>,
    Path(did): Path<String>,
    Json(op): Json<Value>,
) -> StatusCode {
    if !did.starts_with("did:plc:") || op.get("type").is_none() {
        return StatusCode::BAD_REQUEST;
    }
    kept.write().await.insert(did, op);
    StatusCode::OK
}

async fn last(
    State(kept): State<Kept>,
    Path(did): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    kept.read()
        .await
        .get(&did)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn data(
    State(kept): State<Kept>,
    Path(did): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let op = last(State(kept), Path(did.clone())).await?.0;
    Ok(Json(json!({
        "did": did,
        "verificationMethods": op["verificationMethods"],
        "rotationKeys": op["rotationKeys"],
        "alsoKnownAs": op["alsoKnownAs"],
        "services": op["services"],
    })))
}

async fn document(
    State(kept): State<Kept>,
    Path(did): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let op = last(State(kept), Path(did.clone())).await?.0;
    Ok(Json(document_of(&did, &op)))
}

/// The DID document an operation describes, as the real directory renders it.
pub fn document_of(did: &str, op: &Value) -> Value {
    let keys: Vec<Value> = op["verificationMethods"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, key)| {
            Some(json!({
                "id": format!("{did}#{name}"),
                "type": "Multikey",
                "controller": did,
                "publicKeyMultibase": key.as_str()?.strip_prefix("did:key:")?,
            }))
        })
        .collect();
    let services: Vec<Value> = op["services"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, service)| {
            json!({
                "id": format!("#{name}"),
                "type": service["type"],
                "serviceEndpoint": service["endpoint"],
            })
        })
        .collect();
    json!({
        "@context": [
            "https://www.w3.org/ns/did/v1",
            "https://w3id.org/security/multikey/v1",
            "https://w3id.org/security/suites/secp256k1-2019/v1",
        ],
        "id": did,
        "alsoKnownAs": op["alsoKnownAs"],
        "verificationMethod": keys,
        "service": services,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_is_rendered_as_the_document_a_resolver_expects() {
        let op = json!({
            "type": "plc_operation",
            "rotationKeys": ["did:key:zRotation"],
            "verificationMethods": {"atproto": "did:key:zQ3shSigning"},
            "alsoKnownAs": ["at://alice.test"],
            "services": {"atproto_pds": {
                "type": "AtprotoPersonalDataServer", "endpoint": "http://localhost:2583"
            }},
            "prev": null, "sig": "unchecked",
        });
        let doc = document_of("did:plc:abc", &op);
        assert_eq!(doc["id"], "did:plc:abc");
        assert_eq!(doc["alsoKnownAs"][0], "at://alice.test");
        assert_eq!(doc["verificationMethod"][0]["id"], "did:plc:abc#atproto");
        assert_eq!(
            doc["verificationMethod"][0]["publicKeyMultibase"],
            "zQ3shSigning"
        );
        assert_eq!(doc["service"][0]["id"], "#atproto_pds");
        assert_eq!(
            doc["service"][0]["serviceEndpoint"],
            "http://localhost:2583"
        );
    }
}

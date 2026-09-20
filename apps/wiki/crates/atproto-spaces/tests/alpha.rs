//! The crate against a real spaces PDS, which `scripts/test-spaces.nu` starts in
//! a container beside `fake-plc` (`SPACES_ALPHA_PDS`, `SPACES_ALPHA_PLC`). The
//! proposal promises breaking changes; this is where they show.
//!
//! The test stands in for the AppView: it is the space's managing app, which
//! the PDS asks who may read and write, and the service the PDS tells of
//! writes. Both calls come under a service token, checked as the AppView will.

use atproto_spaces::client::{Auth, Host};
use atproto_spaces::credential::Credential;
use atproto_spaces::directory::Directory;
use atproto_spaces::sync::{Change, Copy, Pulled, pull};
use atproto_spaces::{jwt, service_auth};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio::sync::Mutex;

const SPACE_TYPE: &str = "wiki.radikal.context";
const CHECK: &str = "com.atproto.simplespace.checkUserAccess";
const NOTIFY: &str = "com.atproto.space.notifyWrite";

#[derive(Clone)]
struct StandIn {
    directory: Directory,
    /// `did#fragment`, as the space names its managing app.
    service: String,
    /// Who the roster admits.
    members: Arc<BTreeSet<String>>,
    asked: Arc<Mutex<Vec<(String, String)>>>,
    notified: Arc<Mutex<Vec<Value>>>,
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get("authorization")?.to_str().ok()
}

async fn check_user_access(
    State(app): State<StandIn>,
    headers: HeaderMap,
    Query(q): Query<BTreeMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    service_auth::caller(&app.directory, bearer(&headers), &app.service, CHECK)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let user = q.get("user").cloned().unwrap_or_default();
    app.asked
        .lock()
        .await
        .push((user.clone(), q.get("access").cloned().unwrap_or_default()));
    Ok(Json(json!({"authorized": app.members.contains(&user)})))
}

async fn notify_write(
    State(app): State<StandIn>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    service_auth::caller(&app.directory, bearer(&headers), &app.service, NOTIFY)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    app.notified.lock().await.push(body);
    Ok(Json(json!({})))
}

async fn account(http: &reqwest::Client, pds: &str, name: &str) -> (String, String) {
    let body = json!({
        "handle": format!("{name}.test"), "email": format!("{name}@wiki.test"),
        "password": "a-password-for-a-made-up-account",
    });
    let said: Value = http
        .post(format!("{pds}/xrpc/com.atproto.server.createAccount"))
        .json(&body)
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a spaces PDS: run scripts/test-spaces.nu"]
async fn the_appviews_part_of_a_space_against_a_real_pds() {
    let pds_url = std::env::var("SPACES_ALPHA_PDS").expect("SPACES_ALPHA_PDS");
    let plc_url = std::env::var("SPACES_ALPHA_PLC").expect("SPACES_ALPHA_PLC");
    let http = reqwest::Client::new();
    let pds = Host::new(http.clone(), &pds_url);
    let directory = Directory::new(http.clone(), &plc_url);
    let run = &jwt::nonce()[..8];

    let (org, org_session) = account(&http, &pds_url, &format!("wiki{run}")).await;
    let (alice, alice_session) = account(&http, &pds_url, &format!("alice{run}")).await;
    let (bob, bob_session) = account(&http, &pds_url, &format!("bob{run}")).await;

    // The stand-in, under a DID of its own in the directory. A DID needs 24
    // characters of base32 and nothing else to be one here.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let endpoint = format!("http://{}", listener.local_addr().expect("an address"));
    let app_did = format!("did:plc:{:a<24}", format!("app{run}"));
    let op = json!({
        "type": "plc_operation", "prev": null, "sig": "unchecked",
        "rotationKeys": [], "alsoKnownAs": [],
        "verificationMethods": {},
        "services": {"wiki_appview": {"type": "WikiAppView", "endpoint": endpoint}},
    });
    let registered = http
        .post(format!("{plc_url}/{app_did}"))
        .json(&op)
        .send()
        .await
        .expect("the directory");
    assert!(registered.status().is_success(), "{registered:?}");
    let app = StandIn {
        directory: directory.clone(),
        service: format!("{app_did}#wiki_appview"),
        members: Arc::new([org.clone(), alice.clone()].into()),
        asked: Arc::default(),
        notified: Arc::default(),
    };
    let router = Router::new()
        .route(
            &format!("/xrpc/{CHECK}"),
            axum::routing::get(check_user_access),
        )
        .route(
            &format!("/xrpc/{NOTIFY}"),
            axum::routing::post(notify_write),
        )
        .with_state(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await });

    // A context's space, under the organization, with the stand-in deciding.
    let space = pds
        .create_managed_space(
            &org_session,
            SPACE_TYPE,
            &format!("c-{run}"),
            &app.service,
            &[],
        )
        .await
        .expect("a space");
    assert_eq!(space, format!("at://{org}/space/{SPACE_TYPE}/c-{run}"));

    let profile = json!({"$type": "wiki.radikal.contextProfile", "kind": "group", "name": "Hovedbestyrelsen",
                         "slug": "hb", "createdAt": "2026-09-20T12:00:00.000Z"});
    let node = |title: &str| {
        json!({"$type": "wiki.radikal.node", "kind": "folder", "title": title,
                                    "slug": title, "createdAt": "2026-09-20T12:00:00.000Z"})
    };
    let put = |rkey: &'static str, collection: &'static str, record: Value| {
        let (pds, session, space, org) = (&pds, &org_session, &space, &org);
        async move {
            pds.put_record(session, space, org, collection, rkey, &record)
                .await
                .expect("a record")
        }
    };
    put("self", "wiki.radikal.contextProfile", profile).await;
    put("d-bilag", "wiki.radikal.node", node("bilag")).await;
    put("d-forslag", "wiki.radikal.node", node("forslag")).await;

    // The AppView reads through the organization's own session.
    let credential = Credential::obtain(&pds, &org_session, &pds, &space, None)
        .await
        .expect("a credential");
    assert!(!credential.runs_out_within(60 * 60), "two hours, as issued");
    let expires = pds
        .register_notify(credential.auth(), &space, &app.service)
        .await
        .expect("a registration");
    assert!(expires.starts_with("20"), "{expires}");

    // A first pull has no revision to follow from, so it is everything.
    let org_key = directory
        .resolve(&org)
        .await
        .expect("the organization")
        .signing_key;
    let mut copy = Copy::default();
    let first = pull(&pds, credential.auth(), &space, &org, &org_key, &mut copy)
        .await
        .expect("a pull");
    let Pulled::Everything(records) = first else {
        panic!("{first:?}");
    };
    let held: BTreeSet<&str> = records.iter().map(|r| r.rkey.as_str()).collect();
    assert_eq!(held, ["d-bilag", "d-forslag", "self"].into());

    // Then an edit and a removal arrive as that, and the copy is the host's.
    put("d-bilag", "wiki.radikal.node", node("bilag-og-noter")).await;
    pds.delete_record(&org_session, &space, &org, "wiki.radikal.node", "d-forslag")
        .await
        .expect("a delete");
    let second = pull(&pds, credential.auth(), &space, &org, &org_key, &mut copy)
        .await
        .expect("a pull");
    let Pulled::Changes(changes) = second else {
        panic!("{second:?}");
    };
    assert!(
        matches!(&changes[..], [
            Change::Put { rkey, value, .. },
            Change::Delete { rkey: gone, .. },
        ] if rkey == "d-bilag" && value["title"] == "bilag-og-noter" && gone == "d-forslag"),
        "{changes:?}"
    );
    let again = pull(&pds, credential.auth(), &space, &org, &org_key, &mut copy)
        .await
        .expect("a pull");
    assert_eq!(again, Pulled::Nothing);

    // A copy that is wrong is found out by the hash, and replaced whole.
    let mut wrong = copy.clone();
    wrong.hash.add("wiki.radikal.node", "d-never", "bafy-never");
    let healed = pull(&pds, credential.auth(), &space, &org, &org_key, &mut wrong)
        .await
        .expect("a pull");
    assert!(
        matches!(healed, Pulled::Everything(ref all) if all.len() == 2),
        "{healed:?}"
    );
    assert_eq!(wrong, copy);

    // A member's write is told to the stand-in; an outsider's is asked about,
    // refused, and neither told nor listed. Both are in their own repos all the
    // same, which is why what syncs a space has to judge every record itself.
    let comment = json!({"$type": "wiki.radikal.comment", "text": "Enig", "createdAt": "2026-09-20T12:01:00.000Z",
                         "subject": {"uri": format!("{space}/{org}/wiki.radikal.node/d-bilag"), "cid": "bafy"}});
    for (did, session) in [(&alice, &alice_session), (&bob, &bob_session)] {
        pds.put_record(
            session,
            &space,
            did,
            "wiki.radikal.comment",
            "k-1",
            &comment,
        )
        .await
        .expect("anyone may write into their own repo");
    }
    let mut told = Vec::new();
    for _ in 0..50 {
        told = app.notified.lock().await.clone();
        if told.iter().any(|n| n["repo"] == alice.as_str()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert!(
        told.iter()
            .any(|n| n["repo"] == alice.as_str() && n["space"] == space.as_str()),
        "{told:?}"
    );
    assert!(told.iter().all(|n| n["repo"] != bob.as_str()), "{told:?}");
    let writers: BTreeSet<String> = pds
        .writers(credential.auth(), &space)
        .await
        .expect("the writer set")
        .into_iter()
        .map(|w| w.did)
        .collect();
    assert_eq!(writers, [org.clone(), alice.clone()].into());
    let asked = app.asked.lock().await.clone();
    assert!(asked.contains(&(bob.clone(), "write".into())), "{asked:?}");

    // A member's repo syncs like the organization's, under their own key.
    let alice_key = directory.resolve(&alice).await.expect("alice").signing_key;
    let hers = pull(
        &pds,
        credential.auth(),
        &space,
        &alice,
        &alice_key,
        &mut Copy::default(),
    )
    .await
    .expect("a pull");
    assert!(
        matches!(hers, Pulled::Everything(ref all) if all.len() == 1),
        "{hers:?}"
    );
    let forged = pull(
        &pds,
        credential.auth(),
        &space,
        &alice,
        &org_key,
        &mut Copy::default(),
    )
    .await;
    assert!(forged.is_err(), "a commit is its author's or nobody's");

    // Whom the roster does not admit gets no credential.
    let refused = Credential::obtain(&pds, &bob_session, &pds, &space, None).await;
    assert_eq!(
        refused.err().as_ref().and_then(|e| e.xrpc_name()),
        Some("UserNotAuthorized")
    );

    // A file: named by a record, served to a credential and to nothing else.
    let blob = pds
        .upload_blob(
            &org_session,
            b"%PDF a made-up agenda".to_vec(),
            "application/pdf",
        )
        .await
        .expect("a blob");
    let mut file = node("dagsorden");
    file["kind"] = json!("file");
    file["file"] = blob.clone();
    put("d-dagsorden", "wiki.radikal.node", file).await;
    let cid = blob["ref"]["$link"].as_str().expect("a cid");
    let bytes = pds
        .blob(credential.auth(), &space, &org, cid)
        .await
        .expect("the blob");
    assert_eq!(bytes, b"%PDF a made-up agenda");
    let public = http
        .get(format!(
            "{pds_url}/xrpc/com.atproto.sync.getBlob?did={org}&cid={cid}"
        ))
        .send()
        .await
        .expect("the PDS");
    assert!(
        !public.status().is_success(),
        "a space's file is not the world's"
    );
    let bare = pds.writers(Auth::Bearer("not-a-credential"), &space).await;
    assert!(bare.is_err());
}

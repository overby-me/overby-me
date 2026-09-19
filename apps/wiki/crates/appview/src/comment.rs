//! Taking a comment away, and bringing it back.
//!
//! The interim's rule, carried over: a comment that has been answered is
//! emptied and stays, because deleting it would take everyone who replied
//! along; any other goes to its context's bin, since a mis-click on somebody
//! else's argument has to be undoable.

use crate::AppState;
use crate::authz::Authz;
use crate::live::Topic;
use crate::session::Caller;
use crate::xrpc::{conflict, err, write_failed};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CommentIdBody {
    pub id: String,
}

/// A comment and the caller's right to delete it: its author's, or an owner's
/// of its context. Anyone else is told it is not there, in the bin or out.
async fn deletable_comment(
    state: &AppState,
    id: &str,
    did: &str,
    what: &str,
) -> Result<crate::store::CommentMeta, Response> {
    let missing = || err(StatusCode::NOT_FOUND, "NotFound", "no such comment");
    let meta = match crate::Store::new(state.db.clone()).comment_meta(id).await {
        Ok(Some(meta)) => meta,
        Ok(None) => return Err(missing()),
        Err(e) => return Err(write_failed(what, e)),
    };
    match Authz::new(state.db.clone())
        .standing(&meta.context_id, meta.author_did.as_deref(), did)
        .await
    {
        Ok(standing) if standing.may_delete() => Ok(meta),
        Ok(_) => Err(missing()),
        Err(e) => Err(write_failed(what, e)),
    }
}

/// `com.example.wiki.deleteComment` (procedure): its author, or an owner of its
/// context, takes a comment away. One that has been answered is emptied and
/// stays, because the answers hang on it: deleting it outright would take
/// everyone who replied along. Any other goes to the context's bin.
pub async fn delete_comment(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<CommentIdBody>,
) -> Response {
    let what = "deleteComment";
    let meta = match deletable_comment(&state, &body.id, &did, what).await {
        Ok(meta) => meta,
        Err(refusal) => return refusal,
    };
    let gone = match crate::Store::new(state.db.clone())
        .delete_comment(&body.id)
        .await
    {
        Ok(Some(gone)) => gone,
        Ok(None) => return err(StatusCode::NOT_FOUND, "NotFound", "no such comment"),
        Err(e) => return write_failed(what, e),
    };
    let outcome = match gone {
        crate::store::CommentGone::Binned => "binned",
        crate::store::CommentGone::Emptied { image } => {
            if let Some(image) = image
                && let Err(e) = crate::blob::forget_if_unreferenced(&state, &image).await
            {
                tracing::warn!("an emptied comment left blob {image} behind: {e}");
            }
            "emptied"
        }
    };
    state.publish(Topic::Context(meta.context_id), "comment", &meta.on_id);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "outcome": outcome })),
    )
        .into_response()
}

/// `com.example.wiki.restoreComment` (procedure): bring a comment back from the
/// bin, with whatever went there with it.
pub async fn restore_comment(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<CommentIdBody>,
) -> Response {
    let what = "restoreComment";
    let meta = match deletable_comment(&state, &body.id, &did, what).await {
        Ok(meta) if meta.bin_entry => meta,
        Ok(_) => {
            return err(
                StatusCode::NOT_FOUND,
                "NotFound",
                "nothing in the bin by that id",
            );
        }
        Err(refusal) => return refusal,
    };
    let store = crate::Store::new(state.db.clone());
    match store.comment_parent_stands(&meta.on_id).await {
        Ok(true) => {}
        Ok(false) => return conflict("ParentInBin", "restore what it answers first"),
        Err(e) => return write_failed(what, e),
    }
    match store.restore_comment(&body.id).await {
        Ok(restored) => {
            state.publish(Topic::Context(meta.context_id), "comment", &meta.on_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "restored": restored })),
            )
                .into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

/// `com.example.wiki.purgeComment` (procedure): delete for good a comment that
/// is in the bin, with the reactions to it and the picture it held. Its author
/// may, as well as an owner: what someone wrote is theirs to have gone.
pub async fn purge_comment(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<CommentIdBody>,
) -> Response {
    let what = "purgeComment";
    let meta = match deletable_comment(&state, &body.id, &did, what).await {
        Ok(meta) if meta.bin_entry => meta,
        Ok(_) => {
            return err(
                StatusCode::NOT_FOUND,
                "NotFound",
                "nothing in the bin by that id",
            );
        }
        Err(refusal) => return refusal,
    };
    match crate::Store::new(state.db.clone())
        .purge_comment(&body.id)
        .await
    {
        Ok((purged, images)) => {
            for image in images {
                if let Err(e) = crate::blob::forget_if_unreferenced(&state, &image).await {
                    tracing::warn!("a purged comment left blob {image} behind: {e}");
                }
            }
            // For whoever has the bin open.
            state.publish(Topic::Context(meta.context_id), "comment", &meta.on_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "purged": purged })),
            )
                .into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

#[cfg(test)]
mod tests {
    use crate::AppState;
    use crate::blob::tests::{fetch, state as blob_state, upload};
    use crate::router;
    use crate::xrpc::tests::{get_as, join, post, token_for};
    use axum::http::StatusCode;
    use serde_json::{Value, json};

    const POST: &str = "/xrpc/com.example.wiki.postComment";
    const DELETE: &str = "/xrpc/com.example.wiki.deleteComment";
    const RESTORE: &str = "/xrpc/com.example.wiki.restoreComment";
    const PURGE: &str = "/xrpc/com.example.wiki.purgeComment";

    /// Post a comment on `on` and return its id.
    async fn say(state: &AppState, who: &str, on: &str, text: &str) -> String {
        let (status, v) = post(
            router(state.clone()),
            POST,
            Some(who),
            json!({"on_id": on, "text": text}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        v["id"].as_str().expect("id").to_string()
    }

    async fn act(state: &AppState, method: &str, who: &str, id: &str) -> (StatusCode, Value) {
        post(router(state.clone()), method, Some(who), json!({"id": id})).await
    }

    async fn thread(state: &AppState, who: &str, on: &str) -> Vec<Value> {
        let uri = format!("/xrpc/com.example.wiki.getComments?on={on}");
        let (status, v) = get_as(router(state.clone()), &uri, who).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        v["comments"].as_array().expect("comments").clone()
    }

    async fn bin(state: &AppState, who: &str) -> Vec<Value> {
        let uri = "/xrpc/com.example.wiki.listDeleted?context=c9";
        let (status, v) = get_as(router(state.clone()), uri, who).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let listed = v["deleted"].as_array().expect("deleted").clone();
        listed
            .into_iter()
            .filter(|entry| entry["node"] == "comment")
            .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_comment_shows_a_picture_of_its_authors_from_its_own_context() {
        let state = blob_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        join(&state, "did:plc:alice", "c10").await;
        let (_, hers) = upload(&state, &alice, "c9", "image/png", b"png").await;
        let (_, elsewhere) = upload(&state, &alice, "c10", "image/png", b"other").await;
        let (_, a_pdf) = upload(&state, &alice, "c9", "application/pdf", b"%PDF").await;
        for uploaded in [&hers, &elsewhere, &a_pdf] {
            assert!(uploaded["id"].is_string(), "{uploaded}");
        }
        let with = |text: &str, image: &Value| json!({"on_id": "s1", "text": text, "image": image});

        let (status, v) = post(
            router(state.clone()),
            POST,
            Some(&alice),
            with("", &hers["id"]),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "a picture says enough: {v}");
        let shown = thread(&state, &bob, "s1").await;
        let shown = shown
            .iter()
            .find(|k| k["id"] == v["id"])
            .expect("the comment");
        assert_eq!(shown["image"], hers["id"]);
        assert_eq!(shown["root_id"], "s1");

        for (why, who, body) in [
            ("somebody else's picture", &bob, with("Se", &hers["id"])),
            (
                "one from another context",
                &alice,
                with("Se", &elsewhere["id"]),
            ),
            (
                "a file that is no picture",
                &alice,
                with("Se", &a_pdf["id"]),
            ),
            ("no such file", &alice, with("Se", &json!("nope"))),
            (
                "nothing said and nothing shown",
                &alice,
                json!({"on_id": "s1", "text": "  "}),
            ),
            (
                "a wall of text",
                &alice,
                json!({"on_id": "s1", "text": "x".repeat(10_001)}),
            ),
        ] {
            let (status, v) = post(router(state.clone()), POST, Some(who), body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {v}");
        }
    }

    /// Bob's comment, which nobody has answered: it goes to the bin, comes back
    /// whole, and goes for good only when purged, picture and reactions with it.
    #[tokio::test(flavor = "current_thread")]
    async fn an_unanswered_comment_goes_to_the_bin_and_can_come_back() {
        let state = blob_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let zoe = token_for(&state, "did:plc:zoe").await;
        let (_, picture) = upload(&state, &bob, "c9", "image/png", b"png").await;
        let (_, v) = post(
            router(state.clone()),
            POST,
            Some(&bob),
            json!({"on_id": "s1", "text": "Punkt 3 mangler", "image": picture["id"]}),
        )
        .await;
        let id = v["id"].as_str().expect("id").to_string();
        let react = json!({"subject": id, "emoji": "👍"});
        let reacted = "/xrpc/com.example.wiki.addReaction";
        assert_eq!(
            post(router(state.clone()), reacted, Some(&alice), react)
                .await
                .0,
            StatusCode::OK
        );

        let (status, _) = act(&state, DELETE, &zoe, &id).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "not hers, and not her context"
        );
        let (status, v) = act(&state, DELETE, &bob, &id).await;
        assert_eq!(
            (status, &v["outcome"]),
            (StatusCode::OK, &json!("binned")),
            "{v}"
        );
        assert!(
            thread(&state, &alice, "s1")
                .await
                .iter()
                .all(|k| k["id"] != id.as_str())
        );
        let reactions = format!("/xrpc/com.example.wiki.getReactions?subject={id}");
        let (_, v) = get_as(router(state.clone()), &reactions, &alice).await;
        assert_eq!(
            v["reactions"],
            json!([]),
            "what is in the bin has no reactions to show"
        );
        let (status, v) = post(
            router(state.clone()),
            POST,
            Some(&alice),
            json!({"on_id": id, "text": "Hvad?"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "nor can it be answered: {v}");

        let listed = bin(&state, &alice).await;
        assert_eq!(listed.len(), 1, "an owner sees it in the bin");
        assert_eq!(listed[0]["title"], "Punkt 3 mangler");
        assert_eq!(listed[0]["path"], "closed/secret_minutes");
        assert_eq!(bin(&state, &bob).await.len(), 1, "and so does who wrote it");

        let (status, v) = act(&state, RESTORE, &bob, &id).await;
        assert_eq!((status, &v["restored"]), (StatusCode::OK, &json!(1)), "{v}");
        let back = thread(&state, &alice, "s1").await;
        let back = back
            .iter()
            .find(|k| k["id"] == id.as_str())
            .expect("it is back");
        assert_eq!(back["image"], picture["id"]);
        let (_, v) = get_as(router(state.clone()), &reactions, &alice).await;
        assert_eq!(v["reactions"].as_array().expect("reactions").len(), 1);

        let (status, _) = act(&state, PURGE, &bob, &id).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "only what is in the bin is purged"
        );
        assert_eq!(
            act(&state, DELETE, &alice, &id).await.0,
            StatusCode::OK,
            "an owner may too"
        );
        let (status, v) = act(&state, PURGE, &bob, &id).await;
        assert_eq!((status, &v["purged"]), (StatusCode::OK, &json!(1)), "{v}");
        assert!(bin(&state, &alice).await.is_empty());
        let uri = format!("/blob/{}", picture["id"].as_str().expect("id"));
        assert_eq!(
            fetch(&state, &uri, Some(&bob), None).await.0,
            StatusCode::NOT_FOUND
        );
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query(
                "SELECT count(*) FROM reaction WHERE subject_uri = ?1",
                [id.as_str()],
            )
            .await
            .expect("q");
        let left = rows.next().await.expect("next").expect("a count");
        assert_eq!(
            left.get::<i64>(0).expect("count"),
            0,
            "its reactions went with it"
        );
    }

    /// Deleting a comment must not take along everyone who answered it. It is
    /// emptied where it stands: no words, no name, no picture, no reactions.
    #[tokio::test(flavor = "current_thread")]
    async fn an_answered_comment_is_emptied_and_its_answers_stay() {
        let state = blob_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let first = say(&state, &bob, "s1", "Jeg er imod").await;
        let answer = say(&state, &alice, &first, "Hvorfor?").await;
        let react = json!({"subject": first, "emoji": "👎"});
        post(
            router(state.clone()),
            "/xrpc/com.example.wiki.addReaction",
            Some(&alice),
            react,
        )
        .await;

        let (status, v) = act(&state, DELETE, &bob, &first).await;
        assert_eq!(
            (status, &v["outcome"]),
            (StatusCode::OK, &json!("emptied")),
            "{v}"
        );
        let on_doc = thread(&state, &alice, "s1").await;
        let emptied = on_doc
            .iter()
            .find(|k| k["id"] == first.as_str())
            .expect("it stays");
        assert_eq!(emptied["tombstone"], true);
        assert_eq!(emptied["text"], "");
        assert_eq!(
            emptied["author"],
            json!({"kind": "free_text", "display": ""})
        );
        let answers = thread(&state, &alice, &first).await;
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["id"], answer.as_str());
        assert_eq!(
            answers[0]["root_id"], "s1",
            "an answer hangs on the document too"
        );
        let reactions = format!("/xrpc/com.example.wiki.getReactions?subject={first}");
        let (_, v) = get_as(router(state.clone()), &reactions, &alice).await;
        assert_eq!(v["reactions"], json!([]));
        assert!(
            bin(&state, &alice).await.is_empty(),
            "emptying is not binning"
        );

        let (status, _) = act(&state, DELETE, &bob, &first).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "there is nothing left of it to delete"
        );

        // With its one answer in the bin it could be binned, were it not emptied
        // already; the answer cannot come back under a comment that is binned.
        let second = say(&state, &bob, "s1", "Et andet punkt").await;
        let reply = say(&state, &alice, &second, "Ja").await;
        assert_eq!(
            act(&state, DELETE, &alice, &reply).await.1["outcome"],
            "binned"
        );
        assert_eq!(
            act(&state, DELETE, &bob, &second).await.1["outcome"],
            "binned"
        );
        let (status, v) = act(&state, RESTORE, &alice, &reply).await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "ParentInBin");
        assert_eq!(act(&state, RESTORE, &bob, &second).await.0, StatusCode::OK);
        assert_eq!(act(&state, RESTORE, &alice, &reply).await.0, StatusCode::OK);
    }
}

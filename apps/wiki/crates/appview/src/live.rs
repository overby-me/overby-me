//! Live updates: what changed, to whoever may watch it.
//!
//! A change names the thing that changed and never what it changed to. A client
//! that hears one refetches through the gated reads, so nothing private travels
//! over the socket and a revoked reader learns at most that something moved.
//! This is the frontend's `use_live` model, which ignores every payload and
//! bumps a refresh counter, and the interim's `context_touch`: one signal per
//! context that every view in it shares (`docs/use-live-topic-inventory.md`).
//!
//! ## The protocol, over `/ws`, as JSON text frames
//!
//! - `{"op":"auth","session":"<token>"}`: who is listening. A browser cannot set
//!   a header on a WebSocket, and a token in the URL would reach the access log,
//!   so it is the first frame instead. Answered `{"op":"auth","ok":bool}`. It
//!   drops every subscription, which were granted to whoever listened before.
//! - `{"op":"sub","topic":"context:<id>"}`, answered with `"ok"`: granted when
//!   the listener may read that context. `user:<did>` is granted to that DID
//!   alone, and `public` to anyone.
//! - `{"op":"unsub","topic":"..."}`.
//! - `{"op":"ping"}`, answered `{"op":"pong"}`. A browser cannot send a
//!   WebSocket ping, and a proxy drops a socket that says nothing for a minute.
//! - From the server: `{"topic":"...","kind":"...","id":"..."}`, with a `"row"`
//!   where the change made a row that `id` does not name: a new comment's or
//!   reaction's own id, `id` being what it is to. A feed fetches that row.
//! - From the server, unasked: `{"op":"unsub","topic":"...","revoked":true}`. A
//!   grant is checked again whenever the context's membership changes, and a
//!   listener who may no longer read it hears this once and then nothing more.
//!
//! A listener that falls behind the broadcast buffer is disconnected; it
//! reconnects and refetches, which is the only honest recovery from a gap.

use crate::AppState;
use crate::session::Sessions;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Who may hear a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Topic {
    /// Whoever may read this context.
    Context(String),
    /// This person alone: their invitations, their sessions.
    User(String),
    /// Anyone: the mirror of public records.
    Public,
}

impl Topic {
    pub fn wire(&self) -> String {
        match self {
            Topic::Context(id) => format!("context:{id}"),
            Topic::User(did) => format!("user:{did}"),
            Topic::Public => "public".to_string(),
        }
    }

    fn parse(wire: &str) -> Option<Topic> {
        if wire == "public" {
            return Some(Topic::Public);
        }
        let (scope, id) = wire.split_once(':')?;
        match (scope, id.is_empty()) {
            ("context", false) => Some(Topic::Context(id.to_string())),
            ("user", false) => Some(Topic::User(id.to_string())),
            _ => None,
        }
    }
}

/// Something changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub topic: Topic,
    /// What sort of thing: `node`, `comment`, `member`, `speak`, `record`.
    pub kind: &'static str,
    pub id: String,
    /// The row the change made, where `id` names what it was made on.
    pub row: Option<String>,
}

#[derive(Serialize)]
struct ChangeFrame<'a> {
    topic: &'a str,
    kind: &'a str,
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    row: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ClientFrame {
    Auth { session: String },
    Sub { topic: String },
    Unsub { topic: String },
    Ping,
}

/// One connection's standing: who it is and what it was granted.
struct Listener {
    state: AppState,
    did: Option<String>,
    topics: BTreeSet<String>,
}

impl Listener {
    /// Whether this listener may watch `topic`.
    async fn may_watch(&self, topic: &Topic) -> bool {
        match topic {
            Topic::Public => true,
            Topic::User(did) => self.did.as_deref() == Some(did.as_str()),
            Topic::Context(id) => crate::Store::new(self.state.db.clone())
                .read_context(id, self.did.as_deref())
                .await
                .is_ok_and(|found| found.is_some()),
        }
    }

    /// What to send for a change, if anything.
    ///
    /// A `member` change is when a grant can have ended: whoever was removed is
    /// told so, and is not told of anything in that context again.
    async fn hear(&mut self, change: &Change) -> Option<serde_json::Value> {
        let topic = change.topic.wire();
        if !self.topics.contains(&topic) {
            return None;
        }
        if change.kind == "member" && !self.may_watch(&change.topic).await {
            self.topics.remove(&topic);
            return Some(serde_json::json!({ "op": "unsub", "topic": topic, "revoked": true }));
        }
        serde_json::to_value(ChangeFrame {
            topic: &topic,
            kind: change.kind,
            id: &change.id,
            row: change.row.as_deref(),
        })
        .ok()
    }

    /// Act on a frame from the client, and say what to answer.
    async fn handle(&mut self, text: &str) -> serde_json::Value {
        let Ok(frame) = serde_json::from_str::<ClientFrame>(text) else {
            return serde_json::json!({ "op": "error", "message": "not a frame" });
        };
        match frame {
            ClientFrame::Auth { session } => {
                self.topics.clear();
                self.did = Sessions::new(self.state.db.clone())
                    .resolve(&session)
                    .await
                    .ok()
                    .flatten();
                serde_json::json!({ "op": "auth", "ok": self.did.is_some() })
            }
            ClientFrame::Sub { topic } => {
                let ok = match Topic::parse(&topic) {
                    Some(parsed) => self.may_watch(&parsed).await,
                    None => false,
                };
                if ok {
                    self.topics.insert(topic.clone());
                }
                serde_json::json!({ "op": "sub", "topic": topic, "ok": ok })
            }
            ClientFrame::Unsub { topic } => {
                self.topics.remove(&topic);
                serde_json::json!({ "op": "unsub", "topic": topic, "ok": true })
            }
            ClientFrame::Ping => serde_json::json!({ "op": "pong" }),
        }
    }
}

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| listen(socket, state))
}

async fn listen(mut socket: WebSocket, state: AppState) {
    let mut changes = state.changes.subscribe();
    let mut listener = Listener {
        state,
        did: None,
        topics: BTreeSet::new(),
    };
    loop {
        let outgoing = tokio::select! {
            change = changes.recv() => match change {
                Ok(change) => match listener.hear(&change).await {
                    Some(frame) => Message::Text(frame.to_string().into()),
                    None => continue,
                },
                // Lagged or closed: a gap cannot be papered over.
                Err(_) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    Message::Text(listener.handle(text.as_str()).await.to_string().into())
                }
                Some(Ok(Message::Ping(payload))) => Message::Pong(payload),
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => continue,
            },
        };
        if socket.send(outgoing).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Db};

    #[test]
    fn a_topic_round_trips_and_a_malformed_one_is_refused() {
        for topic in [
            Topic::Context("c1".into()),
            Topic::User("did:plc:alice".into()),
            Topic::Public,
        ] {
            assert_eq!(Topic::parse(&topic.wire()), Some(topic));
        }
        for bad in ["", "context:", "user:", "nodes:c1", "context"] {
            assert_eq!(Topic::parse(bad), None, "{bad:?}");
        }
    }

    async fn listener() -> Listener {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        let conn = db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO user (did) VALUES ('did:plc:bob');
             INSERT INTO context (id, kind, name, slug, path, visibility) \
               VALUES ('open', 'group', 'O', 'o', 'o', 'public');
             INSERT INTO context (id, kind, name, slug, path) \
               VALUES ('shut', 'group', 'S', 's', 's');
             INSERT INTO member (id, user_did, context_id) VALUES ('m', 'did:plc:bob', 'shut');",
        )
        .await
        .expect("seed");
        Listener {
            state: AppState::new(db, Config::default()),
            did: None,
            topics: BTreeSet::new(),
        }
    }

    async fn sub(l: &mut Listener, topic: &str) -> bool {
        let frame = serde_json::json!({ "op": "sub", "topic": topic }).to_string();
        l.handle(&frame).await["ok"] == true
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_stranger_watches_the_public_and_nothing_else() {
        let mut l = listener().await;
        assert!(sub(&mut l, "public").await);
        assert!(sub(&mut l, "context:open").await);
        assert!(
            !sub(&mut l, "context:shut").await,
            "a closed group was watched"
        );
        assert!(!sub(&mut l, "context:nowhere").await);
        assert!(!sub(&mut l, "user:did:plc:bob").await);
        assert_eq!(l.topics.len(), 2, "a refused topic must not be recorded");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_session_opens_what_its_owner_may_read_and_a_new_one_starts_over() {
        let mut l = listener().await;
        let token = Sessions::new(l.state.db.clone())
            .create("did:plc:bob")
            .await
            .expect("session");
        let auth =
            |session: &str| serde_json::json!({ "op": "auth", "session": session }).to_string();

        assert_eq!(l.handle(&auth("forged")).await["ok"], false);
        assert_eq!(l.handle(&auth(&token)).await["ok"], true);
        assert!(sub(&mut l, "context:shut").await);
        assert!(sub(&mut l, "user:did:plc:bob").await);
        assert!(
            !sub(&mut l, "user:did:plc:alice").await,
            "someone else's feed"
        );

        // Whoever authenticates next is not handed what bob was granted.
        assert_eq!(l.handle(&auth("forged")).await["ok"], false);
        assert!(l.topics.is_empty());
        assert!(!sub(&mut l, "context:shut").await);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn someone_put_out_of_a_group_stops_hearing_about_it() {
        let mut l = listener().await;
        let token = Sessions::new(l.state.db.clone())
            .create("did:plc:bob")
            .await
            .expect("session");
        l.handle(&serde_json::json!({ "op": "auth", "session": token }).to_string())
            .await;
        assert!(sub(&mut l, "context:shut").await);
        let change = |kind: &'static str| Change {
            topic: Topic::Context("shut".into()),
            kind,
            id: "x".into(),
            row: None,
        };
        // Someone else joining or leaving changes nothing for bob.
        let heard = l.hear(&change("member")).await.expect("a frame");
        assert_eq!(heard["kind"], "member");
        assert!(heard.get("row").is_none(), "{heard}");
        let answer = Change {
            row: Some("k2".into()),
            ..change("comment")
        };
        let heard = l.hear(&answer).await.expect("a frame");
        assert_eq!((&heard["id"], &heard["row"]), (&"x".into(), &"k2".into()));

        let conn = l.state.db.acquire().await.expect("conn");
        conn.execute("DELETE FROM member WHERE id = 'm'", ())
            .await
            .expect("bob is put out");
        let heard = l.hear(&change("member")).await.expect("a frame");
        assert_eq!(
            heard,
            serde_json::json!({ "op": "unsub", "topic": "context:shut", "revoked": true })
        );
        assert!(
            l.hear(&change("node")).await.is_none(),
            "still told that the group he was put out of is changing"
        );
        assert!(
            !sub(&mut l, "context:shut").await,
            "and let straight back in"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn nonsense_is_answered_not_obeyed() {
        let mut l = listener().await;
        assert_eq!(l.handle("not json").await["op"], "error");
        assert_eq!(l.handle(r#"{"op":"ping"}"#).await["op"], "pong");
        assert_eq!(l.handle(r#"{"op":"drop_tables"}"#).await["op"], "error");
    }

    // -- Over a real socket: a write made through the API reaches the listeners
    //    allowed to hear it, and only them. --

    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn serve(state: AppState) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, crate::router(state)).await;
        });
        addr
    }

    async fn say(socket: &mut Socket, frame: serde_json::Value) -> serde_json::Value {
        socket
            .send(WsMessage::Text(frame.to_string()))
            .await
            .expect("send");
        hear(socket).await.expect("an answer")
    }

    /// The next text frame, or `None` if nothing comes within a moment.
    async fn hear(socket: &mut Socket) -> Option<serde_json::Value> {
        let wait = std::time::Duration::from_millis(400);
        loop {
            match tokio::time::timeout(wait, socket.next()).await {
                Ok(Some(Ok(WsMessage::Text(text)))) => {
                    return serde_json::from_str(text.as_str()).ok();
                }
                Ok(Some(Ok(_))) => continue,
                _ => return None,
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_is_heard_by_the_members_of_its_context_and_by_nobody_else() {
        let state = listener().await.state;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute("UPDATE member SET role = 'owner' WHERE id = 'm'", ())
            .await
            .expect("owner");
        let token = Sessions::new(state.db.clone())
            .create("did:plc:bob")
            .await
            .expect("session");
        let addr = serve(state.clone()).await;
        let url = format!("ws://{addr}/ws");

        let (mut bob, _) = tokio_tungstenite::connect_async(&url).await.expect("bob");
        let (mut stranger, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("stranger");
        assert_eq!(
            say(
                &mut bob,
                serde_json::json!({"op": "auth", "session": token})
            )
            .await["ok"],
            true
        );
        let watch = serde_json::json!({"op": "sub", "topic": "context:shut"});
        assert_eq!(say(&mut bob, watch.clone()).await["ok"], true);
        assert_eq!(say(&mut stranger, watch).await["ok"], false);

        // Bob makes a folder in his closed group, over plain HTTP.
        let body = serde_json::json!({
            "context_id": "shut", "kind": "folder", "title": "Referater"
        });
        let request = http::Request::builder()
            .method("POST")
            .uri("/xrpc/com.example.wiki.createDocument")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::from(body.to_string()))
            .expect("request");
        let response = tower::ServiceExt::oneshot(crate::router(state.clone()), request)
            .await
            .expect("response");
        assert_eq!(response.status(), 200);

        let heard = hear(&mut bob).await.expect("bob hears his context change");
        assert_eq!(heard["topic"], "context:shut");
        assert_eq!(heard["kind"], "node");
        assert!(
            heard.get("title").is_none(),
            "a change carries no content: {heard}"
        );
        assert_eq!(
            hear(&mut stranger).await,
            None,
            "a listener who may not read the group heard it change"
        );
    }
}

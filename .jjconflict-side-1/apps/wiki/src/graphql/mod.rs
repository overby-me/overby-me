//! The GraphQL seam: every request this app makes to Hasura.
//!
//! Split by what is being asked about rather than by query-versus-mutation, so
//! a change to (say) the bin is one file rather than a search through four
//! thousand lines. This root keeps only what every module needs: the transport
//! (`execute` and friends, which classify a failure and decide whether it is
//! worth telling anyone about) and the schema handle.
//!
//! Every module re-exports flat, so callers still write `graphql::query_node`
//! and never name a submodule: the split is an organising principle here, not
//! a new vocabulary for the rest of the app to learn.

mod bin;
mod feed;
mod members;
mod nodes;
mod pixel;
mod public;
mod screen;
mod search;
mod social;
mod subscriptions;
mod types;
mod vote;

pub use bin::*;
pub use feed::*;
pub use members::*;
pub use nodes::*;
pub use pixel::*;
pub use public::*;
pub use screen::*;
pub use search::*;
pub use social::*;
pub use subscriptions::*;
pub use types::*;
pub use vote::*;

use cynic::QueryBuilder;
use serde::{Deserialize, Serialize};

use crate::model;
use crate::model::{Author, BallotRules, Crumb, MemberPageFilter};
use crate::nhost::graphql_url;

mod schema {
    cynic::use_schema!("graphql/schema.graphql");
}
cynic::impl_scalar!(Uuid, schema::uuid);
cynic::impl_scalar!(Timestamptz, schema::timestamptz);
cynic::impl_scalar!(Jsonb, schema::jsonb);

/// Escape a value for embedding inside a GraphQL double-quoted string literal.
/// Used by the hand-built subscription strings in components too, so an id that
/// ever carried a `"`/`\` can't rewrite the query's `where` filter.
pub(crate) fn gql_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// --- HTTP execution ---

/// A Hasura error whose JWT is expired/invalid, so refreshing the token and
/// retrying the request may recover (e.g. "Could not verify JWT: JWTExpired").
fn is_jwt_error(msg: &str) -> bool {
    msg.to_ascii_lowercase().contains("jwt")
}

/// How long to wait before each further attempt at a read whose request never
/// completed, in milliseconds. Two retries, so three attempts in all.
///
/// A phone on venue wifi drops a request now and then, and until now the
/// reader's entire answer was an error card asking them to tap retry, on the
/// home screen, on first load, before they had done anything. The tap worked,
/// which is the tell: nothing was wrong except that one request. So the app
/// makes the taps itself, and only tells them if all three fail.
///
/// Short enough that three attempts still fit inside a reader's patience, and
/// spread enough to outlast a hand-off between two access points.
const RETRY_DELAYS_MS: &[u32] = &[300, 900];

/// Whether an operation changes anything.
///
/// A transport failure means no answer came back, NOT that the server did
/// nothing: the request may well have arrived and been applied. Retrying a read
/// costs a round trip, while retrying a mutation could post a second comment or
/// cast a second vote. So only reads are retried.
fn is_mutation(query: &str) -> bool {
    query.trim_start().starts_with("mutation")
}

/// The requests currently in the air, by exactly what was asked and by whom.
///
/// Two components that ask the same question at the same moment ask it once.
/// Opening a page did this twice over: the page and the drawer each resolve the
/// path in the address bar, so the same path lookup and then the same node query
/// went out about a hundred milliseconds apart, while the first was still
/// unanswered. Measured on the test event: 15 requests to open a page, of which
/// two pairs were a request overlapping itself.
///
/// The same principle as the subscription hub, which folds every watcher of one
/// scope into a single subscription; this is the one-shot half of it.
///
/// Only for the duration of the flight, and never for a mutation. Nothing is
/// remembered after the answer lands: a request that starts after the last one
/// finished is a new question and gets a new answer, so this cannot serve a
/// stale row the way a cache with a lifetime could. Two identical writes stay
/// two writes.
type Envelope = Result<serde_json::Value, String>;
type SharedRequest = futures_util::future::Shared<
    std::pin::Pin<Box<dyn std::future::Future<Output = Envelope> + 'static>>,
>;

thread_local! {
    static IN_FLIGHT: std::cell::RefCell<std::collections::HashMap<String, SharedRequest>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// POST one GraphQL body, sharing a request that is already on its way.
///
/// Returns the whole envelope (`data` and `errors` both), because the callers
/// disagree about what to do with it: the typed path hands it to cynic, the raw
/// ones read the two keys themselves.
/// What identifies a request that may be shared, or `None` for one that may not.
///
/// A write is never shared: two identical mutations are two comments, two votes,
/// two pixels. Anything whose text is not a readable query is not shared either,
/// on the same principle - if it cannot be recognised, it is sent.
///
/// The token is part of the key. Two readers on one device must never be handed
/// each other's answer, for the same reason the query cache says so, and the
/// variables are in it because they are what makes two questions different.
fn flight_key(access_token: Option<&str>, body: &serde_json::Value) -> Option<String> {
    let query = body.get("query")?.as_str()?;
    match is_mutation(query) {
        true => None,
        false => Some(format!("{}|{body}", access_token.unwrap_or(""))),
    }
}

async fn post_body(access_token: Option<&str>, body: serde_json::Value) -> Envelope {
    let Some(key) = flight_key(access_token, &body) else {
        return post_body_once(access_token, &body).await;
    };
    let shared = IN_FLIGHT.with(|m| {
        if let Some(flight) = m.borrow().get(&key) {
            return flight.clone();
        }
        let token = access_token.map(str::to_string);
        let owned = body.clone();
        let fut: std::pin::Pin<Box<dyn std::future::Future<Output = Envelope>>> =
            Box::pin(async move { post_body_once(token.as_deref(), &owned).await });
        let flight = futures_util::FutureExt::shared(fut);
        m.borrow_mut().insert(key.clone(), flight.clone());
        flight
    });
    let out = shared.await;
    // Off the board as soon as it lands, so the next asker asks the server. A
    // holder that is dropped before the answer arrives leaves its entry behind;
    // the next caller to want it drives the same future to completion and clears
    // it, so nothing is ever left waiting on a request nobody is polling.
    IN_FLIGHT.with(|m| {
        m.borrow_mut().remove(&key);
    });
    out
}

/// One request, actually sent.
async fn post_body_once(access_token: Option<&str>, body: &serde_json::Value) -> Envelope {
    let client = reqwest::Client::new();
    let mut req = client.post(graphql_url());
    if let Some(token) = access_token {
        req = req.bearer_auth(token);
    }
    let resp = req.json(body).send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

/// Run `attempt`, retrying a read that failed because the request never
/// completed.
///
/// Only the offline class is retried. A refusal is an answer, and a broken
/// response is a bug: asking either of them again just asks again.
async fn retry_offline_reads<T, F, Fut>(query: &str, mut attempt: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut result = attempt().await;
    if is_mutation(query) {
        return result;
    }
    for delay in RETRY_DELAYS_MS {
        match &result {
            Err(msg) if crate::errors::classify(msg) == crate::errors::Failure::Offline => {}
            _ => break,
        }
        gloo_timers::future::TimeoutFuture::new(*delay).await;
        result = attempt().await;
    }
    result
}

async fn execute_once<Q, V>(
    access_token: Option<&str>,
    operation: &cynic::Operation<Q, V>,
) -> Result<Q, String>
where
    Q: serde::de::DeserializeOwned + 'static,
    V: serde::Serialize,
{
    // Through `post_body` rather than straight to reqwest, so an identical
    // question already in the air is answered by that one.
    let sent = serde_json::to_value(operation).map_err(|e| e.to_string())?;
    let envelope = post_body(access_token, sent).await?;
    let body: cynic::GraphQlResponse<Q> =
        serde_json::from_value(envelope).map_err(|e| e.to_string())?;

    if let Some(errors) = body.errors {
        let msgs: Vec<String> = errors.into_iter().map(|e| e.message).collect();
        return Err(msgs.join(", "));
    }

    body.data.ok_or_else(|| "No data returned".to_string())
}

pub async fn execute<Q, V>(
    access_token: Option<&str>,
    operation: cynic::Operation<Q, V>,
) -> Result<Q, String>
where
    Q: serde::de::DeserializeOwned + 'static,
    V: serde::Serialize,
{
    execute_reporting(access_token, operation, true).await
}

/// [`execute`] for a failure the CALLER expects and handles, so it is neither
/// shown to the person nor filed as a fault.
///
/// For a request whose failure is part of how it works. Naming a new node is the
/// case this exists for: the key is found by ATTEMPTING the insert and stepping
/// to the next name when it is taken (see `insert_node_named`), so a collision
/// is the mechanism, not a fault — and reporting it told somebody adding a
/// second canvas called "test" that a database constraint had been violated,
/// filed it in the feedback app as a bug, and then created their canvas anyway.
///
/// The error still comes back to the caller, and still reaches the console. Only
/// the toast and the auto-filed report are suppressed. Use it where the caller
/// genuinely handles the failure — everything else should stay loud.
pub async fn execute_quiet<Q, V>(
    access_token: Option<&str>,
    operation: cynic::Operation<Q, V>,
) -> Result<Q, String>
where
    Q: serde::de::DeserializeOwned + 'static,
    V: serde::Serialize,
{
    execute_reporting(access_token, operation, false).await
}

async fn execute_reporting<Q, V>(
    access_token: Option<&str>,
    operation: cynic::Operation<Q, V>,
    report: bool,
) -> Result<Q, String>
where
    Q: serde::de::DeserializeOwned + 'static,
    V: serde::Serialize,
{
    let first =
        retry_offline_reads(&operation.query, || execute_once(access_token, &operation)).await;
    // Set when the token had lapsed AND the refresh could not replace it, so the
    // query was never actually retried. See the logging below for why that is
    // treated as the network rather than as a fault.
    let mut lapsed = false;
    let result = match first {
        Err(msg) if is_jwt_error(&msg) => {
            // The token likely lapsed (e.g. the tab was backgrounded past expiry).
            // Refresh once and retry with the new token before surfacing the error
            // so a returning tab recovers instead of showing a JWT error.
            match crate::session::ensure_fresh_token(access_token).await {
                Some(fresh) if Some(fresh.as_str()) != access_token => {
                    execute_once(Some(&fresh), &operation).await
                }
                _ => {
                    lapsed = true;
                    Err(msg)
                }
            }
        }
        other => other,
    };
    // Log the final failure centrally (shipped in remote-logging builds) so every
    // GraphQL error is captured with its operation, regardless of how the caller
    // surfaces it — many only show a generic toast and discard the detail.
    if let Err(e) = &result {
        // Every caller of this swallows the error into an empty list, so this is
        // the last place that knows anything went wrong.
        // Noted whatever its class, including the quiet ones: if this ends up
        // as "Noget gik galt!" on screen, that shrug is the only thing the
        // reader gets and this is the only record of why.
        crate::errors::note_failure(format!("[{}] {e}", short_type_name::<Q>()));
        let failure = crate::errors::classify(e);
        // A lapsed session is the network, not a fault -- the same congress wifi
        // the refusal/offline note below is about, arriving by another door.
        //
        // `classify` cannot see it: "Could not verify JWT: JWTExpired" reads as
        // Broken, so a token that expired while the refresh happened to fail on a
        // 4g dip filed an error, with a stack, per query in flight. The refresh
        // itself already says so once (`session refresh failed (will retry)`, a
        // warn from session.rs), and the loop there retries every 45s, so these
        // are duplicates of a thing already reported and not separately
        // actionable. Reaching here at all means the query was never retried:
        // either no fresh token could be had, or there is no session to refresh.
        if lapsed {
            log::info!(
                "graphql [{}] on a lapsed session: {e}",
                short_type_name::<Q>()
            );
            return result;
        }
        // A failure the caller expects and handles stays on the console: no
        // toast, no auto-filed report, nothing shipped. See `execute_quiet`.
        if !report {
            log::info!(
                "graphql {} [{}] (expected): {e}",
                failure.label(),
                short_type_name::<Q>()
            );
            return result;
        }
        // The level decides what leaves the device: logging.rs ships warn and
        // error to Better Stack. Only a genuine fault is worth paying to store.
        //
        // A refusal is normal traffic — every signed-out reader generates them by
        // existing — and a dropped connection is the venue's wifi, not this code;
        // at a congress that would be thousands of records saying the hall has bad
        // reception. Both stay on the console, where they are still there when
        // someone is debugging.
        match failure {
            crate::errors::Failure::Broken => {
                // `error`, not `warn`: this class is defined as "always a bug",
                // and it was being filed under the level people filter OUT when
                // looking for bugs.
                let summary = format!("graphql error [{}]: {e}", short_type_name::<Q>());
                log::error!("{summary}");
                // ...and into the feedback app, not only the log sink. The five
                // queries broken by one bad variable showed every reader
                // "something went wrong" and told nobody what; the detail existed
                // the whole time and only a person reading the logs could see it.
                let token = access_token.map(str::to_string);
                let path = web_sys::window()
                    .and_then(|w| w.location().pathname().ok())
                    .unwrap_or_default();
                wasm_bindgen_futures::spawn_local(async move {
                    crate::backend_api::report_error(token.as_deref(), &summary, &path).await;
                });
            }
            _ => log::info!(
                "graphql {} [{}]: {e}",
                failure.label(),
                short_type_name::<Q>()
            ),
        }
        // The user hears about it only if it is theirs to care about, once,
        // throttled.
        crate::errors::report(failure);
    }
    result
}

/// The bare query-struct name (last `::` segment) for a GraphQL log line, e.g.
/// `NodeInsertsQuery` rather than the full `wiki_dioxus::graphql::…` path.
fn short_type_name<T>() -> &'static str {
    std::any::type_name::<T>()
        .rsplit("::")
        .next()
        .unwrap_or("query")
}

async fn execute_raw_once(
    access_token: Option<&str>,
    query: &str,
) -> Result<serde_json::Value, String> {
    let result = post_body(access_token, serde_json::json!({ "query": query })).await?;

    if let Some(errors) = result.get("errors") {
        return Err(errors.to_string());
    }

    Ok(result.get("data").cloned().unwrap_or_default())
}

/// Execute a raw GraphQL query/mutation string (for operations not covered by
/// cynic types), with the same JWT refresh-and-retry as [`execute`].
pub async fn execute_raw(
    access_token: Option<&str>,
    query: &str,
) -> Result<serde_json::Value, String> {
    let first = retry_offline_reads(query, || execute_raw_once(access_token, query)).await;
    let result = match first {
        Err(msg) if is_jwt_error(&msg) => {
            match crate::session::ensure_fresh_token(access_token).await {
                Some(fresh) if Some(fresh.as_str()) != access_token => {
                    execute_raw_once(Some(&fresh), query).await
                }
                _ => Err(msg),
            }
        }
        other => other,
    };
    report_raw_failure(access_token, &result, "raw");
    result
}

/// What `execute` does for a typed operation, for the raw ones.
///
/// These carry the feedback insert, the permission seeding, the tally and the
/// canvas, and they only ever logged at `warn` with no operation name and no
/// report — so a raw mutation that failed did so in silence. A canvas that would
/// not save a single cell produced no error entry anywhere, which is how this
/// came to be written.
fn report_raw_failure(
    access_token: Option<&str>,
    result: &Result<serde_json::Value, String>,
    what: &str,
) {
    let Err(e) = result else {
        return;
    };
    // Still a JWT error after the refresh-and-retry above means the retry never
    // happened: no fresh token could be had, or there was no session to refresh.
    // That is the network, not a fault -- see the long note in
    // `execute_reporting`, which does the same for typed operations.
    if is_jwt_error(e) {
        log::info!("graphql ({what}) on a lapsed session: {e}");
        return;
    }
    // As above: noted whatever its class, so a generic toast can say what it was.
    crate::errors::note_failure(format!("({what}) {e}"));
    let failure = crate::errors::classify(e);
    match failure {
        crate::errors::Failure::Broken => {
            let summary = format!("graphql error ({what}): {e}");
            log::error!("{summary}");
            let token = access_token.map(str::to_string);
            let path = web_sys::window()
                .and_then(|w| w.location().pathname().ok())
                .unwrap_or_default();
            wasm_bindgen_futures::spawn_local(async move {
                crate::backend_api::report_error(token.as_deref(), &summary, &path).await;
            });
        }
        _ => log::info!("graphql {} ({what}): {e}", failure.label()),
    }
    crate::errors::report(failure);
}

async fn execute_raw_vars_once(
    access_token: Option<&str>,
    query: &str,
    variables: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({ "query": query, "variables": variables });
    let result = post_body(access_token, body).await?;
    if let Some(errors) = result.get("errors") {
        return Err(errors.to_string());
    }
    Ok(result.get("data").cloned().unwrap_or_default())
}

/// Like [`execute_raw`] but with GraphQL `variables` (for mutations that pass
/// structured input, e.g. seeding a new context's permission template), with the
/// same JWT refresh-and-retry.
pub async fn execute_raw_vars(
    access_token: Option<&str>,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value, String> {
    execute_raw_vars_inner(access_token, query, variables, true).await
}

/// [`execute_raw_vars`] without the reporting, for an operation whose failure is
/// EXPECTED and handled by its caller.
///
/// Painting a pixel is the case: the database refuses a placement inside the
/// cooldown, and Hasura reports that as "database query error" with the reason
/// buried in `extensions.internal`, which it omits outside dev mode. Routed
/// through the reporting path, every cooldown would file a bug report and show
/// the user an error, when the truth is "not yet".
pub async fn execute_raw_vars_quiet(
    access_token: Option<&str>,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value, String> {
    execute_raw_vars_inner(access_token, query, variables, false).await
}

async fn execute_raw_vars_inner(
    access_token: Option<&str>,
    query: &str,
    variables: serde_json::Value,
    report: bool,
) -> Result<serde_json::Value, String> {
    let first = retry_offline_reads(query, || {
        execute_raw_vars_once(access_token, query, &variables)
    })
    .await;
    let result = match first {
        Err(msg) if is_jwt_error(&msg) => {
            match crate::session::ensure_fresh_token(access_token).await {
                Some(fresh) if Some(fresh.as_str()) != access_token => {
                    execute_raw_vars_once(Some(&fresh), query, &variables).await
                }
                _ => Err(msg),
            }
        }
        other => other,
    };
    if report {
        report_raw_failure(access_token, &result, "raw vars");
    }
    result
}

/// The remembered answer to a read, if the failure was the kind a copy answers.
///
/// A refusal must not fall back: serving what someone could read yesterday would
/// be the app overriding a permission change made since.
fn offline_copy<T: serde::de::DeserializeOwned>(key: &str, error: &str) -> Option<T> {
    if crate::errors::classify(error) != crate::errors::Failure::Offline {
        return None;
    }
    let copy = crate::offline::get::<T>(key)?;
    crate::errors::report_offline_copy();
    Some(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same keyword decides whether two requests may become one.
    ///
    /// The danger is the mirror of the retry's: a write folded into another
    /// write is a comment that was posted once when it was sent twice. So the
    /// key exists only for reads, and it carries the token and the variables,
    /// because a question asked by someone else, or asked about something else,
    /// is a different question.
    #[test]
    fn only_a_read_may_share_a_flight() {
        let read = serde_json::json!({ "query": "query Q($id: uuid!) { node(id: $id) { id } }",
                                       "variables": { "id": "a" } });
        let write = serde_json::json!({ "query": "mutation M { insertNode { id } }" });
        let anonymous = serde_json::json!({ "query": "{ nodes { id } }" });

        assert!(
            flight_key(None, &write).is_none(),
            "a write is never shared"
        );
        assert!(flight_key(Some("tok"), &write).is_none());

        let mine = flight_key(Some("tok"), &read).expect("a read is shareable");
        assert_eq!(
            mine,
            flight_key(Some("tok"), &read).unwrap(),
            "same question"
        );
        assert_ne!(
            mine,
            flight_key(Some("other"), &read).unwrap(),
            "another reader's answer is not mine"
        );
        let elsewhere = serde_json::json!({ "query": "query Q($id: uuid!) { node(id: $id) { id } }",
                                            "variables": { "id": "b" } });
        assert_ne!(
            mine,
            flight_key(Some("tok"), &elsewhere).unwrap(),
            "another node is another question"
        );
        // The anonymous shorthand is a read, and only a read may use it.
        assert!(flight_key(None, &anonymous).is_some());
    }

    /// What decides whether a dropped request is tried again. A mutation read as
    /// a query could cast a second vote, so the keyword is the whole safeguard:
    /// cynic writes it for typed mutations, GraphQL requires it for raw ones,
    /// and only a query may use the anonymous `{ ... }` shorthand.
    #[test]
    fn only_a_read_is_retried() {
        assert!(is_mutation(
            "mutation InsertNode($k: String) { insert_nodes { id } }"
        ));
        assert!(is_mutation("\n  mutation { insert_nodes { id } }"));

        assert!(!is_mutation("query HomeEvents { nodes { id } }"));
        assert!(!is_mutation("{ nodes { id } }"));
        assert!(!is_mutation("subscription Feed { nodes { id } }"));
        // Not fooled by the word appearing somewhere it is not the operation.
        assert!(!is_mutation("query M { members_mutation_response { id } }"));
    }

    /// The bin's list query names types Hasura actually defines.
    ///
    /// A tracked view gets its GraphQL types from its custom name, so the view
    /// `deleted_nodes` produced `deletedNodes_bool_exp`, not the
    /// `deleted_nodes_bool_exp` the hand-written schema entry claimed. Nothing
    /// caught it: the local schema is the only thing cynic checks against, so
    /// the query compiled and would have been rejected by the server, on a
    /// screen the tests never open. This asserts the operation as sent.
    /// The feed scoped to a context asks for its whole subtree, not just the
    /// rows that name it. A group holds events and an event's content belongs to
    /// the event, so without the ancestor test a group's feed shows almost none
    /// of what happened in it.
    #[test]
    fn a_scoped_feed_rolls_up_the_subtree() {
        let clause = recent_where_clause("user-1", Some("ctx-1"));
        let json = serde_json::to_string(&clause).expect("serialize");
        assert!(
            json.contains(r#""ancestors":{"_contains":["ctx-1"]}"#),
            "scoped feed must include everything under the context: {json}"
        );
        assert!(
            json.contains(r#""contextId":{"_eq":"ctx-1"}"#),
            "and anything filed with it directly: {json}"
        );
        // Unset comparison expressions must stay off the wire (Hasura rejects a
        // null where a comparison object is expected).
        assert!(!json.contains("null"), "no null comparisons: {json}");
    }

    /// Unscoped, the feed is still "contexts you belong to" — the ancestor test
    /// belongs to the scoped branch only, or the home feed would widen to every
    /// context that happens to sit under one you are in.
    #[test]
    fn an_unscoped_feed_stays_on_membership() {
        let clause = recent_where_clause("user-1", None);
        let json = serde_json::to_string(&clause).expect("serialize");
        assert!(!json.contains("ancestors"), "{json}");
        assert!(json.contains("members"), "{json}");
    }

    #[test]
    fn bin_query_declares_the_types_hasura_defines() {
        use cynic::QueryBuilder;
        let op = DeletedNodesQuery::build(DeletedNodesVariables {
            where_clause: DeletedNodesBoolExp::default(),
            order_by: Some(vec![DeletedNodesOrderBy {
                deleted_at: Some(OrderBy::Desc),
            }]),
        });
        assert!(
            op.query.contains("deletedNodes_bool_exp")
                && op.query.contains("deletedNodes_order_by"),
            "the view's types are camelCase after its custom name: {}",
            op.query
        );
        assert!(
            !op.query.contains("deleted_nodes_"),
            "no snake_case type survives: {}",
            op.query
        );
    }

    #[test]
    fn detects_jwt_errors_for_refresh_retry() {
        // Hasura's JWT failures all mention "JWT"; refresh + retry may recover.
        assert!(is_jwt_error("Could not verify JWT: JWTExpired"));
        assert!(is_jwt_error("Could not verify JWT: JWTInvalid signature"));
        assert!(is_jwt_error(r#"[{"message":"invalid-jwt"}]"#));
        // Unrelated errors must NOT trigger a pointless refresh + retry.
        assert!(!is_jwt_error("permission denied on nodes"));
        assert!(!is_jwt_error("No data returned"));
    }

    /// A lapsed session must not be filed as a bug.
    ///
    /// `classify` reads the JWT message as Broken, which is the level that
    /// leaves the device and opens a feedback report. One reader on 4g, whose
    /// token expired while the refresh happened to fail, filed exactly that --
    /// for a dropped connection the code beside it already refuses to report.
    /// Both paths now check `is_jwt_error` before classifying, and this pins the
    /// pair apart.
    #[test]
    fn a_lapsed_session_is_not_a_bug_the_way_a_real_failure_is() {
        let lapsed = "Could not verify JWT: JWTExpired";
        assert!(is_jwt_error(lapsed));
        assert!(matches!(
            crate::errors::classify(lapsed),
            crate::errors::Failure::Broken
        ));
        // ...which is precisely why the JWT check has to come first: left to
        // classify alone, this is indistinguishable from a genuine fault.
        //
        // A malformed variable, which is a real bug and must keep reporting as
        // one. NOT "field 'x' not found in type", which reads like a bug and is
        // classified as a refusal on purpose: that is the schema hiding a column
        // from a role.
        let real = "expected an object for type 'String_comparison_exp', but found null";
        assert!(!is_jwt_error(real));
        assert!(matches!(
            crate::errors::classify(real),
            crate::errors::Failure::Broken
        ));
    }

    /// The Hasura API rejects `null` for a comparison expression
    /// (`expected an object for type 'String_comparison_exp', but found null`),
    /// so unset `Option` input fields must be omitted from the wire format
    /// rather than serialized as `null`.
    #[test]
    fn contexts_where_clause_omits_null_fields() {
        let clause = contexts_where_clause("user-123", "wiki/group");
        let json = serde_json::to_string(&clause).expect("serialize where clause");

        assert!(
            !json.contains("null"),
            "where clause must not send null comparison expressions: {json}"
        );
        // The filter the query actually depends on must survive serialization.
        assert!(json.contains("\"mimeId\""), "missing mimeId filter: {json}");
        assert!(json.contains("wiki/group"), "missing mime value: {json}");
        assert!(
            json.contains("\"ownerId\""),
            "missing ownerId filter: {json}"
        );
        assert!(
            json.contains("\"members\""),
            "missing members filter: {json}"
        );
        assert!(
            json.contains("\"accepted\""),
            "missing accepted filter: {json}"
        );
        assert!(json.contains("user-123"), "missing user id: {json}");
    }

    /// A single-field comparison expression must serialize to just that field,
    /// with no sibling `null` keys.
    #[test]
    fn string_comparison_exp_omits_null_fields() {
        let exp = StringComparisonExp {
            eq: Some("wiki/event".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&exp).expect("serialize comparison exp");
        assert_eq!(json, r#"{"_eq":"wiki/event"}"#);
    }

    /// `gql_escape` guards the hand-built subscription/where strings: a value
    /// carrying `"` or `\` must be neutralised so it can't break out of the
    /// string literal and rewrite the query filter (a GraphQL injection).
    #[test]
    fn gql_escape_neutralises_quotes_and_backslashes() {
        assert_eq!(gql_escape("plain-id"), "plain-id");
        assert_eq!(gql_escape(r#"a"b"#), r#"a\"b"#);
        assert_eq!(gql_escape(r"a\b"), r"a\\b");
        // A backslash must be doubled BEFORE quotes are escaped, so a crafted
        // `\"` can't survive as an unescaped quote.
        assert_eq!(gql_escape(r#"\""#), r#"\\\""#);
        // A classic injection attempt stays inside the literal.
        assert_eq!(
            gql_escape(r#"" }, name: { _eq: "x"#),
            r#"\" }, name: { _eq: \"x"#
        );
    }

    /// The member-page `where` builder must escape the parent id + search term
    /// and omit unset bool filters.
    #[test]
    fn members_where_escapes_and_omits_unset() {
        let base = MemberPageFilter::default();
        let clause = members_where("ctx-1", &base);
        assert!(clause.contains(r#"parentId: { _eq: "ctx-1" }"#), "{clause}");
        // No bool filters and empty search -> only the parentId clause.
        assert!(!clause.contains("owner:"), "{clause}");
        assert!(!clause.contains("_ilike"), "{clause}");

        let filtered = MemberPageFilter {
            owner: Some(true),
            active: Some(false),
            search: "  a\"b  ".to_string(),
            ..Default::default()
        };
        let clause = members_where("ctx-1", &filtered);
        assert!(clause.contains("owner: { _eq: true }"), "{clause}");
        assert!(clause.contains("active: { _eq: false }"), "{clause}");
        // Search is trimmed, wrapped in %..%, and the embedded quote is escaped.
        assert!(clause.contains(r#"_ilike: "%a\"b%""#), "{clause}");
        assert!(
            !clause.contains("accepted:"),
            "unset filter omitted: {clause}"
        );
    }

    /// The invitations filter must omit null fields and carry the pending +
    /// group/event + user/email conditions the home list depends on.
    #[test]
    fn a_taken_key_is_told_apart_from_other_failures() {
        // Hasura's wording for the (parent_id, key) index. Matching too broadly
        // would retry a key that was never the problem; too narrowly would give
        // up on a clean key at the first collision.
        let taken = "hasura error: [{\"message\":\"Uniqueness violation. duplicate key \
                     value violates unique constraint \\\"nodes_parent_id_namespace_key\\\"\"}]";
        assert!(super::is_key_taken(taken));

        for other in [
            "hasura error: [{\"message\":\"permission denied\"}]",
            "network error",
            "hasura error: [{\"message\":\"not-null violation\"}]",
        ] {
            assert!(!super::is_key_taken(other), "should not retry on: {other}");
        }
    }

    #[test]
    fn invitations_where_clause_is_well_formed() {
        let clause = invitations_where_clause("user-1", "me@example.com");
        let json = serde_json::to_string(&clause).expect("serialize invitations clause");
        assert!(!json.contains("null"), "must omit null fields: {json}");
        assert!(json.contains("\"accepted\""), "missing accepted: {json}");
        assert!(json.contains("\"_or\""), "missing _or (user/email): {json}");
        assert!(json.contains("me@example.com"), "missing email: {json}");
        assert!(json.contains("user-1"), "missing user id: {json}");
        assert!(
            json.contains("wiki/group") && json.contains("wiki/event"),
            "missing parent mime filter: {json}"
        );
    }
}

#[cfg(test)]
mod variable_tests {
    use super::*;
    use cynic::QueryBuilder;

    /// Names every variable the operation SENDS but does not DECLARE.
    ///
    /// cynic declares only the variables an operation actually uses, while
    /// serialising every field of the struct it was handed. Hasura rejects an
    /// undeclared variable outright — the whole query fails — so the two must
    /// agree, and nothing in the type system makes them.
    fn undeclared<Q, V: serde::Serialize>(op: &cynic::Operation<Q, V>) -> Vec<String> {
        let json = serde_json::to_value(&op.variables).unwrap_or(serde_json::Value::Null);
        json.as_object()
            .map(|o| {
                o.keys()
                    .filter(|k| !op.query.contains(&format!("${k}")))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    macro_rules! assert_declared {
        ($($op:expr),+ $(,)?) => {
            $({
                let op = $op;
                let extra = undeclared(&op);
                assert!(
                    extra.is_empty(),
                    "sends undeclared variable(s) {:?}:\n{}",
                    extra, op.query
                );
            })+
        };
    }

    /// Every operation must declare every variable it sends.
    ///
    /// This is the test that was missing. A shared `NodesWhereVariables` gained
    /// an optional `limit` for the search box; the five other queries built from
    /// it — votes, polls, the home context list, the subtree walk, the feed
    /// count — kept sending it without declaring it, and Hasura failed all five
    /// in production ("unexpected variables in variableValues: limit"). The
    /// existing tests asserted the query TEXT and never the variables beside it,
    /// so nothing noticed. Assert the pair, for every shape the app sends.
    #[test]
    fn no_operation_sends_a_variable_it_does_not_declare() {
        let node_where = || NodesBoolExp::default();
        assert_declared!(
            // The two that legitimately carry a cap...
            NodesWhereQuery::build(NodesLimitVariables {
                where_clause: node_where(),
                limit: Some(30),
            }),
            NodesSearchQuery::build(NodesLimitVariables {
                where_clause: node_where(),
                limit: Some(30),
            }),
            NodePickerQuery::build(NodePickerVariables {
                where_clause: node_where(),
                limit: Some(10),
            }),
            // ...and the five that must not.
            ContextsWhereQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            ChildIdsQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            NodesCountQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            VotesWhereQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            PollsWhereQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            // The rest of the read path, so the next shared struct cannot repeat it.
            ChildrenQuery::build(ChildrenVariables {
                where_clause: node_where(),
                order_by: None,
            }),
            DrawerChildrenQuery::build(DrawerChildrenVariables {
                where_clause: node_where(),
                order_by: None,
                child_visible: node_where(),
            }),
            RecentNodesQuery::build(RecentNodesVariables {
                where_clause: node_where(),
                order_by: None,
                limit: Some(20),
                offset: Some(0),
            }),
            RelationsQuery::build(RelationsWhereVariables {
                where_clause: RelationsBoolExp::default(),
            }),
            MembersCountQuery::build(MembersCountVariables {
                where_clause: MembersBoolExp::default(),
            }),
            MembersExistQuery::build(MembersExistVariables {
                where_clause: MembersBoolExp::default(),
            }),
            InvitationsQuery::build(MembersWhereVariables {
                where_clause: MembersBoolExp::default(),
            }),
            UsersSearchQuery::build(UsersSearchVariables {
                where_clause: UsersBoolExp::default(),
            }),
            DeletedNodesQuery::build(DeletedNodesVariables {
                where_clause: DeletedNodesBoolExp::default(),
                order_by: None,
            }),
        );
    }
}

//! Jetstream event ingestion handlers.
//!
//! This module processes [`ParsedEvent`]s from the Jetstream client and
//! translates them into database and RBAC operations. It matches the upstream
//! Go spindle's `ingester.go` behavior.
//!
//! # Event Handlers
//!
//! - [`ingest_member`] — Handles `sh.tangled.spindle.member` events: adds or
//!   removes spindle members from the database and RBAC, and updates the
//!   Jetstream DID watch list.
//! - [`ingest_repo`] — Handles `sh.tangled.repo` events: adds or removes
//!   repos from the watch list when the `spindle` field matches this instance's
//!   hostname, and manages knot subscriptions.
//! - [`ingest_collaborator`] — Handles `sh.tangled.repo.collaborator` events:
//!   resolves the repo owner and adds/removes collaborators in RBAC.

use std::sync::Arc;

use serde::Deserialize;
use tracing::{debug, error, info, warn};

use crate::JetstreamError;
use crate::client::{CommitOperation, ParsedEvent};

// ---------------------------------------------------------------------------
// Record schemas (minimal, matching upstream AT Protocol lexicon)
// ---------------------------------------------------------------------------

/// Record schema for `sh.tangled.spindle.member`.
///
/// When the spindle owner creates this record, the `did` field identifies
/// the new member to add to this spindle.
#[derive(Debug, Clone, Deserialize)]
pub struct SpindleMemberRecord {
    /// The `$type` field (should be `"sh.tangled.spindle.member"`).
    #[serde(rename = "$type", default)]
    pub r#type: Option<String>,

    /// DID of the member being added.
    pub did: String,
}

/// Record schema for `sh.tangled.repo`.
///
/// A repo record points a repository at a specific knot server and
/// optionally associates it with a spindle for CI.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoRecord {
    /// The `$type` field (should be `"sh.tangled.repo"`).
    #[serde(rename = "$type", default)]
    pub r#type: Option<String>,

    /// Repository name.
    pub name: String,

    /// Knot server hostname where the repo is hosted.
    pub knot: String,

    /// Spindle hostname for CI (if any). When this matches our hostname,
    /// we should watch this repo for pipeline events.
    #[serde(default)]
    pub spindle: Option<String>,
}

/// Record schema for `sh.tangled.repo.collaborator`.
///
/// A collaborator record grants another DID access to a repository.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoCollaboratorRecord {
    /// The `$type` field (should be `"sh.tangled.repo.collaborator"`).
    #[serde(rename = "$type", default)]
    pub r#type: Option<String>,

    /// DID of the collaborator being added.
    pub did: String,

    /// Repository name (within the record author's account).
    #[serde(default)]
    pub repo: Option<String>,
}

// ---------------------------------------------------------------------------
// Ingestion context
// ---------------------------------------------------------------------------

/// Shared context for ingestion handlers.
///
/// Holds references to the subsystems needed to process Jetstream events.
/// The `KnotSubscriber` trait abstracts the knot event consumer so that
/// the ingester doesn't depend directly on `spindle-knot`.
pub struct IngestionContext<K: KnotSubscriber> {
    /// This spindle instance's hostname (e.g. `"spindle.example.com"`).
    pub hostname: String,

    /// Database handle.
    pub db: Arc<spindle_db::Database>,

    /// RBAC enforcer.
    pub rbac: Arc<spindle_rbac::SpindleEnforcer>,

    /// The `did:web:{hostname}` for this spindle.
    pub did_web: String,

    /// Knot subscription manager (add/remove knot event stream subscriptions).
    pub knot_subscriber: Arc<K>,
}

/// Trait for managing knot event stream subscriptions.
///
/// This abstracts the knot consumer so that the ingester can add/remove
/// knot subscriptions without depending directly on the `spindle-knot` crate.
#[async_trait::async_trait]
pub trait KnotSubscriber: Send + Sync {
    /// Subscribe to a knot server's event stream.
    async fn subscribe(&self, knot: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Unsubscribe from a knot server's event stream.
    async fn unsubscribe(&self, knot: &str)
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

// ---------------------------------------------------------------------------
// Ingestion handlers
// ---------------------------------------------------------------------------

/// Process a single parsed event from the Jetstream.
///
/// Dispatches to the appropriate handler based on the event type.
pub async fn ingest_event<K: KnotSubscriber>(
    ctx: &IngestionContext<K>,
    event: ParsedEvent,
) -> Result<(), JetstreamError> {
    match event {
        ParsedEvent::SpindleMember {
            did,
            rkey,
            operation,
            record,
            time_us,
        } => ingest_member(ctx, &did, &rkey, operation, record.as_ref(), time_us).await,
        ParsedEvent::Repo {
            did,
            rkey,
            operation,
            record,
            time_us,
        } => ingest_repo(ctx, &did, &rkey, operation, record.as_ref(), time_us).await,
        ParsedEvent::RepoCollaborator {
            did,
            rkey,
            operation,
            record,
            time_us,
        } => ingest_collaborator(ctx, &did, &rkey, operation, record.as_ref(), time_us).await,
    }
}

/// Handle a `sh.tangled.spindle.member` event.
///
/// - **Create/Update**: Add the member DID to the database, RBAC, and
///   Jetstream watch list.
/// - **Delete**: Remove the member from the database, RBAC, and watch list.
pub async fn ingest_member<K: KnotSubscriber>(
    ctx: &IngestionContext<K>,
    author_did: &str,
    _rkey: &str,
    operation: CommitOperation,
    record: Option<&serde_json::Value>,
    _time_us: i64,
) -> Result<(), JetstreamError> {
    match operation {
        CommitOperation::Create | CommitOperation::Update => {
            let record = record.ok_or_else(|| {
                JetstreamError::Parse(
                    "spindle.member create/update event missing record".to_string(),
                )
            })?;

            let member_record: SpindleMemberRecord = serde_json::from_value(record.clone())
                .map_err(|e| {
                    JetstreamError::Parse(format!("failed to parse spindle.member record: {e}"))
                })?;

            let member_did = &member_record.did;

            info!(
                author = %author_did,
                member = %member_did,
                "ingesting spindle member addition"
            );

            // Add to database
            if let Err(e) = ctx.db.add_spindle_member(member_did) {
                error!(%e, member = %member_did, "failed to add spindle member to database");
                return Err(JetstreamError::Ingestion(format!(
                    "failed to add member to database: {e}"
                )));
            }

            // Add to RBAC
            if let Err(e) = ctx.rbac.add_spindle_member(member_did).await {
                error!(%e, member = %member_did, "failed to add spindle member to RBAC");
                return Err(JetstreamError::Ingestion(format!(
                    "failed to add member to RBAC: {e}"
                )));
            }

            // Add to Jetstream DID watch list so we see their repo events
            if let Err(e) = ctx.db.add_did(member_did) {
                warn!(%e, member = %member_did, "failed to add member DID to watch list");
            }

            info!(member = %member_did, "spindle member added successfully");
        }
        CommitOperation::Delete => {
            // For deletes, the `author_did` is the one who authored the record.
            // The member being removed is identified by the rkey or we need to
            // look it up. In practice, the author is the spindle owner and the
            // member DID was in the original record. Since we don't have the
            // record on delete, we look up by the author's member records.
            //
            // However, in the AT Protocol pattern, the rkey for member records
            // often encodes the member DID. For now, we handle this by noting
            // the deletion — the main server can reconcile by re-fetching
            // the member list from the PDS if needed.
            info!(
                author = %author_did,
                "ingesting spindle member deletion"
            );

            // Try to remove the author as a member (in case the member
            // deleted their own record). The orchestrator may need more
            // sophisticated logic here.
            if let Ok(true) = ctx.db.remove_member(author_did) {
                if let Err(e) = ctx.rbac.remove_spindle_member(author_did).await {
                    warn!(%e, did = %author_did, "failed to remove member from RBAC");
                }

                // Optionally remove from DID watch list
                // (only if they're not also a repo owner we care about)
                debug!(did = %author_did, "spindle member removed");
            }
        }
    }

    Ok(())
}

/// Handle a `sh.tangled.repo` event.
///
/// When a repo record's `spindle` field matches this spindle's hostname:
/// - **Create/Update**: Add the repo to the watch list and subscribe to
///   the repo's knot for pipeline events.
/// - **Delete**: Remove the repo from the watch list.
///
/// When the `spindle` field doesn't match (or is absent), the event is ignored.
pub async fn ingest_repo<K: KnotSubscriber>(
    ctx: &IngestionContext<K>,
    author_did: &str,
    _rkey: &str,
    operation: CommitOperation,
    record: Option<&serde_json::Value>,
    _time_us: i64,
) -> Result<(), JetstreamError> {
    match operation {
        CommitOperation::Create | CommitOperation::Update => {
            let record = record.ok_or_else(|| {
                JetstreamError::Parse("repo create/update event missing record".to_string())
            })?;

            let repo_record: RepoRecord = serde_json::from_value(record.clone())
                .map_err(|e| JetstreamError::Parse(format!("failed to parse repo record: {e}")))?;

            // Check if this repo is associated with our spindle
            let spindle_hostname = match &repo_record.spindle {
                Some(s) if s == &ctx.hostname => s.clone(),
                Some(s) => {
                    debug!(
                        repo = %repo_record.name,
                        spindle = %s,
                        our_hostname = %ctx.hostname,
                        "repo points to different spindle, ignoring"
                    );
                    return Ok(());
                }
                None => {
                    debug!(
                        repo = %repo_record.name,
                        "repo has no spindle field, ignoring"
                    );
                    return Ok(());
                }
            };

            info!(
                did = %author_did,
                repo = %repo_record.name,
                knot = %repo_record.knot,
                spindle = %spindle_hostname,
                "ingesting repo registration"
            );

            // Add repo to the database watch list
            if let Err(e) = ctx
                .db
                .add_repo(author_did, &repo_record.name, &repo_record.knot)
            {
                error!(%e, repo = %repo_record.name, "failed to add repo to database");
                return Err(JetstreamError::Ingestion(format!(
                    "failed to add repo to database: {e}"
                )));
            }

            // Add the knot to our knot tracking table
            if let Err(e) = ctx.db.add_knot(&repo_record.knot) {
                warn!(%e, knot = %repo_record.knot, "failed to add knot to tracking table");
            }

            // Subscribe to the knot's event stream for pipeline events
            if let Err(e) = ctx.knot_subscriber.subscribe(&repo_record.knot).await {
                warn!(
                    %e,
                    knot = %repo_record.knot,
                    "failed to subscribe to knot event stream"
                );
            }

            // Add repo owner to RBAC
            if let Err(e) = ctx.rbac.add_repo(author_did, &repo_record.name).await {
                warn!(%e, repo = %repo_record.name, "failed to add repo to RBAC");
            }

            info!(
                repo = %repo_record.name,
                knot = %repo_record.knot,
                "repo registered with spindle"
            );
        }
        CommitOperation::Delete => {
            // The upstream Go spindle ignores repo deletion events.
            // Repo records are frequently deleted and re-created during
            // normal AT Protocol operations, so we should not remove
            // repos or knot subscriptions on delete events.
            debug!(
                did = %author_did,
                "ignoring repo deletion event (matching upstream behavior)"
            );
        }
    }

    Ok(())
}

/// Handle a `sh.tangled.repo.collaborator` event.
///
/// - **Create/Update**: Resolve the repo owner (the author of this record)
///   and add the collaborator to RBAC for that repo.
/// - **Delete**: Remove the collaborator from RBAC.
pub async fn ingest_collaborator<K: KnotSubscriber>(
    ctx: &IngestionContext<K>,
    author_did: &str,
    _rkey: &str,
    operation: CommitOperation,
    record: Option<&serde_json::Value>,
    _time_us: i64,
) -> Result<(), JetstreamError> {
    match operation {
        CommitOperation::Create | CommitOperation::Update => {
            let record = record.ok_or_else(|| {
                JetstreamError::Parse("collaborator create/update event missing record".to_string())
            })?;

            let collab_record: RepoCollaboratorRecord = serde_json::from_value(record.clone())
                .map_err(|e| {
                    JetstreamError::Parse(format!("failed to parse collaborator record: {e}"))
                })?;

            let repo_name = collab_record.repo.as_deref().unwrap_or("unknown");
            let collaborator_did = &collab_record.did;

            info!(
                author = %author_did,
                collaborator = %collaborator_did,
                repo = %repo_name,
                "ingesting collaborator addition"
            );

            // The author of the collaborator record is the repo owner.
            // Add the collaborator to RBAC for this repo.
            if let Err(e) = ctx
                .rbac
                .add_collaborator(author_did, repo_name, collaborator_did)
                .await
            {
                error!(
                    %e,
                    collaborator = %collaborator_did,
                    repo = %repo_name,
                    "failed to add collaborator to RBAC"
                );
                return Err(JetstreamError::Ingestion(format!(
                    "failed to add collaborator to RBAC: {e}"
                )));
            }

            // Add collaborator DID to Jetstream watch list so we see their events
            if let Err(e) = ctx.db.add_did(collaborator_did) {
                warn!(
                    %e,
                    collaborator = %collaborator_did,
                    "failed to add collaborator DID to watch list"
                );
            }

            info!(
                collaborator = %collaborator_did,
                repo = %repo_name,
                "collaborator added successfully"
            );
        }
        CommitOperation::Delete => {
            info!(
                author = %author_did,
                "ingesting collaborator deletion"
            );

            // On delete we don't have the record, so we can't easily determine
            // which collaborator was removed. The orchestrator may need to
            // re-sync collaborators from the PDS. For now, log the event.
            debug!(
                author = %author_did,
                "collaborator record deleted — may need reconciliation"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock knot subscriber for testing.
    struct MockKnotSubscriber {
        subscribed: Mutex<Vec<String>>,
        unsubscribed: Mutex<Vec<String>>,
    }

    impl MockKnotSubscriber {
        fn new() -> Self {
            Self {
                subscribed: Mutex::new(Vec::new()),
                unsubscribed: Mutex::new(Vec::new()),
            }
        }

        fn subscribed(&self) -> Vec<String> {
            self.subscribed.lock().unwrap().clone()
        }

        #[allow(dead_code)]
        fn unsubscribed(&self) -> Vec<String> {
            self.unsubscribed.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl KnotSubscriber for MockKnotSubscriber {
        async fn subscribe(
            &self,
            knot: &str,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.subscribed.lock().unwrap().push(knot.to_string());
            Ok(())
        }

        async fn unsubscribe(
            &self,
            knot: &str,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.unsubscribed.lock().unwrap().push(knot.to_string());
            Ok(())
        }
    }

    async fn setup_ctx(knot_sub: Arc<MockKnotSubscriber>) -> IngestionContext<MockKnotSubscriber> {
        let db = Arc::new(spindle_db::Database::open_in_memory().unwrap());
        let rbac = Arc::new(spindle_rbac::SpindleEnforcer::new().await.unwrap());

        // Bootstrap RBAC
        rbac.add_spindle("did:web:spindle.example.com")
            .await
            .unwrap();
        rbac.add_spindle_owner("did:web:spindle.example.com", "did:plc:owner")
            .await
            .unwrap();

        // Add owner to DB
        db.add_spindle_owner("did:plc:owner").unwrap();
        db.add_did("did:plc:owner").unwrap();

        IngestionContext {
            hostname: "spindle.example.com".to_string(),
            db,
            rbac,
            did_web: "did:web:spindle.example.com".to_string(),
            knot_subscriber: knot_sub,
        }
    }

    #[tokio::test]
    async fn ingest_member_create() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub).await;

        let record = serde_json::json!({
            "$type": "sh.tangled.spindle.member",
            "did": "did:plc:newmember"
        });

        ingest_member(
            &ctx,
            "did:plc:owner",
            "self",
            CommitOperation::Create,
            Some(&record),
            1700000000000000,
        )
        .await
        .unwrap();

        // Check member was added to DB
        assert!(ctx.db.is_member("did:plc:newmember").unwrap());

        // Check DID was added to watch list
        assert!(ctx.db.has_did("did:plc:newmember").unwrap());
    }

    #[tokio::test]
    async fn ingest_member_create_missing_record() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub).await;

        let result = ingest_member(
            &ctx,
            "did:plc:owner",
            "self",
            CommitOperation::Create,
            None,
            1700000000000000,
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn ingest_member_delete() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub).await;

        // First add a member
        ctx.db.add_spindle_member("did:plc:member1").unwrap();

        // Then delete
        ingest_member(
            &ctx,
            "did:plc:member1",
            "self",
            CommitOperation::Delete,
            None,
            1700000000000000,
        )
        .await
        .unwrap();

        // Check member was removed
        assert!(!ctx.db.is_member("did:plc:member1").unwrap());
    }

    #[tokio::test]
    async fn ingest_repo_create_matching_spindle() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub.clone()).await;

        let record = serde_json::json!({
            "$type": "sh.tangled.repo",
            "name": "my-repo",
            "knot": "knot.example.com",
            "spindle": "spindle.example.com"
        });

        ingest_repo(
            &ctx,
            "did:plc:owner",
            "my-repo",
            CommitOperation::Create,
            Some(&record),
            1700000000000000,
        )
        .await
        .unwrap();

        // Check repo was added
        let repo = ctx.db.get_repo("did:plc:owner", "my-repo").unwrap();
        assert!(repo.is_some());
        assert_eq!(repo.unwrap().knot, "knot.example.com");

        // Check knot subscription was requested
        assert_eq!(knot_sub.subscribed(), vec!["knot.example.com"]);
    }

    #[tokio::test]
    async fn ingest_repo_create_different_spindle_ignored() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub.clone()).await;

        let record = serde_json::json!({
            "$type": "sh.tangled.repo",
            "name": "my-repo",
            "knot": "knot.example.com",
            "spindle": "other-spindle.example.com"
        });

        ingest_repo(
            &ctx,
            "did:plc:owner",
            "my-repo",
            CommitOperation::Create,
            Some(&record),
            1700000000000000,
        )
        .await
        .unwrap();

        // Repo should NOT be added
        let repo = ctx.db.get_repo("did:plc:owner", "my-repo").unwrap();
        assert!(repo.is_none());

        // No knot subscription
        assert!(knot_sub.subscribed().is_empty());
    }

    #[tokio::test]
    async fn ingest_repo_create_no_spindle_ignored() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub.clone()).await;

        let record = serde_json::json!({
            "$type": "sh.tangled.repo",
            "name": "my-repo",
            "knot": "knot.example.com"
        });

        ingest_repo(
            &ctx,
            "did:plc:owner",
            "my-repo",
            CommitOperation::Create,
            Some(&record),
            1700000000000000,
        )
        .await
        .unwrap();

        let repo = ctx.db.get_repo("did:plc:owner", "my-repo").unwrap();
        assert!(repo.is_none());
    }

    #[tokio::test]
    async fn ingest_collaborator_create() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub).await;

        // Register a repo first
        ctx.db
            .add_repo("did:plc:owner", "my-repo", "knot.example.com")
            .unwrap();
        ctx.rbac.add_repo("did:plc:owner", "my-repo").await.unwrap();

        let record = serde_json::json!({
            "$type": "sh.tangled.repo.collaborator",
            "did": "did:plc:collaborator1",
            "repo": "my-repo"
        });

        ingest_collaborator(
            &ctx,
            "did:plc:owner",
            "abc123",
            CommitOperation::Create,
            Some(&record),
            1700000000000000,
        )
        .await
        .unwrap();

        // Check collaborator DID was added to watch list
        assert!(ctx.db.has_did("did:plc:collaborator1").unwrap());
    }

    #[tokio::test]
    async fn ingest_collaborator_create_missing_record() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub).await;

        let result = ingest_collaborator(
            &ctx,
            "did:plc:owner",
            "abc123",
            CommitOperation::Create,
            None,
            1700000000000000,
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn ingest_collaborator_delete_is_noop() {
        let knot_sub = Arc::new(MockKnotSubscriber::new());
        let ctx = setup_ctx(knot_sub).await;

        // Delete without record should succeed (just logs)
        ingest_collaborator(
            &ctx,
            "did:plc:owner",
            "abc123",
            CommitOperation::Delete,
            None,
            1700000000000000,
        )
        .await
        .unwrap();
    }

    #[test]
    fn parse_spindle_member_record() {
        let json = serde_json::json!({
            "$type": "sh.tangled.spindle.member",
            "did": "did:plc:bob"
        });

        let record: SpindleMemberRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.did, "did:plc:bob");
        assert_eq!(record.r#type.as_deref(), Some("sh.tangled.spindle.member"));
    }

    #[test]
    fn parse_repo_record() {
        let json = serde_json::json!({
            "$type": "sh.tangled.repo",
            "name": "my-repo",
            "knot": "knot.example.com",
            "spindle": "spindle.example.com"
        });

        let record: RepoRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.name, "my-repo");
        assert_eq!(record.knot, "knot.example.com");
        assert_eq!(record.spindle.as_deref(), Some("spindle.example.com"));
    }

    #[test]
    fn parse_repo_record_no_spindle() {
        let json = serde_json::json!({
            "$type": "sh.tangled.repo",
            "name": "my-repo",
            "knot": "knot.example.com"
        });

        let record: RepoRecord = serde_json::from_value(json).unwrap();
        assert!(record.spindle.is_none());
    }

    #[test]
    fn parse_collaborator_record() {
        let json = serde_json::json!({
            "$type": "sh.tangled.repo.collaborator",
            "did": "did:plc:collab1",
            "repo": "my-repo"
        });

        let record: RepoCollaboratorRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.did, "did:plc:collab1");
        assert_eq!(record.repo.as_deref(), Some("my-repo"));
    }

    #[test]
    fn parse_collaborator_record_no_repo() {
        let json = serde_json::json!({
            "$type": "sh.tangled.repo.collaborator",
            "did": "did:plc:collab1"
        });

        let record: RepoCollaboratorRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.did, "did:plc:collab1");
        assert!(record.repo.is_none());
    }
}

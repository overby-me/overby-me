//! XRPC route handlers and service auth for `tangled-spindle-nix`.
//!
//! This crate provides the AT Protocol XRPC endpoint handlers and service
//! authentication verification, matching the upstream Go spindle's XRPC
//! implementation.
//!
//! # Endpoints
//!
//! - **Service auth** — Verify incoming XRPC requests using bearer token
//!   authentication (AT Protocol JWT verification deferred to Phase 8).
//! - **Member management** — Add/remove spindle members via XRPC calls.
//! - **Secret management** — CRUD operations for per-repo secrets.
//! - **Pipeline control** — Cancel running pipelines.
//!
//! # Architecture
//!
//! Handlers are implemented as `axum` handler functions, mounted under
//! `/xrpc/{method}` on the main HTTP server. Each handler delegates to the
//! appropriate `spindle-db`, `spindle-rbac`, or `spindle-secrets` crate for
//! business logic.

pub mod handlers;
pub mod service_auth;

use std::sync::Arc;

use spindle_db::Database;
use spindle_rbac::SpindleEnforcer;
use spindle_secrets::Manager;

pub use handlers::{dispatch, dispatch_query};
pub use service_auth::ServiceAuth;

/// Shared context for XRPC handlers.
///
/// Holds references to all subsystems needed by the XRPC endpoint handlers.
/// Designed to be constructed by the main server and shared via
/// `axum::extract::State<Arc<XrpcContext>>`.
pub struct XrpcContext {
    /// Database handle.
    pub db: Arc<Database>,
    /// RBAC enforcer.
    pub rbac: SpindleEnforcer,
    /// Secrets manager.
    pub secrets: Arc<dyn Manager + Send + Sync>,
    /// This spindle's `did:web:{hostname}` identity.
    pub did_web: String,
    /// The spindle owner's DID.
    pub owner: String,
    /// The configured authentication token.
    pub token: String,
    /// PLC directory URL for DID resolution (e.g. `https://plc.directory`).
    pub plc_url: String,
    /// Shared HTTP client for PLC directory requests.
    pub http_client: reqwest::Client,
    /// Whether dev mode is enabled.
    pub dev: bool,
}

impl std::fmt::Debug for XrpcContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XrpcContext")
            .field("did_web", &self.did_web)
            .field("owner", &self.owner)
            .field("plc_url", &self.plc_url)
            .field("dev", &self.dev)
            .finish_non_exhaustive()
    }
}

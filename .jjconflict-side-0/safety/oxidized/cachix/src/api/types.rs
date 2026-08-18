use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Binary cache metadata returned by the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryCache {
    pub name: String,
    pub uri: String,
    pub is_public: bool,
    pub public_signing_keys: Vec<String>,
    pub github_username: Option<String>,
    pub permission: Permission,
    pub preferred_compression_method: Option<CompressionMethod>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Permission {
    #[serde(alias = "read", alias = "Read")]
    Read,
    #[serde(alias = "write", alias = "Write")]
    Write,
    #[serde(alias = "admin", alias = "Admin")]
    Admin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompressionMethod {
    #[serde(alias = "XZ")]
    Xz,
    #[serde(alias = "ZSTD")]
    Zstd,
}

impl std::fmt::Display for CompressionMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompressionMethod::Xz => write!(f, "xz"),
            CompressionMethod::Zstd => write!(f, "zst"),
        }
    }
}

impl CompressionMethod {
    pub fn file_extension(&self) -> &'static str {
        match self {
            CompressionMethod::Xz => "xz",
            CompressionMethod::Zstd => "zst",
        }
    }
}

/// Info about a Nix cache (nix-cache-info).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NixCacheInfo {
    pub store_dir: String,
    pub want_mass_query: i64,
    pub priority: i64,
}

/// NarInfo creation request sent after uploading a NAR.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarInfoCreate {
    pub c_store_hash: String,
    pub c_store_suffix: String,
    pub c_nar_hash: String,
    pub c_nar_size: u64,
    pub c_file_hash: String,
    pub c_file_size: u64,
    pub c_references: Vec<String>,
    pub c_deriver: String,
    pub c_sig: Option<String>,
}

/// Response from creating a multipart upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMultipartUploadResponse {
    pub nar_id: Uuid,
    pub upload_id: String,
}

/// Request body for getting a presigned upload URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningData {
    #[serde(rename = "contentMD5")]
    pub content_md5: String,
}

/// Response with presigned URL for uploading a part.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadPartResponse {
    pub upload_url: String,
}

/// A completed upload part.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedPart {
    pub part_number: u32,
    pub e_tag: String,
}

/// Request body for completing a multipart upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedMultipartUpload {
    pub parts: Option<Vec<CompletedPart>>,
    pub nar_info_create: NarInfoCreate,
}

/// Bulk narinfo query: send store hashes, get back which are missing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarInfoBulkResponse {
    pub missing_hashes: Vec<String>,
}

/// Pin creation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinCreate {
    pub name: String,
    pub store_path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep: Option<PinKeep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum PinKeep {
    #[serde(rename = "days")]
    Days(u64),
    #[serde(rename = "revisions")]
    Revisions(u64),
    #[serde(rename = "forever")]
    Forever,
}

/// Deploy specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploySpec {
    pub agents: std::collections::HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_script: Option<std::collections::HashMap<String, String>>,
}

/// Response from deploy activate (V2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployActivateResponse {
    pub agents: Vec<DeployAgentResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployAgentResponse {
    pub id: Uuid,
    pub name: String,
    pub deployment_id: Uuid,
    pub log_url: Option<String>,
}

/// Deployment status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub id: Uuid,
    pub status: DeploymentStatus,
    pub agent_name: Option<String>,
    pub store_path: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentStatus {
    Pending,
    InProgress,
    Succeeded,
    Failed,
    Cancelled,
}

/// WebSocket messages for deploy agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tag", content = "contents")]
pub enum BackendMessage {
    AgentRegistered(AgentInformation),
    Deployment(DeploymentDetails),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInformation {
    pub cache: Option<AgentCache>,
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCache {
    pub name: String,
    pub public_signing_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentDetails {
    pub store_path: String,
    pub id: Uuid,
    pub index: i64,
    pub rollback_script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tag", content = "contents")]
pub enum AgentMessage {
    DeploymentStarted {
        id: Uuid,
        time: DateTime<Utc>,
        closure_size: Option<u64>,
    },
    DeploymentFinished {
        id: Uuid,
        time: DateTime<Utc>,
        has_succeeded: bool,
    },
}

/// Daemon protocol messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tag", content = "contents")]
#[allow(clippy::enum_variant_names)]
pub enum DaemonClientMessage {
    ClientPushRequest(PushRequest),
    ClientStop,
    ClientPing,
    ClientDiagnosticsRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub store_paths: Vec<String>,
    pub subscribe_to_updates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tag", content = "contents")]
#[allow(clippy::enum_variant_names)]
pub enum DaemonServerMessage {
    DaemonPong,
    DaemonExit(DaemonExitInfo),
    DaemonPushEvent(PushEvent),
    DaemonError(DaemonError),
    DaemonDiagnosticsResult(DaemonDiagnostics),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonExitInfo {
    pub exit_code: i32,
    pub exit_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PushEvent {
    #[serde(rename = "pushStarted")]
    PushStarted { path: String },
    #[serde(rename = "pushCompleted")]
    PushCompleted { path: String },
    #[serde(rename = "pushFailed")]
    PushFailed { path: String, error: String },
    #[serde(rename = "allComplete")]
    AllComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonError {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonDiagnostics {
    pub is_healthy: bool,
    pub message: String,
}

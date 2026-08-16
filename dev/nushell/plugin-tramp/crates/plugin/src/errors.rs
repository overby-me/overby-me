use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrampError {
    #[error("invalid tramp path: {0}")]
    ParseError(String),

    #[error("invalid tramp path: missing remote path after ':'")]
    MissingRemotePath,

    #[error("invalid tramp path: unknown backend '{0}'")]
    UnknownBackend(String),

    #[error("invalid tramp path: missing host")]
    MissingHost,

    #[error("invalid tramp path: invalid port '{0}'")]
    InvalidPort(String),

    #[error("ssh: could not connect to {host}: {reason}")]
    ConnectionFailed { host: String, reason: String },

    #[error("ssh: authentication failed for {user}@{host}")]
    AuthFailed { user: String, host: String },

    #[error("remote: no such file or directory: {0}")]
    NotFound(String),

    #[error("remote: permission denied: {0}")]
    PermissionDenied(String),

    #[error("remote: {0}")]
    RemoteError(String),

    #[error("sftp: {0}")]
    SftpError(String),

    #[error("tramp: {0}")]
    Internal(String),
}

impl TrampError {
    /// Classify an SSH/SFTP error into a more specific `TrampError` when possible.
    pub fn from_ssh(host: &str, err: impl std::fmt::Display) -> Self {
        let msg = err.to_string();
        if msg.contains("No such file") || msg.contains("not found") {
            TrampError::NotFound(msg)
        } else if msg.contains("Permission denied") || msg.contains("permission denied") {
            TrampError::PermissionDenied(msg)
        } else if msg.contains("Authentication failed") || msg.contains("auth") {
            TrampError::AuthFailed {
                user: String::new(),
                host: host.to_string(),
            }
        } else if msg.contains("Connection refused")
            || msg.contains("Connection timed out")
            || msg.contains("Could not resolve")
        {
            TrampError::ConnectionFailed {
                host: host.to_string(),
                reason: msg,
            }
        } else {
            TrampError::RemoteError(msg)
        }
    }
}

impl From<TrampError> for nu_protocol::LabeledError {
    fn from(err: TrampError) -> Self {
        nu_protocol::LabeledError::new(err.to_string())
            .with_label("tramp error", nu_protocol::Span::unknown())
    }
}

pub type TrampResult<T> = Result<T, TrampError>;

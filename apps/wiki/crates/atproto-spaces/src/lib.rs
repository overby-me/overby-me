//! atproto spaces, as far as the wiki's AppView needs them
//! (`docs/atproto-spaces-redesign.md`): who it is when it asks (`jwt`, `dpop`,
//! `credential`), what it asks (`client`), how it knows its copy of a repo is
//! the host's (`sethash`, `commit`, `sync`), and how it knows a PDS is who
//! calls it back (`service_auth`).

pub mod attestation;
pub mod client;
pub mod commit;
pub mod credential;
pub mod directory;
pub mod dpop;
pub mod jwt;
pub mod keys;
pub mod service_auth;
pub mod sethash;
pub mod sync;

pub use commit::{CommitError, SignedCommit};
pub use sethash::SetHash;

/// What went wrong, in the words of whoever has to read a log.
#[derive(Debug)]
pub enum Error {
    /// The request never got an answer.
    Http(reqwest::Error),
    /// The host answered with an XRPC error.
    Xrpc {
        status: u16,
        error: String,
        message: String,
    },
    /// An answer, a token or a document that is not what it has to be.
    Malformed(String),
    /// A signature or a hash that does not hold.
    Unverified(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(e) => write!(f, "no answer: {e}"),
            Error::Xrpc {
                status,
                error,
                message,
            } => write!(f, "{status} {error}: {message}"),
            Error::Malformed(what) => write!(f, "malformed: {what}"),
            Error::Unverified(what) => write!(f, "not verified: {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e)
    }
}

impl Error {
    /// The XRPC error's name, for what a caller acts on (`UserNotAuthorized`,
    /// `SpaceDeleted`, `RecordAlreadyExists`).
    pub fn xrpc_name(&self) -> Option<&str> {
        match self {
            Error::Xrpc { error, .. } => Some(error),
            _ => None,
        }
    }

    /// Whether a write was refused for what the record is, so that the same
    /// record would be refused again: past the size a PDS takes (about 1 MB of
    /// request on the alpha), or data atproto cannot hold, such as a fraction.
    pub fn is_about_the_record(&self) -> bool {
        let named = matches!(
            self.xrpc_name(),
            Some("PayloadTooLargeError" | "InvalidRequest" | "InvalidRecord")
        );
        match self {
            Error::Xrpc { status: 413, .. } => true,
            Error::Xrpc { status: 400, .. } => named,
            _ => false,
        }
    }
}

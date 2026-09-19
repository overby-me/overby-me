//! A typed client for the AppView, generated from its lexicons
//! (`apps/wiki/lexicons`, by `cargo run -p lexgen`).
//!
//! One method per XRPC method, taking and returning the types the lexicon
//! describes. It compiles for a browser and for a host alike, which is what
//! lets `tests/contract.rs` drive the real router with it.

// Its own formatting, so that regenerating it is never a diff of whitespace.
#[rustfmt::skip]
mod generated;

pub use generated::*;

/// What a call can fail with.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// No answer: the network, or a server that is not there.
    Transport(String),
    /// The AppView's refusal: the HTTP status, and its `error` and `message`.
    Api {
        status: u16,
        error: String,
        message: String,
    },
    /// An answer that is not what the lexicon says.
    Decode(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "no answer: {e}"),
            Error::Api {
                status,
                error,
                message,
            } => write!(f, "{status} {error}: {message}"),
            Error::Decode(e) => write!(f, "not what the lexicon says: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Decode(e.to_string())
    }
}

impl Error {
    /// The AppView's `error` name, such as `NotFound`, if it answered at all.
    pub fn name(&self) -> Option<&str> {
        match self {
            Error::Api { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Bytes that are not JSON, with what they say they are.
#[derive(Debug, Clone, PartialEq)]
pub struct Binary {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

pub(crate) enum Verb {
    Get,
    Post,
}

pub(crate) enum Body {
    None,
    Json(serde_json::Value),
    Bytes(Vec<u8>, String),
}

pub(crate) struct Answer {
    bytes: Vec<u8>,
    content_type: String,
}

impl Answer {
    pub(crate) fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_slice(&self.bytes)?)
    }

    pub(crate) fn binary(self) -> Binary {
        Binary {
            bytes: self.bytes,
            content_type: self.content_type,
        }
    }
}

/// A connection to one AppView, as one person or as nobody.
#[derive(Debug, Clone)]
pub struct Client {
    base: String,
    session: Option<String>,
    http: reqwest::Client,
}

impl Client {
    /// `base` is the AppView's origin, such as `https://appview.example`.
    pub fn new(base: &str) -> Self {
        Client {
            base: base.trim_end_matches('/').to_string(),
            session: None,
            http: reqwest::Client::new(),
        }
    }

    /// The same AppView, as whoever holds `session` (from `create_session`).
    pub fn with_session(&self, session: &str) -> Self {
        Client {
            session: Some(session.to_string()),
            ..self.clone()
        }
    }

    /// Where a file is read from. Give the session as a bearer token, or use a
    /// signed link (`get_blob_link`) for an element that cannot send a header.
    pub fn blob_url(&self, id: &str) -> String {
        format!("{}/blob/{id}", self.base)
    }

    /// A file's bytes, as whoever this client is.
    pub async fn get_blob(&self, id: &str) -> Result<Binary, Error> {
        let request = self.http.get(self.blob_url(id));
        Ok(self.send(request).await?.binary())
    }

    /// Where a browser goes to sign in as `handle`. It comes back to
    /// `return_to` with `#code=<code>`, which `create_session` takes.
    pub fn login_url(&self, handle: &str, return_to: &str) -> String {
        let login = format!("{}/login", self.base);
        let pairs = [("handle", handle), ("return", return_to)];
        reqwest::Url::parse_with_params(&login, pairs).map_or(login, String::from)
    }

    pub(crate) async fn call(
        &self,
        verb: Verb,
        nsid: &str,
        pairs: Vec<(&'static str, String)>,
        body: Body,
    ) -> Result<Answer, Error> {
        let url = format!("{}/xrpc/{nsid}", self.base);
        let request = match verb {
            Verb::Get => self.http.get(url),
            Verb::Post => self.http.post(url),
        }
        .query(&pairs);
        self.send(match body {
            Body::None => request,
            Body::Json(value) => request.json(&value),
            Body::Bytes(bytes, content_type) => {
                request.header("content-type", content_type).body(bytes)
            }
        })
        .await
    }

    async fn send(&self, mut request: reqwest::RequestBuilder) -> Result<Answer, Error> {
        if let Some(session) = &self.session {
            request = request.bearer_auth(session);
        }
        let response = request
            .send()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?
            .to_vec();
        if status.is_success() {
            return Ok(Answer {
                bytes,
                content_type,
            });
        }
        let refusal: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let text = |key: &str| refusal[key].as_str().unwrap_or_default().to_string();
        Err(Error::Api {
            status: status.as_u16(),
            error: text("error"),
            message: text("message"),
        })
    }
}

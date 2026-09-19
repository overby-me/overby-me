//! The one outbound HTTP client (`docs/atproto-stack-decisions.md`, Rust server
//! libraries): a single pooled `reqwest::Client` on rustls, shared by the OAuth
//! client and the DID/handle resolvers.
//!
//! `atrium-oauth`'s own `DefaultHttpClient` enables `reqwest/default-tls`, which
//! links OpenSSL. It is switched off in `Cargo.toml`, and this takes its place.

use atrium_api::xrpc::HttpClient;
use atrium_api::xrpc::http::{Request, Response};
use std::time::Duration;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A caller-supplied PDS can stall forever; nothing outbound may hang a login.
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct RustlsHttpClient {
    client: reqwest::Client,
}

impl RustlsHttpClient {
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("wiki-appview/", env!("CARGO_PKG_VERSION")))
            .timeout(TIMEOUT)
            .build()?;
        Ok(Self { client })
    }
}

impl HttpClient for RustlsHttpClient {
    async fn send_http(&self, request: Request<Vec<u8>>) -> Result<Response<Vec<u8>>, BoxError> {
        let response = self.client.execute(request.try_into()?).await?;
        let mut builder = Response::builder().status(response.status());
        for (name, value) in response.headers() {
            builder = builder.header(name, value);
        }
        Ok(builder.body(response.bytes().await?.to_vec())?)
    }
}

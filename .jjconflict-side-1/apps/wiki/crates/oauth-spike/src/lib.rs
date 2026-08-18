//! Round-2 item 14: the THIN wrapper the stack decision mandates over
//! `atrium-oauth` (atproto-stack-decisions.md: "own a thin wrapper layer").
//! One file, one type: `WikiOAuth`. It builds the OAuth client (handle -> DID
//! -> PDS resolution, PAR, DPoP P-256, PKCE S256, DoH DNS-TXT handle
//! resolution, in-memory session/state stores) and exposes exactly the two
//! operations the AppView needs: begin login (returns the authorization URL to
//! redirect the member to) and, later, finish it from the callback.
//!
//! The point of the spike is to prove the 0.x crate supports the full
//! server-side flow against ARBITRARY member-chosen PDSes (not just
//! bsky.social). `begin_login` runs handle resolution + PAR against whatever
//! PDS the handle lives on; that is the load-bearing unknown. Completing the
//! token exchange needs a real browser redirect (a human), so the binary and
//! the ignored test drive `begin_login` and record how far the flow gets.

use atrium_identity::did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL};
use atrium_identity::handle::{
    AtprotoHandleResolver, AtprotoHandleResolverConfig, DohDnsTxtResolver, DohDnsTxtResolverConfig,
};
use atrium_oauth::store::session::MemorySessionStore;
use atrium_oauth::store::state::MemoryStateStore;
use atrium_oauth::{
    AtprotoLocalhostClientMetadata, AuthorizeOptions, DefaultHttpClient, KnownScope, OAuthClient,
    OAuthClientConfig, OAuthResolverConfig, Scope,
};
use std::sync::Arc;

type HttpClient = DefaultHttpClient;
type DidRes = CommonDidResolver<HttpClient>;
type HandleRes = AtprotoHandleResolver<DohDnsTxtResolver<HttpClient>, HttpClient>;
type Client = OAuthClient<MemoryStateStore, MemorySessionStore, DidRes, HandleRes>;

/// The wiki's atproto OAuth client. Construct once; `begin_login` per member.
pub struct WikiOAuth {
    client: Client,
}

impl WikiOAuth {
    /// Build the OAuth client. Uses the localhost client-metadata profile
    /// (a public client, so no client secret or JWKS) with the atproto +
    /// transitional-generic scopes the app needs, Cloudflare DoH for handle
    /// TXT resolution, and the default PLC directory for DID resolution.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let http_client = Arc::new(DefaultHttpClient::default());
        let config = OAuthClientConfig {
            client_metadata: AtprotoLocalhostClientMetadata {
                redirect_uris: Some(vec![String::from("http://127.0.0.1/callback")]),
                scopes: Some(vec![
                    Scope::Known(KnownScope::Atproto),
                    Scope::Known(KnownScope::TransitionGeneric),
                ]),
            },
            keys: None,
            resolver: OAuthResolverConfig {
                did_resolver: CommonDidResolver::new(CommonDidResolverConfig {
                    plc_directory_url: DEFAULT_PLC_DIRECTORY_URL.to_string(),
                    http_client: Arc::clone(&http_client),
                }),
                handle_resolver: AtprotoHandleResolver::new(AtprotoHandleResolverConfig {
                    dns_txt_resolver: DohDnsTxtResolver::new(DohDnsTxtResolverConfig {
                        service_url: String::from("https://cloudflare-dns.com/dns-query"),
                        http_client: Arc::clone(&http_client),
                    }),
                    http_client: Arc::clone(&http_client),
                }),
                authorization_server_metadata: Default::default(),
                protected_resource_metadata: Default::default(),
            },
            state_store: MemoryStateStore::default(),
            session_store: MemorySessionStore::default(),
        };
        Ok(Self {
            client: OAuthClient::new(config)?,
        })
    }

    /// Begin login for a member identified by their handle or PDS URL. Runs
    /// the full server-side pre-redirect flow (handle -> DID -> PDS resolution,
    /// PAR with a fresh DPoP key + PKCE challenge) against whatever PDS hosts
    /// the identity, and returns the authorization URL to redirect them to.
    pub async fn begin_login(
        &self,
        handle_or_pds: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = self
            .client
            .authorize(
                handle_or_pds,
                AuthorizeOptions {
                    scopes: vec![
                        Scope::Known(KnownScope::Atproto),
                        Scope::Known(KnownScope::TransitionGeneric),
                    ],
                    ..Default::default()
                },
            )
            .await?;
        Ok(url)
    }
}

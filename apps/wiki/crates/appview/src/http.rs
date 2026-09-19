//! The one outbound HTTP client (`docs/atproto-stack-decisions.md`, Rust server
//! libraries): a single pooled `reqwest::Client` on rustls, shared by the OAuth
//! client and the DID/handle resolvers.
//!
//! `atrium-oauth`'s own `DefaultHttpClient` enables `reqwest/default-tls`, which
//! links OpenSSL. It is switched off in `Cargo.toml`, and this takes its place.
//!
//! It is also the SSRF guard. `/login` is unauthenticated and fetches whatever
//! host the caller names (a handle, then the DID document and PDS it points
//! at), so every request made here is held to public addresses.

use atrium_api::xrpc::HttpClient;
use atrium_api::xrpc::http::{Request, Response};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// A caller-supplied PDS can stall forever; nothing outbound may hang a login.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Which hosts the client may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// https to public addresses only. What a deployed AppView uses.
    PublicOnly,
    /// Anything, so a dev instance can log in against a PDS on localhost.
    Any,
}

#[derive(Clone)]
pub struct RustlsHttpClient {
    client: reqwest::Client,
    reach: Reach,
}

impl RustlsHttpClient {
    pub fn new(reach: Reach) -> Result<Self, reqwest::Error> {
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("wiki-appview/", env!("CARGO_PKG_VERSION")))
            .timeout(TIMEOUT);
        if reach == Reach::PublicOnly {
            // A redirect would be followed without passing `refuse` again. The
            // resolver still vets where it leads, but an IP-literal would not
            // reach the resolver at all.
            builder = builder
                .dns_resolver(Arc::new(PublicOnlyResolver))
                .redirect(reqwest::redirect::Policy::none());
        }
        Ok(Self {
            client: builder.build()?,
            reach,
        })
    }
}

impl RustlsHttpClient {
    /// A request of the crate's own on the shared client, held to the same
    /// reach as everything else it sends.
    pub fn request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<reqwest::RequestBuilder, BoxError> {
        if self.reach == Reach::PublicOnly
            && let Some(why) = refuse(&url.parse()?)
        {
            return Err(format!("refused {url}: {why}").into());
        }
        Ok(self.client.request(method, url))
    }

    pub fn get(&self, url: &str) -> Result<reqwest::RequestBuilder, BoxError> {
        self.request(reqwest::Method::GET, url)
    }

    pub fn post(&self, url: &str) -> Result<reqwest::RequestBuilder, BoxError> {
        self.request(reqwest::Method::POST, url)
    }
}

impl HttpClient for RustlsHttpClient {
    async fn send_http(&self, request: Request<Vec<u8>>) -> Result<Response<Vec<u8>>, BoxError> {
        if self.reach == Reach::PublicOnly
            && let Some(why) = refuse(request.uri())
        {
            return Err(format!("refused {}: {why}", request.uri()).into());
        }
        let response = self.client.execute(request.try_into()?).await?;
        let mut builder = Response::builder().status(response.status());
        for (name, value) in response.headers() {
            builder = builder.header(name, value);
        }
        Ok(builder.body(response.bytes().await?.to_vec())?)
    }
}

/// Why a URL may not be fetched, judged without resolving it. A DNS name passes
/// here and is vetted by [`PublicOnlyResolver`] when it is connected to.
fn refuse(uri: &atrium_api::xrpc::http::Uri) -> Option<&'static str> {
    if uri.scheme_str() != Some("https") {
        return Some("not https");
    }
    let host = uri.host()?;
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    match literal.parse::<IpAddr>() {
        Ok(ip) if !is_public(ip) => Some("not a public address"),
        _ => None,
    }
}

/// Resolves a name and keeps only its public addresses. The connection is then
/// made to exactly these, so a name that re-resolves to an internal address
/// after the check (DNS rebinding) gains nothing.
struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let public: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| is_public(addr.ip()))
                .collect();
            if public.is_empty() {
                return Err(format!("{host} has no public address").into());
            }
            Ok(Box::new(public.into_iter()) as Addrs)
        })
    }
}

/// Whether an address is on the public internet. `IpAddr::is_global` is still
/// unstable, so the special-purpose ranges are spelled out (IANA registries
/// for IPv4 and IPv6 special-purpose addresses).
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (b & 0xc0) == 64) // 100.64/10, carrier-grade NAT
        || (a == 192 && b == 0 && c == 0) // 192.0.0/24, protocol assignments
        || (a == 198 && (b & 0xfe) == 18) // 198.18/15, benchmarking
        || a >= 240) // 240/4, reserved
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    // An address that carries an IPv4 one is as public as that one is.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let seg = ip.segments();
    if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let [hi, lo] = [seg[6], seg[7]];
        return is_public_v4(Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo)));
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7, unique local
        || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10, link local
        || (seg[0] == 0x2001 && seg[1] == 0x0db8)) // 2001:db8::/32, documentation
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("an ip address")
    }

    #[test]
    fn internal_addresses_are_not_public() {
        for internal in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::7f00:1",
        ] {
            assert!(!is_public(ip(internal)), "{internal} must not be reachable");
        }
    }

    #[test]
    fn ordinary_addresses_are_public() {
        for public in [
            "1.1.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.1",
            "198.17.255.255",
            "2606:4700:4700::1111",
            "::ffff:1.1.1.1",
            "64:ff9b::101:101",
        ] {
            assert!(is_public(ip(public)), "{public} should be reachable");
        }
    }

    /// Built from the host rather than written out, so the repository's link
    /// checker does not go and probe the test's URLs.
    fn refused(scheme: &str, host: &str) -> Option<&'static str> {
        let uri = format!("{scheme}://{host}/x").parse().expect("a uri");
        refuse(&uri)
    }

    #[test]
    fn only_https_to_a_public_host_passes() {
        assert_eq!(refused("https", "bsky.social"), None);
        assert_eq!(refused("https", "1.1.1.1"), None);
        assert_eq!(refused("http", "bsky.social"), Some("not https"));
        for internal in [
            "169.254.169.254",
            "127.0.0.1:8080",
            "[::1]",
            "[::ffff:10.0.0.1]",
        ] {
            assert_eq!(
                refused("https", internal),
                Some("not a public address"),
                "{internal}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_name_that_resolves_inward_has_no_address_to_connect_to() {
        let name: Name = "localhost".parse().expect("a dns name");
        let resolved = PublicOnlyResolver.resolve(name).await;
        assert!(
            resolved.is_err(),
            "localhost resolved to something connectable"
        );
    }

    /// The guard must not cost a real login anything. Run with
    /// `cargo test -p appview -- --ignored`.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "hits the live network"]
    async fn the_guarded_client_still_reaches_a_real_pds() {
        let client = RustlsHttpClient::new(Reach::PublicOnly).expect("client");
        let request = Request::builder()
            .uri("https://bsky.social/.well-known/oauth-authorization-server")
            .body(Vec::new())
            .expect("request");
        let response = client.send_http(request).await.expect("a public fetch");
        assert_eq!(response.status(), 200, "bsky.social metadata");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_guarded_client_will_not_fetch_an_internal_url() {
        let client = RustlsHttpClient::new(Reach::PublicOnly).expect("client");
        let request = Request::builder()
            .uri(format!("https://{}/latest/meta-data", "169.254.169.254"))
            .body(Vec::new())
            .expect("request");
        let error = client
            .send_http(request)
            .await
            .expect_err("an internal fetch went out");
        assert!(error.to_string().contains("refused"), "{error}");
    }
}

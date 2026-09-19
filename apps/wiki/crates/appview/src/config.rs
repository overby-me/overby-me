//! AppView configuration from the environment, mirroring the interim backend's
//! `Config::from_env` pattern. The stateful AppView reads its Turso path, bind
//! port, firehose endpoint, and observability sink here.

#[derive(Clone, Debug)]
pub struct Config {
    /// TCP port to bind (containers inject `$PORT`).
    pub port: u16,
    /// Turso/SQLite database file path (`:memory:` for tests and dev).
    pub db_path: String,
    /// Jetstream firehose endpoint (consumed by the firehose task; a stub for
    /// now, the actual consumer is a later kickoff item).
    pub firehose_url: String,
    /// BetterStack (Logtail) ingest host + token for structured server logs
    /// (the same sink the frontend and interim backend ship to). Empty token
    /// disables remote shipping.
    pub betterstack_host: String,
    pub betterstack_token: String,
    /// Off-node ballot replica-log path. When set, every committed cast is
    /// appended here and shipped to an independent node (the E2E-V integrity
    /// control); empty disables replication (dev/tests). See `crate::ballot`.
    pub ballot_replica_log: String,
    /// Where a browser reaches this AppView, without a trailing slash
    /// (`APPVIEW_PUBLIC_URL`). Empty means a loopback dev instance, which
    /// selects the OAuth loopback client profile.
    pub public_url: String,
    /// Browser origins that may call the API (CORS) and receive a login
    /// redirect (`APPVIEW_FRONTEND_ORIGINS`, comma-separated). Empty keeps the
    /// API same-origin.
    pub frontend_origins: Vec<String>,
}

impl Config {
    /// Whether a login may hand control back to `url`. An open redirect here
    /// would deliver a fresh login code to whoever crafted the link.
    pub fn allows_return(&self, url: &str) -> bool {
        if !url.bytes().all(|b| b.is_ascii_graphic()) {
            return false;
        }
        self.frontend_origins.iter().any(|origin| {
            url.strip_prefix(origin.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
        })
    }

    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        Config {
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            db_path: std::env::var("APPVIEW_DB").unwrap_or_else(|_| ":memory:".to_string()),
            firehose_url: std::env::var("JETSTREAM_URL")
                .unwrap_or_else(|_| "wss://jetstream2.us-east.bsky.network/subscribe".to_string()),
            betterstack_host: std::env::var("BETTERSTACK_INGEST_HOST")
                .unwrap_or_else(|_| "in.logs.betterstack.com".to_string()),
            betterstack_token: env("BETTERSTACK_SOURCE_TOKEN"),
            ballot_replica_log: env("BALLOT_REPLICA_LOG"),
            public_url: env("APPVIEW_PUBLIC_URL").trim_end_matches('/').to_string(),
            frontend_origins: parse_origins(&env("APPVIEW_FRONTEND_ORIGINS")),
        }
    }
}

fn parse_origins(list: &str) -> Vec<String> {
    list.split(',')
        .map(|o| o.trim().trim_end_matches('/').to_string())
        .filter(|o| !o.is_empty())
        .collect()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: 8080,
            db_path: ":memory:".to_string(),
            firehose_url: String::new(),
            betterstack_host: "in.logs.betterstack.com".to_string(),
            betterstack_token: String::new(),
            ballot_replica_log: String::new(),
            public_url: String::new(),
            frontend_origins: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(origins: &str) -> Config {
        Config {
            frontend_origins: parse_origins(origins),
            ..Config::default()
        }
    }

    #[test]
    fn a_return_url_must_sit_on_an_allowed_origin() {
        let c = config("https://wiki.example/, http://localhost:8080");
        for ok in [
            "https://wiki.example",
            "https://wiki.example/",
            "https://wiki.example/a/b?app=vote",
            "https://wiki.example?x=1",
            "http://localhost:8080/#top",
        ] {
            assert!(c.allows_return(ok), "{ok} should be allowed");
        }
        for bad in [
            "https://wiki.example.evil.example/",
            "https://wiki.example@evil.example/",
            "https://wiki.example\\@evil.example/",
            "https://wiki.example:8443/",
            "http://wiki.example/",
            "https://evil.example/?https://wiki.example/",
            "https://wiki.example/\r\nSet-Cookie: x=1",
            "",
        ] {
            assert!(!c.allows_return(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn no_configured_origin_allows_no_return() {
        assert!(!config("").allows_return("https://wiki.example/"));
    }
}

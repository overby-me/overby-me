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
}

impl Config {
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
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: 8080,
            db_path: ":memory:".to_string(),
            firehose_url: String::new(),
            betterstack_host: "in.logs.betterstack.com".to_string(),
            betterstack_token: String::new(),
        }
    }
}

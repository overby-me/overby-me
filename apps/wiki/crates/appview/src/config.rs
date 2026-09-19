//! AppView configuration from the environment, mirroring the interim backend's
//! `Config::from_env` pattern. The stateful AppView reads its Turso path, bind
//! port, firehose endpoint, and observability sink here.

/// A key or a token. Its `Debug` says nothing, so a `{:?}` of the `Config`
/// holding it, in an error path nobody thought about, cannot print it.
#[derive(Clone, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// The value itself, for the one place it is sent.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// TCP port to bind (containers inject `$PORT`).
    pub port: u16,
    /// Turso/SQLite database file path (`:memory:` for tests and dev).
    pub db_path: String,
    /// Jetstream firehose endpoint (consumed by the firehose task; a stub for
    /// now, the actual consumer is a later kickoff item).
    pub firehose_url: String,
    /// BetterStack (Logtail) ingest host + token: where `/log` forwards what the
    /// frontend ships (`crate::logs`). Empty token disables it. A host with a
    /// scheme is used as the whole URL, for a test sink.
    pub betterstack_host: String,
    pub betterstack_token: Secret,
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
    /// The PDS hosts whose word is taken that an account's email address is
    /// confirmed (`APPVIEW_TRUSTED_EMAIL_PDS`, comma-separated; a leading dot
    /// matches any host under it). Anyone can run a PDS, and one that lies about
    /// an address would walk its owner into that address's invitations.
    pub trusted_email_pds: Vec<String>,
    /// The VAPID (RFC 8292) application-server key for Web Push, as the interim
    /// has it: the 32-byte P-256 scalar and the uncompressed point, base64url
    /// (`VAPID_PRIVATE_KEY`, `VAPID_PUBLIC_KEY`). No private key, no push.
    pub vapid_private: Secret,
    pub vapid_public: String,
    /// The VAPID `sub` claim, a contact for the push service (`VAPID_SUBJECT`):
    /// a `mailto:` or `https:` URL. Unset, the AppView's own public URL.
    pub vapid_subject: String,
    /// Where the frontend is served (`APPVIEW_APP_ORIGIN`): its builds publish
    /// their debug symbols there. Unset, the first of `frontend_origins`.
    pub app_origin: String,
    /// Where uploaded files are kept (`APPVIEW_BLOB_DIR`). Unset, they go beside
    /// the database file, or to a temporary directory when that is in memory.
    pub blob_dir: String,
    /// The largest file accepted (`APPVIEW_MAX_BLOB_BYTES`).
    pub max_blob_bytes: u64,
    /// What one member, and one context, may store in all
    /// (`APPVIEW_MAX_MEMBER_BLOB_BYTES`, `APPVIEW_MAX_CONTEXT_BLOB_BYTES`).
    pub max_member_blob_bytes: u64,
    pub max_context_blob_bytes: u64,
    /// Keys the signed blob links (`APPVIEW_SECRET`). Unset, one is made on
    /// first start and kept beside the database file.
    pub secret: Secret,
    /// What a home made at start is called (`APPVIEW_SITE_NAME`). One loaded
    /// from the interim keeps its own name.
    pub site_name: String,
    /// A DID seated as an owner of the home at every start
    /// (`APPVIEW_SITE_OWNER`): the operator's way in, to a new site or to one
    /// none of whose owners can get in.
    pub site_owner: Option<String>,
}

/// Room for scanned minutes or a short video.
const DEFAULT_MAX_BLOB_BYTES: u64 = 64 * 1024 * 1024;

/// Far past what a member or a group has needed (the interim holds 322 Office
/// files in all), and short of what would fill a small host's disk unnoticed.
const DEFAULT_MAX_MEMBER_BLOB_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_MAX_CONTEXT_BLOB_BYTES: u64 = 20 * 1024 * 1024 * 1024;

const DEFAULT_SITE_NAME: &str = "Wiki";

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

    /// Where this AppView is reached: its public URL, or a dev instance's
    /// loopback address.
    pub fn base_url(&self) -> String {
        if self.public_url.is_empty() {
            format!("http://127.0.0.1:{}", self.port)
        } else {
            self.public_url.clone()
        }
    }

    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        let db_path = std::env::var("APPVIEW_DB").unwrap_or_else(|_| ":memory:".to_string());
        let secret = Secret::new(secret_for(&db_path, env("APPVIEW_SECRET")));
        let frontend_origins = parse_origins(&env("APPVIEW_FRONTEND_ORIGINS"));
        let public_url = env("APPVIEW_PUBLIC_URL").trim_end_matches('/').to_string();
        let vapid_subject = match env("VAPID_SUBJECT") {
            subject if subject.is_empty() => public_url.clone(),
            subject => subject,
        };
        Config {
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            blob_dir: match env("APPVIEW_BLOB_DIR") {
                dir if dir.is_empty() => blobs_beside(&db_path),
                dir => dir,
            },
            max_blob_bytes: env("APPVIEW_MAX_BLOB_BYTES")
                .parse()
                .unwrap_or(DEFAULT_MAX_BLOB_BYTES),
            max_member_blob_bytes: env("APPVIEW_MAX_MEMBER_BLOB_BYTES")
                .parse()
                .unwrap_or(DEFAULT_MAX_MEMBER_BLOB_BYTES),
            max_context_blob_bytes: env("APPVIEW_MAX_CONTEXT_BLOB_BYTES")
                .parse()
                .unwrap_or(DEFAULT_MAX_CONTEXT_BLOB_BYTES),
            secret,
            site_name: match env("APPVIEW_SITE_NAME") {
                name if name.trim().is_empty() => DEFAULT_SITE_NAME.to_string(),
                name => name.trim().to_string(),
            },
            site_owner: Some(env("APPVIEW_SITE_OWNER").trim().to_string())
                .filter(|did| did.starts_with("did:")),
            db_path,
            firehose_url: std::env::var("JETSTREAM_URL")
                .unwrap_or_else(|_| "wss://jetstream2.us-east.bsky.network/subscribe".to_string()),
            betterstack_host: std::env::var("BETTERSTACK_INGEST_HOST")
                .unwrap_or_else(|_| "in.logs.betterstack.com".to_string()),
            betterstack_token: Secret::new(env("BETTERSTACK_SOURCE_TOKEN")),
            ballot_replica_log: env("BALLOT_REPLICA_LOG"),
            public_url,
            trusted_email_pds: match env("APPVIEW_TRUSTED_EMAIL_PDS") {
                hosts if hosts.trim().is_empty() => default_trusted_email_pds(),
                hosts => hosts
                    .split(',')
                    .map(|h| h.trim().to_ascii_lowercase())
                    .filter(|h| !h.is_empty())
                    .collect(),
            },
            vapid_private: Secret::new(env("VAPID_PRIVATE_KEY")),
            vapid_public: env("VAPID_PUBLIC_KEY"),
            vapid_subject,
            app_origin: match env("APPVIEW_APP_ORIGIN") {
                origin if origin.is_empty() => {
                    frontend_origins.first().cloned().unwrap_or_default()
                }
                origin => origin.trim_end_matches('/').to_string(),
            },
            frontend_origins,
        }
    }
}

/// Bluesky's own: the entryway, and the hosts its accounts live on. It confirms
/// an address by mailing it, and is who nearly every member will sign in with.
fn default_trusted_email_pds() -> Vec<String> {
    vec!["bsky.social".to_string(), ".host.bsky.network".to_string()]
}

fn blobs_beside(db_path: &str) -> String {
    let dir = match std::path::Path::new(db_path).parent() {
        Some(dir) if db_path != ":memory:" => dir.to_path_buf(),
        _ => std::env::temp_dir().join(format!("appview-{}", std::process::id())),
    };
    dir.join("blobs").to_string_lossy().into_owned()
}

/// The configured secret, else the one kept beside the database, made now if
/// this is the first start. Beside the database because whoever reads that file
/// already holds every session in it.
fn secret_for(db_path: &str, configured: String) -> String {
    if !configured.is_empty() {
        return configured;
    }
    let fresh = crate::util::random_token(32);
    if db_path == ":memory:" {
        return fresh;
    }
    let file = format!("{db_path}.secret");
    if let Ok(kept) = std::fs::read_to_string(&file)
        && !kept.trim().is_empty()
    {
        return kept.trim().to_string();
    }
    if let Err(e) = write_private(&file, &fresh) {
        tracing::warn!("no signing secret kept at {file} ({e}): a restart will break blob links");
    }
    fresh
}

fn write_private(file: &str, secret: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(file)?.write_all(secret.as_bytes())
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
            betterstack_token: Secret::default(),
            ballot_replica_log: String::new(),
            public_url: String::new(),
            frontend_origins: Vec::new(),
            trusted_email_pds: default_trusted_email_pds(),
            vapid_private: Secret::default(),
            vapid_public: String::new(),
            vapid_subject: String::new(),
            app_origin: String::new(),
            blob_dir: blobs_beside(":memory:"),
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
            max_member_blob_bytes: DEFAULT_MAX_MEMBER_BLOB_BYTES,
            max_context_blob_bytes: DEFAULT_MAX_CONTEXT_BLOB_BYTES,
            secret: Secret::new(crate::util::random_token(32)),
            site_name: DEFAULT_SITE_NAME.to_string(),
            site_owner: None,
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
    fn a_secret_is_made_once_and_kept_beside_the_database() {
        let dir = std::env::temp_dir().join(format!("appview-{}", crate::util::random_token(8)));
        std::fs::create_dir_all(&dir).expect("dir");
        let db = dir.join("appview.db").to_string_lossy().into_owned();

        let first = secret_for(&db, String::new());
        assert_eq!(
            secret_for(&db, String::new()),
            first,
            "a restart made a new secret"
        );
        assert_eq!(secret_for(&db, "configured".to_string()), "configured");
        assert_ne!(
            secret_for(":memory:", String::new()),
            secret_for(":memory:", String::new())
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(format!("{db}.secret"))
                .expect("kept")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the secret is readable by others");
        }
    }

    #[test]
    fn a_config_can_be_printed_without_printing_its_keys() {
        let config = Config {
            secret: Secret::new("hunter2-signing-key"),
            betterstack_token: Secret::new("hunter2-log-token"),
            ..Config::default()
        };
        assert!(!format!("{config:?}").contains("hunter2"));
    }

    #[test]
    fn no_configured_origin_allows_no_return() {
        assert!(!config("").allows_return("https://wiki.example/"));
    }
}

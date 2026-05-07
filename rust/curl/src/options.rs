use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum TimeCond {
    IfModifiedSince(i64),   // unix timestamp
    IfUnmodifiedSince(i64), // unix timestamp
}

/// Per-URL option snapshot. These fields are reset by `--next` and must be
/// captured at the time each URL is added so that earlier URLs keep their
/// original values even after a `--next` reset.
#[derive(Clone, Debug, Default)]
pub(crate) struct PerUrlOptions {
    pub(crate) data: Option<Vec<u8>>,
    pub(crate) data_raw: bool,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) method: Option<String>,
    pub(crate) json: bool,
    pub(crate) form_fields: Vec<FormField>,
    pub(crate) upload_file: Option<PathBuf>,
    pub(crate) head: bool,
    pub(crate) get: bool,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub(crate) urls: Vec<String>,
    pub(crate) method: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) data: Option<Vec<u8>>,
    pub(crate) data_raw: bool,
    pub(crate) get: bool,
    pub(crate) form_fields: Vec<FormField>,
    pub(crate) outputs: Vec<PathBuf>,
    pub(crate) output_dir: Option<PathBuf>,
    pub(crate) remote_name: bool,
    pub(crate) remote_header_name: bool,
    pub(crate) globoff: bool,
    pub(crate) http09: bool,
    pub(crate) remove_on_error: bool,
    pub(crate) json: bool,
    pub(crate) etag_compare: Option<PathBuf>,
    pub(crate) etag_save: Option<PathBuf>,
    pub(crate) location: bool,
    pub(crate) location_trusted: bool,
    pub(crate) post301: bool,
    pub(crate) post302: bool,
    pub(crate) post303: bool,
    pub(crate) tr_encoding: bool,
    pub(crate) max_redirs: usize,
    pub(crate) proto_redir: Option<String>,
    pub(crate) max_filesize: Option<u64>,
    pub(crate) retry_max_time: Option<u64>,
    pub(crate) verbose: bool,
    pub(crate) silent: bool,
    pub(crate) show_error: bool,
    pub(crate) fail: bool,
    pub(crate) fail_with_body: bool,
    pub(crate) include_headers: bool,
    pub(crate) head: bool,
    pub(crate) user_agent: Option<String>,
    pub(crate) referer: Option<String>,
    pub(crate) auto_referer: bool, // -e ".*;auto" — update Referer to prior URL on redirect
    pub(crate) cookies: Vec<String>,
    pub(crate) cookie_jar: Option<PathBuf>,
    pub(crate) junk_session_cookies: bool,
    pub(crate) user: Option<String>,
    pub(crate) defer_auth: bool,
    pub(crate) connect_timeout: Option<Duration>,
    pub(crate) max_time: Option<Duration>,
    pub(crate) insecure: bool,
    pub(crate) compressed: bool,
    pub(crate) dump_header: Option<PathBuf>,
    pub(crate) write_out: Option<String>,
    pub(crate) retry: usize,
    pub(crate) retry_all_errors: bool,
    pub(crate) range: Option<String>,
    pub(crate) upload_file: Option<PathBuf>,
    pub(crate) http_version: Option<String>,
    pub(crate) no_keepalive: bool,
    pub(crate) path_as_is: bool,
    pub(crate) request_target: Option<String>,
    /// "host:port:addr" entries for --resolve.
    pub(crate) resolves: Vec<String>,
    pub(crate) cacert: Option<PathBuf>,
    pub(crate) cert: Option<PathBuf>,
    pub(crate) cert_key: Option<PathBuf>,
    pub(crate) resume_from: Option<String>, // -C / --continue-at
    pub(crate) time_cond: Option<TimeCond>,
    /// --stderr: redirect stderr to file; "-" means stdout.
    pub(crate) stderr_redirect: Option<PathBuf>,
    pub(crate) proxy: Option<String>,                // -x / --proxy
    pub(crate) proxy_user: Option<String>,           // --proxy-user "user:pass"
    pub(crate) proxy_tunnel: bool,                   // -p / --proxytunnel — force CONNECT tunnel
    pub(crate) proxy_1_0: bool,                      // --proxy1.0 — use HTTP/1.0 for CONNECT
    pub(crate) proxy_headers: Vec<(String, String)>, // --proxy-header — extra headers for proxy CONNECT or HTTP-via-proxy
    pub(crate) noproxy: Option<String>, // --noproxy (comma-separated hosts to skip proxy)
    pub(crate) fail_early: bool, // --fail-early — abort processing further URLs after the first error
    pub(crate) disallow_userinfo: bool, // --disallow-username-in-url — reject URLs that carry user:pass@
    pub(crate) create_dirs: bool,       // --create-dirs — mkdir -p the parent of any output path
    pub(crate) remote_time: bool, // -R / --remote-time — set output file mtime from Last-Modified
    pub(crate) cookie_engine: bool, // true when -b is used (enables cookie accumulation)
    pub(crate) memory_cookies: Vec<String>, // Netscape-format cookie lines accumulated from responses
    pub(crate) deleted_cookies: Vec<(String, String, String)>, // (domain, path, name) tuples of cookies deleted via Max-Age=0
    pub(crate) skip_existing: bool, // --skip-existing — skip transfer when output file exists
    pub(crate) no_clobber: bool,    // --no-clobber — write to file.N suffix when output exists
    pub(crate) raw: bool,           // --raw — disable content decoding
    pub(crate) ignore_content_length: bool, // --ignore-content-length — read until EOF
    pub(crate) max_filesize_str: Option<String>, // raw string for overflow detection
    /// Per-URL option snapshots. Index corresponds to `urls` index.
    pub(crate) per_url_opts: Vec<PerUrlOptions>,
}

#[derive(Clone, Debug)]
pub struct FormField {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) is_file: bool,
    pub(crate) content_type: Option<String>,
    pub(crate) filename: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            urls: Vec::new(),
            method: None,
            headers: Vec::new(),
            data: None,
            data_raw: false,
            get: false,
            form_fields: Vec::new(),
            outputs: Vec::new(),
            output_dir: None,
            remote_name: false,
            remote_header_name: false,
            globoff: false,
            http09: false,
            remove_on_error: false,
            json: false,
            etag_compare: None,
            etag_save: None,
            location: false,
            location_trusted: false,
            post301: false,
            post302: false,
            post303: false,
            tr_encoding: false,
            max_redirs: 50,
            proto_redir: None,
            max_filesize: None,
            retry_max_time: None,
            verbose: false,
            silent: false,
            show_error: false,
            fail: false,
            fail_with_body: false,
            include_headers: false,
            head: false,
            user_agent: None,
            referer: None,
            auto_referer: false,
            cookies: Vec::new(),
            cookie_jar: None,
            junk_session_cookies: false,
            user: None,
            defer_auth: false,
            connect_timeout: None,
            max_time: None,
            insecure: false,
            compressed: false,
            dump_header: None,
            write_out: None,
            retry: 0,
            retry_all_errors: false,
            range: None,
            upload_file: None,
            http_version: None,
            no_keepalive: false,
            path_as_is: false,
            request_target: None,
            resolves: Vec::new(),
            cacert: None,
            cert: None,
            cert_key: None,
            resume_from: None,
            time_cond: None,
            stderr_redirect: None,
            proxy: None,
            proxy_user: None,
            proxy_tunnel: false,
            proxy_1_0: false,
            proxy_headers: Vec::new(),
            noproxy: None,
            fail_early: false,
            disallow_userinfo: false,
            create_dirs: false,
            remote_time: false,
            cookie_engine: false,
            memory_cookies: Vec::new(),
            deleted_cookies: Vec::new(),
            skip_existing: false,
            no_clobber: false,
            raw: false,
            ignore_content_length: false,
            max_filesize_str: None,
            per_url_opts: Vec::new(),
        }
    }
}

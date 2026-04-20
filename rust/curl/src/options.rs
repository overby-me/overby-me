use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum TimeCond {
    IfModifiedSince(i64),   // unix timestamp
    IfUnmodifiedSince(i64), // unix timestamp
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
    pub(crate) max_redirs: usize,
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
    pub(crate) proxy: Option<String>,       // -x / --proxy
    pub(crate) proxy_user: Option<String>,  // --proxy-user "user:pass"
    pub(crate) proxy_tunnel: bool,          // -p / --proxytunnel — force CONNECT tunnel
    pub(crate) proxy_1_0: bool,             // --proxy1.0 — use HTTP/1.0 for CONNECT
    pub(crate) cookie_engine: bool,         // true when -b is used (enables cookie accumulation)
    pub(crate) memory_cookies: Vec<String>, // Netscape-format cookie lines accumulated from responses
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
            max_redirs: 50,
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
            cookie_engine: false,
            memory_cookies: Vec::new(),
        }
    }
}

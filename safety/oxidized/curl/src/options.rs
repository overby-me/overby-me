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
    pub(crate) connect_tos: Vec<String>,
    pub(crate) resolves: Vec<String>,
    pub(crate) no_basic: bool,
    pub(crate) user: Option<String>,
    pub(crate) dump_header: Option<PathBuf>,
    pub(crate) proxy: Option<String>,
    pub(crate) proxy_user: Option<String>,
    pub(crate) etag_save: Option<PathBuf>,
    pub(crate) etag_compare: Option<PathBuf>,
    pub(crate) include_headers: bool,
    /// True when this URL is the first transfer of its operation group
    /// (i.e. URL index 0, or the first URL after a --next). Used by `-D`
    /// to decide whether to truncate or append the dump-header file
    /// (test 3030 for append-within-group, test 3029 for truncate-on-next).
    pub(crate) first_in_group: bool,
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
    /// Parallel to `outputs`: `true` when the slot came from `--out-null`
    /// (which discards the body but, under `--include`, still writes the
    /// headers to stdout — test 756); `false` for explicit `-o`.
    pub(crate) outputs_null: Vec<bool>,
    /// When `--digest` triggers a 401 retry, the computed Digest header value
    /// (everything after `Authorization: `). Set during perform(); cleared
    /// across URLs.
    pub(crate) digest_authorization: Option<String>,
    /// Stored Digest challenge state from a prior 401. When set, redirects
    /// rebuild the Authorization header with the new URI and increment the
    /// nonce counter (test 1286, RFC 2617 §3.3).
    pub(crate) digest_challenge_state: Option<String>,
    /// Monotonic nonce counter (nc) used with qop=auth. Increments per
    /// request that sends Authorization: Digest.
    pub(crate) digest_nc: u32,
    /// Same as `digest_authorization` but for `Proxy-Authorization:`. Set
    /// after a 407 with a Digest challenge.
    pub(crate) proxy_digest_authorization: Option<String>,
    /// Proxy-Authorization for the CONNECT request specifically. Distinct
    /// from `proxy_digest_authorization` because the CONNECT line uses
    /// `host:port` as the uri in the Digest computation, while the relayed
    /// request uses its own path.
    pub(crate) connect_proxy_digest_authorization: Option<String>,
    /// `--digest`/`--ntlm`/`--negotiate` (but not `--anyauth`): probe with an
    /// empty body for PUT/POST so the body isn't wasted on the 401 challenge
    /// (test 88 vs test 156).
    pub(crate) auth_probe_empty_upload: bool,
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
    pub(crate) proto_default: Option<String>,
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
    /// True when `-u`/`--user` was explicitly passed on the CLI. URL
    /// userinfo only overrides this on cross-host redirects when the user
    /// did NOT supply `-u` (test 979 — `-u` always wins).
    pub(crate) user_from_cli: bool,
    pub(crate) defer_auth: bool,
    pub(crate) connect_timeout: Option<Duration>,
    pub(crate) max_time: Option<Duration>,
    pub(crate) insecure: bool,
    pub(crate) compressed: bool,
    /// --haproxy-protocol: prefix the HTTP request connection with the
    /// PROXY TCP4/TCP6 header (test 3028).
    pub(crate) haproxy_protocol: bool,
    /// --haproxy-clientip: override the source IP in the PROXY header.
    pub(crate) haproxy_clientip: Option<String>,
    /// --suppress-connect-headers: hide proxy CONNECT response headers from
    /// `--include` and `--dump-header` output (test 1288). They still count
    /// toward `%{size_header}`.
    pub(crate) suppress_connect_headers: bool,
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
    /// "HOST1:PORT1:HOST2:PORT2" entries for --connect-to.
    pub(crate) connect_tos: Vec<String>,
    pub(crate) no_basic: bool,
    pub(crate) oauth2_bearer: Option<String>,
    pub(crate) cacert: Option<PathBuf>,
    pub(crate) cert: Option<PathBuf>,
    pub(crate) cert_key: Option<PathBuf>,
    /// --tls-max <version>: cap TLS to this protocol version ("1.2", "1.3").
    pub(crate) tls_max: Option<String>,
    /// --tlsv1.x: minimum TLS version requested ("1.0", "1.1", "1.2", "1.3").
    /// Only tracked for --libcurl emission (rustls picks its own min).
    pub(crate) tlsv1_min: Option<String>,
    /// --proxy-tlsv1: emit CURLOPT_PROXY_SSLVERSION = CURL_SSLVERSION_TLSv1.
    pub(crate) proxy_tlsv1: bool,
    /// --proto arg as given (for --libcurl emitter).
    pub(crate) proto_arg: Option<String>,
    /// --basic was explicitly passed (for --libcurl emitter — emits CURLOPT_HTTPAUTH).
    pub(crate) basic_explicit: bool,
    /// --hsts <file>: load HSTS DB; upgrade http:// → https:// for matching hosts.
    pub(crate) hsts_file: Option<PathBuf>,
    /// --ipfs-gateway URL: HTTP(S) gateway used to dereference ipfs:// and ipns:// URLs.
    pub(crate) ipfs_gateway: Option<String>,
    /// --form-escape: backslash-escape `"`/`\r`/`\n` in -F filenames instead
    /// of percent-encoding them (test 1186, 1189).
    pub(crate) form_escape: bool,
    /// --xattr: write the URL and Content-Type to extended attributes on the
    /// output file. With `CURL_FAKE_XATTR=1` (Debug-only env), the operations
    /// are echoed to stdout instead of actually setting attrs (tests 687, 688).
    pub(crate) xattr: bool,
    /// --alt-svc <file>: load Alt-Svc cache from `<file>`. When non-None, the
    /// file is loaded and entries are used to redirect (origin_host, origin_port)
    /// requests to (alt_host, alt_port) at the TCP layer while keeping the
    /// original Host header (tests 412, 413, 437, 438).
    pub(crate) alt_svc_file: Option<PathBuf>,
    /// `<host>:<port>` value to emit as `Alt-Used:` on the next request when
    /// alt-svc routed it (test 412). Cleared after one use.
    pub(crate) alt_used: Option<String>,
    /// Warnings collected during argument parsing that should be emitted on
    /// stderr AFTER any `--stderr <file>` redirection (test 1268).
    pub(crate) deferred_warnings: Vec<String>,
    /// --unix-socket PATH: connect via a Unix domain socket at PATH instead
    /// of resolving the URL's host via DNS (tests 1435, 1436).
    pub(crate) unix_socket: Option<PathBuf>,
    /// True when `--ntlm` (or `--anyauth` falling back to NTLM after seeing the
    /// server's challenge). The first request sends a Type 1 message and the
    /// 401 handler parses the Type 2 and resends with the Type 3 response.
    pub(crate) ntlm: bool,
    /// True when `--anyauth`. After a redirect we re-probe and reset the
    /// runtime `ntlm` flag so the new origin can advertise different auth
    /// (test 90).
    pub(crate) anyauth: bool,
    /// Pre-computed `NTLM <base64-type3>` Authorization header value, set
    /// after a 401 carrying a Type 2 challenge so the retry sends Type 3.
    pub(crate) ntlm_authorization: Option<String>,
    /// True once a Type 3 has been sent and the site accepted it — the
    /// connection is NTLM-authenticated and subsequent requests on the same
    /// pooled connection must NOT carry Type 1 again (test 1100).
    pub(crate) ntlm_done: bool,
    /// --libcurl <file>: write a C code template using libcurl that
    /// reproduces this curl command (tests 1400-1481).
    pub(crate) libcurl_file: Option<PathBuf>,
    /// True when `--proxy-ntlm` (or `--proxy-anyauth` picked NTLM after seeing
    /// the proxy challenge). The first request through an HTTP proxy sends a
    /// Type 1 in `Proxy-Authorization:`; the 407 reply carries a Type 2 and
    /// the retry sends Type 3.
    pub(crate) proxy_ntlm: bool,
    /// Pre-computed `NTLM <base64-type3>` value for `Proxy-Authorization:`,
    /// set after a 407 carrying the proxy Type 2 challenge.
    pub(crate) proxy_ntlm_authorization: Option<String>,
    /// True once a Type 3 message has been sent on this connection and the
    /// proxy accepted it — subsequent requests must NOT carry Type 1 again
    /// (test 169).
    pub(crate) proxy_ntlm_done: bool,
    pub(crate) resume_from: Option<String>, // -C / --continue-at
    pub(crate) time_cond: Option<TimeCond>,
    /// --stderr: redirect stderr to file; "-" means stdout.
    pub(crate) stderr_redirect: Option<PathBuf>,
    pub(crate) proxy: Option<String>,      // -x / --proxy
    pub(crate) proxy_user: Option<String>, // --proxy-user "user:pass"
    /// `--proxy-anyauth` / `--proxy-digest` / `--proxy-ntlm` /
    /// `--proxy-negotiate`: defer proxy auth until a 407 challenge.
    pub(crate) defer_proxy_auth: bool,
    /// 0 = off, 1 = --netrc (required), 2 = --netrc-optional.
    pub(crate) netrc_mode: u8,
    pub(crate) netrc_file: Option<PathBuf>,
    pub(crate) proxy_tunnel: bool, // -p / --proxytunnel — force CONNECT tunnel
    pub(crate) proxy_1_0: bool,    // --proxy1.0 — use HTTP/1.0 for CONNECT
    pub(crate) proxy_headers: Vec<(String, String)>, // --proxy-header — extra headers for proxy CONNECT or HTTP-via-proxy
    pub(crate) noproxy: Option<String>, // --noproxy (comma-separated hosts to skip proxy)
    pub(crate) fail_early: bool, // --fail-early — abort processing further URLs after the first error
    pub(crate) disallow_userinfo: bool, // --disallow-username-in-url — reject URLs that carry user:pass@
    pub(crate) create_dirs: bool,       // --create-dirs — mkdir -p the parent of any output path
    pub(crate) remote_time: bool, // -R / --remote-time — set output file mtime from Last-Modified
    pub(crate) cookie_engine: bool, // true when -b is used (enables cookie accumulation)
    pub(crate) memory_cookies: Vec<String>, // Netscape-format cookie lines accumulated from responses
    pub(crate) deleted_cookies: Vec<(String, String, String)>, // (domain, path, name) tuples of cookies deleted via Max-Age=0
    pub(crate) progress_bar: bool, // -# / --progress-bar — emit a final fill-bar line on stderr
    pub(crate) skip_existing: bool, // --skip-existing — skip transfer when output file exists
    pub(crate) no_clobber: bool,   // --no-clobber — write to file.N suffix when output exists
    pub(crate) raw: bool,          // --raw — disable content decoding
    pub(crate) ignore_content_length: bool, // --ignore-content-length — read until EOF
    pub(crate) max_filesize_str: Option<String>, // raw string for overflow detection
    /// `--variable name=value` / `name@file` / `%ENV[=default]`. Stored
    /// in declaration order so later assignments override earlier ones.
    /// Values are bytes so binary file content survives round-tripping.
    pub(crate) variables: Vec<(String, Vec<u8>)>,
    /// `--url-query` items. Each is the already-encoded `name=value` (or
    /// just `value`) string. They are joined with `&` and appended to the
    /// URL's query string at request time (test 1221).
    pub(crate) url_queries: Vec<String>,
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
    /// Additional files (path, type, filename) when -F used comma-separated
    /// paths: `-F file=@a,b;type=t,c`. When non-empty, the field's outer part
    /// is `multipart/mixed` and each file (including the primary in
    /// `value`/`content_type`/`filename`) becomes an inner attachment.
    pub(crate) extra_files: Vec<(String, Option<String>, Option<String>)>,
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
            outputs_null: Vec::new(),
            digest_authorization: None,
            digest_challenge_state: None,
            digest_nc: 0,
            proxy_digest_authorization: None,
            connect_proxy_digest_authorization: None,
            auth_probe_empty_upload: false,
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
            proto_default: None,
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
            user_from_cli: false,
            defer_auth: false,
            connect_timeout: None,
            max_time: None,
            insecure: false,
            compressed: false,
            haproxy_protocol: false,
            haproxy_clientip: None,
            suppress_connect_headers: false,
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
            connect_tos: Vec::new(),
            no_basic: false,
            oauth2_bearer: None,
            cacert: None,
            cert: None,
            cert_key: None,
            tls_max: None,
            tlsv1_min: None,
            proxy_tlsv1: false,
            proto_arg: None,
            basic_explicit: false,
            hsts_file: None,
            ipfs_gateway: None,
            form_escape: false,
            xattr: false,
            alt_svc_file: None,
            alt_used: None,
            deferred_warnings: Vec::new(),
            unix_socket: None,
            ntlm: false,
            anyauth: false,
            ntlm_authorization: None,
            ntlm_done: false,
            libcurl_file: None,
            proxy_ntlm: false,
            proxy_ntlm_authorization: None,
            proxy_ntlm_done: false,
            resume_from: None,
            time_cond: None,
            stderr_redirect: None,
            proxy: None,
            proxy_user: None,
            defer_proxy_auth: false,
            netrc_mode: 0,
            netrc_file: None,
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
            progress_bar: false,
            skip_existing: false,
            no_clobber: false,
            raw: false,
            ignore_content_length: false,
            max_filesize_str: None,
            variables: Vec::new(),
            url_queries: Vec::new(),
            per_url_opts: Vec::new(),
        }
    }
}
